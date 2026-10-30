//! Process-wide in-memory caches for the enrichment pipeline.

use super::enrich::Enrichment;
use super::soundcloud_meta::SoundCloudMeta;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    title: String,
    artist: String,
}

static CACHE: LazyLock<Mutex<HashMap<Key, Enrichment>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static SC_CACHE: LazyLock<Mutex<HashMap<String, SoundCloudMeta>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static LOCKS: LazyLock<Mutex<HashMap<Key, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn get(title: &str, artist: &str) -> Option<Enrichment> {
    let key = make_key(title, artist);
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .cloned()
}

pub fn put(title: &str, artist: &str, value: Enrichment) {
    let key = make_key(title, artist);
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if g.len() >= MAX_ENTRIES && !g.contains_key(&key) {
        g.clear();
        let mut locks = LOCKS.lock().unwrap_or_else(|e| e.into_inner());
        locks.clear();
    }
    g.insert(key, value);
}

/// Atomically checks the cache and, if missing, returns a lock that the caller
/// must hold while fetching metadata. This prevents parallel downloads of
/// (Original) and (Slowed) from both missing the cache and doing redundant work.
pub fn get_or_lock(title: &str, artist: &str) -> (Option<Enrichment>, Option<Arc<Mutex<()>>>) {
    let key = make_key(title, artist);
    
    {
        let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(enr) = cache.get(&key) {
            return (Some(enr.clone()), None);
        }
    }
    
    let mut locks = LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    let lock = locks.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))).clone();
    (None, Some(lock))
}

pub fn get_sc(url: &str) -> Option<SoundCloudMeta> {
    SC_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(url)
        .cloned()
}

pub fn put_sc(url: &str, meta: SoundCloudMeta) {
    let mut g = SC_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if g.len() >= MAX_ENTRIES && !g.contains_key(url) {
        g.clear();
    }
    g.insert(url.to_string(), meta);
}

fn make_key(title: &str, artist: &str) -> Key {
    // Normalize speed modifiers so "Song (Slowed)" and "Song" map to the same key.
    let clean = crate::backend::search_score::clean_speed_modifiers(title)
        .unwrap_or_else(|| title.to_string());
    Key {
        title: clean.trim().to_lowercase(),
        artist: artist.trim().to_lowercase(),
    }
}