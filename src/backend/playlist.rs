//! Playlist support: detecting playlist URLs, expanding them into tracks with
//! `yt-dlp --flat-playlist` (one fast call, no per-track process), and loading
//! the track thumbnails on a small pool of threads.

use super::tools::{self, Tool};
use super::utils::USER_AGENT;
use super::{audio, detect_source, emit, log, Inbox, LogLevel, Source, WorkerEvent};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const MAX_PLAYLIST_ENTRIES: usize = 500;
const THUMB_WORKERS: usize = 4;
const THUMB_MAX_BYTES: u64 = 4 * 1024 * 1024;
const THUMB_PX: u32 = 40;
const THUMB_RADIUS: f32 = 8.0;
const THUMB_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct PlaylistEntry {
    pub url: String,
    pub title: String,
    pub thumb_url: Option<String>,
    pub spotify: Option<crate::backend::spotify::SpotifyMeta>,
    pub deezer: Option<crate::backend::deezer::DeezerMeta>,
}

struct Playlist {
    title: String,
    entries: Vec<PlaylistEntry>,
    main: usize,
}

pub fn is_playlist_url(url: &str, source: Source) -> bool {
    match source {
        Source::YouTube => super::youtube::playlist::is_playlist_url(url),
        Source::SoundCloud => super::soundcloud::playlist::is_playlist_url(url),
        Source::Spotify => false,
        Source::Deezer => false,
        Source::Unsupported => false,
    }
}

pub fn spawn_resolve(
    id: u64,
    url: String,
    cookies: Option<PathBuf>,
    inbox: Inbox,
    cancel: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let result = fetch_playlist(&url, cookies.as_deref(), &inbox, &cancel);
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        match result {
            Ok(p) => emit(
                &inbox,
                WorkerEvent::PlaylistResolved {
                    id,
                    title: p.title,
                    entries: p.entries,
                    main: p.main,
                },
            ),
            Err(error) => emit(&inbox, WorkerEvent::PlaylistFailed { id, error }),
        }
    });
}

fn fetch_playlist(
    url: &str,
    cookies: Option<&Path>,
    inbox: &Inbox,
    cancel: &AtomicBool,
) -> Result<Playlist, String> {
    let _permit = super::metadata_limit::acquire(cancel)
        .ok_or_else(|| "cancelled".to_string())?;

    let mut cmd = tools::command(Tool::YtDlp);
    cmd.arg("--flat-playlist")
        .arg("--dump-single-json")
        .arg("--ignore-errors")
        .arg("--no-warnings")
        .arg("--no-color")
        .arg("--playlist-end")
        .arg(MAX_PLAYLIST_ENTRIES.to_string());
    if let Some(c) = cookies {
        cmd.arg("--cookies").arg(c);
    }
    cmd.arg(url);

    let output = super::process::output_stoppable(&mut cmd, cancel)
        .map_err(|e| format!("could not start yt-dlp: {e}"))?;
    if output.stdout.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr);
        let last = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error");
        return Err(last.to_string());
    }

    let root: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let raw = root
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "not a playlist".to_string())?;

    if raw.len() >= MAX_PLAYLIST_ENTRIES {
        log(
            inbox,
            LogLevel::Warn,
            format!("Playlist is long; only the first {MAX_PLAYLIST_ENTRIES} tracks were loaded."),
        );
    }

    let entries: Vec<PlaylistEntry> = raw.iter().filter_map(parse_entry).collect();
    if entries.is_empty() {
        return Err("playlist has no downloadable tracks".to_string());
    }

    let main = super::youtube::playlist::video_id_from_url(url)
        .and_then(|vid| entries.iter().position(|e| e.url.contains(&vid)))
        .unwrap_or(0);
    let title = root
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Playlist")
        .to_string();
    Ok(Playlist {
        title,
        entries,
        main,
    })
}

fn parse_entry(entry: &Value) -> Option<PlaylistEntry> {
    let text = |key: &str| entry.get(key).and_then(Value::as_str);

    let title = text("title").unwrap_or("Untitled");
    if title.starts_with("[Private video]") || title.starts_with("[Deleted video]") {
        return None;
    }

    let id = text("id");
    let url = ["webpage_url", "url"]
        .iter()
        .filter_map(|k| text(k))
        .find(|u| u.starts_with("http"))
        .map(String::from)
        .or_else(|| id.map(|i| format!("https://www.youtube.com/watch?v={i}")))?;

    let source = detect_source(&url);
    if source == Source::Unsupported {
        return None;
    }

    let thumb_url = match (source, id) {
        (Source::YouTube, Some(id)) => Some(super::youtube::playlist::thumbnail_url(id)),
        _ => pick_thumbnail(entry),
    };

    Some(PlaylistEntry {
        url,
        title: title.to_string(),
        thumb_url,
        spotify: None,
        deezer: None,
    })
}

fn pick_thumbnail(entry: &Value) -> Option<String> {
    if let Some(u) = entry.get("thumbnail").and_then(Value::as_str) {
        return Some(u.to_string());
    }
    let list = entry.get("thumbnails")?.as_array()?;
    let width = |t: &Value| t.get("width").and_then(Value::as_u64);
    let url = |t: &Value| t.get("url").and_then(Value::as_str).map(String::from);
    list.iter()
        .filter(|t| width(t).is_some_and(|w| w >= 120))
        .min_by_key(|t| width(t))
        .and_then(url)
        .or_else(|| list.last().and_then(url))
}

pub fn spawn_thumbnails(jobs: Vec<(u64, String)>, inbox: Inbox) {
    if jobs.is_empty() {
        return;
    }
    let workers = THUMB_WORKERS.min(jobs.len());
    let jobs = Arc::new(jobs);
    let next = Arc::new(AtomicUsize::new(0));

    for _ in 0..workers {
        let (jobs, next, inbox) = (jobs.clone(), next.clone(), inbox.clone());
        thread::spawn(move || {
            let agent = ureq::AgentBuilder::new().timeout(THUMB_TIMEOUT).build();
            while let Some((id, url)) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                let thumb = fetch_thumb(&agent, url).ok();
                emit(&inbox, WorkerEvent::Thumbnail { id: *id, thumb });
            }
        });
    }
}

fn fetch_thumb(agent: &ureq::Agent, url: &str) -> Result<super::Thumb, String> {
    let response = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "image/avif,image/webp,image/apng,image/*,*/*;q=0.8")
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(THUMB_MAX_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    audio::make_rounded_thumb_from_bytes(&bytes, THUMB_PX, THUMB_RADIUS)
}