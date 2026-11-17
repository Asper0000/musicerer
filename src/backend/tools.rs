//! Locates external tools used by Musicerer.
//!
//! Required tools are normally installed and updated through the Installation tab.
//! Resolution order: updater-managed directory, local `tools` directories, then PATH.
//! No third-party tool binaries are embedded in the executable or application packages.

use super::process::hidden_command;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    YtDlp,
    Ffmpeg,
    Ffprobe,
    Deno,
    /// Optional helper for the best-effort SoundCloud DRM fallback.
    /// It is not required for normal downloads and is not shipped with releases.
    SoundCloud,
}

impl Tool {
    /// Required tools managed by the Installation tab.
    pub const ALL: [Tool; 4] = [Tool::YtDlp, Tool::Ffmpeg, Tool::Ffprobe, Tool::Deno];

    pub fn label(self) -> &'static str {
        match self {
            Tool::YtDlp => "yt-dlp",
            Tool::Ffmpeg => "ffmpeg",
            Tool::Ffprobe => "ffprobe",
            Tool::Deno => "deno",
            Tool::SoundCloud => "soundcloud",
        }
    }

    pub fn file_name(self) -> String {
        if cfg!(windows) { format!("{}.exe", self.label()) } else { self.label().to_string() }
    }
}

static PATH_CACHE: LazyLock<Mutex<HashMap<Tool, Option<PathBuf>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Where updater-managed tools are stored.
pub fn managed_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Musicerer").join("tools"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin { Missing, Managed, Local, System }

pub fn origin(tool: Tool) -> Origin {
    let Some(path) = find(tool) else { return Origin::Missing; };
    if managed_dir().is_some_and(|dir| path.starts_with(dir)) { return Origin::Managed; }
    if tools_dirs().iter().any(|dir| path.starts_with(dir)) { return Origin::Local; }
    Origin::System
}

fn tools_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::with_capacity(2);
    if let Ok(cwd) = std::env::current_dir() { dirs.push(cwd.join("tools")); }
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|p| p.join("tools"))) {
        if !dirs.contains(&dir) { dirs.push(dir); }
    }
    dirs
}

fn find_in_dirs(dirs: impl IntoIterator<Item = PathBuf>, file_name: &str) -> Option<PathBuf> {
    dirs.into_iter().map(|dir| dir.join(file_name)).find(|path| path.is_file())
}

/// Locates a tool. Managed tools take precedence; PATH is searched once per tool.
pub fn find(tool: Tool) -> Option<PathBuf> {
    let name = tool.file_name();
    if let Some(dir) = managed_dir() {
        let path = dir.join(&name);
        if path.is_file() { return Some(path); }
    }
    if let Some(path) = find_in_dirs(tools_dirs(), &name) { return Some(path); }
    {
        let cache = PATH_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = cache.get(&tool) { return cached.clone(); }
    }
    let from_path = std::env::var_os("PATH")
        .and_then(|path| find_in_dirs(std::env::split_paths(&path), &name));
    let mut cache = PATH_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.insert(tool, from_path.clone());
    from_path
}

pub fn missing() -> Vec<Tool> {
    Tool::ALL.into_iter().filter(|tool| find(*tool).is_none()).collect()
}

fn search_path() -> Option<OsString> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for tool in Tool::ALL {
        if let Some(dir) = find(tool).and_then(|path| path.parent().map(Path::to_path_buf)) {
            if !dirs.contains(&dir) { dirs.push(dir); }
        }
    }
    if dirs.is_empty() { return None; }
    dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    std::env::join_paths(dirs).ok()
}

pub fn command(tool: Tool) -> Command {
    let program = find(tool).unwrap_or_else(|| PathBuf::from(tool.file_name()));
    let mut cmd = hidden_command(program);
    if let Some(path) = search_path() { cmd.env("PATH", path); }
    cmd
}

pub fn add_ffmpeg_location(cmd: &mut Command) {
    if let Some(dir) = find(Tool::Ffmpeg).and_then(|path| path.parent().map(Path::to_path_buf)) {
        cmd.arg("--ffmpeg-location").arg(dir);
    }
}

pub fn missing_report() -> Option<String> {
    let missing = missing();
    if missing.is_empty() { return None; }
    let mut text = String::from("These tools could not be found:\n\n");
    for tool in &missing { text.push_str(&format!("  \u{2022} {}", tool.file_name())); text.push('\n'); }
    text.push_str(
        "\nOpen the Installation tab to download and install missing tools automatically, or install them so they are available on your PATH.\n\n\
         Musicerer will open, but downloads will not work until the required tools are available.",
    );
    Some(text)
}
