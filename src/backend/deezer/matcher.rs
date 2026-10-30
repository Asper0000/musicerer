use super::models::DeezerMeta;
use std::sync::atomic::AtomicBool;

pub fn resolve_track(meta: &DeezerMeta, cancel: &AtomicBool) -> Result<String, String> {
    crate::backend::search_score::resolve(
        &meta.title,
        &meta.artists,
        meta.duration_ms,
        cancel,
    )
}