//! Everything about the external tools Musicerer depends on, in one place.
//!
//! Resolution order for every tool:
//!   1. the copy embedded in the executable (only in `--features bundled` builds),
//!   2. a `tools` folder in the current directory, then next to the executable,
//!   3. the system `PATH`.

use super::process::hidden_command;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    YtDlp,
    Ffmpeg,
    Ffprobe,
    Deno,
    SoundCloud,
}

impl Tool {
    /// Declaration order matters: it is also the index used by the embedded-tool cache.
    pub const ALL: [Tool; 5] = [Tool::YtDlp, Tool::Ffmpeg, Tool::Ffprobe, Tool::Deno, Tool::SoundCloud];

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

    /// The SoundCloud fallback only matters for DRM-protected SoundCloud tracks.
    pub fn is_required(self) -> bool {
        self != Tool::SoundCloud
    }
}

/// True when this executable carries the tools inside itself.
pub const BUNDLED: bool = cfg!(all(feature = "bundled", target_os = "windows"));

#[cfg(all(feature = "bundled", target_os = "windows"))]
mod embedded {
    use super::Tool;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    const YT_DLP: &[u8] = include_bytes!("../../tools/yt-dlp.exe");
    const FFMPEG: &[u8] = include_bytes!("../../tools/ffmpeg.exe");
    const FFPROBE: &[u8] = include_bytes!("../../tools/ffprobe.exe");
    const DENO: &[u8] = include_bytes!("../../tools/deno.exe");
    const SOUNDCLOUD: &[u8] = include_bytes!("../../tools/soundcloud.exe");

    static EXTRACTED: [OnceLock<Option<PathBuf>>; 5] =
        [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];

    fn bytes(tool: Tool) -> &'static [u8] {
        match tool {
            Tool::YtDlp => YT_DLP,
            Tool::Ffmpeg => FFMPEG,
            Tool::Ffprobe => FFPROBE,
            Tool::Deno => DENO,
            Tool::SoundCloud => SOUNDCLOUD,
        }
    }

    /// Path of the extracted copy; extracted lazily, once per tool per run.
    pub fn path(tool: Tool) -> Option<PathBuf> {
        EXTRACTED[tool as usize].get_or_init(|| extract(tool)).clone()
    }

    fn extract(tool: Tool) -> Option<PathBuf> {
        let data = bytes(tool);
        let dir = std::env::temp_dir().join(format!("musicerer-tools-{}", env!("CARGO_PKG_VERSION")));
        std::fs::create_dir_all(&dir).ok()?;

        let target = dir.join(tool.file_name());
        let up_to_date = std::fs::metadata(&target).map(|m| m.len() == data.len() as u64).unwrap_or(false);
        if up_to_date {
            return Some(target);
        }

        // Write to a private name first so a second instance never sees a half-written exe.
        let tmp = dir.join(format!("{}.{}.tmp", tool.file_name(), std::process::id()));
        std::fs::write(&tmp, data).ok()?;
        if std::fs::rename(&tmp, &target).is_err() {
            // Another instance may be running the existing copy; that copy is fine to reuse.
            let _ = std::fs::remove_file(&tmp);
            if !target.is_file() {
                return None;
            }
        }
        Some(target)
    }
}

#[cfg(not(all(feature = "bundled", target_os = "windows")))]
mod embedded {
    use super::Tool;
    use std::path::PathBuf;

    pub fn path(_tool: Tool) -> Option<PathBuf> {
        None
    }
}

fn tools_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::with_capacity(2);
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("tools"));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|p| p.join("tools"))) {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

fn find_in_dirs(dirs: impl IntoIterator<Item = PathBuf>, file_name: &str) -> Option<PathBuf> {
    dirs.into_iter().map(|d| d.join(file_name)).find(|p| p.is_file())
}

/// Locates a tool. Cheap enough to call before every spawn, so a tool dropped into
/// the `tools` folder while the app is running is picked up without a restart.
pub fn find(tool: Tool) -> Option<PathBuf> {
    let name = tool.file_name();
    embedded::path(tool)
        .or_else(|| find_in_dirs(tools_dirs(), &name))
        .or_else(|| {
            let path_var = std::env::var_os("PATH")?;
            find_in_dirs(std::env::split_paths(&path_var), &name)
        })
}

pub fn missing() -> Vec<Tool> {
    Tool::ALL.into_iter().filter(|t| find(*t).is_none()).collect()
}

/// `PATH` with the folder of every resolved tool in front, so tools that look for
/// each other by name (yt-dlp -> deno/ffmpeg, soundcloud -> ffmpeg) find the right copy.
fn search_path() -> Option<OsString> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for tool in Tool::ALL {
        if let Some(dir) = find(tool).and_then(|p| p.parent().map(Path::to_path_buf)) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    if dirs.is_empty() {
        return None;
    }
    dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    std::env::join_paths(dirs).ok()
}

/// A hidden-window `Command` for the tool. If the tool cannot be found the bare name is
/// used, so the caller gets the usual "not found" error from `spawn`.
pub fn command(tool: Tool) -> Command {
    let program = find(tool).unwrap_or_else(|| PathBuf::from(tool.file_name()));
    let mut cmd = hidden_command(program);
    if let Some(path) = search_path() {
        cmd.env("PATH", path);
    }
    cmd
}

/// Points yt-dlp at the folder holding ffmpeg/ffprobe, if they were found.
pub fn add_ffmpeg_location(cmd: &mut Command) {
    if let Some(dir) = find(Tool::Ffmpeg).and_then(|p| p.parent().map(Path::to_path_buf)) {
        cmd.arg("--ffmpeg-location").arg(dir);
    }
}

/// Text for the "tools missing" warning, or `None` when nothing required is missing.
pub fn missing_report() -> Option<String> {
    let missing = missing();
    if !missing.iter().any(|t| t.is_required()) {
        return None;
    }
    let mut text = String::from("These tools could not be found:\n\n");
    for tool in &missing {
        text.push_str(&format!("  \u{2022} {}", tool.file_name()));
        if !tool.is_required() {
            text.push_str("  (optional - only for the SoundCloud DRM fallback)");
        }
        text.push('\n');
    }
    text.push_str(
        "\nPlace them in a \"tools\" folder next to Musicerer, or install them so they are on your PATH.\n\n\
         Musicerer will open, but downloads will not work until the required tools are available.",
    );
    Some(text)
}
