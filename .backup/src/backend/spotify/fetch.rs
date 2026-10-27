//! Public Spotify metadata: oEmbed + embed page (no API keys).

use super::models::SpotifyMeta;
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build()
}

pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = agent()
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "image/*,*/*;q=0.8")
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(4 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// Track metadata from public oEmbed, with optional embed-page enrichment.
pub fn fetch_track_meta(spotify_url: &str) -> Result<SpotifyMeta, String> {
    let mut meta = fetch_oembed(spotify_url)?;
    // Prefer richer artist/title from the embed page when available.
    if let Ok(richer) = fetch_embed_meta(spotify_url) {
        if !richer.title.is_empty() {
            meta.title = richer.title;
        }
        if !richer.artists.is_empty() {
            meta.artists = richer.artists;
        }
        if meta.cover_url.is_none() {
            meta.cover_url = richer.cover_url;
        }
        if meta.duration_ms.is_none() {
            meta.duration_ms = richer.duration_ms;
        }
    }
    if meta.title.trim().is_empty() {
        return Err("could not read track title from Spotify".into());
    }
    Ok(meta)
}

fn fetch_oembed(spotify_url: &str) -> Result<SpotifyMeta, String> {
    let resp = agent()
        .get("https://open.spotify.com/oembed")
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/json")
        .query("url", spotify_url)
        .call()
        .map_err(|e| format!("Spotify oEmbed: {e}"))?;

    let v: Value = resp
        .into_json()
        .map_err(|e| format!("Spotify oEmbed JSON: {e}"))?;

    let raw_title = v
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    // oEmbed title is often just the track name; sometimes "Track - Artist".
    let (title, artists) = split_title_artist(&raw_title);

    let cover_url = v
        .get("thumbnail_url")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());

    Ok(SpotifyMeta {
        title,
        artists,
        cover_url,
        ..Default::default()
    })
}

fn split_title_artist(raw: &str) -> (String, String) {
    // Common patterns: "Song - Artist", "Song by Artist", "Artist - Song"
    if let Some((a, b)) = raw.split_once(" - ") {
        // Prefer longer right side as title when left looks like artist list
        return (b.trim().to_string(), a.trim().to_string());
    }
    if let Some((a, b)) = raw.split_once(" - ") {
        return (b.trim().to_string(), a.trim().to_string());
    }
    if let Some((title, artist)) = raw.split_once(" by ") {
        return (title.trim().to_string(), artist.trim().to_string());
    }
    (raw.to_string(), String::new())
}

/// Scrape open.spotify.com/embed/... for __NEXT_DATA__ entity fields.
fn fetch_embed_meta(spotify_url: &str) -> Result<SpotifyMeta, String> {
    let (kind, id) = super::spotify_id(spotify_url)
        .ok_or_else(|| "not a Spotify URL".to_string())?;
    let embed_url = format!("https://open.spotify.com/embed/{kind}/{id}");

    let resp = agent()
        .get(&embed_url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "text/html,application/xhtml+xml")
        .call()
        .map_err(|e| format!("Spotify embed: {e}"))?;

    let mut html = String::new();
    resp.into_reader()
        .take(2 * 1024 * 1024)
        .read_to_string(&mut html)
        .map_err(|e| e.to_string())?;

    let marker = "type=\"application/json\">";
    // Prefer __NEXT_DATA__
    let json_str = if let Some(start) = html.find("__NEXT_DATA__") {
        let after = &html[start..];
        let gt = after.find('>').ok_or("embed: no NEXT_DATA end")?;
        let rest = &after[gt + 1..];
        let end = rest.find("</script>").ok_or("embed: no NEXT_DATA close")?;
        rest[..end].trim()
    } else if let Some(idx) = html.find(marker) {
        let rest = &html[idx + marker.len()..];
        let end = rest.find("</script>").ok_or("embed: no script close")?;
        rest[..end].trim()
    } else {
        return Err("embed page has no JSON blob".into());
    };

    let root: Value =
        serde_json::from_str(json_str).map_err(|e| format!("embed JSON: {e}"))?;

    // Walk a few known shapes for entity data.
    let entity = find_entity(&root).ok_or_else(|| "embed: entity not found".to_string())?;

    let title = entity
        .get("name")
        .or_else(|| entity.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let artists = entity
        .get("artists")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .or_else(|| {
            entity
                .get("subtitle")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    let cover_url = entity
        .pointer("/visualIdentity/image/0/url")
        .or_else(|| entity.pointer("/images/0/url"))
        .or_else(|| entity.get("image").and_then(|v| v.as_str()).map(|_| entity.get("image").unwrap()))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            entity
                .get("visualIdentity")
                .and_then(|v| v.get("image"))
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|i| i.get("url"))
                .and_then(|u| u.as_str())
                .map(|s| s.to_string())
        });

    let duration_ms = entity
        .get("duration")
        .and_then(|v| v.as_u64())
        .or_else(|| entity.get("durationMs").and_then(|v| v.as_u64()));

    Ok(SpotifyMeta {
        title,
        artists,
        cover_url,
        duration_ms,
        ..Default::default()
    })
}

fn find_entity(root: &Value) -> Option<&Value> {
    // Direct
    if root.get("name").is_some() && (root.get("artists").is_some() || root.get("uri").is_some()) {
        return Some(root);
    }
    // NEXT_DATA props.pageProps.state.data.entity
    if let Some(e) = root.pointer("/props/pageProps/state/data/entity") {
        return Some(e);
    }
    if let Some(e) = root.pointer("/props/pageProps/entity") {
        return Some(e);
    }
    if let Some(e) = root.pointer("/entity") {
        return Some(e);
    }
    // Shallow search for an object with name + artists
    fn walk<'a>(v: &'a Value, depth: usize) -> Option<&'a Value> {
        if depth > 8 {
            return None;
        }
        match v {
            Value::Object(map) => {
                if map.contains_key("name")
                    && (map.contains_key("artists")
                        || map.contains_key("duration")
                        || map.contains_key("durationMs"))
                {
                    return Some(v);
                }
                for child in map.values() {
                    if let Some(f) = walk(child, depth + 1) {
                        return Some(f);
                    }
                }
            }
            Value::Array(arr) => {
                for child in arr {
                    if let Some(f) = walk(child, depth + 1) {
                        return Some(f);
                    }
                }
            }
            _ => {}
        }
        None
    }
    walk(root, 0)
}

/// Best-effort playlist/album track list from the public embed page.
pub fn fetch_playlist_metas(spotify_url: &str) -> Result<Vec<SpotifyMeta>, String> {
    let (kind, id) = super::spotify_id(spotify_url)
        .ok_or_else(|| "not a Spotify URL".to_string())?;
    if kind != "playlist" && kind != "album" {
        return Err("not a playlist or album URL".into());
    }
    let embed_url = format!("https://open.spotify.com/embed/{kind}/{id}");

    let resp = agent()
        .get(&embed_url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "text/html,application/xhtml+xml")
        .call()
        .map_err(|e| format!("Spotify embed: {e}"))?;

    let mut html = String::new();
    resp.into_reader()
        .take(3 * 1024 * 1024)
        .read_to_string(&mut html)
        .map_err(|e| e.to_string())?;

    let json_str = if let Some(start) = html.find("__NEXT_DATA__") {
        let after = &html[start..];
        let gt = after.find('>').ok_or("embed: no NEXT_DATA end")?;
        let rest = &after[gt + 1..];
        let end = rest.find("</script>").ok_or("embed: no NEXT_DATA close")?;
        rest[..end].trim()
    } else {
        return Err("playlist embed has no __NEXT_DATA__".into());
    };

    let root: Value =
        serde_json::from_str(json_str).map_err(|e| format!("embed JSON: {e}"))?;

    let mut out = Vec::new();
    collect_tracks(&root, &mut out, 0);
    // Cap like the rest of the app
    if out.len() > 500 {
        out.truncate(500);
    }
    if out.is_empty() {
        return Err("no tracks found in public playlist embed".into());
    }
    Ok(out)
}

fn collect_tracks(v: &Value, out: &mut Vec<SpotifyMeta>, depth: usize) {
    if depth > 12 || out.len() >= 500 {
        return;
    }
    match v {
        Value::Object(map) => {
            let is_track = map.get("type").and_then(|t| t.as_str()) == Some("track")
                || (map.contains_key("name")
                    && map.contains_key("artists")
                    && (map.contains_key("duration") || map.contains_key("durationMs") || map.contains_key("uri")));
            if is_track {
                if let Some(name) = map.get("name").and_then(|n| n.as_str()) {
                    if !name.is_empty() {
                        let artists = map
                            .get("artists")
                            .and_then(|a| a.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        let duration_ms = map
                            .get("duration")
                            .and_then(|d| d.as_u64())
                            .or_else(|| map.get("durationMs").and_then(|d| d.as_u64()));
                        out.push(SpotifyMeta {
                            title: name.to_string(),
                            artists,
                            duration_ms,
                            ..Default::default()
                        });
                    }
                }
            }
            for child in map.values() {
                collect_tracks(child, out, depth + 1);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                collect_tracks(child, out, depth + 1);
            }
        }
        _ => {}
    }
}