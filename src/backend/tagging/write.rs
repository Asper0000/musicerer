use super::merge::MergedMeta;
use mp4ameta::{Data, FreeformIdent, Img, ImgFmt, Tag as Mp4Tag};
use std::path::Path;

/// Writes every field in a single mp4ameta read-modify-write pass.
///
/// mp4ameta covers the standard iTunes atoms (title, artist, album,
/// album_artist, year, track#, disc#, genre, cover) and the extra atoms
/// that audiotags 0.5.x has no setter for (label, bpm, isrc, gain). Doing
/// them all in one pass halves file I/O (one read, one write instead of
/// two of each). `read_existing_tags` in the parent module still uses
/// audiotags to read; that's a read-only path and unaffected.
pub fn write_tags(audio_path: &Path, meta: &MergedMeta) -> Result<(), String> {
    let mut tag = Mp4Tag::read_from_path(audio_path).map_err(|e| e.to_string())?;

    tag.set_title(&meta.title);
    if let Some(artist) = meta.artist.as_deref().filter(|a| !a.trim().is_empty()) {
        tag.set_artist(artist);
    }
    if let Some(album) = meta.album.as_deref().filter(|a| !a.trim().is_empty()) {
        tag.set_album(album);
    }
    if let Some(album_artist) = meta.album_artist.as_deref().filter(|a| !a.trim().is_empty()) {
        tag.set_album_artist(album_artist);
    }
    if let Some(year) = meta.year {
        tag.set_year(year.to_string());
    }
    if let Some(track) = meta.track_number.filter(|n| *n > 0) {
        tag.set_track_number(track as u16);
    }
    if let Some(disc) = meta.disc_number.filter(|n| *n > 0) {
        tag.set_disc_number(disc as u16);
    }
    if let Some(genre) = meta.genre.as_deref().filter(|g| !g.trim().is_empty()) {
        tag.set_genre(genre);
    }
    if let Some(bytes) = meta.cover.as_deref() {
        tag.set_artwork(Img {
            fmt: ImgFmt::Jpeg,
            data: bytes.to_vec(),
        });
    }
    if let Some(label) = meta.label.as_deref().filter(|l| !l.trim().is_empty()) {
        tag.set_data(
            FreeformIdent::new_static("com.apple.iTunes", "LABEL"),
            Data::Utf8(label.to_string()),
        );
    }
    if let Some(bpm) = meta.bpm {
        let bpm_u16 = bpm.round().clamp(0.0, u16::MAX as f32) as u16;
        tag.set_bpm(bpm_u16);
    }
    if let Some(isrc) = meta.isrc.as_deref().filter(|s| !s.trim().is_empty()) {
        tag.set_data(
            FreeformIdent::new_static("com.apple.iTunes", "ISRC"),
            Data::Utf8(isrc.to_string()),
        );
    }
    if let Some(gain) = meta.gain {
        tag.set_data(
            FreeformIdent::new_static("com.apple.iTunes", "REPLAYGAIN_TRACK_GAIN"),
            Data::Utf8(format!("{gain:.2} dB")),
        );
    }

    tag.write_to_path(audio_path).map_err(|e| e.to_string())
}