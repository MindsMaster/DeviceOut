use std::path::{Path, PathBuf};

pub const BUNDLE_NAME: &str = "DeviceOut.vst3";
pub const SETUP_PREFIX: &str = "DeviceOut-Setup-";
pub const BUILTIN_FEED_URL: &str =
    "https://repo.azuramc.cc/repository/raw-public/deviceout/latest.json";
pub const BUILTIN_FEEDBACK_URL: &str = "https://repo.azuramc.cc/deviceout-feedback";
pub const BUILTIN_FEEDBACK_TOKEN: &str = "do1_7f3c2a91e4b64c0e9d18";

pub fn appdata_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".").join("DeviceOut-data"));
    base.join("DeviceOut")
}

pub fn updater_exe() -> PathBuf {
    appdata_dir().join("deviceout-updater.exe")
}

pub fn state_path() -> PathBuf {
    appdata_dir().join("state.json")
}

pub fn ui_json_path() -> PathBuf {
    appdata_dir().join("ui.json")
}

pub fn update_log_path() -> PathBuf {
    appdata_dir().join("update.log")
}

pub fn feedback_log_path() -> PathBuf {
    appdata_dir().join("feedback.log")
}

pub fn engine_log_path() -> PathBuf {
    appdata_dir().join("engine.log")
}

pub fn panic_log_path() -> PathBuf {
    appdata_dir().join("panic.log")
}

pub fn pending_dir() -> PathBuf {
    appdata_dir().join("pending")
}

pub fn pending_setup_path() -> PathBuf {
    pending_dir().join("setup.exe")
}

pub fn pending_manifest_path() -> PathBuf {
    pending_dir().join("manifest.json")
}

pub fn outbox_dir() -> PathBuf {
    appdata_dir().join("outbox")
}

pub fn sent_dir() -> PathBuf {
    appdata_dir().join("sent")
}

pub fn failed_dir() -> PathBuf {
    appdata_dir().join("failed")
}

pub fn feedback_id_path() -> PathBuf {
    appdata_dir().join("feedback.id")
}

pub fn bundle_dll(bundle: &Path) -> PathBuf {
    bundle.join("Contents").join("x86_64-win").join(BUNDLE_NAME)
}
