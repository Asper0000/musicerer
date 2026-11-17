//! Tag writing: reads yt-dlp's info.json, picks the best title/artist/cover,
//! and writes the tags. When Spotify or Deezer metadata is present, its title,
//! artists, album and cover win; the download backend only fills in what the
//! metadata source did not supply.

mod cover;
mod merge;
mod native;
mod write;

use crate::backend::deezer::{self, DeezerMeta};
use crate::backend::enrich::Enrichment;
use crate::backend::itunes::{self, ITunesMeta};
use crate::backend::soundcloud_oembed;
use crate::backend::soundcloud_meta;
use crate::backend::spotify::{self, SpotifyMeta};
use crate::backend::{audio, detect_source, emit, log, Inbox, LogLevel, Source, WorkerEvent};
use audiotags::Tag;
use merge::{ExistingTags, MergeInput};
use std::path::Path;

/// Tags the file in place and returns the final display title.
///
/// `url` identifies the track whose metadata is written. `stand_in_audio` is true
/// when the audio was downloaded from a different upload (a SoundCloud track
/// served from YouTube): that file's own info.json, tags and thumbnail describe
/// the other upload, so they are ignored.
#[allow(clippy::too_many_arguments)]
pub fn tag_audio(
    audio_path: &Path,
    url: &str,
    stand_in_audio: bool,
    inbox: &Inbox,
    id: u64,
    label: &str,
    spotify: Option<&SpotifyMeta>,
    deezer: Option<&DeezerMeta>,
    itunes: Option<&ITunesMeta>,
    enrichment: Option<&Enrichment>,
) -> String {
    let native = if stand_in_audio { native::NativeMeta::empty() } else { native::extract(audio_path) };
    let source = detect_source(url);

    // Collect the existing tags as one more (lowest-priority) source.
    let existing_owned = if native.info_missing() && !stand_in_audio {
        read_existing_tags(audio_path)
    } else {
        None
    };

    // SoundCloud hydration: authoritative for real artist / album /
    // release year on label uploads. The cache is checked first because
    // enrich::for_download (which ran earlier in download_one) already
    // fetched the same URL for SC-sourced audio - reuse it for free.
    let sc_owned = if source == Source::SoundCloud {
        match soundcloud_meta::fetch_cached(url) {
            Ok(sc) => Some(sc),
            Err(e) => {
                log(
                    inbox,
                    LogLevel::Info,
                    format!("[{label}] SoundCloud hydration: {e}"),
                );
                None
            }
        }
    } else {
        None
    };

    // oEmbed is only meaningful for SoundCloud sources. On any other source
    // the request is a guaranteed round-trip that fails, so skip it.
    let (oembed_title, oembed_cover) = if source == Source::SoundCloud {
        match soundcloud_oembed::fetch_oembed(url) {
            Ok((t, maybe_thumb)) => {
                let cover = maybe_thumb
                    .as_deref()
                    .and_then(|bytes| audio::square_cover_from_bytes(bytes).ok());
                (Some(t), cover)
            }
            Err(_) => (None, None),
        }
    } else {
        (None, None)
    };

    // Metadata-source cover bytes (downloaded here so the merge can pick
    // the correct precedence without extra HTTP inside write_tags).
    let spotify_cover = spotify
        .and_then(|sp| sp.cover_url.as_deref())
        .and_then(|u| spotify::fetch_cover_bytes(u).ok())
        .and_then(|bytes| audio::square_cover_from_bytes(&bytes).ok());
    let deezer_cover = deezer
        .and_then(|dz| dz.cover_url.as_deref())
        .and_then(|u| deezer::fetch_cover_bytes(u).ok())
        .and_then(|bytes| audio::square_cover_from_bytes(&bytes).ok());
    let itunes_cover = itunes
        .and_then(|it| it.cover_url.as_deref())
        .and_then(|u| itunes::fetch_cover_bytes(u).ok())
        .and_then(|bytes| audio::square_cover_from_bytes(&bytes).ok());

    let disk_cover = if stand_in_audio { None } else { cover::from_disk(audio_path, inbox, label) };

    let fallback_filename = audio_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string());

    let existing_ref = existing_owned.as_ref();
    let input = MergeInput {
        native: &native,
        existing: existing_ref,
        oembed_title,
        sc: sc_owned.as_ref(),
        enrichment,
        spotify,
        deezer,
        itunes,
        disk_cover,
        oembed_cover,
        spotify_cover,
        deezer_cover,
        itunes_cover,
        fallback_filename,
    };
    let merged = merge::merge(&input);

    // Do not push a yt-dlp title into the UI when a metadata source owns this row.
    if spotify.is_none() && deezer.is_none() && itunes.is_none() {
        emit(
            inbox,
            WorkerEvent::TrackTitle {
                id,
                title: merged.title.clone(),
            },
        );
    }

    if let Err(err) = write::write_tags(audio_path, &merged) {
        log(
            inbox,
            LogLevel::Warn,
            format!("[{label}] Metadata write failed (audio still saved): {err}"),
        );
    }

    merged.title
}

/// Reads whatever tags the file already carries. Only queried when
/// native info.json is missing — otherwise native is already a richer
/// (and higher-priority) source and re-reading the file just to discard
/// the values is wasted work.
fn read_existing_tags(audio_path: &Path) -> Option<ExistingTags> {
    let tag = Tag::new().read_from_path(audio_path).ok()?;
    Some(ExistingTags {
        title: tag.title().map(|s| s.to_string()),
        artist: tag.artist().map(|s| s.to_string()),
        album: tag.album().map(|album| album.title.to_string()),
        cover: tag.album_cover().map(|pic| pic.data.to_vec()),
    })
}

/// Identity for enrichment: (title, artist, duration_ms, source). Reads
/// native info.json fields; returns None when title or artist is empty,
/// which prevents a title-only lookup from matching on generic words.
pub fn enrichment_identity(
    audio_path: &Path,
    source: Source,
) -> Option<(String, String, Option<u64>, Source)> {
    let native = native::extract(audio_path);
    let title = native.title?;
    let artist = native.artist?;
    if title.trim().is_empty() || artist.trim().is_empty() {
        return None;
    }
    let duration_ms = read_info_duration(audio_path);
    Some((title, artist, duration_ms, source))
}

fn read_info_duration(audio_path: &Path) -> Option<u64> {
    let info_path = audio_path.with_extension("info.json");
    let text = std::fs::read_to_string(info_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("duration")
        .and_then(|d| d.as_f64())
        .map(|s| (s * 1000.0).round() as u64)
}