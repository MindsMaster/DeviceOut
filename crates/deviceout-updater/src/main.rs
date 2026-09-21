#![cfg_attr(windows, windows_subsystem = "windows")]

mod apply;
mod check;
mod http;
mod job;
mod logutil;
mod mutex;
mod outbox_send;
mod ping;

use std::path::PathBuf;

use deviceout_update::paths;

const UPDATE_MUTEX: &str = "Local\\DeviceOut.Update";
const OUTBOX_MUTEX: &str = "Local\\DeviceOut.Outbox";
const PING_MUTEX: &str = "Local\\DeviceOut.Ping";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let send_outbox = args.iter().any(|a| a == "--send-outbox");
    let apply_pending = args.iter().any(|a| a == "--apply-pending");
    let from_temp = args.iter().any(|a| a == "--from-temp");
    let check_now = args.iter().any(|a| a == "--check-now");
    let do_ping = args.iter().any(|a| a == "--ping");
    let plugin_version = args
        .windows(2)
        .find(|w| w[0] == "--plugin-version")
        .map(|w| w[1].clone());

    if do_ping {
        let Some(_guard) = mutex::try_acquire(PING_MUTEX) else {
            return;
        };
        logutil::set_update();
        ping::run(plugin_version.as_deref());
        return;
    }

    if send_outbox {
        let Some(_guard) = mutex::try_acquire(OUTBOX_MUTEX) else {
            return;
        };
        logutil::set_feedback();
        if let Err(e) = outbox_send::send_all() {
            logutil::log(&format!("outbox: {e:#}"));
        }
        return;
    }

    if apply_pending {
        let Some(_guard) = mutex::wait_acquire(UPDATE_MUTEX, 60_000) else {
            return;
        };
        logutil::set_update();
        cleanup_temp_copies();
        if let Err(e) = apply::apply_pending(from_temp) {
            logutil::log(&format!("apply: {e:#}"));
        }
        return;
    }

    let Some(_guard) = (if check_now {
        mutex::wait_acquire(UPDATE_MUTEX, 15_000)
    } else {
        mutex::try_acquire(UPDATE_MUTEX)
    }) else {
        return;
    };
    logutil::set_update();
    cleanup_temp_copies();

    let bundle = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .filter(|p| deviceout_update::valid_bundle_path(p));
    let Some(bundle) = bundle else {
        logutil::log("check: refused, no valid bundle path argument");
        return;
    };

    let force = check_now || std::env::var_os("DEVICEOUT_FORCE_CHECK").is_some();
    if let Err(e) = check::run(&bundle, force) {
        logutil::log(&format!("check: {e:#}"));
        let mut state = deviceout_update::load_state();
        state.last_error = Some(format!("{e:#}"));
        let _ = deviceout_update::save_state(&state);
    }
}

fn cleanup_temp_copies() {
    let Ok(tmp) = std::env::temp_dir()
        .canonicalize()
        .or_else(|_| Ok::<_, ()>(std::env::temp_dir()))
    else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&tmp) else {
        return;
    };
    let self_path = std::env::current_exe().ok();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("deviceout-updater-") || !name.ends_with(".exe") {
            continue;
        }
        let path = entry.path();
        if self_path.as_ref().is_some_and(|s| s == &path) {
            continue;
        }
        let _ = std::fs::remove_file(&path);
    }
    let _ = paths::appdata_dir();
}

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
