//! SoundCloud track metadata from `window.__sc_hydration`.

use crate::backend::utils::USER_AGENT;
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_HTML_BYTES: u64 = 4 * 1024 * 1024;
const HYDRATION_MARKER: &str = "window.__sc_hydration";

#[derive(Debug, Clone, Default)]
pub struct SoundCloudMeta {
    pub title: Option<String>,
    pub genre: Option<String>,
    pub release_date: Option<String>,
    pub label_name: Option<String>,
    pub isrc: Option<String>,
    pub artist: Option<String>,
    pub album_title: Option<String>,
    pub album_artist: Option<String>,
}

pub fn fetch(url: &str) -> Result<SoundCloudMeta, String> {
    let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
    let resp = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "text/html,application/xhtml+xml")
        .call()
        .map_err(|e| format!("SoundCloud page fetch: {e}"))?;

    let mut html = String::new();
    resp.into_reader()
        .take(MAX_HTML_BYTES)
        .read_to_string(&mut html)
        .map_err(|e| e.to_string())?;

    parse(&html)
}

fn parse(html: &str) -> Result<SoundCloudMeta, String> {
    let array_json = extract_hydration_array(html)
        .ok_or_else(|| "window.__sc_hydration not found".to_string())?;
    let arr: Value = serde_json::from_str(array_json)
        .map_err(|e| format!("SoundCloud hydration JSON: {e}"))?;
    let items = arr
        .as_array()
        .ok_or_else(|| "hydration is not an array".to_string())?;

    let sound = items
        .iter()
        .find(|item| {
            item.get("hydratable")
                .and_then(Value::as_str)
                .is_some_and(|s| s == "sound")
        })
        .ok_or_else(|| "no sound entry in hydration".to_string())?;
    let data = sound
        .get("data")
        .ok_or_else(|| "hydration sound entry has no data".to_string())?;

    let publisher = data.get("publisher_metadata");

    let str_field = |v: Option<&Value>| -> Option<String> {
        v.and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };

    let title = str_field(data.get("title"));
    let genre = str_field(data.get("genre"));
    let release_date = str_field(data.get("release_date"));
    let label_name = str_field(data.get("label_name"));

    let isrc = str_field(data.get("isrc"))
        .or_else(|| publisher.and_then(|p| str_field(p.get("isrc"))));
    let artist = publisher.and_then(|p| str_field(p.get("artist")));
    let album_title = publisher.and_then(|p| str_field(p.get("album_title")));
    let album_artist = publisher.and_then(|p| str_field(p.get("album_artist")));

    Ok(SoundCloudMeta {
        title,
        genre,
        release_date,
        label_name,
        isrc,
        artist,
        album_title,
        album_artist,
    })
}

/// Locates `window.__sc_hydration = [...]` and returns the bracketed slice.
/// A brace scanner is used instead of a regex because the JSON array
/// contains nested `]` characters that a non-greedy match would truncate.
fn extract_hydration_array(html: &str) -> Option<&str> {
    let start = html.find(HYDRATION_MARKER)?;
    let after = &html[start + HYDRATION_MARKER.len()..];
    let eq = after.find('=')?;
    let tail = &after[eq + 1..];
    let open = tail.find('[')?;
    let slice = &tail[open..];

    let mut depth: usize = 0;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in slice.as_bytes().iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&slice[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_hydration_array_with_nested_brackets() {
        let html = r#"<script>window.__sc_hydration = [{"hydratable":"sound","data":{"title":"T","genre":"G","publisher_metadata":{"artist":"A","album_title":"Al"}}}];</script>"#;
        let m = parse(html).unwrap();
        assert_eq!(m.title.as_deref(), Some("T"));
        assert_eq!(m.genre.as_deref(), Some("G"));
        assert_eq!(m.artist.as_deref(), Some("A"));
        assert_eq!(m.album_title.as_deref(), Some("Al"));
    }

    #[test]
    fn returns_error_when_marker_absent() {
        assert!(parse("<html>nothing</html>").is_err());
    }

    #[test]
    fn treats_whitespace_fields_as_absent() {
        let html = r#"window.__sc_hydration = [{"hydratable":"sound","data":{"title":"   ","genre":""}}];"#;
        let m = parse(html).unwrap();
        assert!(m.title.is_none());
        assert!(m.genre.is_none());
    }
}