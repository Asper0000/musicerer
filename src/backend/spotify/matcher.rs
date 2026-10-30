//! Delegates audio matching to `backend::search_score`.

use super::models::SpotifyMeta;
use std::sync::atomic::AtomicBool;

pub fn resolve_track(meta: &SpotifyMeta, cancel: &AtomicBool) -> Result<String, String> {
    crate::backend::search_score::resolve(
        &meta.title,
        &meta.artists,
        meta.duration_ms,
        cancel,
    )
}