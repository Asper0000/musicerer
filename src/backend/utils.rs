use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                let _ = chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() { break; }
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

pub fn parse_percent(line: &str) -> Option<f32> {
    let percent_index = line.find('%')?;
    let before = &line[..percent_index];
    let token = before.split_whitespace().last()?;
    let value = token.parse::<f32>().ok()?;
    Some((value / 100.0).clamp(0.0, 1.0))
}

pub fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let cleaned = out.trim().trim_matches('.').trim();
    let mut result: String = cleaned.chars().take(180).collect();
    if result.is_empty() { result.push_str("track"); }
    result
}

pub fn unique_path(dir: &Path, stem: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.m4a"));
    let mut counter = 1u32;
    while path.exists() {
        path = dir.join(format!("{stem} ({counter}).m4a"));
        counter += 1;
    }
    path
}

/// Serialises "pick a free name + move the file there" so two parallel downloads with the
/// same title can never pick the same destination and overwrite each other.
static PLACE_LOCK: Mutex<()> = Mutex::new(());

pub fn move_into_dir(src: &Path, dir: &Path, stem: &str) -> io::Result<PathBuf> {
    let _guard = PLACE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dest = unique_path(dir, stem);
    fs::rename(src, &dest)?;
    Ok(dest)
}

pub fn find_file_ext(dir: &Path, ext: &str) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() { continue; }
        let matches = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case(ext))
            .unwrap_or(false);
        if matches {
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if best.as_ref().is_none_or(|(s, _)| size > *s) {
                best = Some((size, path));
            }
        }
    }
    best.map(|(_, p)| p)
}
