//! SoundCloud oEmbed lookups (title and thumbnail) used for previews and tagging.

use crate::backend::utils::USER_AGENT;
use std::io::Read;
use std::sync::LazyLock;
use std::time::Duration;

static OEMBED_AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    crate::backend::http::agent_with_global_timeout(None, OEMBED_TIMEOUT)
});
static THUMB_AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    crate::backend::http::agent_with_global_timeout(None, THUMBNAIL_TIMEOUT)
});

const OEMBED_TIMEOUT: Duration = Duration::from_secs(5);
const THUMBNAIL_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_OEMBED_THUMBNAIL_BYTES: u64 = 4 * 1024 * 1024;

fn fetch_oembed_metadata(url: &str) -> Result<(String, Option<String>), String> {
    // Build query parameters separately so playlist URLs that already contain a
    // query string are encoded correctly rather than truncating the oEmbed URL.
    let mut res = OEMBED_AGENT
        .get("https://soundcloud.com/oembed")
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json,text/plain,*/*")
        .query("format", "json")
        .query("url", url)
        .call()
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = res.body_mut().read_json().map_err(|e| e.to_string())?;
    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let thumb_url = json
        .get("thumbnail_url")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Ok((title, thumb_url))
}

/// Fetches a SoundCloud title and optional cover bytes without creating temporary files.
/// Thumbnail failures do not discard a title that was retrieved successfully.
pub fn fetch_oembed(url: &str) -> Result<(String, Option<Vec<u8>>), String> {
    let (title, thumb_url) = fetch_oembed_metadata(url)?;
    let thumbnail = thumb_url.and_then(|url| fetch_thumbnail_bytes(&url).ok());
    Ok((title, thumbnail))
}

fn fetch_thumbnail_bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut response = THUMB_AGENT
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header(
            "Accept",
            "image/avif,image/webp,image/apng,image/*,*/*;q=0.8",
        )
        .call()
        .map_err(|e| e.to_string())?;

    let mut bytes = Vec::with_capacity(64 * 1024);
    response
        .body_mut().as_reader()
        .take(MAX_OEMBED_THUMBNAIL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_OEMBED_THUMBNAIL_BYTES {
        return Err("SoundCloud thumbnail exceeds the 4 MiB limit".to_string());
    }
    Ok(bytes)
}
