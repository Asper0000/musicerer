use super::{audio, detect_source, emit, log, soundcloud_fallback, utils, LogLevel, Source, TrackResult, WorkerEvent};
use audiotags::{MimeType, Picture, Tag};
use serde::Deserialize;
use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

pub fn spawn_worker(
    items: Vec<(u64, String)>,
    output_dir: PathBuf,
    cookies: Option<PathBuf>,
    inbox: Arc<Mutex<Vec<WorkerEvent>>>,
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
    inbox: Arc<Mutex<Vec<WorkerEvent>>>,
    stop_flag: Arc<AtomicBool>,
) {
    let total = items.len();
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let mut drm = 0usize;
    let mut stopped = false;

    if let Err(err) = fs::create_dir_all(&output_dir) {
        log(&inbox, LogLevel::Error, format!("Cannot create output folder: {err}"));
        emit(&inbox, WorkerEvent::QueueFinished { succeeded: 0, failed: total, drm: 0, stopped: false });
        return;
    }

    for (index, (_id, url)) in items.iter().enumerate() {
        if stop_flag.load(Ordering::Relaxed) { stopped = true; break; }
        emit(&inbox, WorkerEvent::TrackStarted { index });
        log(&inbox, LogLevel::Info, format!("Track {}/{}", index + 1, total));

        let result = download_one(url, &output_dir, cookies.as_deref(), index, &inbox, &stop_flag);

        match result {
            TrackResult::Ok => { succeeded += 1; log(&inbox, LogLevel::Info, "Completed."); }
            TrackResult::Drm => {
                drm += 1;
                log(&inbox, LogLevel::Warn, "DRM-protected - skipped. yt-dlp does not bypass DRM.");
            }
            TrackResult::Stopped => { stopped = true; }
            TrackResult::Error => { failed += 1; log(&inbox, LogLevel::Error, "Failed."); }
        }

        emit(&inbox, WorkerEvent::TrackFinished { index, result });
        if stopped { break; }
    }

    emit(&inbox, WorkerEvent::QueueFinished { succeeded, failed, drm, stopped });
}

fn download_one(
    url: &str,
    output_dir: &Path,
    cookies: Option<&Path>,
    index: usize,
    inbox: &Arc<Mutex<Vec<WorkerEvent>>>,
    stop_flag: &Arc<AtomicBool>,
) -> TrackResult {
    let source = detect_source(url);
    let temp = match tempfile::Builder::new().prefix("musicerer-tmp-").tempdir_in(output_dir) {
        Ok(dir) => dir,
        Err(err) => {
            log(inbox, LogLevel::Error, format!("Cannot create temp folder: {err}"));
            return TrackResult::Error;
        }
    };

    let output_template = temp.path().join("%(title)s.%(ext)s");
    let mut cmd = Command::new("yt-dlp");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

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

    if let Some(ff) = soundcloud_fallback::find_tool("ffmpeg.exe") {
        if let Some(parent) = ff.parent() {
            cmd.arg("--ffmpeg-location").arg(parent);
        }
    }

    match source {
        Source::YouTube => {
            cmd.arg("--format").arg("bestaudio[ext=m4a]/bestaudio/best")
                .arg("--audio-quality").arg("256K")
                .arg("--js-runtimes").arg("deno")
                .arg("--remote-components").arg("ejs:github")
                .arg("--extractor-args").arg("youtube:player_client=default,web_embedded,tv")
                .arg("--concurrent-fragments").arg("10");
        }
        Source::SoundCloud => {
            cmd.arg("--format").arg("bestaudio[ext=m4a]/bestaudio/best");
        }
        Source::Unsupported => {
            log(inbox, LogLevel::Error, "Unsupported URL.");
            return TrackResult::Error;
        }
    }

    if let Some(cookies) = cookies { cmd.arg("--cookies").arg(cookies); }

    cmd.arg(url).stdout(Stdio::null()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            log(inbox, LogLevel::Error, format!("Could not start yt-dlp (is it on PATH?): {err}"));
            return TrackResult::Error;
        }
    };

    let Some(stderr) = child.stderr.take() else {
        log(inbox, LogLevel::Error, "yt-dlp produced no stderr pipe.");
        return TrackResult::Error;
    };

    let reader = BufReader::new(stderr);
    let mut errors = VecDeque::with_capacity(40);
    let mut was_stopped = false;

    for line in reader.lines().map_while(Result::ok) {
        if stop_flag.load(Ordering::Relaxed) {
            was_stopped = true;
            let _ = child.kill();
            break;
        }

        let clean = utils::strip_ansi(&line);
        if clean.trim().is_empty() { continue; }

        if let Some(rest) = clean.strip_prefix("[download] Destination: ") {
            if let Some(stem) = Path::new(rest).file_stem() {
                emit(inbox, WorkerEvent::TrackTitle { index, title: stem.to_string_lossy().to_string() });
            }
            continue;
        }

        if let Some(value) = utils::parse_percent(&clean) {
            emit(inbox, WorkerEvent::Progress { index, value });
            continue;
        }
        if clean.contains("[download]") { continue; }

        let lower = clean.to_ascii_lowercase();
        if lower.contains("error") || lower.contains("drm") || lower.contains("protected") {
            errors.push_back(clean.clone());
            while errors.len() > 40 { errors.pop_front(); }
        }
        log(inbox, LogLevel::Info, clean);
    }

    let status = child.wait();
    if was_stopped || stop_flag.load(Ordering::Relaxed) {
        return TrackResult::Stopped;
    }

    if let Ok(s) = status {
        if !s.success() {
            let text = errors.into_iter().collect::<Vec<_>>().join("\n").to_ascii_lowercase();
            if text.contains("drm") || text.contains("protected") {
                if let Err(r) = soundcloud_fallback::handle_drm(url, source, temp.path(), inbox) {
                    return r;
                }
            } else {
                return TrackResult::Error;
            }
        }
    }

    let Some(audio_path) = audio::find_audio(temp.path()) else {
        log(inbox, LogLevel::Error, "yt-dlp finished but no audio file was found.");
        return TrackResult::Error;
    };

    // --- Go+ Preview Detection ---
    if source == Source::SoundCloud {
        if let Some(ffprobe) = soundcloud_fallback::find_tool("ffprobe.exe") {
            if let Ok(duration) = get_audio_duration(&ffprobe, &audio_path) {
                if duration < 45.0 {
                    log(inbox, LogLevel::Error, "SoundCloud Go+ track detected (preview only). Failing download due to Go+ protection.");
                    return TrackResult::Error;
                }
            }
        }
    }

    let title = process_metadata_and_tags(&audio_path, url, temp.path(), inbox, index);

    let final_path = utils::unique_path(output_dir, &utils::sanitize_filename(&title));
    if let Err(err) = fs::rename(&audio_path, &final_path) {
        log(inbox, LogLevel::Error, format!("Could not move final file: {err}"));
        return TrackResult::Error;
    }
    log(inbox, LogLevel::Info, format!("Saved: {}", final_path.display()));
    TrackResult::Ok
}

fn get_audio_duration(ffprobe: &Path, path: &Path) -> Result<f64, String> {
    let mut cmd = Command::new(ffprobe);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let output = cmd
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

fn process_metadata_and_tags(
    audio_path: &Path,
    url: &str,
    temp_dir: &Path,
    inbox: &Arc<Mutex<Vec<WorkerEvent>>>,
    index: usize,
) -> String {
    let info = read_info(audio_path);
    let mut title = info.as_ref().and_then(|i| i.track.clone().or_else(|| i.title.clone()));
    let mut artist = info.as_ref().and_then(|i| {
        i.artist.clone().or_else(|| i.uploader.clone()).or_else(|| i.channel.clone())
    });
    let mut album = info.as_ref().and_then(|i| i.album.clone());
    
    let mut cover_bytes = if let Some(thumb) = audio::find_thumbnail(audio_path) {
        match audio::square_cover(&thumb, temp_dir) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                log(inbox, LogLevel::Warn, format!("Could not crop cover art: {err}"));
                None
            }
        }
    } else { None };

    if info.is_none() {
        if let Ok(tag) = Tag::new().read_from_path(audio_path) {
            if title.is_none() { title = tag.title().map(|s| s.to_string()); }
            if artist.is_none() { artist = tag.artist().map(|s| s.to_string()); }
            if album.is_none() { album = tag.album_title().map(|s| s.to_string()); }
            if cover_bytes.is_none() {
                if let Some(pic) = tag.album_cover() {
                    cover_bytes = Some(pic.data.to_vec());
                }
            }
        }
    }

    if title.is_none() || cover_bytes.is_none() {
        if let Ok((t, p)) = soundcloud_fallback::fetch_oembed(url) {
            if title.is_none() { title = Some(t); }
            if cover_bytes.is_none() {
                if let Some(p) = p {
                    if let Ok(bytes) = audio::square_cover(&p, temp_dir) {
                        cover_bytes = Some(bytes);
                    }
                }
            }
        }
    }

    let final_title = title.unwrap_or_else(|| {
        audio_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Unknown title".to_string())
    });
    emit(inbox, WorkerEvent::TrackTitle { index, title: final_title.clone() });

    let final_album = album.unwrap_or_else(|| "Musicerer".to_string());
    let year = info.as_ref().and_then(|i| {
        i.release_date.as_deref().or(i.upload_date.as_deref())
            .and_then(|d| d.get(0..4)).and_then(|y| y.parse::<i32>().ok())
    });
    let track_number = info.as_ref().and_then(|i| i.track_number.map(|n| n as i32));

    if let Err(err) = write_tags(
        audio_path, &final_title, artist.as_deref(), &final_album,
        year, track_number, cover_bytes.as_deref(),
    ) {
        log(inbox, LogLevel::Warn, format!("Metadata write failed (audio still saved): {err}"));
    }

    final_title
}

#[derive(Debug, Default, Deserialize)]
struct TrackInfo {
    title: Option<String>,
    track: Option<String>,
    artist: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    album: Option<String>,
    release_date: Option<String>,
    upload_date: Option<String>,
    track_number: Option<i64>,
}

fn read_info(audio_path: &Path) -> Option<TrackInfo> {
    let info_path = audio_path.with_extension("info.json");
    let text = fs::read_to_string(info_path).ok()?;
    serde_json::from_str::<TrackInfo>(&text).ok()
}

fn write_tags(
    audio_path: &Path,
    title: &str,
    artist: Option<&str>,
    album: &str,
    year: Option<i32>,
    track_number: Option<i32>,
    cover: Option<&[u8]>,
) -> Result<(), String> {
    let path_str = audio_path.to_string_lossy().to_string();
    let mut tag = Tag::new().read_from_path(&path_str).map_err(|e| e.to_string())?;
    tag.set_title(title);
    if let Some(artist) = artist.filter(|a| !a.trim().is_empty()) { tag.set_artist(artist); }
    if !album.trim().is_empty() { tag.set_album_title(album); }
    if let Some(year) = year { tag.set_year(year); }
    if let Some(track) = track_number.filter(|n| *n > 0) { tag.set_track_number(track as u16); }
    if let Some(bytes) = cover {
        tag.set_album_cover(Picture { mime_type: MimeType::Jpeg, data: bytes });
    }
    tag.write_to_path(&path_str).map_err(|e| e.to_string())
}
