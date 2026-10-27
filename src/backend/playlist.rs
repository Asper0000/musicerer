//! Playlist support: detecting playlist URLs, expanding them into tracks with
//! `yt-dlp --flat-playlist` (one fast call, no per-track process), and loading
//! the track thumbnails on a small pool of threads.

use super::tools::{self, Tool};
use super::utils::USER_AGENT;
use super::{audio, detect_source, emit, log, Inbox, LogLevel, Source, WorkerEvent};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Hard cap so pasting a huge playlist cannot flood the queue.
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
    /// Set for Spotify playlist/album tracks: Spotify owns title, artist and cover.
    pub spotify: Option<crate::backend::spotify::SpotifyMeta>,
}

struct Playlist {
    title: String,
    entries: Vec<PlaylistEntry>,
    main: usize,
}

/// True for YouTube playlists (`list=` present) and SoundCloud sets (`/sets/<name>`).
/// YouTube "Mix" radios (`list=RD…`) are endless and auto-generated, so they are
/// treated as the single video they were opened from.
pub fn is_playlist_url(url: &str, source: Source) -> bool {
    let Ok(parsed) = url::Url::parse(url) else { return false; };
    match source {
        Source::YouTube => parsed
            .query_pairs()
            .any(|(k, v)| k == "list" && !v.is_empty() && !v.starts_with("RD")),
        Source::SoundCloud => {
            let segments: Vec<&str> = parsed
                .path_segments()
                .map(|s| s.filter(|p| !p.is_empty()).collect())
                .unwrap_or_default();
            segments
                .iter()
                .position(|s| *s == "sets")
                .is_some_and(|i| i + 1 < segments.len())
        }
        Source::Spotify => false, // handled by backend::spotify
        Source::Unsupported => false,
    }
}

/// The video the user pasted, so it can be shown as the playlist's main row.
fn youtube_video_id(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if parsed.host_str().is_some_and(|h| h.eq_ignore_ascii_case("youtu.be")) {
        return parsed.path_segments()?.find(|s| !s.is_empty()).map(String::from);
    }
    parsed.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.into_owned())
}

pub fn spawn_resolve(id: u64, url: String, cookies: Option<PathBuf>, inbox: Inbox) {
    thread::spawn(move || {
        match fetch_playlist(&url, cookies.as_deref(), &inbox) {
            Ok(p) => emit(
                &inbox,
                WorkerEvent::PlaylistResolved { id, title: p.title, entries: p.entries, main: p.main },
            ),
            Err(error) => emit(&inbox, WorkerEvent::PlaylistFailed { id, error }),
        }
    });
}

fn fetch_playlist(url: &str, cookies: Option<&Path>, inbox: &Inbox) -> Result<Playlist, String> {
    let mut cmd = tools::command(Tool::YtDlp);
    cmd.arg("--flat-playlist")
        .arg("--dump-single-json")
        .arg("--ignore-errors")
        .arg("--no-warnings")
        .arg("--no-color")
        .arg("--playlist-end").arg(MAX_PLAYLIST_ENTRIES.to_string());
    if let Some(c) = cookies { cmd.arg("--cookies").arg(c); }
    cmd.arg(url);

    let output = cmd.output().map_err(|e| format!("could not start yt-dlp: {e}"))?;
    if output.stdout.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
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

    let main = youtube_video_id(url)
        .and_then(|vid| entries.iter().position(|e| e.url.contains(&vid)))
        .unwrap_or(0);
    let title = root.get("title").and_then(Value::as_str).unwrap_or("Playlist").to_string();
    Ok(Playlist { title, entries, main })
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
        (Source::YouTube, Some(id)) => Some(format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")),
        _ => pick_thumbnail(entry),
    };

    Some(PlaylistEntry { url, title: title.to_string(), thumb_url, spotify: None })
}

/// Smallest image that is still at least 120px wide; otherwise whatever is last.
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

/// Loads thumbnails for `(track id, image url)` pairs, in order, on a few threads.
/// Each result (or failure) is posted as a `Thumbnail` event.
pub fn spawn_thumbnails(jobs: Vec<(u64, String)>, inbox: Inbox) {
    if jobs.is_empty() { return; }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_youtube_playlists() {
        let yt = Source::YouTube;
        assert!(is_playlist_url("https://www.youtube.com/playlist?list=PL123", yt));
        assert!(is_playlist_url("https://www.youtube.com/watch?v=abc&list=PL123&index=2", yt));
        assert!(is_playlist_url("https://youtu.be/abc?list=OLAK5uy_x", yt));
        assert!(!is_playlist_url("https://www.youtube.com/watch?v=abc", yt));
        assert!(!is_playlist_url("https://www.youtube.com/watch?v=abc&list=RDabc", yt), "mixes are not playlists");
        assert!(!is_playlist_url("https://www.youtube.com/watch?v=abc&list=", yt));
    }

    #[test]
    fn detects_soundcloud_sets() {
        let sc = Source::SoundCloud;
        assert!(is_playlist_url("https://soundcloud.com/artist/sets/my-album", sc));
        assert!(is_playlist_url("https://soundcloud.com/artist/sets/private-one/s-AbCd", sc));
        assert!(is_playlist_url("https://soundcloud.com/artist/sets/my-album?si=x", sc));
        assert!(!is_playlist_url("https://soundcloud.com/artist/sets", sc), "bare sets listing is not one set");
        assert!(!is_playlist_url("https://soundcloud.com/artist/a-track", sc));
        assert!(!is_playlist_url("https://soundcloud.com/artist/a-track?in=artist/sets/my-album", sc));
    }

    #[test]
    fn extracts_main_video_id() {
        assert_eq!(youtube_video_id("https://www.youtube.com/watch?v=abc123&list=PL1").as_deref(), Some("abc123"));
        assert_eq!(youtube_video_id("https://youtu.be/abc123?list=PL1").as_deref(), Some("abc123"));
        assert_eq!(youtube_video_id("https://www.youtube.com/playlist?list=PL1"), None);
    }

    #[test]
    fn parses_youtube_flat_entry() {
        let e = json!({"id":"abc123","title":"Song","url":"https://www.youtube.com/watch?v=abc123"});
        let p = parse_entry(&e).unwrap();
        assert_eq!(p.url, "https://www.youtube.com/watch?v=abc123");
        assert_eq!(p.thumb_url.as_deref(), Some("https://i.ytimg.com/vi/abc123/mqdefault.jpg"));
    }

    #[test]
    fn builds_url_from_bare_id() {
        let e = json!({"id":"xyz","title":"Song"});
        assert_eq!(parse_entry(&e).unwrap().url, "https://www.youtube.com/watch?v=xyz");
    }

    #[test]
    fn skips_private_deleted_and_foreign_entries() {
        assert!(parse_entry(&json!({"id":"a","title":"[Private video]"})).is_none());
        assert!(parse_entry(&json!({"id":"a","title":"[Deleted video]"})).is_none());
        assert!(parse_entry(&json!({"title":"x","url":"https://example.com/a.mp3"})).is_none());
    }

    #[test]
    fn soundcloud_entry_prefers_permalink_and_right_sized_thumb() {
        let e = json!({
            "title": "Track",
            "webpage_url": "https://soundcloud.com/a/track",
            "url": "https://api.soundcloud.com/tracks/1",
            "thumbnails": [
                {"url": "https://i1.sndcdn.com/t500.jpg", "width": 500},
                {"url": "https://i1.sndcdn.com/t200.jpg", "width": 200},
                {"url": "https://i1.sndcdn.com/t20.jpg", "width": 20}
            ]
        });
        let p = parse_entry(&e).unwrap();
        assert_eq!(p.url, "https://soundcloud.com/a/track");
        assert_eq!(p.thumb_url.as_deref(), Some("https://i1.sndcdn.com/t200.jpg"));
    }
}
