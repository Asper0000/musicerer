use crate::backend::{audio, log, Inbox, LogLevel};
use std::path::Path;

pub fn from_disk(audio_path: &Path, inbox: &Inbox, label: &str) -> Option<Vec<u8>> {
    let thumb = audio::find_thumbnail(audio_path)?;
    match audio::square_cover(&thumb) {
        Ok(bytes) => Some(bytes),
        Err(err) => {
            log(inbox, LogLevel::Warn, format!("[{label}] Could not crop cover art: {err}"));
            None
        }
    }
}