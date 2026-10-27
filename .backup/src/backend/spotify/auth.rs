//! Client-credentials OAuth token cache for the Spotify Web API.

use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const USER_AGENT: &str = "Musicerer/0.2 (Spotify resolver)";

#[derive(Debug, Clone)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

static TOKEN_CACHE: Mutex<Option<CachedToken>> = Mutex::new(None);

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
}

/// Returns a bearer token, refreshing when expired or missing.
pub fn get_token(creds: &Credentials) -> Result<String, String> {
    if let Ok(guard) = TOKEN_CACHE.lock() {
        if let Some(ref t) = *guard {
            if t.expires_at > Instant::now() + Duration::from_secs(30) {
                return Ok(t.access_token.clone());
            }
        }
    }
    refresh(creds)
}

fn refresh(creds: &Credentials) -> Result<String, String> {
    if creds.client_id.trim().is_empty() || creds.client_secret.trim().is_empty() {
        return Err(
            "Spotify client ID and secret are not configured. Add them in Settings."
                .to_string(),
        );
    }

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();

    let resp = agent
        .post(TOKEN_URL)
        .set("User-Agent", USER_AGENT)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .auth(&creds.client_id, &creds.client_secret)
        .send_form(&[("grant_type", "client_credentials")])
        .map_err(|e| format!("Spotify token request failed: {e}"))?;

    let body: TokenResponse = resp
        .into_json()
        .map_err(|e| format!("Spotify token JSON: {e}"))?;

    if !body.token_type.eq_ignore_ascii_case("bearer") {
        return Err(format!("Unexpected Spotify token type: {}", body.token_type));
    }

    let token = body.access_token;
    let expires_at = Instant::now() + Duration::from_secs(body.expires_in.saturating_sub(30));
    if let Ok(mut guard) = TOKEN_CACHE.lock() {
        *guard = Some(CachedToken {
            access_token: token.clone(),
            expires_at,
        });
    }
    Ok(token)
}

/// Drop the in-memory token (e.g. after a 401).
pub fn invalidate() {
    if let Ok(mut guard) = TOKEN_CACHE.lock() {
        *guard = None;
    }
}