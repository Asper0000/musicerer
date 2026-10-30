//! Deezer public API client. No key required for catalogue reads.
//!
//! Rate limit: 50 requests per 5 seconds per IP. Tracked by a shared
//! in-process limiter so parallel playlist expansion cannot exceed it.
//! Cover art is fetched from a separate CDN and is not rate-limited.

use super::models::DeezerMeta;
use crate::backend::utils::USER_AGENT;
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::io::Read;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const MAX_REQUESTS: usize = 50;
const WINDOW: Duration = Duration::from_secs(5);

struct RateLimiter {
    log: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            log: Mutex::new(VecDeque::with_capacity(MAX_REQUESTS)),
        }
    }

    fn wait(&self) {
        loop {
            let now = Instant::now();
            let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
            while let Some(&t) = log.front() {
                if now.duration_since(t) >= WINDOW {
                    log.pop_front();
                } else {
                    break;
                }
            }
            if log.len() < MAX_REQUESTS {
                log.push_back(now);
                return;
            }
            drop(log);
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

static RATE_LIMITER: LazyLock<RateLimiter> = LazyLock::new(RateLimiter::new);

fn api_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build()
}

fn get_json(url: &str) -> Result<Value, String> {
    RATE_LIMITER.wait();
    let resp = api_agent()
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| format!("Deezer API: {e}"))?;
    let v: Value = resp
        .into_json()
        .map_err(|e| format!("Deezer API JSON: {e}"))?;
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("Deezer API error: {msg}"));
    }
    Ok(v)
}

pub fn fetch_track_meta(deezer_url: &str) -> Result<DeezerMeta, String> {
    let (kind, id) = super::deezer_id(deezer_url)
        .ok_or_else(|| "not a Deezer URL".to_string())?;
    if kind != "track" {
        return Err(format!("expected a track URL, got {kind}"));
    }
    fetch_track_by_id(&id)
}

pub fn fetch_track_by_id(id: &str) -> Result<DeezerMeta, String> {
    let url = format!("https://api.deezer.com/track/{id}");
    let v = get_json(&url)?;
    parse_track(&v).ok_or_else(|| format!("Deezer track {id}: missing title"))
}

pub struct PlaylistSummary {
    pub kind: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub track_ids: Vec<String>,
}

pub fn fetch_playlist_summary(deezer_url: &str) -> Result<PlaylistSummary, String> {
    let (kind, id) = super::deezer_id(deezer_url)
        .ok_or_else(|| "not a Deezer URL".to_string())?;
    let endpoint = match kind.as_str() {
        "playlist" => "playlist",
        "album" => "album",
        _ => return Err(format!("expected playlist or album URL, got {kind}")),
    };
    let url = format!("https://api.deezer.com/{endpoint}/{id}");
    let v = get_json(&url)?;

    let title = v
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Deezer playlist")
        .to_string();
    let cover_url = v
        .get("picture_xl")
        .or_else(|| v.get("cover_xl"))
        .or_else(|| v.get("picture_big"))
        .or_else(|| v.get("cover_big"))
        .and_then(Value::as_str)
        .map(String::from);

    let tracks = v
        .get("tracks")
        .and_then(|t| t.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| "Deezer response missing tracks.data".to_string())?;

    let track_ids: Vec<String> = tracks
        .iter()
        .filter_map(|t| t.get("id").and_then(Value::as_u64).map(|n| n.to_string()))
        .collect();

    if track_ids.is_empty() {
        return Err("Deezer playlist has no tracks".into());
    }

    Ok(PlaylistSummary {
        kind,
        title,
        cover_url,
        track_ids,
    })
}

pub fn fetch_cover_bytes(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();
    let resp = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "image/avif,image/webp,image/apng,image/*,*/*;q=0.8")
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(4 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn parse_track(v: &Value) -> Option<DeezerMeta> {
    let title = v.get("title").and_then(Value::as_str)?.to_string();
    let mut artists = v
        .get("artist")
        .and_then(|a| a.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    
    // Deezer provides a `contributors` array with all featured artists.
    if let Some(contribs) = v.get("contributors").and_then(Value::as_array) {
        let names: Vec<&str> = contribs.iter().filter_map(|c| c.get("name").and_then(Value::as_str)).collect();
        if !names.is_empty() {
            artists = names.join(", ");
        }
    }

    let album = v
        .get("album")
        .and_then(|a| a.get("title"))
        .and_then(Value::as_str)
        .map(String::from);
    let album_artist = v
        .get("album")
        .and_then(|a| a.get("artist"))
        .and_then(|a| a.get("name"))
        .and_then(Value::as_str)
        .map(String::from);
    let album_id = v
        .get("album")
        .and_then(|a| a.get("id"))
        .and_then(Value::as_u64)
        .map(|n| n.to_string());
    let cover_url = v
        .get("album")
        .and_then(|a| a.get("cover_xl").or_else(|| a.get("cover_big")))
        .and_then(Value::as_str)
        .map(String::from);
    let duration_ms = v
        .get("duration")
        .and_then(Value::as_u64)
        .map(|s| s * 1000);
    let track_position = v
        .get("track_position")
        .and_then(Value::as_u64)
        .map(|n| n as u32);
    let disk_number = v
        .get("disk_number")
        .and_then(Value::as_u64)
        .map(|n| n as u32);
    let bpm = v
        .get("bpm")
        .and_then(Value::as_f64)
        .map(|f| f as f32)
        .filter(|&b| b > 0.0);
    let gain = v
        .get("gain")
        .and_then(Value::as_f64)
        .map(|f| f as f32)
        .filter(|&g| g != 0.0);
    let isrc = v.get("isrc").and_then(Value::as_str).map(String::from);
    let release_date = v
        .get("release_date")
        .and_then(Value::as_str)
        .map(String::from);

    Some(DeezerMeta {
        title,
        artists,
        album,
        album_artist,
        album_id,
        duration_ms,
        cover_url,
        release_date,
        track_position,
        disk_number,
        bpm,
        gain,
        isrc,
    })
}

/// Search Deezer's public catalogue for a track matching the given
/// identity, and return its full metadata when the match is strict
/// enough. Requires a non-empty title; artist is used as a filter but
/// the search succeeds on title alone if `artist` is blank.
pub fn search_track(
    title: &str,
    artist: &str,
    duration_ms: Option<u64>,
) -> Result<Option<DeezerMeta>, String> {
    let title = crate::backend::search_score::clean_title_for_search(title);
    if title.trim().is_empty() {
        return Ok(None);
    }
    let artist = artist.split(',').next().unwrap_or(artist).trim();
    // Use standard fuzzy search instead of exact tag matching. The local
    // scoring will filter out false positives. Exact tags fail on "feat."
    // or when the artist string slightly differs.
    let q = if artist.is_empty() {
        escape_quotes(&title)
    } else {
        format!("{} {}", escape_quotes(artist), escape_quotes(&title))
    };
    let v = get_json_query("https://api.deezer.com/search", &[("q", &q), ("limit", "10")])?;
    let data = match v.get("data").and_then(Value::as_array) {
        Some(arr) => arr,
        None => return Ok(None),
    };

    let target_secs = duration_ms.map(|ms| ms as f64 / 1000.0);
    let wanted_title = tokens(&title);
    let wanted_artist = tokens(artist);

    let mut best: Option<(f64, String)> = None;
    for entry in data {
        let entry_title = entry.get("title").and_then(Value::as_str).unwrap_or("");
        let entry_artist = entry
            .get("artist")
            .and_then(|a| a.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let entry_duration = entry.get("duration").and_then(Value::as_u64);
        let Some(score) = score_candidate(
            entry_title,
            entry_artist,
            entry_duration,
            &wanted_title,
            &wanted_artist,
            target_secs,
        ) else {
            continue;
        };
        let Some(id) = entry.get("id").and_then(Value::as_u64).map(|n| n.to_string()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(bs, _)| score > *bs) {
            best = Some((score, id));
        }
    }

    match best {
        Some((_, id)) => Ok(Some(fetch_track_by_id(&id)?)),
        None => Ok(None),
    }
}

fn escape_quotes(s: &str) -> String {
    s.replace('"', "")
}

fn tokens(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn coverage(want: &HashSet<String>, have: &HashSet<String>) -> f64 {
    if want.is_empty() {
        return 1.0;
    }
    want.iter().filter(|t| have.contains(t.as_str())).count() as f64 / want.len() as f64
}

fn score_candidate(
    entry_title: &str,
    entry_artist: &str,
    entry_duration_secs: Option<u64>,
    wanted_title: &HashSet<String>,
    wanted_artist: &HashSet<String>,
    target_secs: Option<f64>,
) -> Option<f64> {
    if entry_title.is_empty() || entry_artist.is_empty() {
        return None;
    }
    let title_cov = coverage(wanted_title, &tokens(entry_title));
    if title_cov < 0.5 { // Lowered from 0.7 to allow slight title differences
        return None;
    }
    let artist_cov = coverage(wanted_artist, &tokens(entry_artist));
    if artist_cov < 0.3 { // Lowered from 0.6 to allow missing "feat." artists
        return None;
    }
    let duration_score = match (target_secs, entry_duration_secs) {
        (Some(t), Some(d)) => {
            let d = d as f64;
            let diff = (t - d).abs();
            // Loosened to 40% / 120s to allow matching slowed/sped-up versions
            if diff > (t * 0.40).max(120.0) { 
                return None;
            }
            (60.0 - diff).max(0.0)
        }
        _ => 30.0,
    };
    Some(title_cov * 400.0 + artist_cov * 200.0 + duration_score)
}

/// Fetches the genre from a Deezer album.
pub fn fetch_album_genre(album_id: &str) -> Result<Option<String>, String> {
    let url = format!("https://api.deezer.com/album/{album_id}");
    let v = get_json(&url)?;
    let genre = v
        .pointer("/genres/data/0/name")
        .and_then(Value::as_str)
        .map(String::from);
    Ok(genre)
}

fn get_json_query(base: &str, params: &[(&str, &str)]) -> Result<Value, String> {
    RATE_LIMITER.wait();
    let mut req = api_agent()
        .get(base)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/json");
    for (k, v) in params {
        req = req.query(k, v);
    }
    let resp = req.call().map_err(|e| format!("Deezer API: {e}"))?;
    let v: Value = resp
        .into_json()
        .map_err(|e| format!("Deezer API JSON: {e}"))?;
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("Deezer API error: {msg}"));
    }
    Ok(v)
}