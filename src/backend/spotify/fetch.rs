//! Public Spotify metadata: oEmbed + embed page (no API keys).

use super::models::SpotifyMeta;
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
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

pub fn fetch_track_meta(spotify_url: &str) -> Result<SpotifyMeta, String> {
    let mut meta = fetch_oembed(spotify_url)?;
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
    if let Some((a, b)) = raw.split_once(" - ") {
        return (b.trim().to_string(), a.trim().to_string());
    }
    if let Some((title, artist)) = raw.split_once(" by ") {
        return (title.trim().to_string(), artist.trim().to_string());
    }
    (raw.to_string(), String::new())
}

fn fetch_embed_html(kind: &str, id: &str) -> Result<String, String> {
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
    Ok(html)
}

fn extract_embed_jsons(html: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(start) = html.find(r#"id="__NEXT_DATA__""#) {
        if let Some(gt) = html[start..].find('>') {
            let rest = &html[start + gt + 1..];
            if let Some(end) = rest.find("</script>") {
                if let Ok(v) = serde_json::from_str::<Value>(rest[..end].trim()) {
                    out.push(v);
                }
            }
        }
    }
    for marker in [r#"id="resource""#, r#"id="initial-state""#] {
        let mut search = html;
        while let Some(start) = search.find(marker) {
            let after = &search[start..];
            if let Some(gt) = after.find('>') {
                let rest = &after[gt + 1..];
                if let Some(end) = rest.find("</script>") {
                    let raw = rest[..end].trim();
                    if let Ok(v) = serde_json::from_str::<Value>(raw) {
                        out.push(v);
                    } else if let Ok(decoded) = base64_decode(raw) {
                        if let Ok(v) = serde_json::from_slice::<Value>(&decoded) {
                            out.push(v);
                        }
                    }
                    search = &rest[end + 9..];
                    continue;
                }
            }
            break;
        }
    }
    out
}

fn base64_decode(s: &str) -> Result<Vec<u8>, ()> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if bytes.is_empty() {
        return Err(());
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;
    while i + 3 < bytes.len() {
        let (a, b, c, d) = (bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]);
        i += 4;
        if a == b'=' {
            break;
        }
        let va = val(a).ok_or(())?;
        let vb = val(b).ok_or(())?;
        out.push((va << 2) | (vb >> 4));
        if c != b'=' {
            let vc = val(c).ok_or(())?;
            out.push(((vb & 0x0f) << 4) | (vc >> 2));
            if d != b'=' {
                let vd = val(d).ok_or(())?;
                out.push(((vc & 0x03) << 6) | vd);
            }
        }
    }
    Ok(out)
}

fn meta_from_track_obj(obj: &Value) -> Option<SpotifyMeta> {
    let t = obj.get("track").unwrap_or(obj);
    let title = t
        .get("name")
        .or_else(|| t.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if title.is_empty() {
        return None;
    }
    if let Some(ty) = t.get("type").and_then(|v| v.as_str()) {
        if ty != "track" && ty != "episode" {
            if t.get("artists").is_none() && t.get("subtitle").is_none() {
                return None;
            }
        }
    }
    let artists = t
        .get("artists")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.get("name").and_then(|n| n.as_str()).or_else(|| a.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .or_else(|| t.get("subtitle").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .unwrap_or_default();
    let duration_ms = t
        .get("duration")
        .and_then(|v| v.as_u64())
        .or_else(|| t.get("duration_ms").and_then(|v| v.as_u64()))
        .or_else(|| t.get("durationMs").and_then(|v| v.as_u64()));
    let cover_url = t
        .pointer("/album/images/0/url")
        .or_else(|| t.pointer("/visualIdentity/image/0/url"))
        .or_else(|| t.pointer("/images/0/url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(SpotifyMeta {
        title,
        artists,
        duration_ms,
        cover_url,
        ..Default::default()
    })
}

fn collect_from_entity(entity: &Value, out: &mut Vec<SpotifyMeta>) {
    if let Some(list) = entity.get("trackList").and_then(|v| v.as_array()) {
        for item in list {
            if let Some(m) = meta_from_track_obj(item) {
                out.push(m);
            }
        }
    }
    if let Some(items) = entity.pointer("/tracks/items").and_then(|v| v.as_array()) {
        for item in items {
            if let Some(m) = meta_from_track_obj(item) {
                out.push(m);
            }
        }
    }
    if let Some(items) = entity.get("tracks").and_then(|v| v.as_array()) {
        for item in items {
            if let Some(m) = meta_from_track_obj(item) {
                out.push(m);
            }
        }
    }
}

fn walk_for_tracklists(v: &Value, out: &mut Vec<SpotifyMeta>, depth: usize) {
    if depth > 14 || out.len() >= 500 {
        return;
    }
    match v {
        Value::Object(map) => {
            if map.contains_key("trackList") || map.contains_key("tracks") {
                collect_from_entity(v, out);
            }
            for child in map.values() {
                walk_for_tracklists(child, out, depth + 1);
            }
        }
        Value::Array(arr) => {
            let mut added = 0;
            for item in arr {
                if let Some(m) = meta_from_track_obj(item) {
                    out.push(m);
                    added += 1;
                }
            }
            if added == 0 {
                for child in arr {
                    walk_for_tracklists(child, out, depth + 1);
                }
            }
        }
        _ => {}
    }
}

fn dedupe_tracks(tracks: Vec<SpotifyMeta>) -> Vec<SpotifyMeta> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(tracks.len());
    for t in tracks {
        let key = format!(
            "{}|{}",
            t.title.to_ascii_lowercase(),
            t.artists.to_ascii_lowercase()
        );
        if seen.insert(key) {
            out.push(t);
        }
    }
    out
}

pub fn fetch_playlist_metas(spotify_url: &str) -> Result<Vec<SpotifyMeta>, String> {
    let (kind, id) =
        super::spotify_id(spotify_url).ok_or_else(|| "not a Spotify URL".to_string())?;
    if kind != "playlist" && kind != "album" {
        return Err("not a playlist or album URL".into());
    }
    let html = fetch_embed_html(&kind, &id)?;
    let blobs = extract_embed_jsons(&html);
    if blobs.is_empty() {
        return Err("playlist embed has no JSON data".into());
    }
    let mut tracks = Vec::new();
    for blob in &blobs {
        if let Some(entity) = blob
            .pointer("/props/pageProps/state/data/entity")
            .or_else(|| blob.pointer("/props/pageProps/entity"))
            .or_else(|| blob.get("entity"))
            .or_else(|| blob.pointer("/data/entity"))
        {
            collect_from_entity(entity, &mut tracks);
        }
        if blob.get("trackList").is_some() || blob.get("tracks").is_some() {
            collect_from_entity(blob, &mut tracks);
        }
        if tracks.is_empty() {
            walk_for_tracklists(blob, &mut tracks, 0);
        }
    }
    let tracks = dedupe_tracks(tracks);
    if tracks.is_empty() {
        return Err(format!(
            "no tracks found in public {kind} embed (Spotify may have truncated the page)"
        ));
    }
    Ok(tracks.into_iter().take(500).collect())
}

fn fetch_embed_meta(spotify_url: &str) -> Result<SpotifyMeta, String> {
    let (kind, id) =
        super::spotify_id(spotify_url).ok_or_else(|| "not a Spotify URL".to_string())?;
    let html = fetch_embed_html(&kind, &id)?;
    let blobs = extract_embed_jsons(&html);
    for blob in blobs {
        if let Some(entity) = blob
            .pointer("/props/pageProps/state/data/entity")
            .or_else(|| blob.pointer("/props/pageProps/entity"))
            .or_else(|| blob.get("entity"))
        {
            if let Some(m) = meta_from_track_obj(entity) {
                return Ok(m);
            }
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
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let duration_ms = entity
                .get("duration")
                .and_then(|v| v.as_u64())
                .or_else(|| entity.get("durationMs").and_then(|v| v.as_u64()));
            if !title.is_empty() {
                return Ok(SpotifyMeta {
                    title,
                    artists,
                    cover_url,
                    duration_ms,
                    ..Default::default()
                });
            }
        }
    }
    Err("embed: entity not found".into())
}
