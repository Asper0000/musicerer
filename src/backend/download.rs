use super::process;
use super::scheduler::{self, Job, Outcome};
use super::spotify::SpotifyMeta;
use super::{
    audio, detect_source, emit, log, soundcloud_fallback, tagging,
    tools::{self, Tool},
    utils, Inbox, LogLevel, Source, TrackResult, WorkerEvent,
};
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

const FRAGMENT_BUDGET: usize = 20;
const MAX_FRAGMENTS_PER_JOB: usize = 10;
const MIN_FRAGMENTS_PER_JOB: usize = 2;
const ERROR_HISTORY: usize = 40;

pub type QueueJob = (u64, String, Option<SpotifyMeta>, Option<String>);

struct DownloadCtx {
    output_dir: PathBuf,
    cookies: Option<PathBuf>,
    inbox: Inbox,
    stop_flag: Arc<AtomicBool>,
    fragments: usize,
    skip_existing: bool,
}

pub fn spawn_worker(
    items: Vec<QueueJob>,
    output_dir: PathBuf,
    cookies: Option<PathBuf>,
    inbox: Inbox,
    stop_flag: Arc<AtomicBool>,
    skip_existing: bool,
) {
    thread::spawn(move || {
        run_queue(items, output_dir, cookies, inbox, stop_flag, skip_existing);
    });
}

fn run_queue(
    items: Vec<QueueJob>,
    output_dir: PathBuf,
    cookies: Option<PathBuf>,
    inbox: Inbox,
    stop_flag: Arc<AtomicBool>,
    skip_existing: bool,
) {
    let total = items.len();
    if let Err(err) = fs::create_dir_all(&output_dir) {
        log(
            &inbox,
            LogLevel::Error,
            format!("Cannot create output folder: {err}"),
        );
        emit(
            &inbox,
            WorkerEvent::QueueFinished {
                succeeded: 0,
                failed: total,
                unavailable: 0,
                drm: 0,
                skipped: 0,
                stopped: false,
            },
        );
        return;
    }
    let workers = scheduler::auto_worker_count().min(total.max(1));
    log(
        &inbox,
        LogLevel::Info,
        format!("Downloading {total} track(s), up to {workers} at a time."),
    );
    let fragments = (FRAGMENT_BUDGET / workers).clamp(MIN_FRAGMENTS_PER_JOB, MAX_FRAGMENTS_PER_JOB);
    let ctx = DownloadCtx {
        output_dir,
        cookies,
        inbox,
        stop_flag,
        fragments,
        skip_existing,
    };

    let jobs = items
        .into_iter()
        .enumerate()
        .map(|(i, (id, url, spotify, folder))| {
            let mut job = Job::new(id, url, format!("{}/{}", i + 1, total));
            job.spotify = spotify;
            job.folder = folder;
            job
        })
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
            skipped: tally.skipped,
            stopped: tally.stopped,
        },
    );
}

fn run_job(ctx: &DownloadCtx, job: &Job) -> Outcome {
    emit(&ctx.inbox, WorkerEvent::TrackStarted { id: job.id });
    tlog(
        ctx,
        job,
        LogLevel::Info,
        if job.attempts == 0 {
            "Started."
        } else {
            "Retrying\u{2026}"
        },
    );

    let result = match download_one(ctx, job) {
        Outcome::RateLimited if job.can_retry() => {
            tlog(
                ctx,
                job,
                LogLevel::Warn,
                "Rate limited by the server; will retry shortly.",
            );
            return Outcome::RateLimited;
        }
        Outcome::RetryWithFallback if job.can_retry() && job.fallback_attempt == 0 => {
            tlog(
                ctx,
                job,
                LogLevel::Warn,
                "HTTP 403; retrying with the alternate YouTube player client.",
            );
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
        TrackResult::Unavailable => tlog(
            ctx,
            job,
            LogLevel::Warn,
            "YouTube reports this video as unavailable; skipped.",
        ),
        TrackResult::Skipped => {}
        TrackResult::Error => tlog(ctx, job, LogLevel::Error, "Failed."),
        TrackResult::Stopped => {}
    }
    emit(
        &ctx.inbox,
        WorkerEvent::TrackFinished { id: job.id, result },
    );
    Outcome::Finished(result)
}

fn tlog(ctx: &DownloadCtx, job: &Job, level: LogLevel, text: impl AsRef<str>) {
    log(
        &ctx.inbox,
        level,
        format!("[{}] {}", job.label, text.as_ref()),
    );
}

fn is_rate_limited(error_text: &str) -> bool {
    error_text.contains("http error 429")
        || error_text.contains("too many requests")
        || error_text.contains("rate limit")
        || error_text.contains("rate-limit")
}

fn is_video_unavailable(error_text: &str) -> bool {
    error_text.contains("video unavailable") || error_text.contains("this video is unavailable")
}

fn is_http_403(error_text: &str) -> bool {
    error_text.contains("http error 403") || error_text.contains("http 403")
}

fn is_drm_protected(error_text: &str) -> bool {
    error_text
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word == "drm")
        || error_text.contains("is protected")
}

fn add_common_args(cmd: &mut Command, output_template: &Path) {
    cmd.arg("--newline")
        .arg("--no-playlist")
        .arg("--no-color")
        .arg("--write-info-json")
        .arg("--write-thumbnail")
        .arg("--convert-thumbnails")
        .arg("jpg")
        .arg("--extract-audio")
        .arg("--audio-format")
        .arg("m4a")
        .arg("--retries")
        .arg("20")
        .arg("--fragment-retries")
        .arg("20")
        .arg("-o")
        .arg(output_template);
    tools::add_ffmpeg_location(cmd);
}

fn pump_stderr(
    ctx: &DownloadCtx,
    job: &Job,
    source: Source,
    stderr: ChildStderr,
    errors: &mut VecDeque<String>,
) {
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let raw = String::from_utf8_lossy(&buf);
        let clean = utils::strip_ansi(raw.trim_end());
        if clean.trim().is_empty() {
            continue;
        }
        if let Some(rest) = clean.strip_prefix("[download] Destination: ") {
            if let Some(stem) = Path::new(rest).file_stem() {
                emit(
                    &ctx.inbox,
                    WorkerEvent::TrackTitle {
                        id: job.id,
                        title: stem.to_string_lossy().to_string(),
                    },
                );
            }
            continue;
        }
        if let Some(value) = utils::parse_percent(&clean) {
            emit(&ctx.inbox, WorkerEvent::Progress { id: job.id, value });
            continue;
        }
        if clean.contains("[download]") {
            continue;
        }
        let lower = clean.to_ascii_lowercase();
        if lower.contains("error") || is_drm_protected(&lower) {
            errors.push_back(clean.clone());
            while errors.len() > ERROR_HISTORY {
                errors.pop_front();
            }
        }
        let expected_youtube_failure = source == Source::YouTube
            && (is_video_unavailable(&lower) || (is_http_403(&lower) && job.fallback_attempt == 0));
        let level = if expected_youtube_failure {
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
}

fn download_one(ctx: &DownloadCtx, job: &Job) -> Outcome {
    let fin = Outcome::Finished;
    let url = job.url.as_str();
    if url.starts_with("spotify:") {
        tlog(
            ctx,
            job,
            LogLevel::Error,
            "Spotify track was not resolved to a media URL.",
        );
        return fin(TrackResult::Error);
    }
    let source = detect_source(url);
    let temp = match tempfile::Builder::new().prefix("musicerer-tmp-").tempdir() {
        Ok(dir) => dir,
        Err(err) => {
            tlog(
                ctx,
                job,
                LogLevel::Error,
                format!("Cannot create temp folder: {err}"),
            );
            return fin(TrackResult::Error);
        }
    };
    let mut cmd = tools::command(Tool::YtDlp);
    add_common_args(&mut cmd, &temp.path().join("%(title)s.%(ext)s"));
    match source {
        Source::YouTube => {
            let player_client = if job.fallback_attempt > 0 {
                "android"
            } else {
                "default,web_embedded,tv"
            };
            cmd.arg("--format")
                .arg("bestaudio[ext=m4a]/bestaudio/best")
                .arg("--audio-quality")
                .arg("256K")
                .arg("--js-runtimes")
                .arg("deno")
                .arg("--remote-components")
                .arg("ejs:github")
                .arg("--extractor-args")
                .arg(format!("youtube:player_client={player_client}"))
                .arg("--concurrent-fragments")
                .arg(ctx.fragments.to_string());
        }
        Source::SoundCloud => {
            cmd.arg("--format").arg("bestaudio[ext=m4a]/bestaudio/best");
        }
        Source::Spotify => {
            tlog(
                ctx,
                job,
                LogLevel::Error,
                "Spotify URL reached download without resolution.",
            );
            return fin(TrackResult::Error);
        }
        Source::Unsupported => {
            tlog(ctx, job, LogLevel::Error, "Unsupported URL.");
            return fin(TrackResult::Error);
        }
    }
    if let Some(cookies) = &ctx.cookies {
        cmd.arg("--cookies").arg(cookies);
    }
    cmd.arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            tlog(
                ctx,
                job,
                LogLevel::Error,
                format!("Could not start yt-dlp (missing from tools folder and PATH?): {err}"),
            );
            return fin(TrackResult::Error);
        }
    };
    let Some(stderr) = child.stderr.take() else {
        tlog(ctx, job, LogLevel::Error, "yt-dlp produced no stderr pipe.");
        return fin(TrackResult::Error);
    };
    let pid = child.id();
    let finished = AtomicBool::new(false);
    let mut errors = VecDeque::with_capacity(ERROR_HISTORY);
    let status = thread::scope(|scope| {
        scope.spawn(|| process::watch_for_stop(pid, &ctx.stop_flag, &finished));
        pump_stderr(ctx, job, source, stderr, &mut errors);
        let status = child.wait();
        finished.store(true, Ordering::SeqCst);
        status
    });
    if ctx.stop_flag.load(Ordering::Relaxed) {
        return fin(TrackResult::Stopped);
    }
    let error_text = errors
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
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
            if !is_drm_protected(&error_text) {
                return fin(TrackResult::Error);
            }
            if let Err(r) = soundcloud_fallback::handle_drm(
                url,
                source,
                temp.path(),
                &ctx.inbox,
                &ctx.stop_flag,
            ) {
                return fin(r);
            }
        }
    }
    let Some(audio_path) = audio::find_audio(temp.path()) else {
        if source == Source::YouTube && is_video_unavailable(&error_text) {
            return fin(TrackResult::Unavailable);
        }
        tlog(
            ctx,
            job,
            LogLevel::Error,
            "yt-dlp finished but no audio file was found.",
        );
        return fin(TrackResult::Error);
    };

    if source == Source::SoundCloud {
        let reported = get_yt_dlp_duration(&audio_path);
        let actual = tools::find(Tool::Ffprobe)
            .and_then(|ffprobe| get_audio_duration(&ffprobe, &audio_path).ok());
        let is_preview = match (reported, actual) {
            (Some(r), Some(a)) => r >= 45.0 && a < 45.0,
            (None, Some(a)) => a < 45.0,
            _ => false,
        };
        if is_preview {
            tlog(
                ctx,
                job,
                LogLevel::Error,
                "SoundCloud Go+ preview detected. Failing due to Go+ protection.",
            );
            return fin(TrackResult::Error);
        }
    }

    let title = tagging::tag_audio(
        &audio_path,
        url,
        &ctx.inbox,
        job.id,
        &job.label,
        job.spotify.as_ref(),
    );
    let dest_dir = match job
        .folder
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        Some(name) => {
            let dir = ctx.output_dir.join(utils::sanitize_filename(name));
            match fs::create_dir_all(&dir) {
                Ok(()) => dir,
                Err(err) => {
                    tlog(
                        ctx,
                        job,
                        LogLevel::Warn,
                        format!(
                            "Cannot create playlist folder: {err}; saving to the output folder."
                        ),
                    );
                    ctx.output_dir.clone()
                }
            }
        }
        None => ctx.output_dir.clone(),
    };
    let final_stem = utils::sanitize_filename(&title);
    if ctx.skip_existing {
        let candidate = dest_dir.join(format!("{final_stem}.m4a"));
        if candidate.exists() {
            tlog(
                ctx,
                job,
                LogLevel::Info,
                format!("Skipping: {} already exists.", candidate.display()),
            );
            return fin(TrackResult::Skipped);
        }
    }
    match utils::move_into_dir(&audio_path, &dest_dir, &final_stem) {
        Ok(final_path) => {
            tlog(
                ctx,
                job,
                LogLevel::Info,
                format!("Saved: {}", final_path.display()),
            );
            fin(TrackResult::Ok)
        }
        Err(err) => {
            tlog(
                ctx,
                job,
                LogLevel::Error,
                format!("Could not move final file: {err}"),
            );
            fin(TrackResult::Error)
        }
    }
}

fn get_yt_dlp_duration(audio_path: &Path) -> Option<f64> {
    let info = audio_path.with_extension("info.json");
    let text = fs::read_to_string(info).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("duration").and_then(|d| d.as_f64())
}

fn get_audio_duration(ffprobe: &Path, path: &Path) -> Result<f64, String> {
    let output = process::hidden_command(ffprobe)
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(path)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("ffprobe failed".into());
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|_| "could not parse duration".into())
}
