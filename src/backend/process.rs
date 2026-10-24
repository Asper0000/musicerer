use std::ffi::OsStr;
use std::process::Command;

/// Builds a `Command` that never flashes a console window on Windows.
#[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
pub fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}
