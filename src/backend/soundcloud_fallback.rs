use super::tools::{self, Tool};
use super::utils::USER_AGENT;
use super::{log, LogLevel, Source, TrackResult, WorkerEvent};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const OEMBED_TIMEOUT: Duration = Duration::from_secs(5);
const THUMBNAIL_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_OEMBED_THUMBNAIL_BYTES: u64 = 4 * 1024 * 1024;

fn fetch_oembed_metadata(url: &str) -> Result<(String, Option<String>), String> {
    // Build query parameters separately so playlist URLs that already contain a
    // query string are encoded correctly rather than truncating the oEmbed URL.
    let agent = ureq::AgentBuilder::new().timeout(OEMBED_TIMEOUT).build();
    let res = agent
        .get("https://soundcloud.com/oembed")
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/json,text/plain,*/*")
        .query("format", "json")
        .query("url", url)
        .call()
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = res.into_json().map_err(|e| e.to_string())?;
    let title = json.get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let thumb_url = json.get("thumbnail_url")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Ok((title, thumb_url))
}

/// Fetches a SoundCloud title and optional cover bytes without creating temporary files.
/// Thumbnail failures do not discard a title that was retrieved successfully.
pub fn fetch_oembed(url: &str) -> Result<(String, Option<Vec<u8>>), String> {
    let (title, thumb_url) = fetch_oembed_metadata(url)?;
    let thumbnail = thumb_url.and_then(|url| fetch_thumbnail_bytes(&url).ok());
    Ok((title, thumbnail))
}

fn fetch_thumbnail_bytes(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(THUMBNAIL_TIMEOUT)
        .build();
    let response = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "image/avif,image/webp,image/apng,image/*,*/*;q=0.8")
        .call()
        .map_err(|e| e.to_string())?;

    let mut bytes = Vec::with_capacity(64 * 1024);
    response
        .into_reader()
        .take(MAX_OEMBED_THUMBNAIL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_OEMBED_THUMBNAIL_BYTES {
        return Err("SoundCloud thumbnail exceeds the 4 MiB limit".to_string());
    }
    Ok(bytes)
}

pub fn handle_drm(
    url: &str,
    source: Source,
    temp_dir: &Path,
    inbox: &Arc<Mutex<Vec<WorkerEvent>>>,
) -> Result<(), TrackResult> {
    if source != Source::SoundCloud {
        return Err(TrackResult::Drm);
    }
    match try_fallback(url, temp_dir, inbox) {
        Ok(_) => Ok(()),
        Err(e) => {
            log(inbox, LogLevel::Error, format!("SoundCloud fallback failed: {e}"));
            Err(TrackResult::Drm)
        }
    }
}

fn try_fallback(
    url: &str,
    temp_dir: &Path,
    inbox: &Arc<Mutex<Vec<WorkerEvent>>>,
) -> Result<PathBuf, String> {
    let tool = tools::find(Tool::SoundCloud).ok_or_else(|| {
        "optional soundcloud fallback tool not found; only DRM-protected SoundCloud tracks need it"
            .to_string()
    })?;
    let ffmpeg = tools::find(Tool::Ffmpeg).ok_or_else(|| "ffmpeg not found".to_string())?;

    log(inbox, LogLevel::Info, format!("Running fallback tool: {:?}", tool));

    // `command` also puts the ffmpeg folder on PATH for the child process.
    let mut cmd = tools::command(Tool::SoundCloud);
    cmd.arg(url)
       .arg("--ffmpeg")
       .arg(&ffmpeg)
       .current_dir(temp_dir);

    let output = cmd.output().map_err(|e| format!("soundcloud tool spawn failed: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    if !output.status.success() {
        let details = if !stderr.trim().is_empty() { &stderr } else { &stdout };
        let last = details.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
        return Err(format!("soundcloud tool exited with {:?}: {}", output.status.code(), last));
    }

    let mut found_file = None;
    for ext in &["mp3", "m4a", "opus", "webm", "wav"] {
        if let Some(p) = super::utils::find_file_ext(temp_dir, ext) {
            found_file = Some(p);
            break;
        }
    }

    let src_audio = found_file
        .ok_or_else(|| "soundcloud tool produced no recognizable audio file".to_string())?;

    log(inbox, LogLevel::Info, format!("Fallback downloaded: {:?}", src_audio.file_name().unwrap_or_default()));

    let size = std::fs::metadata(&src_audio).map(|m| m.len()).unwrap_or(0);
    if size < 10_000 {
        return Err(format!("Downloaded file is too small ({} bytes), maybe an error page", size));
    }

    if src_audio.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("m4a")).unwrap_or(false) {
        return Ok(src_audio);
    }

    let m4a = src_audio.with_extension("m4a");
    log(inbox, LogLevel::Info, "Transcoding fallback audio to m4a...");

    let ff = tools::command(Tool::Ffmpeg)
        .arg("-y")
        .arg("-i").arg(&src_audio)
        .arg("-vn")
        .arg("-c:a").arg("aac")
        .arg("-b:a").arg("256k")
        .arg(&m4a)
        .output()
        .map_err(|e| format!("ffmpeg spawn failed: {e}"))?;

    if !ff.status.success() {
        let ff_stderr = String::from_utf8_lossy(&ff.stderr);
        let lines: Vec<&str> = ff_stderr.lines().filter(|l| !l.trim().is_empty()).collect();
        let err_msg = if lines.len() >= 4 { lines[lines.len()-4..].join(" | ") } else { lines.join(" | ") };
        return Err(format!("ffmpeg failed: {}", err_msg));
    }

    Ok(m4a)
}
