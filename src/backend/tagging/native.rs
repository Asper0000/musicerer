use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Default, Deserialize)]
pub struct TrackInfo {
    pub title: Option<String>,
    pub track: Option<String>,
    pub artist: Option<String>,
    pub uploader: Option<String>,
    pub channel: Option<String>,
    pub album: Option<String>,
    pub release_date: Option<String>,
    pub upload_date: Option<String>,
    pub track_number: Option<i64>,
}

pub struct NativeMeta {
    pub track_info: Option<TrackInfo>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<i32>,
}

impl NativeMeta {
    pub fn info_missing(&self) -> bool {
        self.track_info.is_none()
    }
}

pub fn extract(audio_path: &Path) -> NativeMeta {
    let info = read_info(audio_path);
    let title = info.as_ref().and_then(|i| i.track.clone().or_else(|| i.title.clone()));
    let artist = info.as_ref().and_then(|i| {
        i.artist.clone()
            .or_else(|| i.uploader.clone())
            .or_else(|| i.channel.clone())
    });
    let album = info.as_ref().and_then(|i| i.album.clone());
    let year = info.as_ref().and_then(|i| {
        i.release_date
            .as_deref()
            .or(i.upload_date.as_deref())
            .and_then(|d| d.get(0..4))
            .and_then(|y| y.parse::<i32>().ok())
    });
    let track_number = info.as_ref().and_then(|i| i.track_number.map(|n| n as i32));
    NativeMeta { track_info: info, title, artist, album, year, track_number }
}

fn read_info(audio_path: &Path) -> Option<TrackInfo> {
    let info_path = audio_path.with_extension("info.json");
    let text = fs::read_to_string(info_path).ok()?;
    serde_json::from_str::<TrackInfo>(&text).ok()
}