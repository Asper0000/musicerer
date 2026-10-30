//! Deezer metadata resolver.
//!
//! Deezer provides metadata only; audio is always fetched from YouTube
//! or SoundCloud via yt-dlp. Tracks and albums/playlists are both
//! supported, no API key required.

mod fetch;
mod matcher;
mod models;

pub use fetch::{fetch_album_genre, fetch_cover_bytes, search_track};
pub use models::DeezerMeta;

use crate::backend::playlist::PlaylistEntry;
use crate::backend::{audio, emit, log, Inbox, LogLevel, Thumb, WorkerEvent};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use url::Url;

const MATCH_WORKERS: usize = 8;
const THUMB_PX: u32 = 40;
const THUMB_RADIUS: f32 = 8.0;
const FALLBACK_LIST_TITLE: &str = "Deezer playlist";

fn path_segments(url: &Url) -> Vec<&str> {
    url.path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default()
}

fn parse_resource(url: &Url) -> Option<(String, String)> {
    let mut segments = path_segments(url);
    // Locale prefix such as /en/track/..., /fr/album/...
    if segments
        .first()
        .is_some_and(|s| s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic()))
    {
        segments.remove(0);
    }
    let kind = *segments.first()?;
    if !matches!(kind, "track" | "playlist" | "album") {
        return None;
    }
    let id = *segments.get(1)?;
    if !id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((kind.to_string(), id.to_string()))
}

pub fn is_deezer_url(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else { return false; };
    let Some(host) = url.host_str() else { return false; };
    let host = host.to_ascii_lowercase();
    let deezer_host = host == "deezer.com" || host.ends_with(".deezer.com");
    deezer_host && parse_resource(&url).is_some()
}

pub fn is_deezer_playlist_url(raw: &str) -> bool {
    Url::parse(raw)
        .ok()
        .and_then(|url| parse_resource(&url))
        .is_some_and(|(kind, _)| kind != "track")
}

pub fn deezer_id(raw: &str) -> Option<(String, String)> {
    parse_resource(&Url::parse(raw).ok()?)
}

fn rounded_cover(url: Option<&str>) -> Option<Thumb> {
    let bytes = fetch::fetch_cover_bytes(url?).ok()?;
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
                        format!("No audio match for Deezer track \"{}\".", meta.display_title()),
                    );
                }
                emit(&inbox, WorkerEvent::DeezerResolved { id, meta, resolved_url, thumb });
            }
            Err(error) => emit(
                &inbox,
                WorkerEvent::DeezerFailed { id, title: String::new(), error },
            ),
        }
    });
}

pub fn spawn_resolve_playlist(id: u64, url: String, inbox: Inbox, cancel: Arc<AtomicBool>) {
    thread::spawn(move || run_playlist(id, &url, &inbox, &cancel));
}

fn run_playlist(id: u64, url: &str, inbox: &Inbox, cancel: &AtomicBool) {
    let fail = |error: String| {
        emit(inbox, WorkerEvent::DeezerFailed { id, title: String::new(), error })
    };

    let summary = match fetch::fetch_playlist_summary(url) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };

    let title = if summary.title.trim().is_empty() {
        FALLBACK_LIST_TITLE.to_string()
    } else {
        summary.title.trim().to_string()
    };
    let cover = summary.cover_url.clone();
    let track_ids = summary.track_ids;

    // Placeholder entry: pending URL, "Loading…" title, real DeezerMeta
    // so apply_playlist knows this is a Deezer source row.
    let placeholder_meta = DeezerMeta {
        title: "Loading…".to_string(),
        ..Default::default()
    };
    let entries: Vec<PlaylistEntry> = track_ids
        .iter()
        .map(|_| PlaylistEntry {
            url: "deezer:pending".to_string(),
            title: "Loading…".to_string(),
            thumb_url: cover.clone(),
            spotify: None,
            deezer: Some(placeholder_meta.clone()),
        })
        .collect();

    log(
        inbox,
        LogLevel::Info,
        format!(
            "Deezer {}: {} track(s), resolving...",
            summary.kind,
            track_ids.len()
        ),
    );
    emit(
        inbox,
        WorkerEvent::PlaylistResolved { id, title: title.clone(), entries, main: 0 },
    );
    if cancel.load(Ordering::Relaxed) {
        return;
    }

    let workers = MATCH_WORKERS.min(track_ids.len()).max(1);
    let next = AtomicUsize::new(0);

    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let idx = next.fetch_add(1, Ordering::Relaxed);
                let Some(track_id) = track_ids.get(idx) else { break; };

                let meta = match fetch::fetch_track_by_id(track_id) {
                    Ok(m) => m,
                    Err(e) => {
                        log(
                            inbox,
                            LogLevel::Warn,
                            format!("Deezer track {track_id}: {e}"),
                        );
                        emit(
                            inbox,
                            WorkerEvent::DeezerTrackResolved {
                                parent_id: id,
                                entry_index: idx,
                                meta: DeezerMeta {
                                    title: format!("Track {track_id}"),
                                    ..Default::default()
                                },
                                resolved_url: None,
                                thumb: None,
                            },
                        );
                        continue;
                    }
                };

                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let resolved_url = matcher::resolve_track(&meta, cancel).ok();
                let thumb = rounded_cover(meta.cover_url.as_deref());

                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                emit(
                    inbox,
                    WorkerEvent::DeezerTrackResolved {
                        parent_id: id,
                        entry_index: idx,
                        meta,
                        resolved_url,
                        thumb,
                    },
                );
            });
        }
    });

    if cancel.load(Ordering::Relaxed) {
        return;
    }
    log(
        inbox,
        LogLevel::Info,
        format!("Deezer playlist \"{title}\": all tracks resolved."),
    );
    emit(inbox, WorkerEvent::DeezerPlaylistComplete { parent_id: id });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_track_playlist_and_album_urls() {
        assert_eq!(
            deezer_id("https://www.deezer.com/track/3135556"),
            Some(("track".into(), "3135556".into()))
        );
        assert_eq!(
            deezer_id("https://www.deezer.com/en/track/3135556"),
            Some(("track".into(), "3135556".into()))
        );
        assert_eq!(
            deezer_id("https://deezer.com/fr/playlist/908622995"),
            Some(("playlist".into(), "908622995".into()))
        );
        assert_eq!(
            deezer_id("https://www.deezer.com/album/103248"),
            Some(("album".into(), "103248".into()))
        );
    }

    #[test]
    fn rejects_short_links_and_non_deezer_urls() {
        assert_eq!(deezer_id("https://link.deezer.com/s/abc123"), None);
        assert_eq!(deezer_id("https://www.deezer.com/artist/123"), None);
        assert_eq!(deezer_id("https://example.com/track/123"), None);
    }

    #[test]
    fn playlist_urls_include_albums_but_not_tracks() {
        assert!(is_deezer_playlist_url("https://www.deezer.com/playlist/908622995"));
        assert!(is_deezer_playlist_url("https://www.deezer.com/album/103248"));
        assert!(!is_deezer_playlist_url("https://www.deezer.com/track/3135556"));
    }
}