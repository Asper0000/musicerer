use crate::backend::Source;

/// Placeholder shown until a YouTube title is known. Compared against, so keep in one place.
pub const FETCHING_TITLE: &str = "Fetching title\u{2026}";
pub const READING_PLAYLIST: &str = "Reading playlist\u{2026}";

pub fn guess_name_from_url(url: &str, source: Source) -> String {
    match source {
        Source::SoundCloud => {
            if let Ok(u) = url::Url::parse(url) {
                if let Some(last) = u.path().rsplit('/').find(|s| !s.is_empty()) {
                    return deslug(last);
                }
            }
            "SoundCloud track".to_string()
        }
        Source::YouTube => FETCHING_TITLE.to_string(),
        Source::Unsupported => url.to_string(),
    }
}

pub fn deslug(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();
    cleaned.trim().to_string()
}