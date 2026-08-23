use std::ffi::OsStr;
use std::process::{Command, Stdio};

use crate::paths;

pub fn spawn_updater(args: &[&OsStr]) {
    let exe = paths::updater_exe();
    if !exe.is_file() {
        return;
    }
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let _ = cmd.spawn();
}
