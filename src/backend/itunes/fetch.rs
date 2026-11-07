use super::models::ITunesMeta;
use crate::backend::utils::USER_AGENT;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::Read;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const MAX_REQUESTS: usize = 20;
const WINDOW: Duration = Duration::from_secs(60);
const BURST_DELAY: Duration = Duration::from_secs(1);

static RATE_LIMITER: LazyLock<Mutex<VecDeque<Instant>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));

fn wait_for_slot() {
    loop {
        let now = Instant::now();
        let mut log = RATE_LIMITER.lock().unwrap_or_else(|e| e.into_inner());

        // Clean up old requests
        while let Some(&t) = log.front() {
            if now.duration_since(t) >= WINDOW {
                log.pop_front();
            } else {
                break;
            }
        }

        // Check burst limit (1 request per second)
        if let Some(&last) = log.back() {
            if now.duration_since(last) < BURST_DELAY {
                let wait = BURST_DELAY - now.duration_since(last);
                drop(log);
                std::thread::sleep(wait);
                continue;
            }
        }

        // Check window limit (20 requests per 60 seconds)
        if log.len() >= MAX_REQUESTS {
            if let Some(&oldest) = log.front() {
                let wait = WINDOW - now.duration_since(oldest);
                drop(log);
                std::thread::sleep(wait);
                continue;
            }
        }

        // We have a slot
        log.push_back(now);
        break;
    }
}

fn get_json(url: &str) -> Result<Value, String> {
    wait_for_slot();

    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
    let resp = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| format!("iTunes API: {e}"))?;
    
    let v: Value = resp.into_json().map_err(|e| format!("iTunes JSON: {e}"))?;
    Ok(v)
}

pub fn search_track(title: &str, artist: &str, duration_ms: Option<u64>) -> Result<Option<ITunesMeta>, String> {
    let title = title.trim();
    let artist = artist.split(',').next().unwrap_or(artist).trim();
    if title.is_empty() {
        return Ok(None);
    }

    let term = if artist.is_empty() {
        title.to_string()
    } else {
        format!("{} {}", artist, title)
    };

    wait_for_slot();
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).build();
    let resp = agent
        .get("https://itunes.apple.com/search")
        .query("term", &term)
        .query("entity", "song")
        .query("limit", "5")
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/json")
        .call()
        .map_err(|e| format!("iTunes API: {e}"))?;
        
    let v: Value = resp.into_json().map_err(|e| format!("iTunes JSON: {e}"))?;
    let results = match v.get("results").and_then(Value::as_array) {
        Some(arr) => arr,
        None => return Ok(None),
    };

    let target_secs = duration_ms.map(|ms| ms as f64 / 1000.0);
    let mut best: Option<(f64, &Value)> = None;

    for item in results {
        let entry_title = item.get("trackName").and_then(Value::as_str).unwrap_or("");
        let entry_artist = item.get("artistName").and_then(Value::as_str).unwrap_or("");
        let entry_duration = item.get("trackTimeMillis").and_then(Value::as_u64).map(|ms| ms as f64 / 1000.0);

        let title_match = entry_title.to_lowercase().contains(&title.to_lowercase());
        let artist_match = entry_artist.to_lowercase().contains(&artist.to_lowercase()) || artist.is_empty();
        
        if !title_match || !artist_match {
            continue;
        }

        let score = match (target_secs, entry_duration) {
            (Some(t), Some(d)) => {
                let diff = (t - d).abs();
                if diff > 10.0 { continue; }
                100.0 - diff
            }
            _ => 50.0,
        };

        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, item));
        }
    }

    if let Some((_, best_item)) = best {
        Ok(parse_track(best_item))
    } else {
        Ok(None)
    }
}

pub fn fetch_track_meta(url: &str) -> Result<ITunesMeta, String> {
    let (kind, id, track_id) = super::itunes_id(url).ok_or_else(|| "not an iTunes URL".to_string())?;
    if kind != "track" {
        return Err("expected a track URL".into());
    }
    
    let lookup_id = track_id.unwrap_or(id);
    let api_url = format!("https://itunes.apple.com/lookup?id={lookup_id}");
    let v = get_json(&api_url)?;
    
    let results = v.get("results").and_then(Value::as_array).ok_or_else(|| "missing results".to_string())?;
    if results.is_empty() {
        return Err("iTunes track not found".into());
    }
    
    parse_track(&results[0]).ok_or_else(|| "failed to parse iTunes track".to_string())
}

pub fn fetch_playlist_metas(url: &str) -> Result<Vec<ITunesMeta>, String> {
    let (kind, id, track_id) = super::itunes_id(url).ok_or_else(|| "not an iTunes URL".to_string())?;
    if kind == "track" {
        return Ok(vec![fetch_track_meta(url)?]);
    }

    let api_url = format!("https://itunes.apple.com/lookup?id={id}&entity=song");
    let v = get_json(&api_url)?;
    
    let results = v.get("results").and_then(Value::as_array).ok_or_else(|| "missing results".to_string())?;
    let mut tracks = Vec::new();
    
    for item in results {
        if let Some(wrapper_type) = item.get("wrapperType").and_then(Value::as_str) {
            if wrapper_type == "track" {
                if let Some(meta) = parse_track(item) {
                    if let Some(tid) = &track_id {
                        if item.get("trackId").and_then(Value::as_u64).map(|n| n.to_string()).as_deref() != Some(tid.as_str()) {
                            continue;
                        }
                    }
                    tracks.push(meta);
                }
            }
        }
    }

    if tracks.is_empty() {
        return Err("no tracks found in iTunes URL".into());
    }
    
    Ok(tracks)
}

pub fn fetch_cover_bytes(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build();
    let resp = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(4 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn parse_track(v: &Value) -> Option<ITunesMeta> {
    let title = v.get("trackName").and_then(Value::as_str)?.to_string();
    let artists = v.get("artistName").and_then(Value::as_str).unwrap_or("").to_string();
    let album = v.get("collectionName").and_then(Value::as_str).map(String::from);
    let album_artist = v.get("collectionArtistName").and_then(Value::as_str).map(String::from);
    let duration_ms = v.get("trackTimeMillis").and_then(Value::as_u64);
    let release_date = v.get("releaseDate").and_then(Value::as_str).map(String::from);
    let track_position = v.get("trackNumber").and_then(Value::as_u64).map(|n| n as u32);
    let disk_number = v.get("discNumber").and_then(Value::as_u64).map(|n| n as u32);
    let genre = v.get("primaryGenreName").and_then(Value::as_str).map(String::from);
    
    let cover_url = v.get("artworkUrl100").and_then(Value::as_str).map(|s| {
        s.replace("100x100bb", "3000x3000bb")
    });

    Some(ITunesMeta {
        title,
        artists,
        album,
        album_artist,
        duration_ms,
        cover_url,
        release_date,
        track_position,
        disk_number,
        genre,
    })
}