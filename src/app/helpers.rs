use crate::backend::Source;

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
        Source::YouTube => "Fetching title\u{2026}".to_string(),
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