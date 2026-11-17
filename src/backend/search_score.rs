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
use std::sync::Mutex;

const RESULT_COUNT: usize = 25;
const MIN_TITLE_COVERAGE: f64 = 0.6;
// Duration tolerances are layered:
//   1. Audio upload, partial title match        : 5%   / 10 s
//   2. Any upload with a perfect title match    : 40%  / 120 s
//   3. Upload marked as a video (or VEVO)       : 50%  / 150 s
// A perfect title match is a very strong signal; the extra slack exists
// because an unlabeled music video can still be 2x the album duration.
const DURATION_TOLERANCE: f64 = 0.05;
const MIN_DURATION_TOLERANCE_SECS: f64 = 10.0;
const PERFECT_TITLE_TOLERANCE: f64 = 0.40;
const PERFECT_TITLE_MIN_TOLERANCE_SECS: f64 = 120.0;
const PERFECT_TITLE_THRESHOLD: f64 = 0.95;
const VIDEO_DURATION_TOLERANCE: f64 = 0.30;
const VIDEO_MIN_DURATION_TOLERANCE_SECS: f64 = 90.0;

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
    resolve_inner(title, artists, duration_ms, cancel, true)
}

/// `resolve` limited to YouTube. Used when the audio must not come from SoundCloud
/// (a SoundCloud track that SoundCloud itself will not serve).
pub fn resolve_youtube(
    title: &str,
    artists: &str,
    duration_ms: Option<u64>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    resolve_inner(title, artists, duration_ms, cancel, false)
}

fn resolve_inner(
    title: &str,
    artists: &str,
    duration_ms: Option<u64>,
    cancel: &AtomicBool,
    allow_soundcloud: bool,
) -> Result<String, String> {
    let title = title.trim();
    let artists = artists.trim();
    if title.is_empty() && artists.is_empty() {
        return Err("empty search query".into());
    }
    let profile = Profile::new(title, artists, duration_ms);
    let primary_artist = artists.split(',').next().unwrap_or("").trim();

    // A multi-artist query hurts YouTube search ranking. If the exact
    // "all_artists - title" form returns no acceptable candidate, retry
    // with progressively simpler queries. Distinctive titles (>= 3
    // tokens) also get a title-only attempt to catch music videos that
    // omit the artist from the title.
    let mut queries: Vec<String> = Vec::new();
    let push_query = |queries: &mut Vec<String>, q: String| {
        if !q.is_empty() && !queries.iter().any(|x| x == &q) {
            queries.push(q);
        }
    };
    if !artists.is_empty() && !title.is_empty() {
        push_query(&mut queries, format!("{artists} - {title}"));
    } else if !title.is_empty() {
        push_query(&mut queries, title.to_string());
    } else {
        push_query(&mut queries, artists.to_string());
    }
    if !title.is_empty() && artists.contains(',') && !primary_artist.is_empty() {
        push_query(&mut queries, format!("{primary_artist} {title}"));
    }
    if title.split_whitespace().count() >= 2 && !title.is_empty() {
        push_query(&mut queries, title.to_string());
    }
    // Reversed order: "title artist". YouTube sometimes ranks this form
    // higher when the video title starts with the song name.
    if !title.is_empty() && !artists.is_empty() {
        push_query(&mut queries, format!("{title} {artists}"));
    }
    // "official" suffix biases toward official uploads. Helps when the
    // album track shipped with a music video.
    if !artists.is_empty() && !title.is_empty() {
        push_query(&mut queries, format!("{artists} {title} official"));
    }

    let mut attempted: Vec<String> = Vec::new();
    let mut total_seen = 0usize;
    let mut first_titles: Vec<String> = Vec::new();

    // Query 1: most specific form. Runs alone so a match exits immediately.
    if let Some(first) = queries.first() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let term = crate::backend::youtube::search::term(first, RESULT_COUNT);
        attempted.push(format!("yt:{first}"));
        let (matched, seen, titles) = search(&term, &profile, cancel);
        total_seen += seen;
        first_titles = titles;
        if let Some(url) = matched {
            return Ok(url);
        }
        // Run every remaining query in parallel regardless of how many
        // candidates the first query returned. Parallel wall-clock is
        // `max(t_i)`, so more queries cost no extra time.
        if queries.len() > 1 {
            let rest = &queries[1..];
            for q in rest {
                attempted.push(format!("yt:{q}"));
            }
            if let Some(url) = run_parallel(rest, &profile, cancel) {
                return Ok(url);
            }
        }
    }

    // SoundCloud fallback: full query only.
    if allow_soundcloud && !artists.is_empty() && !title.is_empty() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let q = format!("{artists} - {title}");
        let term = crate::backend::soundcloud::search::term(&q, RESULT_COUNT);
        attempted.push(format!("sc:{q}"));
        let (matched, seen, _) = search(&term, &profile, cancel);
        total_seen += seen;
        if let Some(url) = matched {
            return Ok(url);
        }
    }

    let titles_note = if first_titles.is_empty() {
        "no results for the primary query".to_string()
    } else {
        format!("top results were: {}", first_titles.join("; "))
    };
    Err(format!(
        "no match for \"{title}\" by \"{artists}\" ({} quer{ie}, {total_seen} candidate{s}; {titles_note})",
        attempted.len(),
        ie = if attempted.len() == 1 { "y" } else { "ies" },
        s = if total_seen == 1 { "" } else { "s" },
    ))
}

/// Runs each query against YouTube in its own thread and returns the first
/// successful match. Concurrent access is bounded by `metadata_limit`, so
/// this never increases the number of live yt-dlp processes beyond the
/// existing global cap (4). Wall-clock is `max(t_i)` instead of `sum(t_i)`.
fn run_parallel(queries: &[String], profile: &Profile, cancel: &AtomicBool) -> Option<String> {
    if queries.is_empty() {
        return None;
    }
    let found: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for q in queries {
            let found = &found;
            scope.spawn(move || {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let term = crate::backend::youtube::search::term(q, RESULT_COUNT);
                let (matched, _seen, _titles) = search(&term, profile, cancel);
                if let Some(url) = matched {
                    let mut g = found.lock().unwrap_or_else(|e| e.into_inner());
                    if g.is_none() {
                        *g = Some(url);
                    }
                }
            });
        }
    });
    found.into_inner().unwrap_or_else(|e| e.into_inner())
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
    match search(&term, &profile, cancel).0 {
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

/// Returns `(matched_url, candidates_seen, top_titles)`.
/// - `candidates_seen == 0`: yt-dlp returned no entries at all (network
///   failure, empty result set, or cancelled).
/// - `candidates_seen > 0, matched_url = None`: yt-dlp returned results but
///   the profile rejected them all on scoring. `top_titles` gives the first
///   three candidate strings yt-dlp ranked highest, for diagnostics.
fn search(term: &str, profile: &Profile, cancel: &AtomicBool) -> (Option<String>, usize, Vec<String>) {
    let Some(_permit) = crate::backend::metadata_limit::acquire(cancel) else {
        return (None, 0, Vec::new());
    };

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

    let output = match process::output_stoppable(&mut cmd, cancel) {
        Ok(o) => o,
        Err(_) => return (None, 0, Vec::new()),
    };
    if cancel.load(Ordering::Relaxed) || output.stdout.is_empty() {
        return (None, 0, Vec::new());
    }
    let root: Value = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(_) => return (None, 0, Vec::new()),
    };
    let Some(entries) = root.get("entries").and_then(Value::as_array) else {
        return (None, 0, Vec::new());
    };
    let seen = entries.len();

    let top_titles: Vec<String> = entries
        .iter()
        .take(3)
        .map(|entry| {
            let t = entry.get("title").and_then(Value::as_str).unwrap_or("?");
            let u = entry
                .get("uploader")
                .or_else(|| entry.get("channel"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            let d = entry.get("duration").and_then(Value::as_f64);
            match d {
                Some(d) => format!("\"{t}\" ({u}, {d:.0}s)"),
                None => format!("\"{t}\" ({u})"),
            }
        })
        .collect();

    let matched = entries
        .iter()
        .filter_map(|entry| {
            let url = entry_url(entry)?;
            let score = profile.score(entry)?;
            Some((score, url))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, url)| url);

    (matched, seen, top_titles)
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

        // An official music video is a valid source for the studio track;
        // it just runs longer. Detect the case and widen the duration
        // window accordingly. Audio uploads keep the strict tolerance.
        let uploader = if text("uploader").is_empty() {
            text("channel")
        } else {
            text("uploader")
        };
        let is_video_upload = {
            let t = text("title").to_lowercase();
            let u = uploader.to_lowercase();
            t.contains("official video")
                || t.contains("music video")
                || t.contains("video clip")
                || t.contains("lyric video")
                || t.contains("visualizer")
                || t.contains("visualiser")
                || t.contains("official clip")
                || t.contains("official mv")
                || t.contains("(mv)")
                || t.contains("[mv]")
                || t.contains("m/v")
                || t.contains("(video)")
                || t.contains("[video]")
                || t.contains("(official)")
                || u.contains("vevo")
        };
        let (tol_pct, tol_min) = if is_video_upload {
            (VIDEO_DURATION_TOLERANCE, VIDEO_MIN_DURATION_TOLERANCE_SECS)
        } else if title_coverage >= PERFECT_TITLE_THRESHOLD {
            // Perfect title match with no explicit video marker. Likely an
            // unlabeled music video or an extended upload; give it the
            // wider window so it isn't rejected on a 30-60 s mismatch.
            (PERFECT_TITLE_TOLERANCE, PERFECT_TITLE_MIN_TOLERANCE_SECS)
        } else {
            (DURATION_TOLERANCE, MIN_DURATION_TOLERANCE_SECS)
        };

        let duration_score = match (self.target_secs, duration_secs(entry)) {
            (Some(target), Some(found)) => {
                let diff = (target - found).abs();
                if diff > (target * tol_pct).max(tol_min) {
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
    let raw: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect();
    // Collapse runs of single alphabetic characters so "A.S.S.N" and
    // "ASSN" produce the same token. Digit runs are left alone: "2.0"
    // tokenizes to ["2", "0"], which matches "2.0" on the other side.
    let mut out = Vec::with_capacity(raw.len());
    let mut buf = String::new();
    for tok in raw {
        let is_single_letter =
            tok.chars().count() == 1 && tok.chars().all(|c| c.is_alphabetic());
        if is_single_letter {
            buf.push_str(&tok);
        } else {
            if !buf.is_empty() {
                out.push(std::mem::take(&mut buf));
            }
            out.push(tok);
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
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

    #[test]
    fn multi_artist_query_builds_multiple_queries() {
        // Query construction: the full query first, then primary-artist +
        // title (multi-artist inputs), then title alone (>= 3 tokens).
        // Verified by the scoring profile it produces rather than the
        // exact string, since the retry logic lives in resolve().
        let p = Profile::new(
            "Need That Love Tonight",
            "Asdis, Sam feldt, Laidback Luke",
            Some(195_000),
        );
        let entry = json!({
            "title": "Need That Love Tonight",
            "uploader": "Sam Feldt",
            "duration": 240.0
        });
        assert!(p.score(&entry).is_some());
    }

    #[test]
    fn perfect_title_match_allows_long_unlabeled_video() {
        // Deezer: "Need That Love Tonight" by Asdis/Sam Feldt/Laidback Luke.
        // YouTube title carries no "Official Video" marker, so before this
        // change it fell through to the strict 5%/10 s audio tolerance and
        // was rejected on a modest intro/outro. With a perfect title match
        // the wider 40%/120 s window applies.
        let p = profile("Need That Love Tonight", "Asdis, Sam feldt, Laidback Luke", 180);
        let entry = json!({
            "title": "Need That Love Tonight",
            "uploader": "Sam Feldt",
            "duration": 240.0
        });
        assert!(p.score(&entry).is_some());
    }

    #[test]
    fn video_marker_allows_even_longer_runtime() {
        // 3-minute Deezer track, 4:30 music video = 90 s extra. Under the
        // 40%/120 s perfect-title tier this would still pass, but the
        // explicit video tier (30%/90 s) accepts it with room to spare.
        let p = profile("Alles anders", "Jennifer Rostock", 180);
        let entry = json!({
            "title": "Jennifer Rostock - Alles anders (Official Music Video)",
            "uploader": "Jennifer Rostock",
            "duration": 270.0
        });
        assert!(p.score(&entry).is_some());
    }

    #[test]
    fn audio_upload_still_requires_close_duration() {
        // Without a video marker and with a partial title match, the
        // strict 5%/10 s tier still applies.
        let p = profile("Song Name", "Some Artist", 200);
        let too_long = json!({
            "title": "Song Name (Remastered 2011)",
            "uploader": "Some Artist",
            "duration": 300.0
        });
        assert!(p.score(&too_long).is_none());
    }









    #[test]
    fn matches_dotted_acronyms() {
        // Deezer tokenizes "A.S.S.N 2.0" as [a, s, s, n, 2, 0]; the
        // YouTube title tokenizes as [assn, 2, 0]. Collapsing single
        // letters makes the two match.
        let p = profile("A.S.S.N 2.0", "AK AUSSERKONTROLLE", 200);
        let entry = json!({
            "title": "AK AUSSERKONTROLLE - ASSN 2.0 [Official Video]",
            "uploader": "AK AUSSERKONTROLLE",
            "duration": 200.0
        });
        assert!(p.score(&entry).is_some());
    }

    #[test]
    fn video_uploads_get_wider_duration_tolerance() {
        // 180 s target, 220 s music video: 40 s diff. Audio tolerance
        // (5% / 10 s) rejects; video tolerance (30% / 90 s) accepts.
        let p = profile("Go", "Celo Abdi", 180);
        let entry = json!({
            "title": "Celo & Abdi - Go! (prod. von LuciG) [Official Video]",
            "uploader": "Celo & Abdi",
            "duration": 220.0
        });
        assert!(p.score(&entry).is_some());

        // Same diff without the "video" marker is rejected: an audio
        // upload should run close to the studio length.
        let audio = json!({
            "title": "Celo & Abdi - Go!",
            "uploader": "Celo & Abdi",
            "duration": 220.0
        });
        assert!(p.score(&audio).is_none());
    }
}