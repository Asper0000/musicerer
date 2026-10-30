use crate::backend::process;
use crate::backend::tools::{self, Tool};
use crate::backend::utils::USER_AGENT;
use crate::backend::{log, Inbox, LogLevel, Source, TrackResult};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let thumb_url = json
        .get("thumbnail_url")
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
    let agent = ureq::AgentBuilder::new().timeout(THUMBNAIL_TIMEOUT).build();
    let response = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set(
            "Accept",
            "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
        )
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

/// Tries the optional fallback tool for a DRM-protected SoundCloud track.
/// `Ok` means audio was produced in `temp_dir`; the tool and its ffmpeg step both
/// obey `stop`.
pub fn handle_drm(
    url: &str,
    source: Source,
    temp_dir: &Path,
    inbox: &Inbox,
    stop: &AtomicBool,
) -> Result<(), TrackResult> {
    if source != Source::SoundCloud {
        return Err(TrackResult::Drm);
    }
    match try_fallback(url, temp_dir, inbox, stop) {
        Ok(_) => Ok(()),
        Err(_) if stop.load(Ordering::Relaxed) => Err(TrackResult::Stopped),
        Err(e) => {
            log(
                inbox,
                LogLevel::Error,
                format!("SoundCloud fallback failed: {e}"),
            );
            Err(TrackResult::Drm)
        }
    }
}

fn try_fallback(
    url: &str,
    temp_dir: &Path,
    inbox: &Inbox,
    stop: &AtomicBool,
) -> Result<PathBuf, String> {
    let tool = tools::find(Tool::SoundCloud).ok_or_else(|| {
        "optional soundcloud fallback tool not found; only DRM-protected SoundCloud tracks need it"
            .to_string()
    })?;
    let ffmpeg = tools::find(Tool::Ffmpeg).ok_or_else(|| "ffmpeg not found".to_string())?;

    log(
        inbox,
        LogLevel::Info,
        format!("Running fallback tool: {:?}", tool),
    );

    // `command` also puts the ffmpeg folder on PATH for the child process.
    let mut cmd = tools::command(Tool::SoundCloud);
    cmd.arg(url)
        .arg("--ffmpeg")
        .arg(&ffmpeg)
        .current_dir(temp_dir);

    let output = process::output_stoppable(&mut cmd, stop)
        .map_err(|e| format!("soundcloud tool spawn failed: {e}"))?;
    if stop.load(Ordering::Relaxed) {
        return Err("stopped".to_string());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    if !output.status.success() {
        let details = if !stderr.trim().is_empty() {
            &stderr
        } else {
            &stdout
        };
        let last = details
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error");
        return Err(format!(
            "soundcloud tool exited with {:?}: {}",
            output.status.code(),
            last
        ));
    }

    let src_audio = ["mp3", "m4a", "opus", "webm", "wav"]
        .iter()
        .find_map(|ext| crate::backend::utils::find_file_ext(temp_dir, ext))
        .ok_or_else(|| "soundcloud tool produced no recognizable audio file".to_string())?;

    log(
        inbox,
        LogLevel::Info,
        format!(
            "Fallback downloaded: {:?}",
            src_audio.file_name().unwrap_or_default()
        ),
    );

    let size = std::fs::metadata(&src_audio).map(|m| m.len()).unwrap_or(0);
    if size < 10_000 {
        return Err(format!(
            "Downloaded file is too small ({} bytes), maybe an error page",
            size
        ));
    }

    let is_m4a = src_audio
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("m4a"));
    if is_m4a {
        return Ok(src_audio);
    }

    let m4a = src_audio.with_extension("m4a");
    log(
        inbox,
        LogLevel::Info,
        "Transcoding fallback audio to m4a...",
    );

    let mut ffmpeg_cmd = tools::command(Tool::Ffmpeg);
    ffmpeg_cmd
        .arg("-y")
        .arg("-i")
        .arg(&src_audio)
        .arg("-vn")
        .arg("-c:a")
        .arg("aac")
        .arg("-b:a")
        .arg("256k")
        .arg(&m4a);
    let ff = process::output_stoppable(&mut ffmpeg_cmd, stop)
        .map_err(|e| format!("ffmpeg spawn failed: {e}"))?;
    if stop.load(Ordering::Relaxed) {
        return Err("stopped".to_string());
    }

    if !ff.status.success() {
        let ff_stderr = String::from_utf8_lossy(&ff.stderr);
        let lines: Vec<&str> = ff_stderr.lines().filter(|l| !l.trim().is_empty()).collect();
        let err_msg = if lines.len() >= 4 {
            lines[lines.len() - 4..].join(" | ")
        } else {
            lines.join(" | ")
        };
        return Err(format!("ffmpeg failed: {}", err_msg));
    }

    Ok(m4a)
}
