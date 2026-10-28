@echo off
cd /d "%~dp0"
if not exist "src" ( echo ERROR: Run from project root. & pause & exit /b 1 )

powershell -NoProfile -ExecutionPolicy Bypass -Command "$f = '%~f0'; $l = [IO.File]::ReadAllLines($f); $i = 0; for ($j = 0; $j -lt $l.Count; $j++) { if ($l[$j] -eq '#__PS_START__#') { $i = $j + 1; break } }; if ($i -eq 0) { Write-Error 'PS marker missing'; exit 1 }; $c = ($l[$i..($l.Count-1)] -join [char]10); Invoke-Expression $c"

if errorlevel 1 ( echo. & echo Patch failed. & pause & exit /b 1 )
echo. & echo Done. Rebuild with: cargo build & pause
exit /b

#__PS_START__#
$ErrorActionPreference = 'Stop'
if (-not (Test-Path 'src')) { throw "Not in project root. Working dir: $PWD" }

function Write-File([string]$path, [string]$content) {
    $full = Join-Path $PWD $path
    $dir = Split-Path -Parent $full
    if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    [IO.File]::WriteAllText($full, $content, [Text.UTF8Encoding]::new($false))
    Write-Host "  Wrote $path"
}

function Patch([string]$path, [string]$old, [string]$new) {
    $full = Join-Path $PWD $path
    $raw = [IO.File]::ReadAllText($full)
    $oldN = $old.Replace("`r`n","`n"); $newN = $new.Replace("`r`n","`n"); $rawN = $raw.Replace("`r`n","`n")
    if ($rawN.Contains($newN)) { Write-Host "  Already applied: $path"; return }
    if (-not $rawN.Contains($oldN)) { throw "Pattern not found in $path`n--- Looking for:`n$old" }
    $res = $rawN.Replace($oldN, $newN)
    if ($raw.Contains("`r`n")) { $res = $res.Replace("`n","`r`n") }
    [IO.File]::WriteAllText($full, $res, [Text.UTF8Encoding]::new($false))
    Write-Host "  Patched $path"
}

Write-Host ''
Write-Host 'Finishing clippy fixes...'
Write-Host ''

# ---- 9+10. utils.rs: full overwrite with both fixes ----
Write-Host '[ 9+10] utils.rs: overwrite with collapsible-if and map_or fixes'
Write-File 'src/backend/utils.rs' @'
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() { break; }
            }
            continue;
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
    while result.ends_with('.') || result.ends_with(' ') { result.pop(); }
    if result.is_empty() { result.push_str("track"); }
    if is_windows_reserved(&result) { result.push('_'); }
    result
}

fn is_windows_reserved(name: &str) -> bool {
    let trimmed = name.trim_end_matches(' ');
    let stem = trimmed.split('.').next().unwrap_or("");
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
            | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9"
            | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9"
    )
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

static PLACE_LOCK: Mutex<()> = Mutex::new(());

pub fn move_into_dir(src: &Path, dir: &Path, stem: &str) -> io::Result<PathBuf> {
    let _guard = PLACE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dest = unique_path(dir, stem);
    if fs::rename(src, &dest).is_err() {
        fs::copy(src, &dest)?;
        let _ = fs::remove_file(src);
    }
    Ok(dest)
}

pub fn find_file_ext(dir: &Path, ext: &str) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() { continue; }
        let matches = path.extension().and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case(ext)).unwrap_or(false);
        if matches {
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if best.as_ref().is_none_or(|(s, _)| size > *s) { best = Some((size, path)); }
        }
    }
    best.map(|(_, p)| p)
}

pub fn cleanup_stale_temp_dirs() {
    use std::time::{Duration, SystemTime};
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else { return; };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("musicerer-tmp-") && !name.starts_with("musicerer-preview-") { continue; }
        let is_old = entry.metadata().and_then(|m| m.modified()).ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > Duration::from_secs(3600));
        if is_old { let _ = fs::remove_dir_all(entry.path()); }
    }
}
'@

# ---- 11. vpn.rs: for_kv_map ----
Write-Host '[11/11] vpn.rs: iterate .values()'
Patch 'src/backend/vpn.rs' @'
    for (_pid, process) in sys.processes() {
'@ @'
    for process in sys.processes().values() {
'@

Write-Host ''
Write-Host 'All clippy warnings fixed.'