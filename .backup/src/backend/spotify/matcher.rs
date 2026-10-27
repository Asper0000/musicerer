//! Resolve a Spotify track to a YouTube / SoundCloud URL via yt-dlp search.

use super::models::SpotifyMeta;
use crate::backend::tools::{self, Tool};
use serde_json::Value;
use std::process::Stdio;

/// Best-effort match. Returns the chosen media URL or an error message.
pub fn resolve_track(meta: &SpotifyMeta) -> Result<String, String> {
    if let Some(isrc) = meta.isrc.as_deref().filter(|s| !s.is_empty()) {
        if let Ok(url) = search_one(&format!("ytsearch5:{isrc}"), meta) {
            return Ok(url);
        }
    }

    let query = build_query(meta);
    if query.is_empty() {
        return Err("empty search query".into());
    }

    if let Ok(url) = search_one(&format!("ytsearch5:{query}"), meta) {
        return Ok(url);
    }

    if let Ok(url) = search_one(&format!("scsearch5:{query}"), meta) {
        return Ok(url);
    }

    Err(format!("no matching audio found for \"{}\"", meta.display_title()))
}

fn build_query(meta: &SpotifyMeta) -> String {
    let title = meta.title.trim();
    let artists = meta.artists.trim();
    if title.is_empty() && artists.is_empty() {
        return String::new();
    }
    if artists.is_empty() {
        title.to_string()
    } else if title.is_empty() {
        artists.to_string()
    } else {
        format!("{artists} - {title}")
    }
}

fn search_one(search_term: &str, meta: &SpotifyMeta) -> Result<String, String> {
    let mut cmd = tools::command(Tool::YtDlp);
    cmd.arg("--flat-playlist")
        .arg("--dump-single-json")
        .arg("--no-warnings")
        .arg("--no-color")
        .arg("--playlist-end")
        .arg("5")
        .arg(search_term)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let output = cmd
        .output()
        .map_err(|e| format!("yt-dlp search spawn: {e}"))?;
    if output.stdout.is_empty() {
        return Err("empty search result".into());
    }

    let root: Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("search json: {e}"))?;

    let entries = root
        .get("entries")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_else(|| {
            if root.get("id").is_some() {
                vec![root.clone()]
            } else {
                vec![]
            }
        });

    let target_secs = meta.duration_ms.map(|ms| ms as f64 / 1000.0);
    let mut best: Option<(f64, String)> = None;

    for entry in entries {
        let url = entry_url(&entry);
        let Some(url) = url else { continue; };
        let dur = entry
            .get("duration")
            .and_then(|v| v.as_f64())
            .or_else(|| {
                entry
                    .get("duration_string")
                    .and_then(|v| v.as_str())
                    .and_then(parse_duration_string)
            });

        let score = match (target_secs, dur) {
            (Some(t), Some(d)) => {
                let diff = (t - d).abs();
                let tol = (t * 0.15).max(20.0);
                if diff > tol {
                    continue;
                }
                1000.0 - diff
            }
            (Some(_), None) => 100.0,
            (None, _) => 500.0,
        };

        let title = entry
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let bonus = if title.contains("official")
            || title.contains("topic")
            || title.contains("audio")
        {
            30.0
        } else {
            0.0
        };

        let total = score + bonus;
        if best.as_ref().map(|(s, _)| total > *s).unwrap_or(true) {
            best = Some((total, url));
        }
    }

    best.map(|(_, u)| u)
        .ok_or_else(|| "no candidate within duration tolerance".into())
}

fn entry_url(entry: &Value) -> Option<String> {
    for key in ["webpage_url", "url", "original_url"] {
        if let Some(u) = entry.get(key).and_then(|v| v.as_str()) {
            if u.starts_with("http") {
                return Some(u.to_string());
            }
        }
    }
    let id = entry.get("id").and_then(|v| v.as_str())?;
    let ie = entry
        .get("ie_key")
        .or_else(|| entry.get("extractor_key"))
        .or_else(|| entry.get("extractor"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ie.contains("youtube") || ie.is_empty() {
        return Some(format!("https://www.youtube.com/watch?v={id}"));
    }
    if ie.contains("soundcloud") {
        return None;
    }
    None
}

fn parse_duration_string(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    match parts.len() {
        1 => parts[0].parse().ok(),
        2 => {
            let m: f64 = parts[0].parse().ok()?;
            let sec: f64 = parts[1].parse().ok()?;
            Some(m * 60.0 + sec)
        }
        3 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let sec: f64 = parts[2].parse().ok()?;
            Some(h * 3600.0 + m * 60.0 + sec)
        }
        _ => None,
    }
}