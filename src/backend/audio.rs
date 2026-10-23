use std::fs;
use std::path::{Path, PathBuf};

pub fn find_audio(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(candidate) = find_audio(&path) {
                let size = fs::metadata(&candidate).map(|m| m.len()).unwrap_or(0);
                if best.as_ref().is_none_or(|(best_size, _)| size > *best_size) {
                    best = Some((size, candidate));
                }
            }
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
        if matches!(ext.as_deref(), Some("m4a") | Some("mp4")) {
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if best.as_ref().is_none_or(|(best_size, _)| size > *best_size) {
                best = Some((size, path));
            }
        }
    }
    best.map(|(_, path)| path)
}

pub fn find_thumbnail(audio_path: &Path) -> Option<PathBuf> {
    let jpg = audio_path.with_extension("jpg");
    if jpg.exists() { return Some(jpg); }
    let png = audio_path.with_extension("png");
    if png.exists() { return Some(png); }
    None
}

pub fn square_cover(src: &Path, temp_dir: &Path) -> Result<Vec<u8>, String> {
    use image::GenericImageView;
    let image = image::open(src).map_err(|e| e.to_string())?;
    let (width, height) = image.dimensions();
    let side = width.min(height).max(1);
    let x = (width - side) / 2;
    let y = (height - side) / 2;
    let cropped = image.crop_imm(x, y, side, side);
    let cover_path = temp_dir.join("cover_square.jpg");
    cropped
        .save_with_format(&cover_path, image::ImageFormat::Jpeg)
        .map_err(|e| e.to_string())?;
    fs::read(cover_path).map_err(|e| e.to_string())
}

pub fn make_rounded_thumb(src: &Path, size: u32, radius: f32) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::open(src).map_err(|e| e.to_string())?;
    let (w0, h0) = (img.width(), img.height());
    let side = w0.min(h0).max(1);
    let x = (w0 - side) / 2;
    let y = (h0 - side) / 2;
    let cropped = img.crop_imm(x, y, side, side);
    let resized = cropped.resize_exact(size, size, image::imageops::FilterType::Lanczos3);
    let rgba = resized.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let mut pixels = rgba.into_raw();

    let wf = w as f32;
    let hf = h as f32;
    let half_w = wf / 2.0;
    let half_h = hf / 2.0;
    let inner_x = (half_w - radius).max(0.0);
    let inner_y = (half_h - radius).max(0.0);

    for y in 0..h {
        for x in 0..w {
            let px = (x as f32 + 0.5) - half_w;
            let py = (y as f32 + 0.5) - half_h;
            let dx = (px.abs() - inner_x).max(0.0);
            let dy = (py.abs() - inner_y).max(0.0);
            let dist = (dx * dx + dy * dy).sqrt() - radius;
            let alpha = (0.5 - dist).clamp(0.0, 1.0);
            let idx = ((y * w + x) * 4) as usize + 3;
            pixels[idx] = (pixels[idx] as f32 * alpha) as u8;
        }
    }
    Ok((w, h, pixels))
}
