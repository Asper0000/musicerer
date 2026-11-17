//! Update checking and installation for Musicerer itself and the helper tools
//! (yt-dlp, deno, ffmpeg + ffprobe).
//!
//! Everything runs on plain threads and reports through a shared `Handle`
//! that the UI polls, the same way the download workers do.
//!
//! Tools are always installed into the *managed* tools folder (see
//! `tools::managed_dir`), which `tools::find` prefers over every other copy.

use super::process::{hidden_command, output_with_timeout};
use super::tools::{self, Origin, Tool};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

mod installation;

pub use installation::{
    installation_info, spawn_uninstall_app, take_uninstall_error, InstallationInfo,
    UninstallOptions,
};
use installation::{detect_install_kind, InstallKind};

const APP_API: &str = "https://api.github.com/repos/Asper0000/musicerer/releases/latest";
const YTDLP_API: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
const DENO_API: &str = "https://api.github.com/repos/denoland/deno/releases/latest";
const FFMPEG_RELEASES_API: &str = "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases?per_page=20";

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const USER_AGENT: &str = "Musicerer-Updater";
const TOOL_PLATFORM_MSG: &str = "Automatic tool downloads are supported on Windows x64, Linux x64 and Linux ARM64.";

// ---------------------------------------------------------------------------
// Public state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Component {
    App,
    YtDlp,
    Deno,
    Ffmpeg,
}

impl Component {
    pub const ALL: [Component; 4] = [
        Component::App,
        Component::YtDlp,
        Component::Deno,
        Component::Ffmpeg,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Component::App => "Musicerer",
            Component::YtDlp => "yt-dlp",
            Component::Deno => "deno",
            Component::Ffmpeg => "ffmpeg + ffprobe",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Component::App => "musicerer",
            Component::YtDlp => "yt-dlp",
            Component::Deno => "deno",
            Component::Ffmpeg => "ffmpeg",
        }
    }

    fn index(self) -> usize {
        match self {
            Component::App => 0,
            Component::YtDlp => 1,
            Component::Deno => 2,
            Component::Ffmpeg => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Unchecked,
    Checking,
    UpToDate,
    Available,
    Downloading(f32),
    Installing,
    Removing,
    Failed(String),
    Unsupported(String),
}

/// Everything needed to download and verify one update.
#[derive(Debug, Clone)]
struct Plan {
    url: String,
    asset_name: String,
    sha256: String,
    /// Identifies the build; stored after install so rolling releases can be compared.
    id: String,
}

#[derive(Debug, Clone)]
pub struct Row {
    /// Display text, e.g. "v0.3.0" or "2025.10.22".
    pub installed: Option<String>,
    pub latest: Option<String>,
    pub status: Status,
    /// Extra line describing installation origin or an incomplete tool pair.
    pub note: String,
    /// At least one executable in this row was resolved from the system PATH.
    pub external: bool,
    /// At least one required executable is in Musicerer's managed directory.
    pub managed: bool,
    /// Human-readable source of the resolved executable(s).
    pub origin: String,
    /// A required executable is absent (including either member of the FFmpeg pair).
    pub missing: bool,
    plan: Option<Plan>,
}

impl Row {
    fn new() -> Self {
        Self {
            installed: None,
            latest: None,
            status: Status::Unchecked,
            note: String::new(),
            external: false,
            managed: false,
            origin: "Not installed".to_string(),
            missing: false,
            plan: None,
        }
    }

    pub fn empty() -> Self {
        Self::new()
    }

    pub fn can_update(&self) -> bool {
        (self.missing || self.status == Status::Available) && self.plan.is_some()
    }
}

pub struct Shared {
    rows: [Row; 4],
    generation: u64,
    exit_requested: bool,
    app_prompt: bool,
    bulk_installing: bool,
    uninstalling: bool,
    uninstall_error: Option<String>,
}

pub type Handle = Arc<Mutex<Shared>>;

pub fn new_handle() -> Handle {
    Arc::new(Mutex::new(Shared {
        rows: [Row::new(), Row::new(), Row::new(), Row::new()],
        generation: 0,
        exit_requested: false,
        app_prompt: false,
        bulk_installing: false,
        uninstalling: false,
        uninstall_error: None,
    }))
}

fn lock(h: &Handle) -> MutexGuard<'_, Shared> {
    h.lock().unwrap_or_else(|e| e.into_inner())
}

fn update_row(h: &Handle, comp: Component, f: impl FnOnce(&mut Row)) {
    let mut g = lock(h);
    f(&mut g.rows[comp.index()]);
    g.generation = g.generation.wrapping_add(1);
}

pub fn snapshot(h: &Handle) -> Vec<(Component, Row)> {
    let g = lock(h);
    Component::ALL
        .into_iter()
        .map(|c| (c, g.rows[c.index()].clone()))
        .collect()
}

/// Bumps on every change, so the UI knows when to redraw.
pub fn generation(h: &Handle) -> u64 {
    lock(h).generation
}

pub fn is_busy(h: &Handle) -> bool {
    let g = lock(h);
    g.bulk_installing || g.uninstalling || g.rows.iter().any(|r| {
        matches!(
            r.status,
            Status::Checking | Status::Downloading(_) | Status::Installing | Status::Removing
        )
    })
}

pub fn any_available(h: &Handle) -> bool {
    lock(h).rows.iter().any(|r| r.can_update())
}

pub fn any_external(h: &Handle) -> bool {
    lock(h).rows.iter().any(|r| r.external)
}

/// The external-tools action is ready only when every external row has a
/// verified release plan, including tools that are already up to date.
pub fn can_install_external(h: &Handle) -> bool {
    let g = lock(h);
    let external: Vec<&Row> = g.rows.iter().skip(1).filter(|r| r.external).collect();
    !g.bulk_installing
        && !external.is_empty()
        && external.iter().all(|r| {
            r.plan.is_some()
                && matches!(r.status, Status::UpToDate | Status::Available | Status::Failed(_))
        })
}

/// True once, when the app finished updating itself and should close.
pub fn take_exit_request(h: &Handle) -> bool {
    std::mem::take(&mut lock(h).exit_requested)
}

/// True once, when the startup check found a newer Musicerer release.
pub fn take_app_prompt(h: &Handle) -> bool {
    std::mem::take(&mut lock(h).app_prompt)
}

fn request_exit(h: &Handle) {
    let mut g = lock(h);
    g.exit_requested = true;
    g.generation = g.generation.wrapping_add(1);
}

// ---------------------------------------------------------------------------
// Checking
// ---------------------------------------------------------------------------

struct Checked {
    installed: Option<String>,
    latest: Option<String>,
    note: String,
    verdict: Verdict,
}

enum Verdict {
    UpToDate(Option<Plan>),
    Available(Plan),
    Unsupported(String),
}

fn unsupported(installed: Option<String>, msg: String) -> Checked {
    Checked {
        installed,
        latest: None,
        note: String::new(),
        verdict: Verdict::Unsupported(msg),
    }
}

/// Starts a check of every component. `startup` also raises the one-time
/// "update available" prompt for Musicerer itself.
pub fn spawn_check_all(h: &Handle, startup: bool) {
    for comp in Component::ALL {
        let skip = {
            let g = lock(h);
            g.bulk_installing
                || matches!(
                    g.rows[comp.index()].status,
                    Status::Checking | Status::Downloading(_) | Status::Installing | Status::Removing
                )
        };
        if skip {
            continue;
        }
        update_row(h, comp, |r| r.status = Status::Checking);
        let h2 = h.clone();
        thread::spawn(move || {
            if catch_unwind(AssertUnwindSafe(|| check_one(&h2, comp, startup))).is_err() {
                update_row(&h2, comp, |r| r.status = Status::Failed("Update check panicked unexpectedly".into()));
            }
        });
    }
}

fn check_one(h: &Handle, comp: Component, startup: bool) {
    let result = match comp {
        Component::App => check_app(),
        Component::YtDlp => check_ytdlp(),
        Component::Deno => check_deno(),
        Component::Ffmpeg => check_ffmpeg(),
    };
    let external = component_is_external(comp);
    let managed = component_is_managed(comp);
    let origin = component_origin(comp);
    let tool_note = component_note(comp, external);
    match result {
        Ok(c) => {
            let available = matches!(c.verdict, Verdict::Available(_));
            let missing = component_is_missing(comp);
            update_row(h, comp, move |r| {
                r.installed = c.installed;
                r.latest = c.latest;
                r.external = external;
                r.managed = managed;
                r.origin = origin;
                r.missing = missing;
                r.note = if tool_note.is_empty() { c.note } else if c.note.is_empty() { tool_note } else {
                    format!("{} {}", c.note, tool_note)
                };
                match c.verdict {
                    Verdict::UpToDate(plan) => {
                        r.status = Status::UpToDate;
                        r.plan = plan;
                    }
                    Verdict::Available(plan) => {
                        r.status = Status::Available;
                        r.plan = Some(plan);
                    }
                    Verdict::Unsupported(msg) => {
                        r.status = Status::Unsupported(msg);
                        r.plan = None;
                    }
                }
            });
            if comp == Component::App && available && startup {
                let mut g = lock(h);
                g.app_prompt = true;
                g.generation = g.generation.wrapping_add(1);
            }
        }
        Err(e) => {
            let installed = local_version(comp);
            let missing = component_is_missing(comp);
            update_row(h, comp, move |r| {
                r.installed = installed;
                r.external = external;
                r.managed = managed;
                r.origin = origin;
                r.missing = missing;
                r.note = tool_note;
                r.status = Status::Failed(format!("Couldn't check for updates: {e}"));
                r.plan = None;
            });
        }
    }
}


fn mark_installed(h: &Handle, comp: Component, plan: &Plan) {
    let installed = local_version(comp);
    let external = component_is_external(comp);
    let managed = component_is_managed(comp);
    let origin = component_origin(comp);
    let missing = component_is_missing(comp);
    let note = component_note(comp, external);
    update_row(h, comp, move |row| {
        row.installed = installed;
        row.latest = Some(plan.id.clone());
        row.external = external;
        row.managed = managed;
        row.origin = origin;
        row.missing = missing;
        row.note = note;
        row.status = Status::UpToDate;
        row.plan = Some(plan.clone());
    });
}

fn component_is_external(comp: Component) -> bool {
    match comp {
        Component::App => false,
        Component::YtDlp => tools::origin(Tool::YtDlp) == Origin::System,
        Component::Deno => tools::origin(Tool::Deno) == Origin::System,
        Component::Ffmpeg => {
            tools::origin(Tool::Ffmpeg) == Origin::System
                || tools::origin(Tool::Ffprobe) == Origin::System
        }
    }
}

fn component_is_managed(comp: Component) -> bool {
    match comp {
        Component::App => false,
        Component::YtDlp => tools::origin(Tool::YtDlp) == Origin::Managed,
        Component::Deno => tools::origin(Tool::Deno) == Origin::Managed,
        Component::Ffmpeg => {
            tools::origin(Tool::Ffmpeg) == Origin::Managed
                || tools::origin(Tool::Ffprobe) == Origin::Managed
        }
    }
}

fn component_origin(comp: Component) -> String {
    match comp {
        Component::App => "Application".to_string(),
        Component::YtDlp => origin_label(tools::origin(Tool::YtDlp)),
        Component::Deno => origin_label(tools::origin(Tool::Deno)),
        Component::Ffmpeg => {
            let ffmpeg = tools::origin(Tool::Ffmpeg);
            let ffprobe = tools::origin(Tool::Ffprobe);
            let has_managed = ffmpeg == Origin::Managed || ffprobe == Origin::Managed;
            let has_system = ffmpeg == Origin::System || ffprobe == Origin::System;
            if has_managed && has_system {
                "Mixed managed and external copies".to_string()
            } else if has_managed {
                "Managed by Musicerer".to_string()
            } else if has_system {
                "Installed externally (system PATH)".to_string()
            } else if ffmpeg == Origin::Local || ffprobe == Origin::Local {
                "Local copy".to_string()
            } else {
                "Not installed".to_string()
            }
        }
    }
}

fn origin_label(origin: Origin) -> String {
    match origin {
        Origin::Missing => "Not installed".to_string(),
        Origin::Managed => "Managed by Musicerer".to_string(),
        Origin::Local => "Local copy".to_string(),
        Origin::System => "Installed externally (system PATH)".to_string(),
    }
}

fn component_is_missing(comp: Component) -> bool {
    match comp {
        Component::App => false,
        Component::YtDlp => tools::find(Tool::YtDlp).is_none(),
        Component::Deno => tools::find(Tool::Deno).is_none(),
        Component::Ffmpeg => {
            tools::find(Tool::Ffmpeg).is_none() || tools::find(Tool::Ffprobe).is_none()
        }
    }
}

fn component_note(comp: Component, _external: bool) -> String {
    match comp {
        // The origin is displayed in its own row field. Keep this note reserved
        // for additional state so external installations are never described twice.
        Component::App | Component::YtDlp | Component::Deno => String::new(),
        Component::Ffmpeg => {
            let ffmpeg_missing = tools::find(Tool::Ffmpeg).is_none();
            let ffprobe_missing = tools::find(Tool::Ffprobe).is_none();
            let mut notes = Vec::new();
            if ffmpeg_missing {
                notes.push("FFmpeg is missing.");
            }
            if ffprobe_missing {
                notes.push("FFprobe is missing.");
            }
            notes.join(" ")
        }
    }
}

fn local_version(comp: Component) -> Option<String> {
    match comp {
        Component::App => Some(format!("v{APP_VERSION}")),
        Component::YtDlp => ytdlp_version(),
        Component::Deno => deno_version().map(|v| format!("v{v}")),
        Component::Ffmpeg => ffmpeg_version().as_deref().map(ffmpeg_label),
    }
}

// --- Musicerer ---------------------------------------------------------------

fn check_app() -> Result<Checked, String> {
    let installed = Some(format!("v{APP_VERSION}"));
    let kind = detect_install_kind();
    let note = kind.label().to_string();

    let rel = get_json(APP_API)?;
    let tag = tag_of(&rel)?;
    let latest_raw = tag.trim_start_matches(['v', 'V']).to_string();
    let latest = Some(format!("v{latest_raw}"));

    if let InstallKind::Unknown(msg) = &kind {
        return Ok(Checked {
            installed,
            latest,
            note,
            verdict: Verdict::Unsupported(msg.clone()),
        });
    }
    if kind == InstallKind::LinuxPortable {
        return Ok(Checked {
            installed,
            latest,
            note,
            verdict: Verdict::Unsupported(
                "Self-updates are unavailable for manually placed Linux executables.".to_string(),
            ),
        });
    }

    let suffix = kind.asset_suffix();
    let asset = pick(&rel, |n| n.ends_with(suffix))?;
    let plan = plan_for(&rel, &asset, tag)?;
    if !is_newer(&latest_raw, APP_VERSION) {
        return Ok(Checked {
            installed,
            latest,
            note,
            verdict: Verdict::UpToDate(Some(plan)),
        });
    }

    Ok(Checked {
        installed,
        latest,
        note,
        verdict: Verdict::Available(plan),
    })
}

// --- yt-dlp ------------------------------------------------------------------

fn check_ytdlp() -> Result<Checked, String> {
    let installed = ytdlp_version();
    let Some(plat) = platform() else {
        return Ok(unsupported(installed, TOOL_PLATFORM_MSG.into()));
    };

    let rel = get_json(YTDLP_API)?;
    let tag = tag_of(&rel)?;
    let want = match plat {
        Plat::WinX64 => "yt-dlp.exe",
        Plat::LinuxX64 => "yt-dlp_linux",
        Plat::LinuxArm64 => "yt-dlp_linux_aarch64",
    };
    let asset = pick(&rel, |n| n == want)?;
    let plan = plan_for(&rel, &asset, tag.clone())?;

    let available = installed.as_deref().map(|v| is_newer(&tag, v)).unwrap_or(true);
    Ok(Checked {
        installed,
        latest: Some(tag),
        note: String::new(),
        verdict: if available {
            Verdict::Available(plan)
        } else {
            Verdict::UpToDate(Some(plan))
        },
    })
}

// --- deno --------------------------------------------------------------------

fn check_deno() -> Result<Checked, String> {
    let raw = deno_version();
    let installed = raw.as_ref().map(|v| format!("v{v}"));
    let Some(plat) = platform() else {
        return Ok(unsupported(installed, TOOL_PLATFORM_MSG.into()));
    };

    let rel = get_json(DENO_API)?;
    let tag = tag_of(&rel)?;
    let latest_raw = tag.trim_start_matches(['v', 'V']).to_string();
    let want = match plat {
        Plat::WinX64 => "deno-x86_64-pc-windows-msvc.zip",
        Plat::LinuxX64 => "deno-x86_64-unknown-linux-gnu.zip",
        Plat::LinuxArm64 => "deno-aarch64-unknown-linux-gnu.zip",
    };
    let asset = pick(&rel, |n| n == want)?;
    let plan = plan_for(&rel, &asset, tag.clone())?;

    let available = raw.as_deref().map(|v| is_newer(&latest_raw, v)).unwrap_or(true);
    Ok(Checked {
        installed,
        latest: Some(format!("v{latest_raw}")),
        note: String::new(),
        verdict: if available {
            Verdict::Available(plan)
        } else {
            Verdict::UpToDate(Some(plan))
        },
    })
}

// --- ffmpeg + ffprobe --------------------------------------------------------

fn check_ffmpeg() -> Result<Checked, String> {
    let raw = ffmpeg_version();
    let installed = raw.as_deref().map(ffmpeg_label);
    let Some(plat) = platform() else {
        return Ok(unsupported(installed, TOOL_PLATFORM_MSG.into()));
    };

    // BtbN publishes a separate autobuild release for each build day. Each
    // release contains versioned assets for master and the stable branches.
    // Select the newest autobuild containing a stable-branch asset for this
    // platform, then choose the highest stable version available in it.
    let releases = get_json(FFMPEG_RELEASES_API)?;
    let release = releases
        .as_array()
        .and_then(|list| {
            list.iter()
                .filter(|candidate| {
                    candidate
                        .get("tag_name")
                        .and_then(Value::as_str)
                        .map(|tag| tag.starts_with("autobuild-"))
                        .unwrap_or(false)
                        && assets_of(candidate)
                            .iter()
                            .any(|asset| ffmpeg_asset_version(&asset.name, plat).is_some())
                })
                .max_by(|a, b| {
                    let a_date = a
                        .get("published_at")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let b_date = b
                        .get("published_at")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    a_date.cmp(b_date)
                })
        })
        .ok_or_else(|| "No stable FFmpeg build was found for this platform".to_string())?;

    let (asset, latest_label, latest_numbers, latest_build) = assets_of(release)
        .into_iter()
        .filter_map(|asset| {
            ffmpeg_asset_version(&asset.name, plat).map(|(label, numbers, build)| {
                (asset, label, numbers, build)
            })
        })
        .max_by(|a, b| {
            compare_version_parts(&a.2, &b.2).then_with(|| a.3.cmp(&b.3))
        })
        .ok_or_else(|| "No matching stable FFmpeg download was found".to_string())?;

    // Keep the complete build-specific asset name as the marker. The UI only
    // displays the semantic FFmpeg version (for example, 9.0.2).
    let plan = plan_for(release, &asset, asset.name.clone())?;
    let installed_stable = raw.as_deref().and_then(ffmpeg_stable_version);
    let (available, note) = match installed_stable {
        Some((installed_numbers, installed_build)) => {
            match compare_version_parts(&latest_numbers, &installed_numbers) {
                std::cmp::Ordering::Greater => (true, String::new()),
                std::cmp::Ordering::Less => (
                    false,
                    "Installed FFmpeg version is newer than Musicerer's stable download; no downgrade will be offered.".to_string(),
                ),
                std::cmp::Ordering::Equal => match installed_build {
                    Some(build) if latest_build > build => (true, String::new()),
                    Some(build) if build > latest_build => (
                        false,
                        "Installed FFmpeg build is newer than the latest managed stable build.".to_string(),
                    ),
                    _ => (false, String::new()),
                },
            }
        }
        // A master build (N-...) is a different release track. Offer the
        // stable build explicitly rather than comparing incompatible numbers.
        None if raw.is_none() => (true, String::new()),
        None if raw.as_deref().map(|v| v.starts_with("N-")).unwrap_or(false) => (
            true,
            "Stable release available; this will switch from the development build.".to_string(),
        ),
        None => (
            true,
            "Could not compare the installed build with the stable release; installing will replace it.".to_string(),
        ),
    };

    Ok(Checked {
        installed,
        latest: Some(latest_label),
        note,
        verdict: if available {
            Verdict::Available(plan)
        } else {
            Verdict::UpToDate(Some(plan))
        },
    })
}

/// Returns the displayed version, its numeric components and BtbN's build
/// counter from an asset such as ffmpeg-n9.0.2-57-g<hash>-win64-gpl-9.0.zip.
fn ffmpeg_asset_version(name: &str, platform: Plat) -> Option<(String, Vec<u64>, u64)> {
    let (marker, extension) = match platform {
        Plat::WinX64 => ("-win64-gpl-", ".zip"),
        Plat::LinuxX64 => ("-linux64-gpl-", ".tar.xz"),
        Plat::LinuxArm64 => ("-linuxarm64-gpl-", ".tar.xz"),
    };
    let without_extension = name.strip_prefix("ffmpeg-n")?.strip_suffix(extension)?;
    let marker_at = without_extension.rfind(marker)?;
    let versioned = &without_extension[..marker_at];
    let branch = &without_extension[marker_at + marker.len()..];

    let mut parts = versioned.split('-');
    let version = parts.next()?;
    let build = parts.next()?.parse::<u64>().ok()?;
    let numbers = version
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    if numbers.len() < 2 || branch != format!("{}.{}", numbers[0], numbers[1]) {
        return None;
    }
    Some((version.to_string(), numbers, build))
}

/// Parses stable versions like 9.0.2 and BtbN stable builds like n9.0.2-57-g<hash>.
/// Master builds (N-<build>) deliberately do not parse as stable versions.
fn ffmpeg_stable_version(raw: &str) -> Option<(Vec<u64>, Option<u64>)> {
    if raw.starts_with("N-") {
        return None;
    }
    let normalized = raw.strip_prefix('n').unwrap_or(raw);
    let mut parts = normalized.split('-');
    let version = parts.next()?;
    let numbers = version
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    if numbers.len() < 2 {
        return None;
    }
    let build = parts.next().and_then(|part| part.parse::<u64>().ok());
    Some((numbers, build))
}

fn compare_version_parts(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    for index in 0..a.len().max(b.len()) {
        match a.get(index).copied().unwrap_or(0).cmp(&b.get(index).copied().unwrap_or(0)) {
            std::cmp::Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    std::cmp::Ordering::Equal
}

fn ffmpeg_label(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("N-") {
        format!("N-{}", rest.split('-').next().unwrap_or(rest))
    } else {
        raw.strip_prefix('n')
            .unwrap_or(raw)
            .split('-')
            .next()
            .unwrap_or(raw)
            .to_string()
    }
}

// --- installed versions ------------------------------------------------------

fn tool_stdout(tool: Tool, arg: &str) -> Option<String> {
    let mut cmd = tools::command(tool);
    cmd.arg(arg)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let out = output_with_timeout(&mut cmd, Duration::from_secs(15)).ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

fn ytdlp_version() -> Option<String> {
    tool_stdout(Tool::YtDlp, "--version")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn deno_version() -> Option<String> {
    let out = tool_stdout(Tool::Deno, "--version")?;
    out.lines()
        .next()?
        .split_whitespace()
        .nth(1)
        .map(str::to_string)
}

fn ffmpeg_version() -> Option<String> {
    let out = tool_stdout(Tool::Ffmpeg, "-version")?;
    out.lines()
        .next()?
        .split_whitespace()
        .nth(2)
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Platform + install kind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Plat {
    WinX64,
    LinuxX64,
    LinuxArm64,
}

fn platform() -> Option<Plat> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some(Plat::WinX64)
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some(Plat::LinuxX64)
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some(Plat::LinuxArm64)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// GitHub helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Asset {
    name: String,
    url: String,
    digest: Option<String>,
}

fn agent() -> ureq::Agent {
    super::http::agent_with_socket_timeouts(
        Some(USER_AGENT),
        Duration::from_secs(10),
        Duration::from_secs(30),
    )
}

fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(code) => format!("HTTP {code}"),
        other => other.to_string(),
    }
}

fn get_json(url: &str) -> Result<Value, String> {
    let mut response = agent()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(http_err)?;
    response
        .body_mut()
        .read_json::<Value>()
        .map_err(|e| e.to_string())
}

fn tag_of(rel: &Value) -> Result<String, String> {
    rel.get("tag_name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "The release has no version tag".to_string())
}

fn assets_of(rel: &Value) -> Vec<Asset> {
    rel.get("assets")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let name = a.get("name")?.as_str()?.to_string();
                    let url = a.get("browser_download_url")?.as_str()?.to_string();
                    let digest = a
                        .get("digest")
                        .and_then(Value::as_str)
                        .and_then(|d| d.strip_prefix("sha256:"))
                        .map(|d| d.to_ascii_lowercase());
                    Some(Asset { name, url, digest })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn pick(rel: &Value, pred: impl Fn(&str) -> bool) -> Result<Asset, String> {
    assets_of(rel)
        .into_iter()
        .find(|a| pred(&a.name))
        .ok_or_else(|| "No matching download in the latest release".to_string())
}

/// Looks the checksum up in a SHA2-256SUMS style file attached to the release.
fn sums_lookup(rel: &Value, target: &str) -> Option<String> {
    let sums = assets_of(rel)
        .into_iter()
        .find(|a| a.name == "SHA2-256SUMS" || a.name == "checksums.sha256")?;
    let mut response = agent().get(&sums.url).call().ok()?;
    let text = response.body_mut().read_to_string().ok()?;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(hash), Some(name)) = (it.next(), it.next()) else {
            continue;
        };
        if name.trim_start_matches('*') == target {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

fn plan_for(rel: &Value, asset: &Asset, id: String) -> Result<Plan, String> {
    let sha256 = asset
        .digest
        .clone()
        .or_else(|| sums_lookup(rel, &asset.name))
        .ok_or_else(|| "The release publishes no SHA-256 checksum for this file".to_string())?;
    Ok(Plan {
        url: asset.url.clone(),
        asset_name: asset.name.clone(),
        sha256,
        id,
    })
}

fn version_numbers(s: &str) -> Vec<u64> {
    s.trim_start_matches(['v', 'V'])
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// True when `latest` is a strictly higher version than `installed`.
fn is_newer(latest: &str, installed: &str) -> bool {
    let a = version_numbers(latest);
    let b = version_numbers(installed);
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Installing
// ---------------------------------------------------------------------------

pub fn spawn_install(h: &Handle, comp: Component) {
    let plan = {
        let g = lock(h);
        if g.bulk_installing {
            return;
        }
        let row = &g.rows[comp.index()];
        if !row.can_update() || row.external {
            return;
        }
        row.plan.clone()
    };
    let Some(plan) = plan else {
        return;
    };
    update_row(h, comp, |r| r.status = Status::Downloading(0.0));

    let h2 = h.clone();
    thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| match comp {
            Component::App => install_app(&h2, &plan),
            _ => install_tool(&h2, comp, &plan),
        })).unwrap_or_else(|_| Err("Installer panicked unexpectedly".into()));
        match result {
            Ok(()) => {
                if comp != Component::App {
                    write_marker(comp.key(), &plan.id);
                    mark_installed(&h2, comp, &plan);
                }
            }
            Err(e) => update_row(&h2, comp, move |r| {
                r.status = Status::Failed(format!("Update failed: {e}"));
                r.note = if comp == Component::App {
                    detect_install_kind().label().to_string()
                } else {
                    String::new()
                };
            }),
        }
    });
}

/// Reinstalls the latest release asset for Musicerer itself, even when the
/// current version is already up to date.
pub fn spawn_reinstall_app(h: &Handle) {
    let plan = {
        let g = lock(h);
        if g.bulk_installing || g.uninstalling || g.rows.iter().any(|r| {
            matches!(r.status, Status::Checking | Status::Downloading(_) | Status::Installing | Status::Removing)
        }) {
            return;
        }
        g.rows[Component::App.index()].plan.clone()
    };
    let Some(plan) = plan else { return; };
    update_row(h, Component::App, |r| r.status = Status::Downloading(0.0));
    let h2 = h.clone();
    thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| install_app(&h2, &plan)))
            .unwrap_or_else(|_| Err("Reinstaller panicked unexpectedly".into()));
        match result {
            Ok(()) => {},
            Err(e) => update_row(&h2, Component::App, move |r| r.status = Status::Failed(format!("Reinstall failed: {e}"))),
        }
    });
}

pub fn can_reinstall_app(h: &Handle) -> bool {
    let g = lock(h);
    !g.uninstalling
        && !g.bulk_installing
        && !g.rows.iter().any(|r| matches!(r.status, Status::Checking | Status::Downloading(_) | Status::Installing | Status::Removing))
        && g.rows[Component::App.index()].plan.is_some()
}

pub fn can_install_managed(h: &Handle, comp: Component) -> bool {
    if comp == Component::App { return false; }
    let g = lock(h);
    !g.uninstalling
        && !g.bulk_installing
        && !g.rows.iter().any(|r| matches!(r.status, Status::Checking | Status::Downloading(_) | Status::Installing | Status::Removing))
        && g.rows[comp.index()].plan.is_some()
}

pub fn can_reinstall_tool(h: &Handle, comp: Component) -> bool {
    if comp == Component::App { return false; }
    let g = lock(h);
    !g.uninstalling
        && !g.bulk_installing
        && !g.rows.iter().any(|r| matches!(r.status, Status::Checking | Status::Downloading(_) | Status::Installing | Status::Removing))
        && g.rows[comp.index()].managed
        && !g.rows[comp.index()].external
        && !g.rows[comp.index()].missing
        && g.rows[comp.index()].plan.is_some()
}

/// Downloads a Musicerer-managed copy of one tool, even when another copy is
/// currently available from PATH or a local tools directory.
pub fn spawn_install_managed(h: &Handle, comp: Component) {
    if comp == Component::App {
        return;
    }
    let plan = {
        let g = lock(h);
        if g.bulk_installing || g.uninstalling {
            return;
        }
        g.rows[comp.index()].plan.clone()
    };
    let Some(plan) = plan else {
        return;
    };
    update_row(h, comp, |r| r.status = Status::Downloading(0.0));
    let h2 = h.clone();
    thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| install_tool(&h2, comp, &plan)))
            .unwrap_or_else(|_| Err("Installer panicked unexpectedly".into()));
        match result {
            Ok(()) => { write_marker(comp.key(), &plan.id); mark_installed(&h2, comp, &plan); }
            Err(e) => update_row(&h2, comp, move |r| r.status = Status::Failed(format!("Installation failed: {e}"))),
        }
    });
}

/// Reinstalls the currently selected latest release of a managed tool.
pub fn spawn_reinstall_tool(h: &Handle, comp: Component) {
    if comp == Component::App {
        return;
    }
    spawn_install_managed(h, comp);
}

/// Removes only files in Musicerer's managed tools directory. External, local,
/// system-installed and package-manager-owned copies are never touched.
pub fn spawn_uninstall_tool(h: &Handle, comp: Component) {
    if comp == Component::App {
        return;
    }
    {
        let g = lock(h);
        if g.bulk_installing || g.uninstalling
            || g.rows.iter().any(|r| matches!(r.status, Status::Checking | Status::Downloading(_) | Status::Installing | Status::Removing))
        {
            return;
        }
    }
    update_row(h, comp, |r| r.status = Status::Removing);
    let h2 = h.clone();
    thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| uninstall_managed_tool(comp)))
            .unwrap_or_else(|_| Err("Uninstaller panicked unexpectedly".into()));
        match result {
            Ok(()) => check_one(&h2, comp, false),
            Err(e) => update_row(&h2, comp, move |r| r.status = Status::Failed(format!("Uninstall failed: {e}"))),
        }
    });
}

fn uninstall_managed_tool(comp: Component) -> Result<(), String> {
    let dir = tools::managed_dir().ok_or_else(|| "No managed tools directory is available".to_string())?;
    if let Ok(metadata) = fs::symlink_metadata(&dir) {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "The managed tools directory is a symbolic link ({}). Musicerer will not remove files outside its own directory.",
                dir.display()
            ));
        }
    }
    let (names, asset_prefixes): (&[Tool], &[&str]) = match comp {
        Component::App => return Err("Musicerer is not a managed tool".to_string()),
        Component::YtDlp => (&[Tool::YtDlp], &["yt-dlp"]),
        Component::Deno => (&[Tool::Deno], &["deno"]),
        Component::Ffmpeg => (&[Tool::Ffmpeg, Tool::Ffprobe], &["ffmpeg-n", "ffmpeg", "ffprobe"]),
    };
    for tool in names {
        remove_managed_path(&dir.join(tool.file_name()))?;
    }
    if dir.is_dir() {
        let backup_prefixes: Vec<String> = names
            .iter()
            .map(|tool| format!(".{}.musicerer-backup-", tool.file_name()))
            .collect();
        let ffmpeg_temp_prefixes = [".ffmpeg-stage-", ".ffmpeg-backup-"];
        for entry in fs::read_dir(&dir).map_err(|e| format!("Could not inspect managed tools directory: {e}"))? {
            let entry = entry.map_err(|e| format!("Could not inspect a managed tool file: {e}"))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_partial = asset_prefixes.iter().any(|prefix| name.starts_with(prefix) && name.ends_with(".part"));
            let is_backup = backup_prefixes.iter().any(|prefix| name.starts_with(prefix));
            let is_ffmpeg_temp = comp == Component::Ffmpeg
                && ffmpeg_temp_prefixes.iter().any(|prefix| name.starts_with(prefix));
            if is_partial || is_backup || is_ffmpeg_temp {
                remove_managed_path(&entry.path())?;
            }
        }
    }
    if comp == Component::Ffmpeg {
        remove_managed_path(&dir.join("extract-tmp"))?;
    }
    remove_marker(comp.key())?;
    if dir.is_dir() {
        let mut entries = fs::read_dir(&dir).map_err(|e| format!("Could not inspect {}: {e}", dir.display()))?;
        if entries.next().is_none() {
            fs::remove_dir(&dir).map_err(|e| format!("Could not remove empty managed tools directory {}: {e}", dir.display()))?;
        }
    }
    Ok(())
}

fn remove_managed_path(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Could not inspect {}: {error}", path.display())),
    };
    // Treat symlinks as files so cleanup never follows a link outside the
    // managed directory and removes the linked target.
    let result = if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|e| format!("Could not remove {}: {e}", path.display()))
}

fn remove_marker(key: &str) -> Result<(), String> {
    let Some(path) = marker_path() else { return Ok(()); };
    let mut value: Value = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| Value::Object(Default::default()));
    if let Some(object) = value.as_object_mut() {
        object.remove(key);
        if object.is_empty() {
            return remove_managed_path(&path);
        }
    }
    fs::write(&path, value.to_string())
        .map_err(|e| format!("Could not update managed tool version metadata {}: {e}", path.display()))
}

/// Installs managed copies of every tool currently resolved from the system PATH.
/// This intentionally runs even when those external versions are already current.
pub fn spawn_install_external(h: &Handle) {
    let work = {
        let mut g = lock(h);
        if g.bulk_installing
            || g.rows.iter().any(|r| {
                matches!(r.status, Status::Checking | Status::Downloading(_) | Status::Installing | Status::Removing)
            })
        {
            return;
        }
        let external_count = g.rows.iter().skip(1).filter(|r| r.external).count();
        let work: Vec<(Component, Plan)> = Component::ALL
            .into_iter()
            .filter(|comp| *comp != Component::App)
            .filter_map(|comp| {
                let row = &g.rows[comp.index()];
                if row.external
                    && matches!(row.status, Status::UpToDate | Status::Available | Status::Failed(_))
                {
                    row.plan.clone().map(|plan| (comp, plan))
                } else {
                    None
                }
            })
            .collect();
        if external_count == 0 || work.len() != external_count {
            return;
        }
        g.bulk_installing = true;
        g.generation = g.generation.wrapping_add(1);
        work
    };

    let h2 = h.clone();
    thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| {
            for (comp, plan) in work {
                update_row(&h2, comp, |r| r.status = Status::Downloading(0.0));
                match install_tool(&h2, comp, &plan) {
                    Ok(()) => { write_marker(comp.key(), &plan.id); mark_installed(&h2, comp, &plan); }
                    Err(e) => update_row(&h2, comp, move |r| r.status = Status::Failed(format!("Managed installation failed: {e}"))),
                }
            }
        }));
        if result.is_err() {
            for comp in [Component::YtDlp, Component::Deno, Component::Ffmpeg] {
                update_row(&h2, comp, |r| if matches!(r.status, Status::Downloading(_) | Status::Installing) { r.status = Status::Failed("Bulk installation panicked unexpectedly".into()); });
            }
        }
        let mut g = lock(&h2);
        g.bulk_installing = false;
        g.generation = g.generation.wrapping_add(1);
    });
}

/// Downloads `url` to `dest`, reporting progress and verifying the SHA-256.
fn download(h: &Handle, comp: Component, plan: &Plan, dest: &Path) -> Result<(), String> {
    let mut resp = super::http::agent_with_socket_timeouts(
        Some(USER_AGENT),
        Duration::from_secs(15),
        Duration::from_secs(60),
    )
    .get(&plan.url)
    .call()
    .map_err(http_err)?;
    let total = resp
        .headers()
        .get("Content-Length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let mut reader = resp.body_mut().as_reader();
    let mut file = fs::File::create(dest).map_err(|e| format!("Cannot write {}: {e}", dest.display()))?;

    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done: u64 = 0;
    let mut last = Instant::now();
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if let Some(total) = total {
            if total > 0 && last.elapsed() >= Duration::from_millis(120) {
                last = Instant::now();
                let p = (done as f32 / total as f32).clamp(0.0, 1.0);
                update_row(h, comp, |r| r.status = Status::Downloading(p));
            }
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    let digest = hasher.finalize();
    let mut got = String::with_capacity(64);
    {
        use std::fmt::Write as _;
        for byte in digest.iter() {
            write!(&mut got, "{byte:02x}").expect("writing to a String cannot fail");
        }
    }
    if got != plan.sha256 {
        let _ = fs::remove_file(dest);
        return Err("Checksum mismatch: the download was corrupted or tampered with.".to_string());
    }
    update_row(h, comp, |r| r.status = Status::Downloading(1.0));
    Ok(())
}

// --- tools -------------------------------------------------------------------

fn install_tool(h: &Handle, comp: Component, plan: &Plan) -> Result<(), String> {
    let dir = tools::managed_dir().ok_or_else(|| "No config folder available".to_string())?;
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!("{}.part", plan.asset_name));

    if let Err(error) = download(h, comp, plan, &tmp) {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
    update_row(h, comp, |r| r.status = Status::Installing);

    let result = match comp {
        Component::YtDlp => {
            let target = dir.join(Tool::YtDlp.file_name());
            place(&tmp, &target).map(|_| make_executable(&target))
        }
        Component::Deno => extract_zip_members(&tmp, &[Tool::Deno.file_name()], &dir),
        Component::Ffmpeg => install_ffmpeg_pair(&tmp, &dir),
        Component::App => Err("Not a tool".to_string()),
    };
    let _ = fs::remove_file(&tmp);
    result
}

/// Replaces one file without deleting the working copy until the new file is
/// ready to take its place. The temporary backup is on the same filesystem.
fn place(from: &Path, to: &Path) -> Result<(), String> {
    let backup = to.with_file_name(format!(
        ".{}.musicerer-backup-{}",
        to.file_name().and_then(|n| n.to_str()).unwrap_or("tool"),
        std::process::id()
    ));
    if backup.exists() {
        if to.exists() {
            fs::remove_file(&backup).map_err(|e| format!("Could not clear old backup {}: {e}", backup.display()))?;
        } else {
            // Recover the last known-good copy left by an interrupted replacement.
            fs::rename(&backup, to).map_err(|e| format!("Could not restore the previous tool from {}: {e}", backup.display()))?;
        }
    }
    let had_target = to.exists();
    if had_target {
        fs::rename(to, &backup).map_err(|e| format!("Could not preserve the existing {}: {e}", to.display()))?;
    }
    let installed = fs::rename(from, to).or_else(|_| {
        fs::copy(from, to).map(|_| ()).and_then(|_| fs::remove_file(from))
    });
    if let Err(error) = installed {
        let _ = fs::remove_file(to);
        if had_target {
            let _ = fs::rename(&backup, to);
        }
        return Err(format!("Could not install {}: {error}", to.display()));
    }
    if had_target {
        fs::remove_file(&backup).map_err(|e| format!("Installed {}, but could not remove backup {}: {e}", to.display(), backup.display()))?;
    }
    Ok(())
}

/// Downloads and extracts both FFmpeg executables into a staging directory,
/// then swaps the pair together with rollback if either replacement fails.
fn install_ffmpeg_pair(archive: &Path, dir: &Path) -> Result<(), String> {
    let stage = dir.join(format!(".ffmpeg-stage-{}", std::process::id()));
    let backup = dir.join(format!(".ffmpeg-backup-{}", std::process::id()));
    let _ = fs::remove_dir_all(&stage);
    if backup.exists() {
        let pair_is_complete = [Tool::Ffmpeg, Tool::Ffprobe]
            .iter()
            .all(|tool| dir.join(tool.file_name()).is_file());
        if pair_is_complete {
            fs::remove_dir_all(&backup).map_err(|e| format!("Could not remove a previous FFmpeg backup: {e}"))?;
        } else {
            return Err(format!(
                "A previous FFmpeg installation was interrupted and its recovery files remain in {}. The existing pair was not overwritten.",
                backup.display()
            ));
        }
    }
    fs::create_dir_all(&stage).map_err(|e| format!("Could not create FFmpeg staging directory: {e}"))?;

    let names = [Tool::Ffmpeg.file_name(), Tool::Ffprobe.file_name()];
    let extracted = if cfg!(windows) {
        extract_zip_members(archive, &names, &stage)
    } else {
        extract_ffmpeg_tar(archive, &stage)
    };
    if let Err(error) = extracted {
        let _ = fs::remove_dir_all(&stage);
        return Err(error);
    }
    for name in &names {
        if !stage.join(name).is_file() {
            let _ = fs::remove_dir_all(&stage);
            return Err(format!("The FFmpeg archive did not contain {name}"));
        }
    }

    if let Err(error) = fs::create_dir_all(&backup) {
        let _ = fs::remove_dir_all(&stage);
        return Err(format!("Could not create FFmpeg backup directory: {error}"));
    }
    let mut backed_up: Vec<String> = Vec::new();
    for name in &names {
        let target = dir.join(name);
        if target.exists() {
            if let Err(error) = fs::rename(&target, backup.join(name)) {
                let restore_errors = restore_ffmpeg_backups(&backup, dir, &backed_up, &[]);
                let _ = fs::remove_dir_all(&stage);
                let detail = if restore_errors.is_empty() {
                    String::new()
                } else {
                    format!(" Recovery files were retained in {} ({})", backup.display(), restore_errors.join("; "))
                };
                return Err(format!("Could not preserve existing {name}: {error}.{detail}"));
            }
            backed_up.push(name.clone());
        }
    }

    let mut installed: Vec<String> = Vec::new();
    for name in &names {
        let target = dir.join(name);
        if let Err(error) = fs::rename(stage.join(name), &target) {
            let restore_errors = restore_ffmpeg_backups(&backup, dir, &backed_up, &installed);
            let _ = fs::remove_dir_all(&stage);
            if restore_errors.is_empty() {
                return Err(format!("Could not install {name}; the previous FFmpeg pair was restored: {error}"));
            }
            return Err(format!(
                "Could not install {name}: {error}. Recovery files were retained in {} because restoration was incomplete: {}",
                backup.display(),
                restore_errors.join("; ")
            ));
        }
        installed.push(name.clone());
        make_executable(&target);
    }

    // Cleanup is best-effort after both executables have been installed. If a
    // backup remains, the next installation can safely recognize the complete
    // pair and remove the stale backup before proceeding.
    let _ = fs::remove_dir_all(&backup);
    let _ = fs::remove_dir_all(&stage);
    Ok(())
}

fn restore_ffmpeg_backups(
    backup: &Path,
    dir: &Path,
    backed_up: &[String],
    installed: &[String],
) -> Vec<String> {
    let mut errors = Vec::new();
    for name in installed.iter().rev() {
        if let Err(error) = fs::remove_file(dir.join(name)) {
            if error.kind() != std::io::ErrorKind::NotFound {
                errors.push(format!("could not remove new {name}: {error}"));
            }
        }
    }
    for name in backed_up.iter().rev() {
        let source = backup.join(name);
        if source.exists() {
            if let Err(error) = fs::rename(&source, dir.join(name)) {
                errors.push(format!("could not restore {name}: {error}"));
            }
        }
    }
    if errors.is_empty() {
        let _ = fs::remove_dir_all(backup);
    }
    errors
}

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
fn make_executable(_: &Path) {}

/// Pulls the named files (by file name, wherever they sit in the archive) into `out_dir`.
fn extract_zip_members(zip_path: &Path, names: &[String], out_dir: &Path) -> Result<(), String> {
    let file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Bad archive: {e}"))?;
    let mut found = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() || entry.is_symlink() {
            continue;
        }
        let name = {
            let entry_name = entry
                .name()
                .map_err(|e| format!("Invalid ZIP entry name: {e}"))?;
            Path::new(entry_name.as_ref())
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        };
        let Some(name) = name else {
            continue;
        };
        if !names.contains(&name) {
            continue;
        }
        let part = out_dir.join(format!("{name}.part"));
        let mut out = fs::File::create(&part).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("Extraction failed: {e}"))?;
        drop(out);
        let target = out_dir.join(&name);
        place(&part, &target)?;
        make_executable(&target);
        found += 1;
    }
    if found < names.len() {
        return Err("The archive did not contain every expected file".to_string());
    }
    Ok(())
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(found) = find_file(&p, name) {
                return Some(found);
            }
        } else if p.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(p);
        }
    }
    None
}

fn extract_ffmpeg_tar(tar_path: &Path, out_dir: &Path) -> Result<(), String> {
    let work = out_dir.join("extract-tmp");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;

    let status = hidden_command("tar")
        .arg("-xJf")
        .arg(tar_path)
        .arg("-C")
        .arg(&work)
        .arg("--wildcards")
        .arg("*/bin/ffmpeg")
        .arg("*/bin/ffprobe")
        .status()
        .map_err(|e| format!("Could not run tar (is tar/xz installed?): {e}"))?;
    let result = if status.success() {
        let mut outcome = Ok(());
        for tool in [Tool::Ffmpeg, Tool::Ffprobe] {
            let name = tool.file_name();
            match find_file(&work, &name) {
                Some(src) => {
                    let target = out_dir.join(&name);
                    if let Err(e) = place(&src, &target) {
                        outcome = Err(e);
                        break;
                    }
                    make_executable(&target);
                }
                None => {
                    outcome = Err(format!("{name} was missing from the archive"));
                    break;
                }
            }
        }
        outcome
    } else {
        Err("Extracting the ffmpeg archive failed (is xz installed?)".to_string())
    };
    let _ = fs::remove_dir_all(&work);
    result
}

// --- version markers ---------------------------------------------------------

fn marker_path() -> Option<PathBuf> {
    tools::managed_dir().map(|d| d.join("versions.json"))
}

fn write_marker(key: &str, value: &str) {
    let Some(path) = marker_path() else {
        return;
    };
    let mut v: Value = fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| Value::Object(Default::default()));
    if let Some(obj) = v.as_object_mut() {
        obj.insert(key.to_string(), Value::String(value.to_string()));
    }
    let _ = fs::write(path, v.to_string());
}

// --- Musicerer itself --------------------------------------------------------

fn relaunch(exe: &Path) -> Result<(), String> {
    hidden_command(exe)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Updated, but could not restart: {e}"))
}

fn app_update_cache_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("Musicerer").join("updates");
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create the update staging directory: {e}"))?;
    Ok(dir)
}

fn install_app(h: &Handle, plan: &Plan) -> Result<(), String> {
    let comp = Component::App;
    let kind = detect_install_kind();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;

    match kind {
        InstallKind::WinSetup => {
            let setup = app_update_cache_dir()?.join(&plan.asset_name);
            download(h, comp, plan, &setup)?;
            update_row(h, comp, |r| {
                r.status = Status::Installing;
                r.note = "Restarting to finish the update\u{2026}".to_string();
            });
            launch_setup(&setup, &exe)?;
            thread::sleep(Duration::from_millis(400));
            request_exit(h);
            Ok(())
        }
        InstallKind::WinPortable => {
            let file_name = exe
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| "Bad executable name".to_string())?
                .to_string();
            let fresh = exe.with_file_name(format!("{file_name}.new"));
            download(h, comp, plan, &fresh)?;
            update_row(h, comp, |r| {
                r.status = Status::Installing;
                r.note = "Restarting to finish the update\u{2026}".to_string();
            });
            // A running exe can be renamed on Windows, just not overwritten.
            let old = exe.with_file_name(format!("{file_name}.old"));
            let _ = fs::remove_file(&old);
            fs::rename(&exe, &old).map_err(|e| format!("Cannot replace the running app: {e}"))?;
            if let Err(e) = fs::rename(&fresh, &exe) {
                let _ = fs::rename(&old, &exe);
                return Err(format!("Cannot place the new version: {e}"));
            }
            relaunch(&exe)?;
            request_exit(h);
            Ok(())
        }
        InstallKind::LinuxPortable => Err(
            "Self-updates are unavailable for manually placed Linux executables.".to_string(),
        ),
        InstallKind::AppImage => {
            let current = std::env::var_os("APPIMAGE")
                .map(PathBuf::from)
                .ok_or_else(|| "APPIMAGE is not set".to_string())?;
            let file_name = current
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| "Bad AppImage name".to_string())?
                .to_string();
            let fresh = current.with_file_name(format!("{file_name}.new"));
            download(h, comp, plan, &fresh)?;
            update_row(h, comp, |r| {
                r.status = Status::Installing;
                r.note = "Restarting to finish the update\u{2026}".to_string();
            });
            make_executable(&fresh);
            fs::rename(&fresh, &current)
                .map_err(|e| format!("Cannot replace the AppImage: {e}"))?;
            relaunch(&current)?;
            request_exit(h);
            Ok(())
        }
        InstallKind::Deb | InstallKind::Rpm => {
            let pkg = app_update_cache_dir()?.join(&plan.asset_name);
            download(h, comp, plan, &pkg)?;
            update_row(h, comp, |r| {
                r.status = Status::Installing;
                r.note = "Waiting for administrator approval\u{2026}".to_string();
            });
            let mut cmd = hidden_command("pkexec");
            if kind == InstallKind::Deb {
                cmd.arg("dpkg").arg("-i");
            } else {
                cmd.arg("rpm").arg("-U");
            }
            cmd.arg(&pkg);
            match cmd.status() {
                Ok(s) if s.success() => {
                    let _ = fs::remove_file(&pkg);
                    relaunch(&exe)?;
                    request_exit(h);
                    Ok(())
                }
                Ok(_) => Err("The package install failed or was cancelled".to_string()),
                Err(_) => {
                    // No pkexec: hand the package to the desktop's installer instead.
                    let _ = hidden_command("xdg-open").arg(&pkg).spawn();
                    Err(format!(
                        "Downloaded to {}. Install it with your package manager, then restart.",
                        pkg.display()
                    ))
                }
            }
        }
        InstallKind::Unknown(msg) => Err(msg),
    }
}

#[cfg(windows)]
fn launch_setup(setup: &Path, exe: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // Run the installer silently, then start the app again once it finishes.
    let mut cmd = std::process::Command::new("cmd");
    cmd.creation_flags(0x0800_0000);
    cmd.raw_arg(format!(
        "/C \"\"{}\" /SILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS & start \"\" \"{}\"\"",
        setup.display(),
        exe.display()
    ));
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not start the installer: {e}"))
}

#[cfg(not(windows))]
fn launch_setup(_setup: &Path, _exe: &Path) -> Result<(), String> {
    Err("The Windows installer cannot run on this system".to_string())
}

/// Removes leftovers of a previous self-update. Call once at startup.
pub fn cleanup_after_update() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(name) = exe.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let _ = fs::remove_file(exe.with_file_name(format!("{name}.old")));
    let _ = fs::remove_file(exe.with_file_name(format!("{name}.new")));
}
