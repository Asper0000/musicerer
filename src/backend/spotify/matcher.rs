use super::models::SpotifyMeta;
use crate::backend::process;
use crate::backend::tools::{self, Tool};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

const RESULT_COUNT: usize = 5;
const MIN_TITLE_COVERAGE: f64 = 0.6;
const DURATION_TOLERANCE: f64 = 0.15;
const MIN_DURATION_TOLERANCE_SECS: f64 = 20.0;

const UNWANTED_WORDS: &[&str] = &[
    "live",
    "remix",
    "cover",
    "karaoke",
    "instrumental",
    "nightcore",
    "slowed",
    "reverb",
    "sped",
    "mashup",
    "tribute",
    "bootleg",
    "boosted",
    "8d",
    "reaction",
    "tutorial",
];

pub fn resolve_track(meta: &SpotifyMeta, cancel: &AtomicBool) -> Result<String, String> {
    let query = meta.search_query();
    if query.is_empty() {
        return Err("empty search query".into());
    }
    let profile = Profile::new(meta);
    for prefix in ["ytsearch", "scsearch"] {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let term = format!("{prefix}{RESULT_COUNT}:{query}");
        if let Some(url) = search(&term, &profile, cancel) {
            return Ok(url);
        }
    }
    Err(format!(
        "no matching audio found for \"{}\"",
        meta.display_title()
    ))
}

fn search(term: &str, profile: &Profile, cancel: &AtomicBool) -> Option<String> {
    let _permit = crate::backend::metadata_limit::acquire(cancel)?;
    let mut cmd = tools::command(Tool::YtDlp);
    cmd.arg("--flat-playlist")
        .arg("--dump-single-json")
        .arg("--no-warnings")
        .arg("--no-color")
        .arg("--socket-timeout")
        .arg("10")
        .arg("--playlist-end")
        .arg(RESULT_COUNT.to_string())
        .arg(term);
    let output = process::output_stoppable(&mut cmd, cancel).ok()?;
    if cancel.load(Ordering::Relaxed) || output.stdout.is_empty() {
        return None;
    }
    let root: Value = serde_json::from_slice(&output.stdout).ok()?;
    let entries = root.get("entries").and_then(Value::as_array)?;
    entries
        .iter()
        .filter_map(|entry| {
            let url = entry_url(entry)?;
            let score = profile.score(entry)?;
            Some((score, url))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, url)| url)
}

struct Profile {
    core_title: Vec<String>,
    primary_artist: Vec<String>,
    wanted: HashSet<String>,
    target_secs: Option<f64>,
}

impl Profile {
    fn new(meta: &SpotifyMeta) -> Self {
        let mut core_title = tokens(&strip_decorations(&meta.title));
        if core_title.is_empty() {
            core_title = tokens(&meta.title);
        }
        let primary_artist = tokens(meta.artists.split(',').next().unwrap_or(""));
        let wanted = tokens(&meta.title)
            .into_iter()
            .chain(tokens(&meta.artists))
            .collect();
        Self {
            core_title,
            primary_artist,
            wanted,
            target_secs: meta.duration_ms.map(|ms| ms as f64 / 1000.0),
        }
    }

    fn score(&self, entry: &Value) -> Option<f64> {
        let text = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or("");
        let title_tokens = tokens(text("title"));
        let is_other_version = title_tokens
            .iter()
            .any(|t| UNWANTED_WORDS.contains(&t.as_str()) && !self.wanted.contains(t));
        if is_other_version {
            return None;
        }
        let uploader = if text("uploader").is_empty() {
            text("channel")
        } else {
            text("uploader")
        };
        let uploader_tokens = tokens(uploader);
        let haystack: HashSet<&str> = title_tokens
            .iter()
            .chain(&uploader_tokens)
            .map(String::as_str)
            .collect();
        let title_coverage = coverage(&self.core_title, &haystack);
        if title_coverage < MIN_TITLE_COVERAGE {
            return None;
        }
        let artist_coverage = coverage(&self.primary_artist, &haystack);
        let duration_score = match (self.target_secs, duration_secs(entry)) {
            (Some(target), Some(found)) => {
                let diff = (target - found).abs();
                if diff > (target * DURATION_TOLERANCE).max(MIN_DURATION_TOLERANCE_SECS) {
                    return None;
                }
                (300.0 - diff * 20.0).max(0.0)
            }
            (Some(_), None) => 100.0,
            (None, _) => 150.0,
        };
        let mut bonus = 0.0;
        if haystack.contains("topic") {
            bonus += 60.0;
        }
        if haystack.contains("audio") {
            bonus += 20.0;
        }
        if haystack.contains("video") {
            bonus -= 20.0;
        }
        Some(title_coverage * 400.0 + artist_coverage * 200.0 + duration_score + bonus)
    }
}

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn strip_decorations(title: &str) -> String {
    let mut depth = 0usize;
    let mut plain = String::with_capacity(title.len());
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    match plain.split_once(" - ") {
        Some((head, _)) => head.trim().to_string(),
        None => plain.trim().to_string(),
    }
}

fn coverage(needle: &[String], haystack: &HashSet<&str>) -> f64 {
    if needle.is_empty() {
        return 1.0;
    }
    let hits = needle
        .iter()
        .filter(|t| haystack.contains(t.as_str()))
        .count();
    hits as f64 / needle.len() as f64
}

fn duration_secs(entry: &Value) -> Option<f64> {
    entry.get("duration").and_then(Value::as_f64).or_else(|| {
        entry
            .get("duration_string")
            .and_then(Value::as_str)
            .and_then(parse_duration_string)
    })
}

fn parse_duration_string(s: &str) -> Option<f64> {
    s.trim().split(':').try_fold(0.0_f64, |acc, part| {
        part.parse::<f64>().ok().map(|v| acc * 60.0 + v)
    })
}

fn entry_url(entry: &Value) -> Option<String> {
    ["webpage_url", "url", "original_url"]
        .iter()
        .filter_map(|key| entry.get(*key).and_then(Value::as_str))
        .find(|u| u.starts_with("http"))
        .map(String::from)
        .or_else(|| {
            let id = entry.get("id")?.as_str()?;
            let extractor = ["ie_key", "extractor_key", "extractor"]
                .iter()
                .find_map(|key| entry.get(*key).and_then(Value::as_str))
                .unwrap_or("")
                .to_ascii_lowercase();
            (extractor.is_empty() || extractor.contains("youtube"))
                .then(|| format!("https://www.youtube.com/watch?v={id}"))
        })
}
