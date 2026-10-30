use super::merge::MergedMeta;
use audiotags::{MimeType, Picture, Tag};
use mp4ameta::{Data, FreeformIdent, Tag as Mp4Tag};
use std::path::Path;

/// Writes the merged tag set to the file in two passes:
///
/// 1. `audiotags` for the standard iTunes atoms: title, artist, album,
///    album_artist, year, track#, disc#, genre, cover.
/// 2. `mp4ameta` (the crate audiotags uses internally for M4A) for the
///    four fields with no audiotags setter:
///      * label -> ----:com.apple.iTunes:LABEL  (freeform)
///      * bpm   -> tmpo (u16)
///      * isrc  -> ----:com.apple.iTunes:ISRC   (freeform)
///      * gain  -> ----:com.apple.iTunes:REPLAYGAIN_TRACK_GAIN (freeform)
///
/// `mp4ameta` reads the current file state before writing, so atoms set
/// by the audiotags pass are preserved.
pub fn write_tags(audio_path: &Path, meta: &MergedMeta) -> Result<(), String> {
    write_standard_tags(audio_path, meta)?;
    write_mp4_extras(audio_path, meta)?;
    Ok(())
}

fn write_standard_tags(audio_path: &Path, meta: &MergedMeta) -> Result<(), String> {
    let path_str = audio_path.to_string_lossy().to_string();
    let mut tag = Tag::new()
        .read_from_path(&path_str)
        .map_err(|e| e.to_string())?;
    tag.set_title(&meta.title);
    if let Some(artist) = meta.artist.as_deref().filter(|a| !a.trim().is_empty()) {
        tag.set_artist(artist);
    }
    if let Some(album) = meta.album.as_deref().filter(|a| !a.trim().is_empty()) {
        tag.set_album_title(album);
    }
    if let Some(album_artist) = meta.album_artist.as_deref().filter(|a| !a.trim().is_empty()) {
        tag.set_album_artist(album_artist);
    }
    if let Some(year) = meta.year {
        tag.set_year(year);
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
        tag.set_album_cover(Picture {
            mime_type: MimeType::Jpeg,
            data: bytes,
        });
    }
    tag.write_to_path(&path_str).map_err(|e| e.to_string())
}

fn write_mp4_extras(audio_path: &Path, meta: &MergedMeta) -> Result<(), String> {
    let has_label = meta.label.as_deref().is_some_and(|s| !s.trim().is_empty());
    let has_isrc = meta.isrc.as_deref().is_some_and(|s| !s.trim().is_empty());
    let has_bpm = meta.bpm.is_some();
    let has_gain = meta.gain.is_some();
    if !has_label && !has_isrc && !has_bpm && !has_gain {
        return Ok(());
    }

    let mut tag = Mp4Tag::read_from_path(audio_path).map_err(|e| e.to_string())?;

    if let Some(label) = meta.label.as_deref().filter(|l| !l.trim().is_empty()) {
        tag.set_data(
            FreeformIdent::new("com.apple.iTunes", "LABEL"),
            Data::Utf8(label.to_string()),
        );
    }
    if let Some(bpm) = meta.bpm {
        let bpm_u16 = bpm.round().clamp(0.0, u16::MAX as f32) as u16;
        tag.set_bpm(bpm_u16);
    }
    if let Some(isrc) = meta.isrc.as_deref().filter(|s| !s.trim().is_empty()) {
        tag.set_data(
            FreeformIdent::new("com.apple.iTunes", "ISRC"),
            Data::Utf8(isrc.to_string()),
        );
    }
    if let Some(gain) = meta.gain {
        tag.set_data(
            FreeformIdent::new("com.apple.iTunes", "REPLAYGAIN_TRACK_GAIN"),
            Data::Utf8(format!("{gain:.2} dB")),
        );
    }

    tag.write_to_path(audio_path).map_err(|e| e.to_string())
}