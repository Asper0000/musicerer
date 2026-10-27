//! Spotify track / playlist models used across the resolver and tagging.

use serde::{Deserialize, Serialize};

/// Metadata we attach to a queue item so the final .m4a is tagged from Spotify.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpotifyMeta {
    pub title: String,
    pub artists: String,
    pub album: String,
    pub year: Option<i32>,
    pub track_number: Option<u16>,
    pub isrc: Option<String>,
    pub duration_ms: Option<u64>,
    pub cover_url: Option<String>,
    /// Populated after cover bytes are fetched (optional, not serialised to disk).
    #[serde(skip)]
    pub cover_jpeg: Option<Vec<u8>>,
}

impl SpotifyMeta {
    pub fn display_title(&self) -> String {
        if self.artists.is_empty() {
            self.title.clone()
        } else {
            format!("{} - {}", self.artists, self.title)
        }
    }
}

#[derive(Debug, Clone)]
pub struct SpotifyTrack {
    pub id: String,
    pub meta: SpotifyMeta,
}

// --- raw API shapes (minimal) -------------------------------------------

#[derive(Debug, Deserialize, Clone)]
pub struct ApiTrack {
    pub id: String,
    pub name: String,
    pub duration_ms: Option<u64>,
    pub track_number: Option<u32>,
    pub artists: Option<Vec<ApiArtist>>,
    pub album: Option<ApiAlbum>,
    pub external_ids: Option<ApiExternalIds>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ApiArtist {
    pub name: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ApiAlbum {
    pub name: Option<String>,
    pub release_date: Option<String>,
    pub images: Option<Vec<ApiImage>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ApiImage {
    pub url: String,
    pub width: Option<u32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ApiExternalIds {
    pub isrc: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApiPlaylistTracks {
    pub items: Vec<ApiPlaylistItem>,
    pub next: Option<String>,
    pub total: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ApiPlaylistItem {
    pub track: Option<ApiTrack>,
}

#[derive(Debug, Deserialize)]
pub struct ApiAlbumTracks {
    pub items: Vec<ApiTrack>,
    pub next: Option<String>,
}

impl ApiTrack {
    pub fn into_meta(self) -> SpotifyMeta {
        let artists = self
            .artists
            .unwrap_or_default()
            .into_iter()
            .map(|a| a.name)
            .collect::<Vec<_>>()
            .join(", ");
        let album = self.album.as_ref().and_then(|a| a.name.clone()).unwrap_or_default();
        let year = self.album.as_ref().and_then(|a| {
            a.release_date
                .as_deref()
                .and_then(|d| d.get(0..4))
                .and_then(|y| y.parse().ok())
        });
        let cover_url = self.album.as_ref().and_then(|a| {
            let imgs = a.images.as_ref()?;
            imgs.iter()
                .filter(|i| i.width.unwrap_or(0) >= 200)
                .min_by_key(|i| i.width.unwrap_or(u32::MAX))
                .or_else(|| imgs.first())
                .map(|i| i.url.clone())
        });
        let isrc = self.external_ids.and_then(|e| e.isrc);
        SpotifyMeta {
            title: self.name,
            artists,
            album,
            year,
            track_number: self.track_number.map(|n| n as u16),
            isrc,
            duration_ms: self.duration_ms,
            cover_url,
            cover_jpeg: None,
        }
    }
}