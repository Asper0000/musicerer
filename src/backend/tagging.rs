//! Reads yt-dlp's info.json, picks the best title/artist/cover and writes the tags.
//! When Spotify metadata is present, its title, artists and cover win; the download
//! backend only fills in what Spotify did not supply (album, year, track number,
//! and any field Spotify left empty).

use super::spotify::{self, SpotifyMeta};
use super::{audio, emit, log, soundcloud_fallback, Inbox, LogLevel, WorkerEvent};
use audiotags::{MimeType, Picture, Tag};
use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Tags the file in place and returns the final display title.
pub fn tag_audio(
    audio_path: &Path,
    url: &str,
    inbox: &Inbox,
    id: u64,
    label: &str,
    spotify: Option<&SpotifyMeta>,
) -> String {
    let info = read_info(audio_path);
    let mut title = info
        .as_ref()
        .and_then(|i| i.track.clone().or_else(|| i.title.clone()));
    let mut artist = info.as_ref().and_then(|i| {
        i.artist
            .clone()
            .or_else(|| i.uploader.clone())
            .or_else(|| i.channel.clone())
    });
    let mut album = info.as_ref().and_then(|i| i.album.clone());

    let mut cover_bytes = match audio::find_thumbnail(audio_path) {
        Some(thumb) => match audio::square_cover(&thumb) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                log(
                    inbox,
                    LogLevel::Warn,
                    format!("[{label}] Could not crop cover art: {err}"),
                );
                None
            }
        },
        None => None,
    };

    if info.is_none() {
        if let Ok(tag) = Tag::new().read_from_path(audio_path) {
            if title.is_none() {
                title = tag.title().map(|s| s.to_string());
            }
            if artist.is_none() {
                artist = tag.artist().map(|s| s.to_string());
            }
            if album.is_none() {
                album = tag.album_title().map(|s| s.to_string());
            }
            if cover_bytes.is_none() {
                if let Some(pic) = tag.album_cover() {
                    cover_bytes = Some(pic.data.to_vec());
                }
            }
        }
    }

    if title.is_none() || cover_bytes.is_none() {
        if let Ok((t, thumbnail)) = soundcloud_fallback::fetch_oembed(url) {
            if title.is_none() {
                title = Some(t);
            }
            if cover_bytes.is_none() {
                if let Some(bytes) = thumbnail {
                    if let Ok(square_cover) = audio::square_cover_from_bytes(&bytes) {
                        cover_bytes = Some(square_cover);
                    }
                }
            }
        }
    }

    // Spotify wins for title, artist and cover whenever it has them.
    if let Some(sp) = spotify {
        if !sp.title.trim().is_empty() {
            title = Some(sp.title.trim().to_string());
        }
        if !sp.artists.trim().is_empty() {
            artist = Some(sp.artists.trim().to_string());
        }
        let spotify_cover = sp
            .cover_url
            .as_deref()
            .and_then(|u| spotify::fetch_cover_bytes(u).ok())
            .and_then(|bytes| audio::square_cover_from_bytes(&bytes).ok());
        if spotify_cover.is_some() {
            cover_bytes = spotify_cover;
        }
    }

    let final_title = title.unwrap_or_else(|| {
        audio_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Unknown title".to_string())
    });

    // Do not push a YT title into the UI when Spotify already owns this row.
    if spotify.is_none() {
        emit(
            inbox,
            WorkerEvent::TrackTitle {
                id,
                title: final_title.clone(),
            },
        );
    }

    let final_album = album.unwrap_or_default();
    let year = info.as_ref().and_then(|i| {
        i.release_date
            .as_deref()
            .or(i.upload_date.as_deref())
            .and_then(|d| d.get(0..4))
            .and_then(|y| y.parse::<i32>().ok())
    });
    let track_number = info.as_ref().and_then(|i| i.track_number.map(|n| n as i32));

    if let Err(err) = write_tags(
        audio_path,
        &final_title,
        artist.as_deref(),
        &final_album,
        year,
        track_number,
        cover_bytes.as_deref(),
    ) {
        log(
            inbox,
            LogLevel::Warn,
            format!("[{label}] Metadata write failed (audio still saved): {err}"),
        );
    }

    final_title
}

#[derive(Debug, Default, Deserialize)]
struct TrackInfo {
    title: Option<String>,
    track: Option<String>,
    artist: Option<String>,
    uploader: Option<String>,
    channel: Option<String>,
    album: Option<String>,
    release_date: Option<String>,
    upload_date: Option<String>,
    track_number: Option<i64>,
}

fn read_info(audio_path: &Path) -> Option<TrackInfo> {
    let info_path = audio_path.with_extension("info.json");
    let text = fs::read_to_string(info_path).ok()?;
    serde_json::from_str::<TrackInfo>(&text).ok()
}

fn write_tags(
    audio_path: &Path,
    title: &str,
    artist: Option<&str>,
    album: &str,
    year: Option<i32>,
    track_number: Option<i32>,
    cover: Option<&[u8]>,
) -> Result<(), String> {
    let path_str = audio_path.to_string_lossy().to_string();
    let mut tag = Tag::new()
        .read_from_path(&path_str)
        .map_err(|e| e.to_string())?;
    tag.set_title(title);
    if let Some(artist) = artist.filter(|a| !a.trim().is_empty()) {
        tag.set_artist(artist);
    }
    if !album.trim().is_empty() {
        tag.set_album_title(album);
    }
    if let Some(year) = year {
        tag.set_year(year);
    }
    if let Some(track) = track_number.filter(|n| *n > 0) {
        tag.set_track_number(track as u16);
    }
    if let Some(bytes) = cover {
        tag.set_album_cover(Picture {
            mime_type: MimeType::Jpeg,
            data: bytes,
        });
    }
    tag.write_to_path(&path_str).map_err(|e| e.to_string())
}
