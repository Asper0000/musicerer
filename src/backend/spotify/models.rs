//! The few Spotify fields Musicerer needs: to search for audio and to tag the file.

#[derive(Debug, Clone, Default)]
pub struct SpotifyMeta {
    pub title: String,
    pub artists: String,
    pub duration_ms: Option<u64>,
    pub cover_url: Option<String>,
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
        match (artists.is_empty(), title.is_empty()) {
            (true, true) => String::new(),
            (true, false) => title.to_string(),
            (false, true) => artists.to_string(),
            (false, false) => format!("{artists} - {title}"),
        }
    }
}
