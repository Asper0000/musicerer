use super::scheduler::{self, Job, Outcome};
use super::{
    audio, detect_source, emit, log, process, soundcloud_fallback, tagging,
    tools::{self, Tool}, utils, Inbox, LogLevel,
    Source, TrackResult, WorkerEvent,
};
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

/// Total yt-dlp fragment connections we are willing to open across all workers.
const FRAGMENT_BUDGET: usize = 20;
const MAX_FRAGMENTS_PER_JOB: usize = 10;
const MIN_FRAGMENTS_PER_JOB: usize = 2;

/// Everything a download needs that is shared between parallel jobs.
struct DownloadCtx {
    output_dir: PathBuf,
    cookies: Option<PathBuf>,
    inbox: Inbox,
    stop_flag: Arc<AtomicBool>,
    fragments: usize,
}

pub fn spawn_worker(
    items: Vec<(u64, String)>,
    output_dir: PathBuf,
    cookies: Option<PathBuf>,
    inbox: Inbox,
    stop_flag: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        run_queue(items, output_dir, cookies, inbox, stop_flag);
    });
}

fn run_queue(
    items: Vec<(u64, String)>,
    output_dir: PathBuf,
    cookies: Option<PathBuf>,
    inbox: Inbox,
    stop_flag: Arc<AtomicBool>,
) {
    let total = items.len();

    if let Err(err) = fs::create_dir_all(&output_dir) {
        log(&inbox, LogLevel::Error, format!("Cannot create output folder: {err}"));
        emit(&inbox, WorkerEvent::QueueFinished { succeeded: 0, failed: total, unavailable: 0, drm: 0, stopped: false });
        return;
    }

    let workers = scheduler::auto_worker_count().min(total.max(1));
    log(
        &inbox,
        LogLevel::Info,
        format!("Downloading {total} track(s), up to {workers} at a time."),
    );

    // Share the connection budget so N workers don't each open 10 fragment streams.
    let fragments = (FRAGMENT_BUDGET / workers).clamp(MIN_FRAGMENTS_PER_JOB, MAX_FRAGMENTS_PER_JOB);
    let ctx = DownloadCtx { output_dir, cookies, inbox, stop_flag, fragments };

    let jobs = items
        .into_iter()
        .enumerate()
        .map(|(i, (id, url))| Job::new(id, url, format!("{}/{}", i + 1, total)))
        .collect();

    let tally = scheduler::run_all(
        jobs,
        workers,
        &ctx.stop_flag,
        |note| log(&ctx.inbox, LogLevel::Warn, note),
        |job| run_job(&ctx, job),
    );

    emit(
        &ctx.inbox,
        WorkerEvent::QueueFinished {
            succeeded: tally.succeeded,
            failed: tally.failed,
            unavailable: tally.unavailable,
            drm: tally.drm,
            stopped: tally.stopped,
        },
    );
}

/// Runs one job and reports its lifecycle to the UI. A rate-limited attempt is not
/// reported as finished; the scheduler will retry it.
fn run_job(ctx: &DownloadCtx, job: &Job) -> Outcome {
    emit(&ctx.inbox, WorkerEvent::TrackStarted { id: job.id });
    tlog(ctx, job, LogLevel::Info, if job.attempts == 0 { "Started." } else { "Retrying…" });

    let result = match download_one(ctx, job) {
        Outcome::RateLimited if job.can_retry() => {
            tlog(ctx, job, LogLevel::Warn, "Rate limited by the server; will retry shortly.");
            return Outcome::RateLimited;
        }
        Outcome::RetryWithFallback if job.can_retry() && job.fallback_attempt == 0 => {
            tlog(ctx, job, LogLevel::Warn, "HTTP 403; retrying with the alternate YouTube player client.");
            return Outcome::RetryWithFallback;
        }
        Outcome::RateLimited | Outcome::RetryWithFallback => TrackResult::Error,
        Outcome::Finished(result) => result,
    };

    match result {
        TrackResult::Ok => tlog(ctx, job, LogLevel::Info, "Completed."),
        TrackResult::Drm => tlog(
            ctx,
            job,
            LogLevel::Warn,
            "DRM-protected - skipped. yt-dlp does not bypass DRM.",
        ),
        TrackResult::Unavailable => tlog(ctx, job, LogLevel::Warn, "YouTube reports this video as unavailable; skipped."),
        TrackResult::Error => tlog(ctx, job, LogLevel::Error, "Failed."),
        TrackResult::Stopped => {}
    }

    emit(&ctx.inbox, WorkerEvent::TrackFinished { id: job.id, result });
    Outcome::Finished(result)
}

/// Log line tagged with the job label so interleaved parallel output stays readable.
fn tlog(ctx: &DownloadCtx, job: &Job, level: LogLevel, text: impl AsRef<str>) {
    log(&ctx.inbox, level, format!("[{}] {}", job.label, text.as_ref()));
}

fn is_rate_limited(error_text: &str) -> bool {
    error_text.contains("429")
        || error_text.contains("too many requests")
        || error_text.contains("rate limit")
        || error_text.contains("rate-limit")
}

fn is_video_unavailable(error_text: &str) -> bool {
    error_text.contains("video unavailable")
        || error_text.contains("this video is unavailable")
}

fn is_http_403(error_text: &str) -> bool {
    error_text.contains("http error 403") || error_text.contains("http 403")
}

fn download_one(ctx: &DownloadCtx, job: &Job) -> Outcome {
    let fin = Outcome::Finished;
    let url = job.url.as_str();
    let source = detect_source(url);

    let temp = match tempfile::Builder::new()
        .prefix("musicerer-tmp-")
        .tempdir_in(&ctx.output_dir)
    {
        Ok(dir) => dir,
        Err(err) => {
            tlog(ctx, job, LogLevel::Error, format!("Cannot create temp folder: {err}"));
            return fin(TrackResult::Error);
        }
    };

    let output_template = temp.path().join("%(title)s.%(ext)s");
    let mut cmd = tools::command(Tool::YtDlp);

    cmd.arg("--newline")
        .arg("--no-playlist")
        .arg("--no-color")
        .arg("--write-info-json")
        .arg("--write-thumbnail")
        .arg("--convert-thumbnails").arg("jpg")
        .arg("--extract-audio")
        .arg("--audio-format").arg("m4a")
        .arg("--retries").arg("20")
        .arg("--fragment-retries").arg("20")
        .arg("-o").arg(&output_template);

    tools::add_ffmpeg_location(&mut cmd);

    match source {
        Source::YouTube => {
            let player_client = if job.fallback_attempt > 0 { "android" } else { "default,web_embedded,tv" };
            cmd.arg("--format").arg("bestaudio[ext=m4a]/bestaudio/best")
                .arg("--audio-quality").arg("256K")
                .arg("--js-runtimes").arg("deno")
                .arg("--remote-components").arg("ejs:github")
                .arg("--extractor-args").arg(format!("youtube:player_client={player_client}"))
                .arg("--concurrent-fragments").arg(ctx.fragments.to_string());
        }
        Source::SoundCloud => {
            cmd.arg("--format").arg("bestaudio[ext=m4a]/bestaudio/best");
        }
        Source::Unsupported => {
            tlog(ctx, job, LogLevel::Error, "Unsupported URL.");
            return fin(TrackResult::Error);
        }
    }

    if let Some(cookies) = &ctx.cookies { cmd.arg("--cookies").arg(cookies); }

    cmd.arg(url).stdout(Stdio::null()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            tlog(ctx, job, LogLevel::Error, format!("Could not start yt-dlp (missing from the tools folder and PATH?): {err}"));
            return fin(TrackResult::Error);
        }
    };

    let Some(stderr) = child.stderr.take() else {
        tlog(ctx, job, LogLevel::Error, "yt-dlp produced no stderr pipe.");
        return fin(TrackResult::Error);
    };

    let reader = BufReader::new(stderr);
    let mut errors = VecDeque::with_capacity(40);
    let mut was_stopped = false;

    for line in reader.lines().map_while(Result::ok) {
        if ctx.stop_flag.load(Ordering::Relaxed) {
            was_stopped = true;
            let _ = child.kill();
            break;
        }

        let clean = utils::strip_ansi(&line);
        if clean.trim().is_empty() { continue; }

        if let Some(rest) = clean.strip_prefix("[download] Destination: ") {
            if let Some(stem) = Path::new(rest).file_stem() {
                emit(&ctx.inbox, WorkerEvent::TrackTitle { id: job.id, title: stem.to_string_lossy().to_string() });
            }
            continue;
        }

        if let Some(value) = utils::parse_percent(&clean) {
            emit(&ctx.inbox, WorkerEvent::Progress { id: job.id, value });
            continue;
        }
        if clean.contains("[download]") { continue; }

        let lower = clean.to_ascii_lowercase();
        if lower.contains("error") || lower.contains("drm") || lower.contains("protected") {
            errors.push_back(clean.clone());
            while errors.len() > 40 { errors.pop_front(); }
        }
        let level = if source == Source::YouTube
            && (is_video_unavailable(&lower)
                || (is_http_403(&lower) && job.fallback_attempt == 0))
        {
            LogLevel::Warn
        } else if lower.contains("error") {
            LogLevel::Error
        } else if lower.contains("warning") {
            LogLevel::Warn
        } else {
            LogLevel::Info
        };
        tlog(ctx, job, level, clean);
    }

    let status = child.wait();
    if was_stopped || ctx.stop_flag.load(Ordering::Relaxed) {
        return fin(TrackResult::Stopped);
    }

    let error_text = errors.iter().cloned().collect::<Vec<_>>().join("\n").to_ascii_lowercase();
    if let Ok(s) = status {
        if !s.success() {
            if source == Source::YouTube && is_video_unavailable(&error_text) {
                return fin(TrackResult::Unavailable);
            }
            if is_rate_limited(&error_text) {
                return Outcome::RateLimited;
            }
            if source == Source::YouTube && is_http_403(&error_text) && job.fallback_attempt == 0 {
                return Outcome::RetryWithFallback;
            }
            if error_text.contains("drm") || error_text.contains("protected") {
                if let Err(r) = soundcloud_fallback::handle_drm(url, source, temp.path(), &ctx.inbox) {
                    return fin(r);
                }
            } else {
                return fin(TrackResult::Error);
            }
        }
    }

    let Some(audio_path) = audio::find_audio(temp.path()) else {
        if source == Source::YouTube && is_video_unavailable(&error_text) {
            return fin(TrackResult::Unavailable);
        }
        tlog(ctx, job, LogLevel::Error, "yt-dlp finished but no audio file was found.");
        return fin(TrackResult::Error);
    };

    // --- Go+ Preview Detection ---
    if source == Source::SoundCloud {
        if let Some(ffprobe) = tools::find(Tool::Ffprobe) {
            if let Ok(duration) = get_audio_duration(&ffprobe, &audio_path) {
                if duration < 45.0 {
                    tlog(ctx, job, LogLevel::Error, "SoundCloud Go+ track detected (preview only). Failing download due to Go+ protection.");
                    return fin(TrackResult::Error);
                }
            }
        }
    }

    let title = tagging::tag_audio(&audio_path, url, &ctx.inbox, job.id, &job.label);

    match utils::move_into_dir(&audio_path, &ctx.output_dir, &utils::sanitize_filename(&title)) {
        Ok(final_path) => {
            tlog(ctx, job, LogLevel::Info, format!("Saved: {}", final_path.display()));
            fin(TrackResult::Ok)
        }
        Err(err) => {
            tlog(ctx, job, LogLevel::Error, format!("Could not move final file: {err}"));
            fin(TrackResult::Error)
        }
    }
}

fn get_audio_duration(ffprobe: &Path, path: &Path) -> Result<f64, String> {
    let output = process::hidden_command(ffprobe)
        .arg("-v").arg("error")
        .arg("-show_entries").arg("format=duration")
        .arg("-of").arg("default=noprint_wrappers=1:nokey=1")
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() { return Err("ffprobe failed".into()); }
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim().parse::<f64>().map_err(|_| "could not parse duration".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_rate_limit_signatures() {
        assert!(is_rate_limited("error: unable to download: http error 429: too many requests"));
        assert!(is_rate_limited("error: rate-limited by youtube"));
        assert!(!is_rate_limited("error: video unavailable"));
        assert!(!is_rate_limited("error: drm protected"));
    }

    #[test]
    fn classifies_unavailable_and_403_errors() {
        assert!(is_video_unavailable("ERROR: [youtube] abc: Video unavailable"));
        assert!(!is_video_unavailable("ERROR: HTTP Error 403: Forbidden"));
        assert!(is_http_403("ERROR: unable to download video data: HTTP Error 403: Forbidden"));
        assert!(!is_http_403("ERROR: [youtube] abc: Video unavailable"));
    }

    #[test]
    fn fragment_budget_scales_with_workers() {
        let frag = |w: usize| (FRAGMENT_BUDGET / w).clamp(MIN_FRAGMENTS_PER_JOB, MAX_FRAGMENTS_PER_JOB);
        assert_eq!(frag(1), 10);
        assert_eq!(frag(2), 10);
        assert_eq!(frag(4), 5);
        assert_eq!(frag(6), 3);
    }
}
