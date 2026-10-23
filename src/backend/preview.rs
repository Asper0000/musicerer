use super::{audio::make_rounded_thumb, detect_source, emit, soundcloud_fallback, Source, WorkerEvent};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

pub struct Preview {
    pub title: String,
    pub thumb: Option<(u32, u32, Vec<u8>)>,
}

pub fn spawn_preview(
    id: u64,
    url: String,
    cookies: Option<PathBuf>,
    inbox: Arc<Mutex<Vec<WorkerEvent>>>,
) {
    thread::spawn(move || {
        match fetch_preview(&url, cookies.as_deref()) {
            Ok(p) => emit(&inbox, WorkerEvent::PreviewReady {
                id, title: p.title, thumb: p.thumb,
            }),
            Err(e) => emit(&inbox, WorkerEvent::PreviewFailed { id, error: e }),
        }
    });
}

fn fetch_preview(url: &str, cookies: Option<&Path>) -> Result<Preview, String> {
    let source = detect_source(url);
    let tmp = tempfile::Builder::new()
        .prefix("musicerer-preview-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let out_tpl = tmp.path().join("%(id)s.%(ext)s");

    let mut cmd = Command::new("yt-dlp");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    
    cmd.arg("--skip-download")
        .arg("--write-thumbnail")
        .arg("--convert-thumbnails").arg("jpg")
        .arg("--write-info-json")
        .arg("--no-playlist")
        .arg("--no-warnings")
        .arg("--no-color")
        .arg("-o").arg(&out_tpl);

    if let Some(ff) = soundcloud_fallback::find_tool("ffmpeg.exe") {
        if let Some(parent) = ff.parent() {
            cmd.arg("--ffmpeg-location").arg(parent);
        }
    }

    if source == Source::YouTube {
        cmd.arg("--js-runtimes").arg("deno")
            .arg("--remote-components").arg("ejs:github")
            .arg("--extractor-args")
            .arg("youtube:player_client=default,web_embedded,tv");
    }
    if let Some(c) = cookies { cmd.arg("--cookies").arg(c); }
    cmd.arg(url);

    let output = cmd.output().map_err(|e| format!("spawn failed: {e}"))?;
    if !output.status.success() {
        if source == Source::SoundCloud {
            if let Ok((title, thumb_path)) = soundcloud_fallback::fetch_oembed(url) {
                let thumb = thumb_path.and_then(|p| make_rounded_thumb(&p, 40, 8.0).ok());
                return Ok(Preview { title, thumb });
            }
        }
        let err = String::from_utf8_lossy(&output.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
        return Err(last.to_string());
    }

    let mut title = "Untitled".to_string();
    let mut thumb = None;

    if let Ok(rd) = fs::read_dir(tmp.path()) {
        for entry in rd.flatten() {
            let p = entry.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".info.json") {
                if let Ok(text) = fs::read_to_string(&p) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(t) = v.get("title").and_then(|x| x.as_str()) {
                            title = t.to_string();
                        }
                    }
                }
            } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") {
                    thumb = make_rounded_thumb(&p, 40, 8.0).ok();
                }
            }
        }
    }

    Ok(Preview { title, thumb })
}
