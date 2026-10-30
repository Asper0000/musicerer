//! The metadata Deezer returns for a track.

#[derive(Debug, Clone, Default)]
pub struct DeezerMeta {
    pub title: String,
    pub artists: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub album_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub cover_url: Option<String>,
    pub release_date: Option<String>,
    pub track_position: Option<u32>,
    pub disk_number: Option<u32>,
    pub bpm: Option<f32>,
    pub gain: Option<f32>,
    pub isrc: Option<String>,
}

impl DeezerMeta {
    pub fn display_title(&self) -> String {
        if self.artists.is_empty() {
            self.title.clone()
        } else {
            format!("{} - {}", self.artists, self.title)
        }
    }
}