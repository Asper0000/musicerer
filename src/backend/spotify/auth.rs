//! Legacy module: credentials are optional (public oEmbed resolver).

#[derive(Debug, Clone)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
}
