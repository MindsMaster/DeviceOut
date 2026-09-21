use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::lock::with_file_lock;
use crate::paths;

const READ_ATTEMPTS: u32 = 3;
const READ_BACKOFF: std::time::Duration = std::time::Duration::from_millis(20);
const DRIFT_CAP: usize = 32;
const DRIFT_LIMIT_PPM: f64 = 20_000.0;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UiJson {
    #[serde(default)]
    pub telemetry_enabled: bool,
    #[serde(default)]
    pub telemetry_set: bool,
    #[serde(default)]
    pub telemetry_id: Option<String>,
    #[serde(default)]
    pub last_ping_unix: Option<i64>,
    #[serde(default)]
    pub last_ping_try_unix: Option<i64>,
    #[serde(default)]
    pub ping_interval_secs: Option<i64>,
    #[serde(default)]
    pub drift_ppm: BTreeMap<String, DriftMemory>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct DriftMemory {
    pub ppm: f64,
    pub at: i64,
}

pub fn remember_drift(device_id: &str, ppm: f64, at: i64) {
    if device_id.is_empty() || !ppm.is_finite() || ppm.abs() > DRIFT_LIMIT_PPM {
        return;
    }
    let _ = update_ui_json(|ui| record_drift(&mut ui.drift_ppm, device_id, ppm, at));
}

pub fn recall_drift(device_id: &str) -> f64 {
    load_ui_json()
        .drift_ppm
        .get(device_id)
        .map(|m| m.ppm)
        .filter(|ppm| ppm.is_finite() && ppm.abs() <= DRIFT_LIMIT_PPM)
        .unwrap_or(0.0)
}

fn record_drift(store: &mut BTreeMap<String, DriftMemory>, device_id: &str, ppm: f64, at: i64) {
    store.insert(device_id.to_string(), DriftMemory { ppm, at });
    while store.len() > DRIFT_CAP {
        let Some(oldest) = store
            .iter()
            .min_by_key(|(_, m)| m.at)
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        store.remove(&oldest);
    }
}

pub fn load_ui_json() -> UiJson {
    load_at(&paths::ui_json_path())
}

pub fn update_ui_json<T>(edit: impl FnOnce(&mut UiJson) -> T) -> std::io::Result<T> {
    update_at(&paths::ui_json_path(), edit)
}

pub(crate) fn load_at(path: &Path) -> UiJson {
    for attempt in 0..READ_ATTEMPTS {
        match std::fs::read(path) {
            Ok(bytes) if bytes.is_empty() => return UiJson::default(),
            Ok(bytes) => {
                if let Ok(ui) = serde_json::from_slice(&bytes) {
                    return ui;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return UiJson::default(),
            Err(_) => {}
        }
        if attempt + 1 < READ_ATTEMPTS {
            std::thread::sleep(READ_BACKOFF);
        }
    }
    UiJson::default()
}

pub(crate) fn update_at<T>(path: &Path, edit: impl FnOnce(&mut UiJson) -> T) -> std::io::Result<T> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = None;
    with_file_lock(path, |existing| {
        let mut ui = match existing {
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?,
            None => UiJson::default(),
        };
        out = Some(edit(&mut ui));
        serde_json::to_vec_pretty(&ui).map_err(|e| std::io::Error::other(e.to_string()))
    })?;
    out.ok_or_else(|| std::io::Error::other("ui.json was never edited"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_drift_wins_and_the_oldest_is_evicted() {
        let mut store = BTreeMap::new();
        for i in 0..DRIFT_CAP + 4 {
            record_drift(&mut store, &format!("dev{i}"), i as f64, i as i64);
        }
        assert_eq!(store.len(), DRIFT_CAP);
        assert!(!store.contains_key("dev0"));
        assert!(!store.contains_key("dev3"));
        assert!(store.contains_key("dev4"));
        assert_eq!(store["dev35"].ppm, 35.0);

        record_drift(&mut store, "dev35", -12.5, 900);
        assert_eq!(store.len(), DRIFT_CAP);
        assert_eq!(store["dev35"].ppm, -12.5);
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("deviceout-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("ui.json")
    }

    #[test]
    fn edits_round_trip_through_the_lock() {
        let path = temp("roundtrip");
        let id = update_at(&path, |ui| {
            ui.telemetry_id = Some("abc".into());
            ui.last_ping_unix = Some(42);
            ui.telemetry_id.clone().unwrap()
        })
        .unwrap();
        assert_eq!(id, "abc");
        update_at(&path, |ui| ui.last_ping_try_unix = Some(7)).unwrap();
        let ui = load_at(&path);
        assert_eq!(ui.telemetry_id.as_deref(), Some("abc"));
        assert_eq!(ui.last_ping_unix, Some(42));
        assert_eq!(ui.last_ping_try_unix, Some(7));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_damaged_file_refuses_the_edit_instead_of_starting_over() {
        let path = temp("damaged");
        update_at(&path, |ui| ui.telemetry_id = Some("keep-me".into())).unwrap();
        std::fs::write(&path, b"{\"telemetry_id\": \"keep").unwrap();

        let err = update_at(&path, |ui| ui.telemetry_id = Some("new".into())).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"telemetry_id\": \"keep");
        assert!(load_at(&path).telemetry_id.is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_missing_file_starts_from_the_default() {
        let path = temp("missing");
        assert!(load_at(&path).telemetry_id.is_none());
        assert!(!load_at(&path).telemetry_set);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
