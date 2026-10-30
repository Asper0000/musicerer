//! Shared yt-dlp audio search and candidate scoring.
//!
//! Both the Spotify and Deezer resolvers call into this to turn a
//! (title, artists, duration) tuple into a YouTube or SoundCloud URL.
//! Scoring is pure string comparison and adds no real time on top of
//! the yt-dlp search itself.

use crate::backend::process;
use crate::backend::tools::{self, Tool};
use crate::backend::Source;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

const RESULT_COUNT: usize = 10;
const MIN_TITLE_COVERAGE: f64 = 0.6;
const DURATION_TOLERANCE: f64 = 0.05;
const MIN_DURATION_TOLERANCE_SECS: f64 = 10.0;

const UNWANTED_WORDS: &[&str] = &[
    "live", "remix", "cover", "karaoke", "instrumental", "nightcore", "slowed", "reverb",
    "sped", "mashup", "tribute", "bootleg", "boosted", "8d", "reaction", "tutorial",
    "hardtekk", "hardstyle", "hardcore", "gabber", "uptempo", "phonk",
    "tiktok", "extended", "vip", "flip", "remake",
    "bassboost", "bassboosted", "reverbed", "spedup",
];

pub fn resolve(
    title: &str,
    artists: &str,
    duration_ms: Option<u64>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let query = build_query(title, artists);
    if query.is_empty() {
        return Err("empty search query".into());
    }
    let profile = Profile::new(title, artists, duration_ms);

    let terms = [
        crate::backend::youtube::search::term(&query, RESULT_COUNT),
        crate::backend::soundcloud::search::term(&query, RESULT_COUNT),
    ];
    for term in terms {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        if let Some(url) = search(&term, &profile, cancel) {
            return Ok(url);
        }
    }
    Err(format!(
        "no matching audio found for \"{}\"",
        build_query(title, artists)
    ))
}

/// Like `resolve`, but restricted to a single backend. Used by the
/// enrichment pipeline when only one of YouTube/SoundCloud is desired.
pub fn resolve_on(
    source: Source,
    title: &str,
    artists: &str,
    duration_ms: Option<u64>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let query = build_query(title, artists);
    if query.is_empty() {
        return Err("empty search query".into());
    }
    let term = match source {
        Source::YouTube => crate::backend::youtube::search::term(&query, RESULT_COUNT),
        Source::SoundCloud => crate::backend::soundcloud::search::term(&query, RESULT_COUNT),
        _ => return Err("not a supported enrichment source".into()),
    };
    let profile = Profile::new(title, artists, duration_ms);
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    match search(&term, &profile, cancel) {
        Some(url) => Ok(url),
        None => Err(format!("no matching audio found for \"{query}\"")),
    }
}

/// If the title has a speed modifier (slowed, sped up, nightcore, etc.),
/// strips it and returns the cleaned title. Otherwise returns None.
pub fn clean_speed_modifiers(title: &str) -> Option<String> {
    let lower = title.to_lowercase();
    let speed_keywords = [
        "slowed", "sped up", "speed up", "nightcore", "reverb",
        "8d", "bass boosted", "daycore"
    ];
    let is_speed_version = speed_keywords.iter().any(|kw| lower.contains(kw));
    if !is_speed_version {
        return None;
    }

    // Apply strip_decorations (removes parentheses/brackets and everything after " - ")
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

    let stripped = match plain.split_once(" - ") {
        Some((head, _)) => head.trim(),
        None => plain.trim(),
    };

    let lower_stripped = stripped.to_lowercase();
    let suffixes = [
        " sped up", " speed up", " slowed", " nightcore", " reverb",
        " ultra slowed", " super slowed", " 8d", " bass boosted", " daycore",
        " sped up version", " slowed version", " sped up + reverb", " slowed + reverb",
        " speed up version", " nightcore version"
    ];

    for suffix in suffixes {
        if lower_stripped.ends_with(suffix) {
            let mut result = stripped.to_string();
            result.truncate(stripped.len() - suffix.len());
            let clean = result.trim().to_string();
            if !clean.is_empty() {
                return Some(clean);
            }
        }
    }

    let clean = stripped.to_string();
    if clean.is_empty() || clean.to_lowercase() == lower {
        None
    } else {
        Some(clean)
    }
}

/// Strips parenthesized/bracketed decorations from a title for searching.
/// e.g. "Doin' It Right (feat. Panda Bear)" -> "Doin' It Right".
pub fn clean_title_for_search(title: &str) -> String {
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
    plain.trim().to_string()
}

fn build_query(title: &str, artists: &str) -> String {
    let title = title.trim();
    let artists = artists.trim();
    match (artists.is_empty(), title.is_empty()) {
        (true, true) => String::new(),
        (true, false) => title.to_string(),
        (false, true) => artists.to_string(),
        (false, false) => format!("{artists} - {title}"),
    }
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
    fn new(title: &str, artists: &str, duration_ms: Option<u64>) -> Self {
        let mut core_title = tokens(&strip_decorations(title));
        if core_title.is_empty() {
            core_title = tokens(title);
        }
        let primary_artist = tokens(artists.split(',').next().unwrap_or(""));
        let wanted = tokens(title).into_iter().chain(tokens(artists)).collect();
        Self {
            core_title,
            primary_artist,
            wanted,
            target_secs: duration_ms.map(|ms| ms as f64 / 1000.0),
        }
    }

    fn score(&self, entry: &Value) -> Option<f64> {
        let text = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or("");

        let title_tokens = tokens(text("title"));
        let is_speed_word = |t: &str| {
            ["slowed", "sped", "speed", "nightcore", "reverb", "8d", "daycore"].contains(&t)
        };
        // If the search query already contains a speed word (e.g. we are explicitly
        // searching for "Song (Slowed)"), we allow the result to contain other speed words
        // (e.g. "Slowed + Reverb") without rejecting it as an "other version".
        let is_searching_speed = self.wanted.iter().any(|w| is_speed_word(w));

        let is_other_version = title_tokens.iter().any(|t| {
            UNWANTED_WORDS.contains(&t.as_str())
                && !self.wanted.contains(t)
                && !(is_speed_word(t) && is_searching_speed)
        });

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
    let hits = needle.iter().filter(|t| haystack.contains(t.as_str())).count();
    hits as f64 / needle.len() as f64
}

fn duration_secs(entry: &Value) -> Option<f64> {
    entry
        .get("duration")
        .and_then(Value::as_f64)
        .or_else(|| {
            entry
                .get("duration_string")
                .and_then(Value::as_str)
                .and_then(parse_duration_string)
        })
}

fn parse_duration_string(s: &str) -> Option<f64> {
    s.trim()
        .split(':')
        .try_fold(0.0_f64, |acc, part| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile(title: &str, artists: &str, secs: u64) -> Profile {
        Profile::new(title, artists, Some(secs * 1000))
    }

    #[test]
    fn accepts_the_matching_upload() {
        let p = profile("Song Name", "Some Artist", 200);
        let entry = json!({"title": "Some Artist - Song Name (Official Audio)", "uploader": "Some Artist", "duration": 201.0});
        assert!(p.score(&entry).is_some());
    }

    #[test]
    fn rejects_other_versions_unless_query_has_them() {
        let p = profile("Song Name", "Some Artist", 200);
        let live = json!({"title": "Some Artist - Song Name (Live)", "uploader": "x", "duration": 200.0});
        let remix = json!({"title": "Song Name (DJ Mix Remix)", "uploader": "Some Artist", "duration": 200.0});
        assert!(p.score(&live).is_none());
        assert!(p.score(&remix).is_none());

        let live_p = profile("Song Name - Live", "Some Artist", 200);
        let live_again = json!({"title": "Some Artist - Song Name (Live)", "uploader": "x", "duration": 200.0});
        assert!(live_p.score(&live_again).is_some());
    }

    #[test]
    fn rejects_wrong_title_and_wrong_length() {
        let p = profile("Song Name", "Some Artist", 200);
        let other_song = json!({"title": "Totally Different Track", "uploader": "Some Artist", "duration": 200.0});
        let too_long = json!({"title": "Some Artist - Song Name", "uploader": "Some Artist", "duration": 600.0});
        assert!(p.score(&other_song).is_none());
        assert!(p.score(&too_long).is_none());
    }

    #[test]
    fn topic_upload_beats_a_music_video() {
        let p = profile("Song Name", "Some Artist", 200);
        let topic = json!({"title": "Song Name", "uploader": "Some Artist - Topic", "duration": 200.0});
        let video = json!({"title": "Some Artist - Song Name (Official Video)", "uploader": "SomeArtistVEVO", "duration": 200.0});
        assert!(p.score(&topic).unwrap() > p.score(&video).unwrap());
    }

    #[test]
    fn parses_duration_strings() {
        assert_eq!(parse_duration_string("3:25"), Some(205.0));
        assert_eq!(parse_duration_string("1:02:03"), Some(3723.0));
        assert_eq!(parse_duration_string("abc"), None);
    }

    #[test]
    fn strips_decorations_from_titles() {
        assert_eq!(strip_decorations("Song (feat. Someone)"), "Song");
        assert_eq!(strip_decorations("Song - 2011 Remaster"), "Song");
        assert_eq!(strip_decorations("Song [Deluxe] (Live)"), "Song");
    }
}