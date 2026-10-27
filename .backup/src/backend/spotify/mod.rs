//! Spotify metadata + resolver layer for Musicerer.
//!
//! Design decisions
//! ----------------
//! * Option B: Spotify Web API (client-credentials) for metadata only;
//!   yt-dlp searches YouTube / SoundCloud for a matching audio stream.
//!   No Songlink/Odesli/MusicFetch or other third-party resolvers.
//! * Spotify never streams audio. The UI row stays Source::Spotify
//!   (green logo) even after a YT/SC URL is resolved underneath.
//! * Matching priority: ISRC (when present) via ytsearch, then
//!   `"artist - title"` on YouTube, then SoundCloud. Duration must be
//!   within 15% or 20s of the Spotify duration (whichever is larger).
//! * Playlists are expanded via the Spotify API (cap 500). Each track
//!   is matched independently; one failure does not block the rest.
//! * Credentials live in the existing AppConfig (Settings page).
//!   Client-credentials OAuth only - no user login.
//! * Soft failure: unresolved tracks become ItemStatus::Failed with a
//!   clear log line that includes the Spotify title.
//! * Existing YouTube / SoundCloud paths are untouched.

pub mod auth;
pub mod api;
mod models;
mod matcher;

pub use models::{SpotifyMeta, SpotifyTrack};
pub use matcher::resolve_track;
pub use api::{fetch_track, fetch_playlist_tracks};

use crate::backend::{detect_source, Source};
use url::Url;

/// True for open.spotify.com / spotify.com track and playlist/album URLs.
pub fn is_spotify_url(raw: &str) -> bool {
    let Ok(u) = Url::parse(raw) else { return false; };
    let Some(host) = u.host_str() else { return false; };
    let host = host.to_ascii_lowercase();
    if host != "open.spotify.com" && host != "spotify.com" && !host.ends_with(".spotify.com") {
        return false;
    }
    let segs: Vec<&str> = u
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    matches!(segs.first().copied(), Some("track") | Some("playlist") | Some("album"))
}

pub fn is_spotify_playlist_url(raw: &str) -> bool {
    let Ok(u) = Url::parse(raw) else { return false; };
    let segs: Vec<&str> = u
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    matches!(segs.first().copied(), Some("playlist") | Some("album"))
}

/// Extract the Spotify resource id from a track/playlist/album URL.
pub fn spotify_id(raw: &str) -> Option<(String, String)> {
    let u = Url::parse(raw).ok()?;
    let segs: Vec<&str> = u
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    if segs.len() < 2 { return None; }
    let kind = segs[0].to_string();
    let id = segs[1].split('?').next()?.to_string();
    if id.is_empty() { return None; }
    Some((kind, id))
}

/// Convenience: does this look like a Spotify URL we should handle?
pub fn detect_spotify(raw: &str) -> bool {
    is_spotify_url(raw) || detect_source(raw) == Source::Spotify
}

use crate::backend::{emit, log, Inbox, LogLevel, Thumb, WorkerEvent};
use crate::backend::audio;
use std::thread;

/// Resolve a single Spotify track URL on a background thread.
pub fn spawn_resolve_track(
    id: u64,
    url: String,
    creds: auth::Credentials,
    inbox: Inbox,
) {
    thread::spawn(move || {
        let result = (|| -> Result<(models::SpotifyMeta, Option<String>, Option<Thumb>), String> {
            let (_, track_id) = spotify_id(&url)
                .filter(|(k, _)| k == "track")
                .ok_or_else(|| "not a Spotify track URL".into())?;
            let track = api::fetch_track(&creds, &track_id)?;
            let mut meta = track.meta;
            let thumb = if let Some(ref cover_url) = meta.cover_url {
                match api::fetch_cover_bytes(cover_url) {
                    Ok(bytes) => {
                        meta.cover_jpeg = Some(bytes.clone());
                        audio::make_rounded_thumb_from_bytes(&bytes, 40, 8.0).ok()
                    }
                    Err(_) => None,
                }
            } else {
                None
            };
            let resolved = matcher::resolve_track(&meta).ok();
            if resolved.is_none() {
                log(
                    &inbox,
                    LogLevel::Warn,
                    format!(
                        "No audio match for Spotify track \"{}\"",
                        meta.display_title()
                    ),
                );
            }
            Ok((meta, resolved, thumb))
        })();

        match result {
            Ok((meta, resolved_url, thumb)) => {
                let title = meta.display_title();
                let had = resolved_url.is_some();
                emit(
                    &inbox,
                    WorkerEvent::SpotifyResolved {
                        id,
                        meta,
                        resolved_url,
                        thumb,
                    },
                );
                if !had {
                    emit(
                        &inbox,
                        WorkerEvent::SpotifyFailed {
                            id,
                            title,
                            error: "no matching YouTube/SoundCloud audio found".into(),
                        },
                    );
                }
            }
            Err(error) => {
                emit(
                    &inbox,
                    WorkerEvent::SpotifyFailed {
                        id,
                        title: String::new(),
                        error,
                    },
                );
            }
        }
    });
}

/// Expand a Spotify playlist/album, resolve each track, and post per-track events.
pub fn spawn_resolve_playlist(
    id: u64,
    url: String,
    creds: auth::Credentials,
    inbox: Inbox,
    next_id_start: u64,
) {
    thread::spawn(move || {
        let parsed = spotify_id(&url);
        let Some((kind, list_id)) = parsed else {
            emit(
                &inbox,
                WorkerEvent::SpotifyFailed {
                    id,
                    title: String::new(),
                    error: "not a Spotify playlist/album URL".into(),
                },
            );
            return;
        };

        let (title, tracks) = match api::fetch_playlist_tracks(&creds, &kind, &list_id) {
            Ok(v) => v,
            Err(error) => {
                emit(
                    &inbox,
                    WorkerEvent::SpotifyFailed {
                        id,
                        title: String::new(),
                        error,
                    },
                );
                return;
            }
        };

        if tracks.is_empty() {
            emit(
                &inbox,
                WorkerEvent::SpotifyFailed {
                    id,
                    title,
                    error: "playlist has no tracks".into(),
                },
            );
            return;
        }

        log(
            &inbox,
            LogLevel::Info,
            format!("Spotify playlist \"{title}\": {} track(s), matching audio...", tracks.len()),
        );

        use crate::backend::playlist::PlaylistEntry;
        let mut entries = Vec::with_capacity(tracks.len());
        let mut metas = Vec::with_capacity(tracks.len());

        for track in tracks {
            let mut meta = track.meta;
            let thumb_url = meta.cover_url.clone();
            let resolved = matcher::resolve_track(&meta).ok();
            if resolved.is_none() {
                log(
                    &inbox,
                    LogLevel::Warn,
                    format!(
                        "No audio match for \"{}\" - will be marked failed.",
                        meta.display_title()
                    ),
                );
            }
            if let Some(ref cu) = meta.cover_url {
                if let Ok(bytes) = api::fetch_cover_bytes(cu) {
                    meta.cover_jpeg = Some(bytes);
                }
            }
            let entry_url = resolved
                .clone()
                .unwrap_or_else(|| format!("spotify:track:{}", track.id));
            entries.push(PlaylistEntry {
                url: entry_url,
                title: meta.display_title(),
                thumb_url,
            });
            metas.push((meta, resolved));
        }

        emit(
            &inbox,
            WorkerEvent::PlaylistResolved {
                id,
                title: title.clone(),
                entries,
                main: 0,
            },
        );

        for (i, (meta, resolved)) in metas.into_iter().enumerate() {
            let row_id = if i == 0 { id } else { next_id_start + (i as u64 - 1) };
            let thumb = meta
                .cover_jpeg
                .as_deref()
                .and_then(|b| audio::make_rounded_thumb_from_bytes(b, 40, 8.0).ok());
            let title = meta.display_title();
            let had = resolved.is_some();
            emit(
                &inbox,
                WorkerEvent::SpotifyResolved {
                    id: row_id,
                    meta,
                    resolved_url: resolved,
                    thumb,
                },
            );
            if !had {
                emit(
                    &inbox,
                    WorkerEvent::SpotifyFailed {
                        id: row_id,
                        title,
                        error: "no matching YouTube/SoundCloud audio found".into(),
                    },
                );
            }
        }
    });
}