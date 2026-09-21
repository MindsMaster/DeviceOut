use std::ffi::OsStr;

use crate::schedule::within;
use crate::ui_json::{load_ui_json, update_ui_json, UiJson};
use crate::{outbox, spawn_updater};

pub const DEFAULT_PING_GAP_SECS: i64 = 5 * 60;
const RETRY_GAP_SECS: i64 = 60;
const MIN_GAP_SECS: i64 = 60;
const MAX_GAP_SECS: i64 = 6 * 60 * 60;

pub fn is_enabled() -> bool {
    let ui = load_ui_json();
    if ui.telemetry_set {
        ui.telemetry_enabled
    } else {
        true
    }
}

pub fn set_enabled(enabled: bool) {
    let _ = update_ui_json(|ui| {
        ui.telemetry_enabled = enabled;
        ui.telemetry_set = true;
    });
}

pub fn spawn_ping(version: &str) {
    if !is_enabled() {
        return;
    }
    let now = unix_now();
    let Ok(true) = update_ui_json(|ui| {
        if !should_ping(ui, now) {
            return false;
        }
        ui.last_ping_try_unix = Some(now);
        true
    }) else {
        return;
    };
    spawn_updater(&[
        OsStr::new("--ping"),
        OsStr::new("--plugin-version"),
        OsStr::new(version),
    ]);
}

pub fn record_ping_ok(next_secs: Option<i64>) {
    let now = unix_now();
    let _ = update_ui_json(|ui| {
        ui.last_ping_unix = Some(now);
        if let Some(next) = next_secs {
            ui.ping_interval_secs = Some(jitter(next.clamp(MIN_GAP_SECS, MAX_GAP_SECS)));
        }
    });
}

pub fn ensure_telemetry_id() -> Option<String> {
    update_ui_json(ensure_id).ok()
}

pub fn ping_interval() -> i64 {
    gap(&load_ui_json())
}

pub fn should_ping(ui: &UiJson, now: i64) -> bool {
    if within(ui.last_ping_try_unix, now, RETRY_GAP_SECS) {
        return false;
    }
    !within(ui.last_ping_unix, now, gap(ui))
}

fn gap(ui: &UiJson) -> i64 {
    ui.ping_interval_secs
        .unwrap_or(DEFAULT_PING_GAP_SECS)
        .clamp(MIN_GAP_SECS, MAX_GAP_SECS)
}

fn ensure_id(ui: &mut UiJson) -> String {
    if let Some(id) = ui.telemetry_id.as_deref().filter(|s| valid_id(s)) {
        return id.to_string();
    }
    let id = outbox::random_id();
    ui.telemetry_id = Some(id.clone());
    id
}

fn jitter(secs: i64) -> i64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let span = (secs / 10).max(1);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_i64(secs);
    let delta = (hasher.finish() % (span as u64 * 2 + 1)) as i64 - span;
    (secs + delta).clamp(MIN_GAP_SECS, MAX_GAP_SECS)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn valid_id(s: &str) -> bool {
    s.len() == 32
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui(last_ok: Option<i64>, last_try: Option<i64>) -> UiJson {
        UiJson {
            last_ping_unix: last_ok,
            last_ping_try_unix: last_try,
            ..UiJson::default()
        }
    }

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

    #[test]
    fn a_good_id_is_kept_and_a_broken_one_is_replaced() {
        let mut kept = UiJson {
            telemetry_id: Some("a".repeat(32)),
            ..UiJson::default()
        };
        assert_eq!(ensure_id(&mut kept), "a".repeat(32));

        let mut broken = UiJson {
            telemetry_id: Some("nope".into()),
            ..UiJson::default()
        };
        let minted = ensure_id(&mut broken);
        assert!(valid_id(&minted));
        assert_eq!(broken.telemetry_id.as_deref(), Some(minted.as_str()));
        assert_eq!(ensure_id(&mut broken), minted);
    }

    #[test]
    fn the_first_ping_goes_out_immediately() {
        assert!(should_ping(&ui(None, None), 1_000));
    }

    #[test]
    fn a_success_holds_for_the_whole_interval() {
        let state = ui(Some(1_000), Some(1_000));
        assert!(!should_ping(&state, 1_000 + DEFAULT_PING_GAP_SECS - 1));
        assert!(should_ping(&state, 1_000 + DEFAULT_PING_GAP_SECS));
    }

    #[test]
    fn a_failed_attempt_retries_within_the_minute() {
        let state = ui(None, Some(1_000));
        assert!(!should_ping(&state, 1_000 + RETRY_GAP_SECS - 1));
        assert!(should_ping(&state, 1_000 + RETRY_GAP_SECS));
    }

    #[test]
    fn the_server_interval_wins_and_stays_sane() {
        let mut state = ui(Some(1_000), Some(1_000));
        state.ping_interval_secs = Some(1_800);
        assert!(!should_ping(&state, 1_000 + 1_799));
        assert!(should_ping(&state, 1_000 + 1_800));

        state.ping_interval_secs = Some(1);
        assert!(!should_ping(&state, 1_000 + MIN_GAP_SECS - 1));
        state.ping_interval_secs = Some(i64::MAX);
        assert!(should_ping(&state, 1_000 + MAX_GAP_SECS));
    }

    #[test]
    fn a_clock_moving_backwards_does_not_freeze_the_heartbeat() {
        assert!(should_ping(&ui(Some(1_000_000), Some(1_000_000)), 500));
    }

    #[test]
    fn jitter_stays_within_a_tenth_of_the_interval() {
        for _ in 0..64 {
            let v = jitter(600);
            assert!((540..=660).contains(&v), "{v}");
        }
        assert!(jitter(60) >= MIN_GAP_SECS);
    }
}
