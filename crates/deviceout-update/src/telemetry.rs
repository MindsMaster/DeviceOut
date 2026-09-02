use std::ffi::OsStr;

use crate::ui_json::{load_ui_json, save_ui_json};
use crate::{outbox, spawn_updater};

pub fn is_enabled() -> bool {
    let ui = load_ui_json();
    if ui.telemetry_set {
        ui.telemetry_enabled
    } else {
        true
    }
}

pub fn set_enabled(enabled: bool) {
    let mut ui = load_ui_json();
    ui.telemetry_enabled = enabled;
    ui.telemetry_set = true;
    let _ = save_ui_json(&ui);
}

const PING_GAP_SECS: i64 = 15 * 60;

pub fn spawn_ping(version: &str) {
    if !is_enabled() {
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut ui = load_ui_json();
    if crate::schedule::within(ui.last_ping_unix, now, PING_GAP_SECS) {
        return;
    }
    ui.last_ping_unix = Some(now);
    let _ = save_ui_json(&ui);
    spawn_updater(&[
        OsStr::new("--ping"),
        OsStr::new("--plugin-version"),
        OsStr::new(version),
    ]);
}

pub fn ensure_telemetry_id() -> Option<String> {
    let mut ui = load_ui_json();
    if let Some(id) = ui.telemetry_id.as_deref().filter(|s| valid_id(s)) {
        return Some(id.to_string());
    }
    let id = outbox::random_id();
    ui.telemetry_id = Some(id.clone());
    save_ui_json(&ui).ok()?;
    Some(id)
}

fn valid_id(s: &str) -> bool {
    s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_ids_are_lowercase_hex_and_unique() {
        let a = outbox::random_id();
        let b = outbox::random_id();
        assert!(valid_id(&a), "{a}");
        assert!(valid_id(&b), "{b}");
        assert_ne!(a, b);
    }

    #[test]
    fn stored_ids_are_only_reused_when_well_formed() {
        assert!(valid_id(&"a".repeat(32)));
        assert!(!valid_id(""));
        assert!(!valid_id(&"A".repeat(32)));
        assert!(!valid_id(&"a".repeat(31)));
    }
}
