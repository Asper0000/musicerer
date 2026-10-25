use super::{audio::{make_rounded_thumb, make_rounded_thumb_from_bytes}, detect_source, emit, soundcloud_fallback,
    tools::{self, Tool}, Inbox, Source, Thumb, WorkerEvent};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

const PREVIEW_WORKERS: usize = 3;

pub struct Preview {
    pub title: String,
    pub thumb: Option<Thumb>,
}

pub fn spawn_preview(
    id: u64,
    url: String,
    cookies: Option<PathBuf>,
    inbox: Inbox,
) {
    spawn_previews(vec![(id, url)], cookies, inbox);
}

/// Refreshes track-level metadata with a small worker pool. SoundCloud's flat
/// playlist entries can contain creator artwork or omit title/artwork entirely;
/// re-resolving the individual track URL gives the same metadata as adding that
/// track directly, without spawning hundreds of yt-dlp processes at once.
pub fn spawn_previews(
    jobs: Vec<(u64, String)>,
    cookies: Option<PathBuf>,
    inbox: Inbox,
) {
    if jobs.is_empty() {
        return;
    }

    let workers = PREVIEW_WORKERS.min(jobs.len());
    let jobs = Arc::new(jobs);
    let next = Arc::new(AtomicUsize::new(0));

    for _ in 0..workers {
        let (jobs, next, cookies, inbox) = (
            jobs.clone(),
            next.clone(),
            cookies.clone(),
            inbox.clone(),
        );
        thread::spawn(move || loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some((id, url)) = jobs.get(index) else { break; };
            let event = match fetch_preview(url, cookies.as_deref()) {
                Ok(p) => WorkerEvent::PreviewReady {
                    id: *id,
                    title: p.title,
                    thumb: p.thumb,
                },
                Err(error) => WorkerEvent::PreviewFailed { id: *id, error },
            };
            emit(&inbox, event);
        });
    }
}

fn fetch_preview(url: &str, cookies: Option<&Path>) -> Result<Preview, String> {
    let source = detect_source(url);
    let tmp = tempfile::Builder::new()
        .prefix("musicerer-preview-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let out_tpl = tmp.path().join("%(id)s.%(ext)s");

    let mut cmd = tools::command(Tool::YtDlp);

    cmd.arg("--skip-download")
        .arg("--write-thumbnail")
        .arg("--convert-thumbnails").arg("jpg")
        .arg("--write-info-json")
        .arg("--no-playlist")
        .arg("--no-warnings")
        .arg("--no-color")
        .arg("-o").arg(&out_tpl);

    tools::add_ffmpeg_location(&mut cmd);

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
            if let Ok((title, thumb_bytes)) = soundcloud_fallback::fetch_oembed(url) {
                let thumb = thumb_bytes
                    .as_deref()
                    .and_then(|bytes| make_rounded_thumb_from_bytes(bytes, 40, 8.0).ok());
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
