//! The Spotify fields Musicerer needs: to search for audio and to tag the file.

#[derive(Debug, Clone, Default)]
pub struct SpotifyMeta {
    pub title: String,
    pub artists: String,
    pub duration_ms: Option<u64>,
    pub cover_url: Option<String>,
    pub album: Option<String>,
    pub year: Option<u32>,
    pub track_number: Option<u32>,
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