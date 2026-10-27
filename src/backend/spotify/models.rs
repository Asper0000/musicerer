//! Minimal Spotify metadata used only to build a search query and tag files.

use serde::{Deserialize, Serialize};

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

    /// Query string handed to yt-dlp search.
    pub fn search_query(&self) -> String {
        let title = self.title.trim();
        let artists = self.artists.trim();
        if title.is_empty() && artists.is_empty() {
            return String::new();
        }
        if artists.is_empty() {
            title.to_string()
        } else if title.is_empty() {
            artists.to_string()
        } else {
            format!("{artists} - {title}")
        }
    }
}
