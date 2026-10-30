//! One-hop cross-backend metadata enrichment.

use crate::backend::deezer::{self, DeezerMeta};
use crate::backend::search_score;
use crate::backend::soundcloud_meta::{self, SoundCloudMeta};
use crate::backend::{log, Inbox, LogLevel, Source};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Default)]
pub struct Enrichment {
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub genre: Option<String>,
    pub label: Option<String>,
    pub isrc: Option<String>,
    pub bpm: Option<f32>,
    pub gain: Option<f32>,
}

impl Enrichment {
    pub fn is_empty(&self) -> bool {
        self.album.is_none()
            && self.album_artist.is_none()
            && self.year.is_none()
            && self.track_number.is_none()
            && self.disc_number.is_none()
            && self.genre.is_none()
            && self.label.is_none()
            && self.isrc.is_none()
            && self.bpm.is_none()
            && self.gain.is_none()
    }
}

pub struct TrackIdentity<'a> {
    pub title: &'a str,
    pub search_title: &'a str,
    pub search_duration: Option<u64>,
    pub artist: &'a str,
    pub duration_ms: Option<u64>,
    pub meta_source: Source,
    pub audio_source: Source,
    pub audio_url: &'a str,
}

#[allow(clippy::too_many_arguments)]
pub fn for_download(
    audio_path: &std::path::Path,
    audio_url: &str,
    job_spotify: Option<&crate::backend::spotify::SpotifyMeta>,
    job_deezer: Option<&DeezerMeta>,
    audio_source: Source,
    inbox: &Inbox,
    label: &str,
    cancel: &AtomicBool,
) -> Option<Enrichment> {
    let (title, artist, duration_ms, meta_source) = if let Some(dz) = job_deezer {
        (dz.title.clone(), dz.artists.clone(), dz.duration_ms, Source::Deezer)
    } else if let Some(sp) = job_spotify {
        (sp.title.clone(), sp.artists.clone(), sp.duration_ms, Source::Spotify)
    } else {
        let (t, a, d, s) =
            crate::backend::tagging::enrichment_identity(audio_path, audio_source)?;
        (t, a, d, s)
    };

    if title.trim().is_empty() || artist.trim().is_empty() {
        return None;
    }

    let (cached, lock_opt) = super::enrich_cache::get_or_lock(&title, &artist);
    if let Some(enr) = cached {
        return if enr.is_empty() { None } else { Some(enr) };
    }

    // Block this thread until the metadata for this base song is resolved.
    // This guarantees that if track 2 (Slowed) finds the genre, track 1 (Original)
    // will wait for it and get the exact same result.
    let _lock_arc = lock_opt;
    let _guard = _lock_arc.as_ref().map(|arc| arc.lock().unwrap_or_else(|e| e.into_inner()));

    // Double check the cache after acquiring the lock.
    if let Some(enr) = super::enrich_cache::get(&title, &artist) {
        return if enr.is_empty() { None } else { Some(enr) };
    }

    let (search_title, search_duration) = match search_score::clean_speed_modifiers(&title) {
        Some(clean) => (clean, None),
        None => (title.clone(), duration_ms),
    };

    let identity = TrackIdentity {
        title: &title,
        search_title: &search_title,
        search_duration,
        artist: &artist,
        duration_ms,
        meta_source,
        audio_source,
        audio_url,
    };
    let result = enrich(&identity, inbox, label, cancel);
    
    // Do NOT cache empty results. If the "Original" search fails, we want
    // the "Slowed" version to attempt its own search (which might find
    // the slowed version on SoundCloud) rather than returning None.
    if !result.is_empty() {
        super::enrich_cache::put(&title, &artist, result.clone());
    }
    
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

pub fn enrich(
    identity: &TrackIdentity,
    inbox: &Inbox,
    label: &str,
    cancel: &AtomicBool,
) -> Enrichment {
    let mut result = Enrichment::default();

    // 1. Deezer Tier
    if identity.meta_source != Source::Deezer {
        if cancel.load(Ordering::Relaxed) {
            return result;
        }
        let mut dz_result = deezer::search_track(identity.search_title, identity.artist, identity.search_duration).ok().flatten();
        
        // Fallback: try without duration
        if dz_result.is_none() && identity.search_duration.is_some() {
            dz_result = deezer::search_track(identity.search_title, identity.artist, None).ok().flatten();
        }
        // Fallback: try with original unmodified title
        if dz_result.is_none() && identity.search_title != identity.title {
            dz_result = deezer::search_track(identity.title, identity.artist, identity.duration_ms).ok().flatten();
        }

        if let Some(dz) = dz_result {
            merge_deezer(&mut result, &dz);
            if result.genre.is_none() {
                if let Some(album_id) = &dz.album_id {
                    if let Ok(Some(g)) = deezer::fetch_album_genre(album_id) {
                        result.genre = Some(g);
                    }
                }
            }
        }
    }

    // 2. SoundCloud Tier
    if identity.audio_source == Source::SoundCloud {
        if cancel.load(Ordering::Relaxed) {
            return result;
        }
        match soundcloud_meta::fetch(identity.audio_url) {
            Ok(sc) => {
                super::enrich_cache::put_sc(identity.audio_url, sc.clone());
                merge_soundcloud(&mut result, &sc);
            }
            Err(e) => log(
                inbox,
                LogLevel::Info,
                format!("[{label}] SoundCloud hydration: {e}"),
            ),
        }
    } else if identity.meta_source != Source::SoundCloud {
        if cancel.load(Ordering::Relaxed) {
            return result;
        }
        let mut sc_result = find_on_soundcloud(identity, identity.search_title, identity.search_duration, cancel);
        
        // Fallback: try without duration
        if let Ok(None) = sc_result {
            if identity.search_duration.is_some() {
                sc_result = find_on_soundcloud(identity, identity.search_title, None, cancel);
            }
        }
        // Fallback: try with original unmodified title
        if let Ok(None) = sc_result {
            if identity.search_title != identity.title {
                sc_result = find_on_soundcloud(identity, identity.title, identity.duration_ms, cancel);
            }
        }

        match sc_result {
            Ok(Some(sc)) => merge_soundcloud(&mut result, &sc),
            Ok(None) => {}
            Err(e) => log(
                inbox,
                LogLevel::Info,
                format!("[{label}] SoundCloud enrichment: {e}"),
            ),
        }
    }

    result
}

fn find_on_soundcloud(
    identity: &TrackIdentity,
    search_title: &str,
    search_duration: Option<u64>,
    cancel: &AtomicBool,
) -> Result<Option<SoundCloudMeta>, String> {
    let url = match search_score::resolve_on(
        Source::SoundCloud,
        search_title,
        identity.artist,
        search_duration,
        cancel,
    ) {
        Ok(u) => u,
        Err(_) => return Ok(None),
    };
    soundcloud_meta::fetch(&url).map(Some)
}

fn merge_deezer(result: &mut Enrichment, dz: &DeezerMeta) {
    if result.album.is_none() {
        result.album = dz.album.clone();
    }
    if result.album_artist.is_none() {
        result.album_artist = dz.album_artist.clone();
    }
    if result.year.is_none() {
        result.year = dz
            .release_date
            .as_deref()
            .and_then(|d| d.get(0..4))
            .and_then(|y| y.parse::<i32>().ok());
    }
    if result.track_number.is_none() {
        result.track_number = dz.track_position;
    }
    if result.disc_number.is_none() {
        result.disc_number = dz.disk_number;
    }
    if result.isrc.is_none() {
        result.isrc = dz.isrc.clone();
    }
    if result.bpm.is_none() {
        result.bpm = dz.bpm;
    }
    if result.gain.is_none() {
        result.gain = dz.gain;
    }
}

fn merge_soundcloud(result: &mut Enrichment, sc: &SoundCloudMeta) {
    if result.genre.is_none() {
        result.genre = sc.genre.clone();
    }
    if result.label.is_none() {
        result.label = sc.label_name.clone();
    }
    if result.isrc.is_none() {
        result.isrc = sc.isrc.clone();
    }
    if result.album.is_none() {
        result.album = sc.album_title.clone();
    }
    if result.album_artist.is_none() {
        result.album_artist = sc.album_artist.clone();
    }
    if result.year.is_none() {
        result.year = sc
            .release_date
            .as_deref()
            .and_then(|d| d.get(0..4))
            .and_then(|y| y.parse::<i32>().ok());
    }
}