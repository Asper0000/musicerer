mod fetch;
mod models;

pub use fetch::{fetch_cover_bytes, fetch_playlist_metas, fetch_track_meta, search_track};
pub use models::ITunesMeta;

use crate::backend::playlist::PlaylistEntry;
use crate::backend::{audio, emit, log, Inbox, LogLevel, Thumb, WorkerEvent};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use url::Url;

const MATCH_WORKERS: usize = 4; // Keep low due to Apple limits
const THUMB_PX: u32 = 40;
const THUMB_RADIUS: f32 = 8.0;
const FALLBACK_LIST_TITLE: &str = "Apple Music Album";
const MAX_LISTED_UNMATCHED: usize = 5;

fn path_segments(url: &Url) -> Vec<&str> {
    url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default()
}

fn parse_resource(url: &Url) -> Option<(String, String, Option<String>)> {
    let segments = path_segments(url);
    let kind = *segments.get(1)?;
    
    // URL formats:
    // /us/album/album-name/123456?i=789012
    // /us/album/123456
    let mut id = String::new();
    if kind == "album" || kind == "playlist" {
        id = segments.get(3)?.to_string();
        // If there's an extra segment (the album name), the ID is at index 4
        if id.chars().any(|c| !c.is_ascii_digit()) {
            id = segments.get(4)?.to_string();
        }
    } else if kind == "song" {
        id = segments.get(3)?.to_string();
    }

    let mut track_id = None;
    for (k, v) in url.query_pairs() {
        if k == "i" {
            track_id = Some(v.into_owned());
        }
    }

    let final_kind = if track_id.is_some() { "track".to_string() } else { "album".to_string() };
    Some((final_kind, id, track_id))
}

pub fn is_itunes_url(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else { return false; };
    let Some(host) = url.host_str() else { return false; };
    let host = host.to_ascii_lowercase();
    let is_apple = host == "music.apple.com" || host == "itunes.apple.com";
    is_apple && parse_resource(&url).is_some()
}

pub fn is_itunes_playlist_url(raw: &str) -> bool {
    Url::parse(raw).ok().and_then(|url| parse_resource(&url)).is_some_and(|(kind, _, _)| kind != "track")
}

pub fn itunes_id(raw: &str) -> Option<(String, String, Option<String>)> {
    parse_resource(&Url::parse(raw).ok()?)
}

fn rounded_cover(url: Option<&str>) -> Option<Thumb> {
    let bytes = fetch_cover_bytes(url?).ok()?;
    audio::make_rounded_thumb_from_bytes(&bytes, THUMB_PX, THUMB_RADIUS).ok()
}

pub fn spawn_resolve_track(id: u64, url: String, inbox: Inbox, cancel: Arc<AtomicBool>) {
    thread::spawn(move || {
        let result = fetch_track_meta(&url).map(|meta| {
            let (thumb, resolved) = thread::scope(|scope| {
                let cover = scope.spawn(|| rounded_cover(meta.cover_url.as_deref()));
                let resolved = crate::backend::search_score::resolve(&meta.title, &meta.artists, meta.duration_ms, &cancel).ok();
                (cover.join().ok().flatten(), resolved)
            });
            (meta, resolved, thumb)
        });
        if cancel.load(Ordering::Relaxed) { return; }
        
        match result {
            Ok((meta, resolved_url, thumb)) => {
                if resolved_url.is_none() {
                    log(&inbox, LogLevel::Warn, format!("No audio match for iTunes track \"{}\".", meta.display_title()));
                }
                emit(&inbox, WorkerEvent::ITunesResolved { id, meta, resolved_url, thumb });
            }
            Err(error) => emit(&inbox, WorkerEvent::ITunesFailed { id, title: String::new(), error }),
        }
    });
}

pub fn spawn_resolve_playlist(id: u64, url: String, inbox: Inbox, cancel: Arc<AtomicBool>) {
    thread::spawn(move || run_playlist(id, &url, &inbox, &cancel));
}

fn run_playlist(id: u64, url: &str, inbox: &Inbox, cancel: &AtomicBool) {
    let fail = |error: String| emit(inbox, WorkerEvent::ITunesFailed { id, title: String::new(), error });
    
    let tracks = match fetch_playlist_metas(url) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };

    let title = tracks.first().and_then(|t| t.album.clone()).unwrap_or_else(|| FALLBACK_LIST_TITLE.to_string());
    // Cover art is handled per-track by iTunes metadata

    let entries: Vec<PlaylistEntry> = tracks.iter().map(|m| PlaylistEntry {
        url: "itunes:pending".to_string(),
        title: m.display_title(),
        thumb_url: m.cover_url.clone(),
        spotify: None,
        deezer: None,
        itunes: Some(m.clone()),
    }).collect();

    log(inbox, LogLevel::Info, format!("iTunes Album: {} track(s), matching audio...", tracks.len()));
    emit(inbox, WorkerEvent::PlaylistResolved { id, title: title.clone(), entries, main: 0 });
    
    if cancel.load(Ordering::Relaxed) { return; }

    let unmatched = match_and_emit(id, &tracks, inbox, cancel);
    if cancel.load(Ordering::Relaxed) { return; }

    log_match_summary(inbox, &title, tracks.len(), &unmatched);
    emit(inbox, WorkerEvent::ITunesPlaylistComplete { parent_id: id });
}

fn match_and_emit(id: u64, metas: &[ITunesMeta], inbox: &Inbox, cancel: &AtomicBool) -> Vec<String> {
    let workers = MATCH_WORKERS.min(metas.len()).max(1);
    let next = AtomicUsize::new(0);
    let unmatched = Mutex::new(Vec::new());
    
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if cancel.load(Ordering::Relaxed) { break; }
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(meta) = metas.get(index) else { break; };
                
                let result = crate::backend::search_score::resolve(&meta.title, &meta.artists, meta.duration_ms, cancel);
                if cancel.load(Ordering::Relaxed) { break; }
                
                match result {
                    Ok(url) => emit(inbox, WorkerEvent::ITunesTrackMatched { parent_id: id, entry_index: index, url: Some(url) }),
                    Err(_) => {
                        unmatched.lock().unwrap_or_else(|e| e.into_inner()).push(meta.display_title());
                        emit(inbox, WorkerEvent::ITunesTrackMatched { parent_id: id, entry_index: index, url: None });
                    }
                }
            });
        }
    });
    unmatched.into_inner().unwrap_or_else(|e| e.into_inner())
}

fn log_match_summary(inbox: &Inbox, title: &str, total: usize, unmatched: &[String]) {
    if unmatched.is_empty() {
        log(inbox, LogLevel::Info, format!("iTunes Album \"{title}\": all {total} track(s) matched."));
        return;
    }
    let listed = unmatched.iter().take(MAX_LISTED_UNMATCHED).cloned().collect::<Vec<_>>().join("; ");
    let more = unmatched.len().saturating_sub(MAX_LISTED_UNMATCHED);
    let tail = if more > 0 { format!(" (+{more} more)") } else { String::new() };
    log(inbox, LogLevel::Warn, format!("iTunes Album \"{title}\": {} of {total} track(s) could not be matched: {listed}{tail}.", unmatched.len()));
}