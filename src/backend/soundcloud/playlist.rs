use url::Url;

pub fn is_playlist_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else { return false; };
    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    segments
        .iter()
        .position(|s| *s == "sets")
        .is_some_and(|i| i + 1 < segments.len())
}