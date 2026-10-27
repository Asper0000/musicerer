//! Thin Spotify Web API client (tracks + playlist/album expansion).

use super::auth::{self, Credentials};
use super::models::*;
use std::time::Duration;

const API_BASE: &str = "https://api.spotify.com/v1";
const USER_AGENT: &str = "Musicerer/0.2 (Spotify resolver)";
const MAX_PLAYLIST: usize = 500;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build()
}

fn get_json<T: serde::de::DeserializeOwned>(
    creds: &Credentials,
    path: &str,
) -> Result<T, String> {
    let token = auth::get_token(creds)?;
    let url = if path.starts_with("http") {
        path.to_string()
    } else {
        format!("{API_BASE}{path}")
    };
    let resp = agent()
        .get(&url)
        .set("User-Agent", USER_AGENT)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .call();

    match resp {
        Ok(r) => r.into_json().map_err(|e| format!("Spotify JSON decode: {e}")),
        Err(ureq::Error::Status(401, _)) => {
            auth::invalidate();
            let token = auth::get_token(creds)?;
            agent()
                .get(&url)
                .set("User-Agent", USER_AGENT)
                .set("Authorization", &format!("Bearer {token}"))
                .set("Accept", "application/json")
                .call()
                .map_err(|e| format!("Spotify API after re-auth: {e}"))?
                .into_json()
                .map_err(|e| format!("Spotify JSON decode: {e}"))
        }
        Err(e) => Err(format!("Spotify API: {e}")),
    }
}

pub fn fetch_track(creds: &Credentials, track_id: &str) -> Result<SpotifyTrack, String> {
    let api: ApiTrack = get_json(creds, &format!("/tracks/{track_id}"))?;
    let id = api.id.clone();
    Ok(SpotifyTrack {
        id,
        meta: api.into_meta(),
    })
}

/// Expand a playlist or album into individual tracks (capped).
pub fn fetch_playlist_tracks(
    creds: &Credentials,
    kind: &str,
    id: &str,
) -> Result<(String, Vec<SpotifyTrack>), String> {
    match kind {
        "playlist" => fetch_playlist(creds, id),
        "album" => fetch_album(creds, id),
        _ => Err(format!("Unsupported Spotify resource: {kind}")),
    }
}

fn fetch_playlist(
    creds: &Credentials,
    id: &str,
) -> Result<(String, Vec<SpotifyTrack>), String> {
    #[derive(serde::Deserialize)]
    struct PlaylistHead {
        name: Option<String>,
    }
    let head: PlaylistHead = get_json(creds, &format!("/playlists/{id}?fields=name"))?;
    let title = head.name.unwrap_or_else(|| "Playlist".into());

    let mut tracks = Vec::new();
    let mut next: Option<String> = Some(format!(
        "/playlists/{id}/tracks?limit=50&fields=items(track(id,name,duration_ms,track_number,artists(name),album(name,release_date,images),external_ids)),next,total"
    ));

    while let Some(path) = next {
        if tracks.len() >= MAX_PLAYLIST { break; }
        let page: ApiPlaylistTracks = get_json(creds, &path)?;
        for item in page.items {
            if tracks.len() >= MAX_PLAYLIST { break; }
            if let Some(t) = item.track {
                if t.id.is_empty() || t.name.is_empty() { continue; }
                let id = t.id.clone();
                tracks.push(SpotifyTrack {
                    id,
                    meta: t.into_meta(),
                });
            }
        }
        next = page.next;
    }
    Ok((title, tracks))
}

fn fetch_album(
    creds: &Credentials,
    id: &str,
) -> Result<(String, Vec<SpotifyTrack>), String> {
    #[derive(serde::Deserialize)]
    struct AlbumHead {
        name: Option<String>,
        release_date: Option<String>,
        images: Option<Vec<ApiImage>>,
        artists: Option<Vec<ApiArtist>>,
    }
    let head: AlbumHead = get_json(creds, &format!("/albums/{id}"))?;
    let title = head.name.clone().unwrap_or_else(|| "Album".into());
    let album_name = head.name.unwrap_or_default();
    let year = head
        .release_date
        .as_deref()
        .and_then(|d| d.get(0..4))
        .and_then(|y| y.parse().ok());
    let cover = head.images.as_ref().and_then(|imgs| {
        imgs.iter()
            .filter(|i| i.width.unwrap_or(0) >= 200)
            .min_by_key(|i| i.width.unwrap_or(u32::MAX))
            .or_else(|| imgs.first())
            .map(|i| i.url.clone())
    });
    let album_artists = head
        .artists
        .unwrap_or_default()
        .into_iter()
        .map(|a| a.name)
        .collect::<Vec<_>>()
        .join(", ");

    let mut tracks = Vec::new();
    let mut next: Option<String> = Some(format!("/albums/{id}/tracks?limit=50"));

    while let Some(path) = next {
        if tracks.len() >= MAX_PLAYLIST { break; }
        let page: ApiAlbumTracks = get_json(creds, &path)?;
        for mut t in page.items {
            if tracks.len() >= MAX_PLAYLIST { break; }
            if t.id.is_empty() || t.name.is_empty() { continue; }
            if t.album.is_none() {
                t.album = Some(ApiAlbum {
                    name: Some(album_name.clone()),
                    release_date: head.release_date.clone(),
                    images: head.images.clone().or_else(|| {
                        cover.as_ref().map(|u| {
                            vec![ApiImage {
                                url: u.clone(),
                                width: Some(300),
                            }]
                        })
                    }),
                });
            }
            if t.artists.as_ref().map(|a| a.is_empty()).unwrap_or(true) && !album_artists.is_empty() {
                t.artists = Some(
                    album_artists
                        .split(", ")
                        .map(|n| ApiArtist { name: n.to_string() })
                        .collect(),
                );
            }
            let id = t.id.clone();
            let mut meta = t.into_meta();
            if meta.year.is_none() { meta.year = year; }
            if meta.cover_url.is_none() { meta.cover_url = cover.clone(); }
            tracks.push(SpotifyTrack { id, meta });
        }
        next = page.next;
    }
    Ok((title, tracks))
}

/// Download cover art bytes (JPEG preferred by tagging).
pub fn fetch_cover_bytes(url: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(12))
        .build();
    let resp = agent
        .get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "image/jpeg,image/png,image/*,*/*;q=0.8")
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(4 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}