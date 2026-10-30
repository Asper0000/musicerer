use std::process::Command;

pub fn add_args(cmd: &mut Command) {
    cmd.arg("--format").arg("bestaudio[ext=m4a]/bestaudio/best");
}