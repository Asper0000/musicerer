//! Cover fetch helper (public HTTP, no API key).

pub fn fetch_cover_bytes(url: &str) -> Result<Vec<u8>, String> {
    super::fetch::fetch_bytes(url)
}
