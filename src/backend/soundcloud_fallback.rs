use super::{log, LogLevel, Source, TrackResult, WorkerEvent};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const OEMBED_TIMEOUT: Duration = Duration::from_secs(5);
const THUMBNAIL_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_OEMBED_THUMBNAIL_BYTES: u64 = 4 * 1024 * 1024;
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[cfg(target_os = "windows")]
use std::sync::OnceLock;

// FFmpeg and ffprobe are bundled with Windows builds. The SoundCloud fallback is
// intentionally external/optional: normal downloads must not require its binary.
#[cfg(target_os = "windows")]
const FFMPEG_BIN: &[u8] = include_bytes!("../../tools/ffmpeg.exe");
#[cfg(target_os = "windows")]
const FFPROBE_BIN: &[u8] = include_bytes!("../../tools/ffprobe.exe");

#[cfg(target_os = "windows")]
static EMBEDDED: OnceLock<(Option<PathBuf>, Option<PathBuf>)> = OnceLock::new();

#[cfg(target_os = "windows")]
fn ensure_file(path: &Path, bytes: &[u8]) {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() == bytes.len() as u64 { return; }
    }
    let _ = std::fs::write(path, bytes);
}

#[cfg(target_os = "windows")]
fn get_embedded() -> &'static (Option<PathBuf>, Option<PathBuf>) {
    EMBEDDED.get_or_init(|| {
        let mut dir = std::env::temp_dir();
        dir.push("musicerer_embedded_tools");
        let _ = std::fs::create_dir_all(&dir);

        let ff_path = dir.join("ffmpeg.exe");
        let fp_path = dir.join("ffprobe.exe");

        ensure_file(&ff_path, FFMPEG_BIN);
        ensure_file(&fp_path, FFPROBE_BIN);

        (
            if ff_path.exists() { Some(ff_path) } else { None },
            if fp_path.exists() { Some(fp_path) } else { None },
        )
    })
}

pub fn find_tool(name: &str) -> Option<PathBuf> {
    let exe_name = if cfg!(windows) && !name.ends_with(".exe") {
        format!("{}.exe", name)
    } else if !cfg!(windows) && name.ends_with(".exe") {
        name.trim_end_matches(".exe").to_string()
    } else {
        name.to_string()
    };

    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("tools").join(&exe_name));
            candidates.push(dir.join(&exe_name));
        }
    }
    candidates.push(PathBuf::from("tools").join(&exe_name));

    for c in candidates {
        if c.is_file() { return Some(c); }
    }

    #[cfg(target_os = "windows")]
    {
        let (ff, fp) = get_embedded();
        let embedded = match exe_name.as_str() {
            "ffmpeg.exe" => ff.clone(),
            "ffprobe.exe" => fp.clone(),
            _ => None,
        };
        if let Some(p) = embedded {
            return Some(p);
        }
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for path in std::env::split_paths(&path_var) {
            let p = path.join(&exe_name);
            if p.is_file() { return Some(p); }
        }
    }

    None
}

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
    let tool = find_tool("soundcloud.exe").ok_or_else(|| {
        "optional soundcloud.exe fallback not found; only DRM-protected SoundCloud tracks need this tool"
            .to_string()
    })?;
    let ffmpeg = find_tool("ffmpeg.exe").ok_or_else(|| "ffmpeg.exe not found".to_string())?;

    log(inbox, LogLevel::Info, format!("Running fallback tool: {:?}", tool));

    let mut cmd = Command::new(&tool);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    
    cmd.arg(url)
       .arg("--ffmpeg")
       .arg(&ffmpeg)
       .current_dir(temp_dir);

    if let Some(ffmpeg_dir) = ffmpeg.parent() {
        let mut paths = std::env::var_os("PATH").unwrap_or_default();
        paths.push(";");
        paths.push(ffmpeg_dir);
        cmd.env("PATH", paths);
    }

    let output = cmd.output().map_err(|e| format!("soundcloud.exe spawn failed: {e}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    if !output.status.success() {
        let details = if !stderr.trim().is_empty() { &stderr } else { &stdout };
        let last = details.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
        return Err(format!("soundcloud.exe exited with {:?}: {}", output.status.code(), last));
    }

    let mut found_file = None;
    for ext in &["mp3", "m4a", "opus", "webm", "wav"] {
        if let Some(p) = super::utils::find_file_ext(temp_dir, ext) {
            found_file = Some(p);
            break;
        }
    }

    let src_audio = found_file
        .ok_or_else(|| "soundcloud.exe produced no recognizable audio file".to_string())?;

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

    let mut ff_cmd = Command::new(&ffmpeg);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        ff_cmd.creation_flags(0x08000000);
    }
    
    let ff = ff_cmd
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
