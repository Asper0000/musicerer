//! Spawning helpers shared by every external tool (yt-dlp, ffmpeg, ffprobe, ...).

use std::ffi::OsStr;
use std::io;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

const STOP_POLL: Duration = Duration::from_millis(100);

/// Builds a `Command` that never flashes a console window on Windows and, on Unix,
/// runs in its own process group so the whole tree can be killed at once.
#[cfg_attr(not(any(windows, unix)), allow(unused_mut))]
pub fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd
}

/// Kills a process and everything it spawned (yt-dlp starts ffmpeg and deno).
pub fn kill_process_tree(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        let pid = pid.to_string();
        let _ = hidden_command("taskkill")
            .args(["/PID", pid.as_str(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let group = format!("-{pid}");
        let _ = Command::new("kill")
            .args(["-KILL", "--", group.as_str()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Kills `pid` as soon as `stop` is raised; returns once `finished` is set.
/// Polls on a timer so a silent, stalled child is still stopped promptly.
pub fn watch_for_stop(pid: u32, stop: &AtomicBool, finished: &AtomicBool) {
    while !finished.load(Ordering::SeqCst) {
        if stop.load(Ordering::SeqCst) {
            if !finished.load(Ordering::SeqCst) {
                kill_process_tree(pid);
            }
            return;
        }
        thread::sleep(STOP_POLL);
    }
}

/// Like `Command::output`, but the child (and its tree) is killed when `stop` is raised.
/// The caller checks the flag afterwards to tell a stop from a real failure.
pub fn output_stoppable(cmd: &mut Command, stop: &AtomicBool) -> io::Result<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn()?;
    let pid = child.id();
    let finished = AtomicBool::new(false);
    thread::scope(|scope| {
        scope.spawn(|| watch_for_stop(pid, stop, &finished));
        let output = child.wait_with_output();
        finished.store(true, Ordering::SeqCst);
        output
    })
}
