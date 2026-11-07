#[derive(Debug, Clone, Default)]
pub struct ITunesMeta {
    pub title: String,
    pub artists: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub duration_ms: Option<u64>,
    pub cover_url: Option<String>,
    pub release_date: Option<String>,
    pub track_position: Option<u32>,
    pub disk_number: Option<u32>,
    pub genre: Option<String>,
}

impl ITunesMeta {
    pub fn display_title(&self) -> String {
        if self.artists.is_empty() {
            self.title.clone()
        } else {
            format!("{} - {}", self.artists, self.title)
        }
    }
}