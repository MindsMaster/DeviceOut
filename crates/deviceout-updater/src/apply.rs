use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use deviceout_i18n::{fill, t};
use deviceout_update::paths;

use crate::{job, logutil};

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
        bail!(
            "pending manifest names no installed bundle: {}",
            manifest.bundle_path
        );
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
                &fill(t().updater_updated, &[("version", &manifest.version)]),
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
                &fill(t().updater_failed, &[("error", &format!("{e:#}"))]),
                true,
            );
            Err(e)
        }
    }
}

fn inno_args(bundle: &Path, log_path: &Path) -> Vec<String> {
    vec![
        "/VERYSILENT".into(),
        "/SUPPRESSMSGBOXES".into(),
        "/NORESTART".into(),
        "/NOCLOSEAPPLICATIONS".into(),
        format!("/DIR={}", bundle.display()),
        format!("/LOG={}", log_path.display()),
    ]
}

fn run_inno(manifest: &deviceout_update::PendingManifest, bundle: &Path) -> Result<()> {
    let setup = paths::pending_setup_path();
    let log_path = paths::appdata_dir().join("inno-setup.log");

    logutil::log(&format!(
        "apply: inno version={} dir={}",
        manifest.version,
        bundle.display()
    ));

    match job::run_tree(&setup, &inno_args(bundle, &log_path), INNO_TIMEOUT_MS)
        .context("run inno")?
    {
        job::Outcome::TimedOut => bail!("inno timed out after {INNO_TIMEOUT_MS} ms"),
        job::Outcome::Exited(0) => {}
        job::Outcome::Exited(code) => bail!("inno exit {code}"),
    }

    let dll = deviceout_update::bundle_dll(bundle);
    let got = deviceout_update::file_version_string(&dll)
        .ok_or_else(|| anyhow::anyhow!("no VS_VERSIONINFO on {}", dll.display()))?;
    if got != manifest.version {
        bail!(
            "version mismatch: installed {got}, expected {}",
            manifest.version
        );
    }
    Ok(())
}

fn wait_until_unlocked(bundle: &Path, version: &str) -> Result<bool> {
    loop {
        if !deviceout_update::bundle_dll_locked(bundle) {
            return Ok(true);
        }
        let retry = alert(&fill(t().updater_close_host, &[("version", version)]), true);
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
        let title: Vec<u16> = std::ffi::OsStr::new(t().updater_title)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let flags = MB_SETFOREGROUND
            | MB_TOPMOST
            | MB_ICONINFORMATION
            | if retry { MB_RETRYCANCEL } else { MB_OK };
        let rc =
            unsafe { MessageBoxW(std::ptr::null_mut(), text_w.as_ptr(), title.as_ptr(), flags) };
        rc == IDRETRY || !retry
    }
    #[cfg(not(windows))]
    {
        let _ = (text, retry);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inno_switches_use_documented_spellings() {
        let args = inno_args(
            Path::new(r"C:\x\DeviceOut.vst3"),
            Path::new(r"C:\x\inno.log"),
        );
        assert_eq!(
            args,
            [
                "/VERYSILENT",
                "/SUPPRESSMSGBOXES",
                "/NORESTART",
                "/NOCLOSEAPPLICATIONS",
                r"/DIR=C:\x\DeviceOut.vst3",
                r"/LOG=C:\x\inno.log",
            ]
        );
    }
}
