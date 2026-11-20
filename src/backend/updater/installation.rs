//! Installation discovery and application uninstall lifecycle.
//!
//! Kept separate from release checking and tool downloading so the updater
//! remains focused on version checks, verified downloads, and installation.

use super::{lock, request_exit, Handle, Status};
use super::super::process::hidden_command;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

const PLATFORM_MSG: &str = "Updates are only available for x64 Windows and Linux builds.";

#[derive(Debug, Clone)]
pub struct InstallationInfo {
    pub kind: String,
    pub executable: Option<PathBuf>,
    pub package: Option<String>,
    pub can_uninstall: bool,
    pub uninstall_note: String,
}

#[derive(Debug, Clone, Default)]
pub struct UninstallOptions {
    pub keep_settings: bool,
    pub keep_tools: bool,
    pub keep_cache: bool,
    pub keep_logs: bool,
    pub keep_other_data: bool,
    /// User-selected download directories may contain unrelated files. This is
    /// unchecked by default like every other keep choice, so full removal also
    /// removes the configured output directory after an explicit path warning.
    pub keep_output_directory: bool,
}

struct UninstallCleanup {
    remove_paths: Vec<PathBuf>,
    prune_dirs: Vec<PathBuf>,
}


// ---------------------------------------------------------------------------
// Installation type detection
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(super) enum InstallKind {
    WinSetup,
    WinPortable,
    AppImage,
    LinuxPortable,
    Deb,
    Rpm,
    Unknown(String),
}

impl InstallKind {
    pub(super) fn label(&self) -> &'static str {
        match self {
            InstallKind::WinSetup => "Windows installer",
            InstallKind::WinPortable => "Windows portable",
            InstallKind::AppImage => "Linux AppImage",
            InstallKind::LinuxPortable => "Linux portable executable",
            InstallKind::Deb => "Linux .deb package",
            InstallKind::Rpm => "Linux .rpm package",
            InstallKind::Unknown(_) => "Unknown install",
        }
    }

    /// Ending of the release asset that matches this kind of install.
    pub(super) fn asset_suffix(&self) -> &'static str {
        match self {
            InstallKind::WinSetup => "-windows-x64-setup.exe",
            InstallKind::WinPortable => "-windows-x64-portable.exe",
            InstallKind::AppImage => ".AppImage",
            InstallKind::LinuxPortable => "",
            InstallKind::Deb => ".deb",
            InstallKind::Rpm => ".rpm",
            InstallKind::Unknown(_) => "",
        }
    }
}

pub(super) fn detect_install_kind() -> InstallKind {
    if !cfg!(target_arch = "x86_64") {
        return InstallKind::Unknown(PLATFORM_MSG.to_string());
    }
    detect_install_kind_os()
}

#[cfg(windows)]
fn detect_install_kind_os() -> InstallKind {
    let Ok(exe) = std::env::current_exe() else {
        return InstallKind::Unknown("Could not locate the running executable.".to_string());
    };
    // The Inno Setup installer drops its uninstaller next to the exe.
    let has_uninstaller = exe
        .parent()
        .map(|d| d.join("unins000.exe").is_file())
        .unwrap_or(false);
    if has_uninstaller {
        InstallKind::WinSetup
    } else {
        InstallKind::WinPortable
    }
}

#[cfg(target_os = "linux")]
fn detect_install_kind_os() -> InstallKind {
    if std::env::var_os("APPIMAGE").is_some() {
        return InstallKind::AppImage;
    }
    if let Ok(exe) = std::env::current_exe() {
        let ok = |program: &str, args: &[&str]| {
            hidden_command(program)
                .args(args)
                .arg(&exe)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        if ok("dpkg", &["-S"]) {
            return InstallKind::Deb;
        }
        if ok("rpm", &["-qf"]) {
            return InstallKind::Rpm;
        }
        if exe.parent().is_some_and(directory_is_writable) || command_available("pkexec") {
            return InstallKind::LinuxPortable;
        }
    }
    InstallKind::Unknown(
        "The installation location is not writable and no supported package manager owns this executable. Remove it manually."
            .to_string(),
    )
}

#[cfg(not(any(windows, target_os = "linux")))]
fn detect_install_kind_os() -> InstallKind {
    InstallKind::Unknown(PLATFORM_MSG.to_string())
}

fn detect_install_kind_for_management() -> InstallKind {
    #[cfg(target_os = "linux")]
    {
        return detect_install_kind_os();
    }
    #[cfg(any(windows, not(any(windows, target_os = "linux"))))]
    {
        detect_install_kind()
    }
}

fn directory_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".musicerer-write-test-{}", std::process::id()));
    match fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => { drop(file); let _ = fs::remove_file(probe); true }
        Err(_) => false,
    }
}

fn command_available(program: &str) -> bool {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .any(|dir| {
            let candidate = dir.join(program);
            candidate.is_file()
                || (cfg!(windows) && dir.join(format!("{program}.exe")).is_file())
        })
}

#[cfg(target_os = "linux")]
fn deb_package_name(exe: &Path) -> Option<String> {
    let output = hidden_command("dpkg-query")
        .args(["-S", "--"])
        .arg(exe)
        .output()
        .ok()?;
    if !output.status.success() { return None; }
    let line = String::from_utf8_lossy(&output.stdout).lines().next()?.to_string();
    let (package, _) = line.split_once(": ")?;
    Some(package.split(':').next()?.to_string())
}

#[cfg(not(target_os = "linux"))]
fn deb_package_name(_: &Path) -> Option<String> { None }

#[cfg(target_os = "linux")]
fn rpm_package_name(exe: &Path) -> Option<String> {
    let output = hidden_command("rpm")
        .args(["-qf", "--qf", "%{NAME}"])
        .arg(exe)
        .output()
        .ok()?;
    if !output.status.success() { return None; }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

#[cfg(not(target_os = "linux"))]
fn rpm_package_name(_: &Path) -> Option<String> { None }


// ---------------------------------------------------------------------------
// Public installation state
// ---------------------------------------------------------------------------

/// Resolves the active application installation once for display and uninstall gating.
pub fn installation_info() -> InstallationInfo {
    let kind = detect_install_kind_for_management();
    let exe = if kind == InstallKind::AppImage {
        std::env::var_os("APPIMAGE").map(PathBuf::from)
    } else {
        std::env::current_exe().ok()
    };
    let mut package = None;
    let (can_uninstall, uninstall_note) = match &kind {
        InstallKind::WinSetup => {
            let has_uninstaller = exe.as_ref().and_then(|p| p.parent())
                .map(|d| d.join("unins000.exe").is_file()).unwrap_or(false);
            (has_uninstaller, if has_uninstaller { String::new() } else { "The native Windows uninstaller could not be found.".to_string() })
        }
        InstallKind::WinPortable => (exe.is_some(), String::new()),
        InstallKind::AppImage => {
            let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
            let exists = appimage.as_ref().is_some_and(|p| p.is_file());
            let removable = appimage.as_ref().is_some_and(|p| {
                p.parent().is_some_and(directory_is_writable) || command_available("pkexec")
            });
            (exists && removable, if !exists {
                "The active AppImage path could not be resolved.".to_string()
            } else if !removable {
                "The AppImage location is not writable and pkexec is unavailable.".to_string()
            } else { String::new() })
        }
        InstallKind::LinuxPortable => {
            let removable = exe.as_ref().is_some_and(|p| {
                p.parent().is_some_and(directory_is_writable) || command_available("pkexec")
            });
            (exe.is_some() && removable, if removable { String::new() } else {
                "The executable location is not writable and pkexec is unavailable.".to_string()
            })
        },
        InstallKind::Deb => {
            package = exe.as_ref().and_then(|p| deb_package_name(p));
            let ready = package.is_some() && command_available("pkexec");
            (ready, if ready { String::new() } else { "Could not identify the installed package or pkexec is unavailable.".to_string() })
        }
        InstallKind::Rpm => {
            package = exe.as_ref().and_then(|p| rpm_package_name(p));
            let ready = package.is_some() && command_available("pkexec");
            (ready, if ready { String::new() } else { "Could not identify the installed package or pkexec is unavailable.".to_string() })
        }
        InstallKind::Unknown(message) => (false, message.clone()),
    };
    InstallationInfo {
        kind: kind.label().to_string(),
        executable: exe,
        package,
        can_uninstall,
        uninstall_note,
    }
}


// ---------------------------------------------------------------------------
// Application uninstall lifecycle
// ---------------------------------------------------------------------------

/// Starts a post-exit uninstaller. It waits for Musicerer to close before
/// removing its executable/package and selected application data.
pub fn spawn_uninstall_app(h: &Handle, options: UninstallOptions, output_dir: PathBuf) {
    {
        let mut g = lock(h);
        if g.uninstalling || g.bulk_installing
            || g.rows.iter().any(|r| matches!(r.status, Status::Checking | Status::Downloading(_) | Status::Installing | Status::Removing))
        {
            return;
        }
        g.uninstalling = true;
        g.uninstall_error = None;
        g.generation = g.generation.wrapping_add(1);
    }
    let h2 = h.clone();
    thread::spawn(move || {
        match start_uninstall_helper(options, output_dir) {
            Ok(()) => request_exit(&h2),
            Err(error) => {
                let mut g = lock(&h2);
                g.uninstalling = false;
                g.uninstall_error = Some(error);
                g.generation = g.generation.wrapping_add(1);
            }
        }
    });
}

pub fn take_uninstall_error(h: &Handle) -> Option<String> {
    lock(h).uninstall_error.take()
}


// --- Musicerer uninstall -----------------------------------------------------

fn start_uninstall_helper(options: UninstallOptions, output_dir: PathBuf) -> Result<(), String> {
    let info = installation_info();
    if !info.can_uninstall {
        return Err(if info.uninstall_note.is_empty() {
            "This installation type cannot be uninstalled automatically.".to_string()
        } else { info.uninstall_note });
    }
    let exe = info.executable.clone().ok_or_else(|| "Could not locate the running executable".to_string())?;
    let cleanup = uninstall_cleanup_paths(&options, &output_dir)?;
    let pid = std::process::id();

    #[cfg(windows)]
    {
        let kind = detect_install_kind();
        let uninstaller = if kind == InstallKind::WinSetup {
            Some(exe.parent().ok_or_else(|| "The installation directory could not be resolved".to_string())?
                .join("unins000.exe"))
        } else { None };
        let remove_app_file = if matches!(kind, InstallKind::WinPortable) {
            vec![exe.clone(), exe.with_extension("exe.old"), exe.with_extension("exe.new")]
        } else { Vec::new() };
        let mut cleanup = cleanup;
        if let Some(parent) = exe.parent() {
            // The helper removes this directory only if it is empty afterward.
            // This avoids leaving an empty portable/install directory behind
            // without touching any remaining user files.
            cleanup.prune_dirs.push(parent.to_path_buf());
        }
        let elevate = !exe.parent().is_some_and(directory_is_writable);
        spawn_windows_uninstall_helper(pid, uninstaller, elevate, remove_app_file, cleanup)
    }

    #[cfg(target_os = "linux")]
    {
        let kind = detect_install_kind_for_management();
        let package_action = match &kind {
            InstallKind::Deb => {
                let package = deb_package_name(&exe).ok_or_else(|| "Could not determine the installed Debian package name".to_string())?;
                Some(("deb".to_string(), package))
            }
            InstallKind::Rpm => {
                let package = rpm_package_name(&exe).ok_or_else(|| "Could not determine the installed RPM package name".to_string())?;
                Some(("rpm".to_string(), package))
            }
            _ => None,
        };
        let remove_app_file = if matches!(&kind, InstallKind::AppImage | InstallKind::LinuxPortable) {
            std::env::var_os("APPIMAGE").map(PathBuf::from).unwrap_or_else(|| exe.clone())
        } else { PathBuf::new() };
        let mut cleanup = cleanup;
        if kind == InstallKind::LinuxPortable {
            if let Some(parent) = exe.parent() {
                // Remove a dedicated portable folder only if it becomes empty.
                cleanup.prune_dirs.push(parent.to_path_buf());
            }
        }
        if kind == InstallKind::AppImage {
            if let Some(name) = remove_app_file.file_name().and_then(|n| n.to_str()) {
                cleanup.remove_paths.push(remove_app_file.with_file_name(format!("{name}.new")));
            }
        }
        spawn_linux_uninstall_helper(pid, package_action, remove_app_file, cleanup)
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (pid, exe, cleanup);
        Err("Automatic uninstallation is not supported on this platform.".to_string())
    }
}

fn uninstall_cleanup_paths(options: &UninstallOptions, output_dir: &Path) -> Result<UninstallCleanup, String> {
    let mut cleanup = UninstallCleanup { remove_paths: Vec::new(), prune_dirs: Vec::new() };
    let config_root = dirs::config_dir().map(|d| d.join("Musicerer"));
    if let Some(root) = &config_root {
        if !options.keep_settings { cleanup.remove_paths.push(root.join("config.json")); }
        if !options.keep_tools { cleanup.remove_paths.push(root.join("tools")); }
        if !options.keep_logs { cleanup.remove_paths.push(root.join("logs")); }
        if !options.keep_other_data {
            add_unclassified_children(root, &["config.json", "tools", "logs"], &mut cleanup.remove_paths)?;
        }
        cleanup.prune_dirs.push(root.clone());
    }
    if !options.keep_tools {
        // Embedded tools are extracted under a versioned Musicerer-owned temp
        // directory. Remove old-version extraction folders too, not just the
        // directory for the build currently being uninstalled.
        add_matching_children(
            &std::env::temp_dir(),
            "musicerer-tools-",
            &mut cleanup.remove_paths,
        )?;
    }
    if !options.keep_cache {
        cleanup.remove_paths.push(std::env::temp_dir().join("Musicerer").join("updates"));
        cleanup.prune_dirs.push(std::env::temp_dir().join("Musicerer"));
    }

    if let Some(root) = dirs::cache_dir().map(|d| d.join("Musicerer")) {
        if options.keep_cache {
            if !options.keep_logs { cleanup.remove_paths.push(root.join("logs")); }
        } else if options.keep_logs {
            add_unclassified_children(&root, &["logs"], &mut cleanup.remove_paths)?;
        } else {
            cleanup.remove_paths.push(root.clone());
        }
        cleanup.prune_dirs.push(root);
    }

    let mut data_roots = Vec::new();
    if let Some(path) = dirs::data_dir().map(|d| d.join("Musicerer")) { data_roots.push(path); }
    if let Some(path) = dirs::data_local_dir().map(|d| d.join("Musicerer")) { data_roots.push(path); }
    for root in data_roots {
        if config_root.as_ref().is_some_and(|p| same_path(p, &root)) { continue; }
        if options.keep_other_data {
            if !options.keep_logs { cleanup.remove_paths.push(root.join("logs")); }
        } else if options.keep_logs {
            add_unclassified_children(&root, &["logs"], &mut cleanup.remove_paths)?;
        } else {
            cleanup.remove_paths.push(root.clone());
        }
        cleanup.prune_dirs.push(root);
    }

    if !options.keep_output_directory && fs::symlink_metadata(output_dir).is_ok() {
        if !safe_to_remove_output_dir(output_dir) {
            return Err(format!(
                "For safety, Musicerer refused to delete the configured download directory because it may contain unrelated files or application data: {}",
                output_dir.display()
            ));
        }
        cleanup.remove_paths.push(output_dir.to_path_buf());
    }

    cleanup.remove_paths.sort();
    cleanup.remove_paths.dedup();
    cleanup.prune_dirs.sort();
    cleanup.prune_dirs.dedup();
    Ok(cleanup)
}

fn add_unclassified_children(root: &Path, preserve: &[&str], out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Could not inspect {} before uninstalling: {error}", root.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("Could not inspect an item in {}: {e}", root.display()))?;
        let name = entry.file_name();
        if name.to_str().is_some_and(|n| preserve.contains(&n)) { continue; }
        out.push(entry.path());
    }
    Ok(())
}


fn add_matching_children(root: &Path, prefix: &str, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Could not inspect {} before uninstalling: {error}", root.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("Could not inspect an item in {}: {e}", root.display()))?;
        if entry.file_name().to_string_lossy().starts_with(prefix) {
            out.push(entry.path());
        }
    }
    Ok(())
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn safe_to_remove_output_dir(path: &Path) -> bool {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
        Err(_) => return false,
    };
    if metadata.file_type().is_symlink() { return false; }
    let Ok(canonical) = fs::canonicalize(path) else { return false; };
    if canonical.parent().is_none() { return false; }
    if let Some(home) = dirs::home_dir() {
        let home = fs::canonicalize(&home).unwrap_or(home);
        // Never allow deleting the home directory or one of its parents.
        if canonical == home || home.starts_with(&canonical) { return false; }
    }
    if std::env::current_exe().ok().is_some_and(|exe| canonical == exe || exe.starts_with(&canonical)) { return false; }
    #[cfg(target_os = "linux")]
    for protected in ["/tmp", "/var", "/usr", "/etc", "/opt", "/bin", "/sbin", "/lib", "/lib64", "/dev", "/proc", "/sys", "/run", "/mnt", "/media"] {
        let protected = Path::new(protected);
        if canonical == protected || protected.starts_with(&canonical) || canonical.starts_with(protected) { return false; }
    }
    #[cfg(windows)]
    for variable in ["WINDIR", "SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramData"] {
        if let Some(value) = std::env::var_os(variable) {
            let protected = PathBuf::from(value);
            if let Ok(protected) = fs::canonicalize(protected) {
                if canonical == protected || canonical.starts_with(&protected) || protected.starts_with(&canonical) {
                    return false;
                }
            }
        }
    }
    for base in [dirs::config_dir(), dirs::data_dir(), dirs::data_local_dir(), dirs::cache_dir()].into_iter().flatten() {
        if canonical == base || canonical.starts_with(&base) || base.starts_with(&canonical) { return false; }
    }
    true
}

#[cfg(windows)]
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
const WINDOWS_UNINSTALL_SCRIPT: &str = r#"$ErrorActionPreference = 'Continue'
$TargetPid = __PID__
$ScriptPath = $PSCommandPath
$LogPath = Join-Path $env:TEMP 'musicerer-uninstall.log'
$Problems = New-Object System.Collections.Generic.List[string]
function Write-Log($text) { try { Add-Content -LiteralPath $LogPath -Value ('{0} {1}' -f (Get-Date -Format s), $text) } catch {} }
function Show-Error($text) {
    try {
        Add-Type -AssemblyName System.Windows.Forms
        [System.Windows.Forms.MessageBox]::Show($text, 'Musicerer uninstall', [System.Windows.Forms.MessageBoxButtons]::OK, [System.Windows.Forms.MessageBoxIcon]::Warning) | Out-Null
    } catch {}
}
# Retries because antivirus, the search indexer or a dying child process can hold a file for a moment.
function Remove-Path($path) {
    for ($i = 0; $i -lt 20; $i++) {
        if (-not (Test-Path -LiteralPath $path)) { return }
        try { Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction Stop; return } catch { Start-Sleep -Milliseconds 500 }
    }
    if (Test-Path -LiteralPath $path) { $Problems.Add("Could not remove $path"); Write-Log "Could not remove $path" }
}
function Remove-IfEmpty($path) {
    if ((Test-Path -LiteralPath $path -PathType Container) -and -not (Get-ChildItem -LiteralPath $path -Force | Select-Object -First 1)) {
        try { Remove-Item -LiteralPath $path -Force -ErrorAction Stop } catch { Write-Log "Could not remove empty folder ${path}: $($_.Exception.Message)" }
    }
}
try {
    Write-Log 'Uninstall helper started'
    $deadline = (Get-Date).AddSeconds(60)
    while ((Get-Process -Id $TargetPid -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 250 }
    # Let the process release its file handles.
    Start-Sleep -Milliseconds 500
__UNINSTALLER__
__REMOVALS__
__PRUNES__
    if ($Problems.Count -gt 0) {
        Show-Error ("Musicerer was uninstalled, but some files could not be removed:`n`n" + (($Problems | Select-Object -First 8) -join "`n") + "`n`nDetails: $LogPath")
    }
    Write-Log 'Uninstall helper finished'
} catch {
    Write-Log ('Uninstall failed: ' + $_.Exception.Message)
    Show-Error ('Musicerer could not be uninstalled: ' + $_.Exception.Message + "`n`nDetails: $LogPath")
} finally { Remove-Item -LiteralPath $ScriptPath -Force -ErrorAction SilentlyContinue }
"#;

#[cfg(windows)]
const WINDOWS_RUN_UNINSTALLER: &str = r#"    $Uninstaller = __PATH__
    if (-not (Test-Path -LiteralPath $Uninstaller)) { throw 'The native uninstaller is missing.' }
    # Only ask for administrator rights when the install folder needs them. A per-user
    # install does not, and a hidden UAC prompt is what made the first attempt do nothing.
    $arguments = @{ FilePath = $Uninstaller; ArgumentList = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'); Wait = $true; PassThru = $true }
    if (__ELEVATE__) { $arguments['Verb'] = 'RunAs' }
    $Uninstall = Start-Process @arguments
    if ($Uninstall.ExitCode -ne 0) { throw ('The Windows uninstaller returned exit code ' + $Uninstall.ExitCode + '. Your data was left untouched.') }
    # The Inno Setup uninstaller finishes in a temporary copy of itself after this process
    # returns, so wait until it has really removed itself before deleting anything else.
    $deadline = (Get-Date).AddSeconds(60)
    while ((Test-Path -LiteralPath $Uninstaller) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 300 }
    if (Test-Path -LiteralPath $Uninstaller) { throw 'The Windows uninstaller did not finish. Your data was left untouched.' }
    Write-Log 'Native uninstaller finished'"#;

#[cfg(windows)]
fn spawn_windows_uninstall_helper(
    pid: u32,
    uninstaller: Option<PathBuf>,
    elevate: bool,
    remove_app_files: Vec<PathBuf>,
    cleanup: UninstallCleanup,
) -> Result<(), String> {
    let script_path = std::env::temp_dir().join(format!("musicerer-uninstall-{}.ps1", pid));
    let uninstall_step = uninstaller
        .map(|path| {
            WINDOWS_RUN_UNINSTALLER
                .replace("__PATH__", &ps_quote(&path.to_string_lossy()))
                .replace("__ELEVATE__", if elevate { "$true" } else { "$false" })
        })
        .unwrap_or_default();
    // One failed removal must not stop the rest, so every path is handled on its own.
    let removals = remove_app_files
        .iter()
        .chain(cleanup.remove_paths.iter())
        .map(|path| format!("    Remove-Path {}", ps_quote(&path.to_string_lossy())))
        .collect::<Vec<_>>()
        .join("\n");
    // Children come before their parents because the list is sorted.
    let prunes = cleanup
        .prune_dirs
        .iter()
        .rev()
        .map(|dir| format!("    Remove-IfEmpty {}", ps_quote(&dir.to_string_lossy())))
        .collect::<Vec<_>>()
        .join("\n");
    let script = WINDOWS_UNINSTALL_SCRIPT
        .replace("__PID__", &pid.to_string())
        .replace("__UNINSTALLER__", &uninstall_step)
        .replace("__REMOVALS__", &removals)
        .replace("__PRUNES__", &prunes);
    // Windows PowerShell 5 reads a BOM-less file as ANSI, which would corrupt non-ASCII paths.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(script.as_bytes());
    fs::write(&script_path, bytes).map_err(|e| format!("Could not create the uninstall helper: {e}"))?;
    match hidden_command("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script_path)
        .spawn()
    {
        Ok(_) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(script_path);
            Err(format!("Could not start the uninstall helper: {e}"))
        }
    }
}

#[cfg(target_os = "linux")]
fn spawn_linux_uninstall_helper(
    pid: u32,
    package_action: Option<(String, String)>,
    remove_app_file: PathBuf,
    cleanup: UninstallCleanup,
) -> Result<(), String> {
    let mut script = String::from(
        "target_pid=$1\napp_file=$2\nshift 2\nwhile kill -0 \"$target_pid\" 2>/dev/null; do sleep 0.25; done\n\
         show_error() { message=$1; if command -v zenity >/dev/null 2>&1; then zenity --error --title='Musicerer uninstall failed' --text=\"$message\" >/dev/null 2>&1; elif command -v kdialog >/dev/null 2>&1; then kdialog --error \"$message\" --title='Musicerer uninstall failed' >/dev/null 2>&1; elif command -v notify-send >/dev/null 2>&1; then notify-send 'Musicerer uninstall failed' \"$message\" >/dev/null 2>&1; elif command -v xmessage >/dev/null 2>&1; then xmessage -center \"$message\" >/dev/null 2>&1; fi; }\n\
         fail() { code=$1; shift; show_error \"$*\"; exit \"$code\"; }\n",
    );
    if let Some((kind, package)) = package_action {
        let quoted = shell_quote(&package);
        if kind == "deb" {
            script.push_str(&format!("pkexec apt-get purge -y -- {quoted} || fail 1 'Package removal was cancelled or failed.'\n"));
        } else {
            script.push_str(&format!("pkexec rpm -e -- {quoted} || fail 1 'Package removal was cancelled or failed.'\n"));
        }
    } else if !remove_app_file.as_os_str().is_empty() {
        script.push_str(
            "if [ -e \"$app_file\" ] || [ -L \"$app_file\" ]; then rm -f -- \"$app_file\" || (command -v pkexec >/dev/null 2>&1 && pkexec rm -f -- \"$app_file\") || fail 1 'Could not remove the application file. Check permissions.'; fi\n",
        );
    }
    script.push_str(
        "while [ \"$#\" -gt 0 ] && [ \"$1\" != '--PRUNE--' ]; do path=$1; shift; if [ -e \"$path\" ] || [ -L \"$path\" ]; then rm -rf -- \"$path\" || fail 1 \"Could not remove $path. Check permissions or close programs using it.\"; fi; done\n\
         if [ \"$#\" -gt 0 ]; then shift; fi\nwhile [ \"$#\" -gt 0 ]; do path=$1; shift; if [ -d \"$path\" ] && [ -z \"$(find \"$path\" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null)\" ]; then rmdir -- \"$path\" || fail 1 \"Could not remove empty directory $path. Check permissions.\"; fi; done\n",
    );
    let mut command = hidden_command("sh");
    command.arg("-c").arg(script).arg("musicerer-uninstall").arg(pid.to_string()).arg(remove_app_file);
    for path in &cleanup.remove_paths {
        command.arg(path);
    }
    command.arg("--PRUNE--");
    for path in cleanup.prune_dirs.iter().rev() {
        command.arg(path);
    }
    command.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.spawn().map(|_| ()).map_err(|e| format!("Could not start the uninstall helper: {e}"))
}

#[cfg(target_os = "linux")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

