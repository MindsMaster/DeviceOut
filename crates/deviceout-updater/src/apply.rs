use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use deviceout_update::paths;

use crate::logutil;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const INNO_TIMEOUT_MS: u32 = 120_000;

pub fn apply_pending(from_temp: bool) -> Result<()> {
    let Some(manifest) = deviceout_update::pending_ready() else {
        logutil::log("apply: no valid pending");
        return Ok(());
    };

    if !from_temp {
        let copy = reexec_from_temp()?;
        logutil::log(&format!("apply: re-exec {}", copy.display()));
        return Ok(());
    }

    let bundle = PathBuf::from(&manifest.bundle_path);
    if !deviceout_update::valid_bundle_path(&bundle)
        || !deviceout_update::bundle_dll(&bundle).is_file()
    {
        let _ = std::fs::remove_dir_all(paths::pending_dir());
        bail!("pending manifest names no installed bundle: {}", manifest.bundle_path);
    }
    if !wait_until_unlocked(&bundle, &manifest.version)? {
        logutil::log("apply: user cancelled while locked");
        return Ok(());
    }

    match run_inno(&manifest, &bundle) {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(paths::pending_dir());
            let mut state = deviceout_update::load_state();
            state.last_install_error = None;
            let _ = deviceout_update::save_state(&state);
            cleanup_stale_updaters();
            logutil::log(&format!("apply: ok {}", manifest.version));
            alert(
                &format!(
                    "DeviceOut 已更新到 {}。\n\n请重新打开宿主以加载新版本。",
                    manifest.version
                ),
                false,
            );
            Ok(())
        }
        Err(e) => {
            logutil::log(&format!("apply: fail {e:#}"));
            let mut state = deviceout_update::load_state();
            state.last_install_error = Some(format!("{e:#}"));
            let _ = deviceout_update::save_state(&state);
            alert(
                &format!(
                    "安装未完成。\n\n关闭占用插件的程序后，可在插件界面再次打开安装程序。\n\n{e:#}"
                ),
                true,
            );
            Err(e)
        }
    }
}

fn run_inno(manifest: &deviceout_update::PendingManifest, bundle: &Path) -> Result<()> {
    let setup = paths::pending_setup_path();
    let log_path = paths::appdata_dir().join("inno-setup.log");
    let mut cmd = Command::new(&setup);
    cmd.arg("/VERYSILENT")
        .arg("/SUPPRESSMSGBOXES")
        .arg("/NORESTART")
        .arg("/CLOSEAPPLICATIONS=no")
        .arg(format!("/DIR={}", bundle.display()))
        .arg(format!("/LOG={}", log_path.display()));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    logutil::log(&format!(
        "apply: inno version={} dir={}",
        manifest.version,
        bundle.display()
    ));

    let mut child = cmd.spawn().context("spawn inno")?;
    let timed_out = wait_or_kill(&mut child, INNO_TIMEOUT_MS)?;
    if timed_out {
        bail!("inno timed out after {INNO_TIMEOUT_MS} ms");
    }
    let status = child.wait().context("wait inno")?;
    if !status.success() {
        bail!("inno exit {:?}", status.code());
    }

    let dll = deviceout_update::bundle_dll(bundle);
    let got = deviceout_update::file_version_string(&dll)
        .ok_or_else(|| anyhow::anyhow!("no VS_VERSIONINFO on {}", dll.display()))?;
    if got != manifest.version {
        bail!("version mismatch: installed {got}, expected {}", manifest.version);
    }
    Ok(())
}

fn wait_until_unlocked(bundle: &Path, version: &str) -> Result<bool> {
    loop {
        if !deviceout_update::bundle_dll_locked(bundle) {
            return Ok(true);
        }
        let retry = alert(
            &format!(
                "DeviceOut {version} 已下载。\n\n请先关闭占用该插件的程序（Studio One、Reaper 等），然后点击「重试」。\n关闭程序不会关掉这个安装窗口。"
            ),
            true,
        );
        if !retry {
            return Ok(false);
        }
    }
}

fn reexec_from_temp() -> Result<PathBuf> {
    let self_exe = std::env::current_exe().context("current_exe")?;
    let dest = std::env::temp_dir().join(format!("deviceout-updater-{}.exe", std::process::id()));
    std::fs::copy(&self_exe, &dest).with_context(|| format!("copy to {}", dest.display()))?;
    let mut cmd = Command::new(&dest);
    cmd.arg("--apply-pending").arg("--from-temp");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().context("spawn temp updater")?;
    Ok(dest)
}

fn cleanup_stale_updaters() {
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        let self_path = std::env::current_exe().ok();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("deviceout-updater-") && name.ends_with(".exe") {
                let path = entry.path();
                if self_path.as_ref().is_some_and(|s| s == &path) {
                    continue;
                }
                let _ = std::fs::remove_file(path);
            }
        }
    }
    if let Some(roaming) = std::env::var_os("APPDATA") {
        let leftover = PathBuf::from(roaming)
            .join("DeviceOut")
            .join("deviceout-updater.exe");
        if leftover.is_file() {
            let _ = std::fs::remove_file(leftover);
        }
    }
}

fn alert(text: &str, retry: bool) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDRETRY, MB_ICONINFORMATION, MB_OK, MB_RETRYCANCEL, MB_SETFOREGROUND,
            MB_TOPMOST,
        };

        let text_w: Vec<u16> = std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let title: Vec<u16> = std::ffi::OsStr::new("DeviceOut 更新")
            .encode_wide()
            .chain(Some(0))
            .collect();
        let flags = MB_SETFOREGROUND
            | MB_TOPMOST
            | MB_ICONINFORMATION
            | if retry { MB_RETRYCANCEL } else { MB_OK };
        let rc = unsafe { MessageBoxW(std::ptr::null_mut(), text_w.as_ptr(), title.as_ptr(), flags) };
        rc == IDRETRY || !retry
    }
    #[cfg(not(windows))]
    {
        let _ = (text, retry);
        true
    }
}

#[cfg(windows)]
fn wait_or_kill(child: &mut std::process::Child, timeout_ms: u32) -> Result<bool> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};

    let handle = child.as_raw_handle();
    let rc = unsafe { WaitForSingleObject(handle, timeout_ms) };
    match rc {
        WAIT_OBJECT_0 => Ok(false),
        WAIT_TIMEOUT => {
            unsafe { TerminateProcess(handle, 1) };
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(not(windows))]
fn wait_or_kill(child: &mut std::process::Child, _timeout_ms: u32) -> Result<bool> {
    let _ = child.wait();
    Ok(false)
}
