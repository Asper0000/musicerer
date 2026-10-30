//! Field-by-field precedence table for the metadata pipeline.
//!
//! `tag_audio` collects every available source (native info.json, SC
//! hydration, enrichment, Spotify, Deezer, oEmbed, existing tags) into a
//! `MergeInput` and calls `merge()`. `merge()` resolves each field
//! independently following the table below. Empty strings and whitespace
//! are treated as absent so a blank value never wins over a real one.
//!
//!   title        Spotify > Deezer > native > existing > SC-hydration > oEmbed > filename
//!   artist       Spotify > Deezer > SC-hydration > native > existing
//!   album        Deezer > SC-hydration > enrichment > native > existing
//!   year         Deezer > SC-hydration > enrichment > native
//!   track#       Deezer > enrichment > native
//!   disc#        Deezer > enrichment
//!   genre        SC-hydration > enrichment
//!   label        SC-hydration > enrichment
//!   isrc         Deezer > SC-hydration > enrichment
//!   bpm / gain   Deezer > enrichment
//!   album-artist SC-hydration > enrichment
//!   cover        Spotify > Deezer > disk > existing > oEmbed
//!
//! The resulting `MergedMeta` is what `write::write_tags` consumes.

use super::native::NativeMeta;
use crate::backend::deezer::DeezerMeta;
use crate::backend::enrich::Enrichment;
use crate::backend::soundcloud_meta::SoundCloudMeta;
use crate::backend::spotify::SpotifyMeta;

pub struct ExistingTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub cover: Option<Vec<u8>>,
}

pub struct MergeInput<'a> {
    pub native: &'a NativeMeta,
    pub existing: Option<&'a ExistingTags>,
    pub oembed_title: Option<String>,
    pub sc: Option<&'a SoundCloudMeta>,
    pub enrichment: Option<&'a Enrichment>,
    pub spotify: Option<&'a SpotifyMeta>,
    pub deezer: Option<&'a DeezerMeta>,
    pub disk_cover: Option<Vec<u8>>,
    pub oembed_cover: Option<Vec<u8>>,
    pub spotify_cover: Option<Vec<u8>>,
    pub deezer_cover: Option<Vec<u8>>,
    pub fallback_filename: Option<String>,
}

pub struct MergedMeta {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub genre: Option<String>,
    pub cover: Option<Vec<u8>>,
    // Populated by merge but not written to M4A tags by audiotags 0.3.x.
    // Kept here so Phase 4 can layer on top without changing the shape.
    #[allow(dead_code)] pub album_artist: Option<String>,
    #[allow(dead_code)] pub label: Option<String>,
    #[allow(dead_code)] pub isrc: Option<String>,
    #[allow(dead_code)] pub bpm: Option<f32>,
    #[allow(dead_code)] pub gain: Option<f32>,
}

fn non_empty(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn year_from_date(date: &str) -> Option<i32> {
    date.get(0..4).and_then(|y| y.parse::<i32>().ok())
}

pub fn merge(input: &MergeInput) -> MergedMeta {
    let artist = resolve_artist(input);
    let album = resolve_album(input);
    let mut album_artist = resolve_album_artist(input);

    // If there is an album but no album artist, use the main artist.
    // If there is no album, leave album_artist as None.
    if album_artist.is_none() && album.is_some() && artist.is_some() {
        album_artist = artist.clone();
    }

    MergedMeta {
        title: resolve_title(input),
        artist,
        album,
        year: resolve_year(input),
        track_number: resolve_track(input),
        disc_number: resolve_disc(input),
        genre: resolve_genre(input),
        cover: resolve_cover(input),
        album_artist,
        label: resolve_label(input),
        isrc: resolve_isrc(input),
        bpm: resolve_bpm(input),
        gain: resolve_gain(input),
    }
}

fn resolve_title(input: &MergeInput) -> String {
    if let Some(t) = input.spotify.and_then(|s| non_empty(&s.title)) {
        return t.to_string();
    }
    if let Some(t) = input.deezer.and_then(|d| non_empty(&d.title)) {
        return t.to_string();
    }
    if let Some(t) = input.native.title.as_deref().and_then(non_empty) {
        return t.to_string();
    }
    if let Some(t) = input.existing.and_then(|e| e.title.as_deref()).and_then(non_empty) {
        return t.to_string();
    }
    if let Some(t) = input.sc.and_then(|s| s.title.as_deref()).and_then(non_empty) {
        return t.to_string();
    }
    if let Some(t) = input.oembed_title.as_deref().and_then(non_empty) {
        return t.to_string();
    }
    if let Some(t) = input.fallback_filename.as_deref().and_then(non_empty) {
        return t.to_string();
    }
    "Unknown title".to_string()
}

fn resolve_artist(input: &MergeInput) -> Option<String> {
    if let Some(a) = input.spotify.and_then(|s| non_empty(&s.artists)) {
        return Some(a.to_string());
    }
    if let Some(a) = input.deezer.and_then(|d| non_empty(&d.artists)) {
        return Some(a.to_string());
    }
    if let Some(a) = input.sc.and_then(|s| s.artist.as_deref()).and_then(non_empty) {
        return Some(a.to_string());
    }
    if let Some(a) = input.native.artist.as_deref().and_then(non_empty) {
        return Some(a.to_string());
    }
    if let Some(a) = input.existing.and_then(|e| e.artist.as_deref()).and_then(non_empty) {
        return Some(a.to_string());
    }
    None
}

fn resolve_album(input: &MergeInput) -> Option<String> {
    if let Some(a) = input.deezer.and_then(|d| d.album.as_deref()).and_then(non_empty) {
        return Some(a.to_string());
    }
    if let Some(a) = input.sc.and_then(|s| s.album_title.as_deref()).and_then(non_empty) {
        return Some(a.to_string());
    }
    if let Some(a) = input.enrichment.and_then(|e| e.album.as_deref()).and_then(non_empty) {
        return Some(a.to_string());
    }
    if let Some(a) = input.native.album.as_deref().and_then(non_empty) {
        return Some(a.to_string());
    }
    if let Some(a) = input.existing.and_then(|e| e.album.as_deref()).and_then(non_empty) {
        return Some(a.to_string());
    }
    None
}

fn resolve_year(input: &MergeInput) -> Option<i32> {
    if let Some(y) = input.deezer.and_then(|d| d.release_date.as_deref()).and_then(year_from_date) {
        return Some(y);
    }
    if let Some(y) = input.sc.and_then(|s| s.release_date.as_deref()).and_then(year_from_date) {
        return Some(y);
    }
    if let Some(y) = input.enrichment.and_then(|e| e.year) {
        return Some(y);
    }
    if let Some(y) = input.native.year {
        return Some(y);
    }
    None
}

fn resolve_track(input: &MergeInput) -> Option<i32> {
    if let Some(n) = input
        .deezer
        .and_then(|d| d.track_position)
        .map(|n| n as i32)
        .filter(|n| *n > 0)
    {
        return Some(n);
    }
    if let Some(n) = input
        .enrichment
        .and_then(|e| e.track_number)
        .map(|n| n as i32)
        .filter(|n| *n > 0)
    {
        return Some(n);
    }
    if let Some(n) = input.native.track_number.filter(|n| *n > 0) {
        return Some(n);
    }
    None
}

fn resolve_disc(input: &MergeInput) -> Option<i32> {
    if let Some(n) = input
        .deezer
        .and_then(|d| d.disk_number)
        .map(|n| n as i32)
        .filter(|n| *n > 0)
    {
        return Some(n);
    }
    if let Some(n) = input
        .enrichment
        .and_then(|e| e.disc_number)
        .map(|n| n as i32)
        .filter(|n| *n > 0)
    {
        return Some(n);
    }
    None
}

fn resolve_genre(input: &MergeInput) -> Option<String> {
    if let Some(g) = input.sc.and_then(|s| s.genre.as_deref()).and_then(non_empty) {
        return Some(g.to_string());
    }
    if let Some(g) = input.enrichment.and_then(|e| e.genre.as_deref()).and_then(non_empty) {
        return Some(g.to_string());
    }
    None
}

fn resolve_album_artist(input: &MergeInput) -> Option<String> {
    if let Some(a) = input
        .deezer
        .and_then(|d| d.album_artist.as_deref())
        .and_then(non_empty)
    {
        return Some(a.to_string());
    }
    if let Some(a) = input
        .sc
        .and_then(|s| s.album_artist.as_deref())
        .and_then(non_empty)
    {
        return Some(a.to_string());
    }
    if let Some(a) = input
        .enrichment
        .and_then(|e| e.album_artist.as_deref())
        .and_then(non_empty)
    {
        return Some(a.to_string());
    }
    None
}

fn resolve_label(input: &MergeInput) -> Option<String> {
    if let Some(l) = input.sc.and_then(|s| s.label_name.as_deref()).and_then(non_empty) {
        return Some(l.to_string());
    }
    if let Some(l) = input.enrichment.and_then(|e| e.label.as_deref()).and_then(non_empty) {
        return Some(l.to_string());
    }
    None
}

fn resolve_isrc(input: &MergeInput) -> Option<String> {
    if let Some(i) = input.deezer.and_then(|d| d.isrc.as_deref()).and_then(non_empty) {
        return Some(i.to_string());
    }
    if let Some(i) = input.sc.and_then(|s| s.isrc.as_deref()).and_then(non_empty) {
        return Some(i.to_string());
    }
    if let Some(i) = input.enrichment.and_then(|e| e.isrc.as_deref()).and_then(non_empty) {
        return Some(i.to_string());
    }
    None
}

fn resolve_bpm(input: &MergeInput) -> Option<f32> {
    if let Some(b) = input.deezer.and_then(|d| d.bpm) {
        return Some(b);
    }
    if let Some(b) = input.enrichment.and_then(|e| e.bpm) {
        return Some(b);
    }
    None
}

fn resolve_gain(input: &MergeInput) -> Option<f32> {
    if let Some(g) = input.deezer.and_then(|d| d.gain) {
        return Some(g);
    }
    if let Some(g) = input.enrichment.and_then(|e| e.gain) {
        return Some(g);
    }
    None
}

fn resolve_cover(input: &MergeInput) -> Option<Vec<u8>> {
    // Strict precedence: original source wins. If the job was Spotify,
    // `spotify_cover` wins. If the job was YouTube, `spotify_cover` and
    // `deezer_cover` are None, so `disk_cover` (native) wins.
    if let Some(c) = input.spotify_cover.as_deref() {
        return Some(c.to_vec());
    }
    if let Some(c) = input.deezer_cover.as_deref() {
        return Some(c.to_vec());
    }
    if let Some(c) = input.disk_cover.as_deref() {
        return Some(c.to_vec());
    }
    if let Some(c) = input.existing.and_then(|e| e.cover.as_deref()) {
        return Some(c.to_vec());
    }
    if let Some(c) = input.oembed_cover.as_deref() {
        return Some(c.to_vec());
    }
    None
}