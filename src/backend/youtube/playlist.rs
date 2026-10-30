use url::Url;

pub fn is_playlist_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else { return false; };
    parsed.query_pairs().any(|(k, v)| k == "list" && !v.is_empty() && !v.starts_with("RD"))
}

pub fn video_id_from_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if parsed.host_str().is_some_and(|h| h.eq_ignore_ascii_case("youtu.be")) {
        return parsed.path_segments()?.find(|s| !s.is_empty()).map(String::from);
    }
    parsed.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.into_owned())
}

pub fn thumbnail_url(id: &str) -> String {
    format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")
}