//! Spotify metadata resolver for Musicerer (no official Web API).

mod fetch;
mod matcher;
mod models;

pub use models::SpotifyMeta;

use crate::backend::playlist::PlaylistEntry;
use crate::backend::{audio, emit, log, Inbox, LogLevel, Thumb, WorkerEvent};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use url::Url;

pub const PUBLIC_PAGE_LIMIT: usize = 100;

const MATCH_WORKERS: usize = 8;
const THUMB_PX: u32 = 40;
const THUMB_RADIUS: f32 = 8.0;
const FALLBACK_LIST_TITLE: &str = "Spotify playlist";
const MAX_LISTED_UNMATCHED: usize = 5;

fn path_segments(url: &Url) -> Vec<&str> {
    url.path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default()
}

fn parse_resource(url: &Url) -> Option<(String, String)> {
    let mut segments = path_segments(url);
    if segments.first().is_some_and(|s| s.starts_with("intl-")) {
        segments.remove(0);
    }
    let kind = *segments.first()?;
    if !matches!(kind, "track" | "playlist" | "album") {
        return None;
    }
    let id = *segments.get(1)?;
    Some((kind.to_string(), id.to_string()))
}

pub fn is_spotify_url(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    let spotify_host =
        host == "open.spotify.com" || host == "spotify.com" || host.ends_with(".spotify.com");
    spotify_host && parse_resource(&url).is_some()
}

pub fn is_spotify_playlist_url(raw: &str) -> bool {
    Url::parse(raw)
        .ok()
        .and_then(|url| parse_resource(&url))
        .is_some_and(|(kind, _)| kind != "track")
}

pub fn spotify_id(raw: &str) -> Option<(String, String)> {
    parse_resource(&Url::parse(raw).ok()?)
}

pub fn fetch_cover_bytes(url: &str) -> Result<Vec<u8>, String> {
    fetch::fetch_bytes(url)
}

fn rounded_cover(url: Option<&str>) -> Option<Thumb> {
    let bytes = fetch::fetch_bytes(url?).ok()?;
    audio::make_rounded_thumb_from_bytes(&bytes, THUMB_PX, THUMB_RADIUS).ok()
}

pub fn spawn_resolve_track(id: u64, url: String, inbox: Inbox, cancel: Arc<AtomicBool>) {
    thread::spawn(move || {
        let result = fetch::fetch_track_meta(&url).map(|meta| {
            let (thumb, resolved) = thread::scope(|scope| {
                let cover = scope.spawn(|| rounded_cover(meta.cover_url.as_deref()));
                let resolved = matcher::resolve_track(&meta, &cancel).ok();
                (cover.join().ok().flatten(), resolved)
            });
            (meta, resolved, thumb)
        });
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        match result {
            Ok((meta, resolved_url, thumb)) => {
                if resolved_url.is_none() {
                    log(
                        &inbox,
                        LogLevel::Warn,
                        format!(
                            "No audio match for Spotify track \"{}\".",
                            meta.display_title()
                        ),
                    );
                }
                emit(
                    &inbox,
                    WorkerEvent::SpotifyResolved {
                        id,
                        meta,
                        resolved_url,
                        thumb,
                    },
                );
            }
            Err(error) => emit(
                &inbox,
                WorkerEvent::SpotifyFailed {
                    id,
                    title: String::new(),
                    error,
                },
            ),
        }
    });
}

pub fn spawn_resolve_playlist(id: u64, url: String, inbox: Inbox, cancel: Arc<AtomicBool>) {
    thread::spawn(move || run_playlist(id, &url, &inbox, &cancel));
}

fn run_playlist(id: u64, url: &str, inbox: &Inbox, cancel: &AtomicBool) {
    let fail = |error: String| {
        emit(
            inbox,
            WorkerEvent::SpotifyFailed {
                id,
                title: String::new(),
                error,
            },
        )
    };
    let mut tracks = match fetch::fetch_playlist_metas(url) {
        Ok(tracks) if !tracks.is_empty() => tracks,
        Ok(_) => return fail("playlist has no tracks (public embed returned empty)".into()),
        Err(error) => return fail(error),
    };
    let list_url = url.to_string();
    let list_meta = thread::spawn(move || fetch::fetch_track_meta(&list_url).ok());
    let list_meta = list_meta.join().ok().flatten();
    let title = list_meta
        .as_ref()
        .map(|m| m.title.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| FALLBACK_LIST_TITLE.to_string());
    let shared_cover = tracks
        .iter()
        .find_map(|t| t.cover_url.clone())
        .or_else(|| list_meta.as_ref().and_then(|m| m.cover_url.clone()));
    for t in &mut tracks {
        if t.cover_url.is_none() {
            t.cover_url = shared_cover.clone();
        }
    }

    let entries: Vec<PlaylistEntry> = tracks
        .iter()
        .map(|m| PlaylistEntry {
            url: "spotify:pending".to_string(),
            title: m.display_title(),
            thumb_url: m.cover_url.clone(),
            spotify: Some(m.clone()),
        })
        .collect();
    log(
        inbox,
        LogLevel::Info,
        format!(
            "Spotify playlist: {} track(s), matching audio...",
            tracks.len()
        ),
    );
    emit(
        inbox,
        WorkerEvent::PlaylistResolved {
            id,
            title: title.clone(),
            entries,
            main: 0,
        },
    );
    if cancel.load(Ordering::Relaxed) {
        return;
    }

    let unmatched = match_and_emit(id, &tracks, inbox, cancel);
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    log_match_summary(inbox, &title, tracks.len(), &unmatched);
    emit(
        inbox,
        WorkerEvent::SpotifyPlaylistComplete { parent_id: id },
    );
}

fn match_and_emit(
    id: u64,
    metas: &[SpotifyMeta],
    inbox: &Inbox,
    cancel: &AtomicBool,
) -> Vec<String> {
    let workers = MATCH_WORKERS.min(metas.len()).max(1);
    let next = AtomicUsize::new(0);
    let unmatched = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(meta) = metas.get(index) else {
                    break;
                };
                let result = matcher::resolve_track(meta, cancel);
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                match result {
                    Ok(url) => emit(
                        inbox,
                        WorkerEvent::SpotifyTrackMatched {
                            parent_id: id,
                            entry_index: index,
                            url: Some(url),
                        },
                    ),
                    Err(_) => {
                        unmatched
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(meta.display_title());
                        emit(
                            inbox,
                            WorkerEvent::SpotifyTrackMatched {
                                parent_id: id,
                                entry_index: index,
                                url: None,
                            },
                        );
                    }
                }
            });
        }
    });
    unmatched.into_inner().unwrap_or_else(|e| e.into_inner())
}

fn log_match_summary(inbox: &Inbox, title: &str, total: usize, unmatched: &[String]) {
    if unmatched.is_empty() {
        log(
            inbox,
            LogLevel::Info,
            format!("Spotify playlist \"{title}\": all {total} track(s) matched."),
        );
        return;
    }
    let listed = unmatched
        .iter()
        .take(MAX_LISTED_UNMATCHED)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    let more = unmatched.len().saturating_sub(MAX_LISTED_UNMATCHED);
    let tail = if more > 0 {
        format!(" (+{more} more)")
    } else {
        String::new()
    };
    log(inbox, LogLevel::Warn,
        format!("Spotify playlist \"{title}\": {} of {total} track(s) could not be matched: {listed}{tail}.", unmatched.len()));
}
