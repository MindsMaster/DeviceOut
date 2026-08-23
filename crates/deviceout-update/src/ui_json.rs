use serde::{Deserialize, Serialize};

use crate::lock::with_file_lock;
use crate::paths;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UiJson {
    #[serde(default)]
    pub last_notified_version: Option<String>,
    #[serde(default)]
    pub telemetry_enabled: bool,
    #[serde(default)]
    pub telemetry_set: bool,
    #[serde(default)]
    pub telemetry_id: Option<String>,
    #[serde(default)]
    pub last_ping_unix: Option<i64>,
}

pub fn load_ui_json() -> UiJson {
    let path = paths::ui_json_path();
    let Ok(bytes) = std::fs::read(&path) else {
        return UiJson::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save_ui_json(ui: &UiJson) -> std::io::Result<()> {
    let path = paths::ui_json_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    with_file_lock(&path, |existing| {
        let mut merged: UiJson = existing
            .and_then(|b| serde_json::from_slice(b).ok())
            .unwrap_or_default();
        if ui.last_notified_version.is_some() {
            merged.last_notified_version = ui.last_notified_version.clone();
        }
        if ui.telemetry_set {
            merged.telemetry_enabled = ui.telemetry_enabled;
            merged.telemetry_set = true;
        }
        if ui.telemetry_id.is_some() {
            merged.telemetry_id = ui.telemetry_id.clone();
        }
        if ui.last_ping_unix.is_some() {
            merged.last_ping_unix = ui.last_ping_unix;
        }
        serde_json::to_vec_pretty(&merged).map_err(|e| std::io::Error::other(e.to_string()))
    })
}
