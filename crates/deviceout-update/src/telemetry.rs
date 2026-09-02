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
    #[cfg(windows)]
    if let Some(guid) = read_machine_guid() {
        return Some(id_from_machine_guid(&guid));
    }
    let ui = load_ui_json();
    if let Some(id) = ui.telemetry_id.filter(|s| !s.is_empty()) {
        return Some(id);
    }
    let id = outbox::random_id();
    let mut ui = load_ui_json();
    ui.telemetry_id = Some(id.clone());
    save_ui_json(&ui).ok()?;
    Some(id)
}

fn id_from_machine_guid(guid: &str) -> String {
    crate::sha256_hex(guid.as_bytes())[..32].to_string()
}

#[cfg(windows)]
fn read_machine_guid() -> Option<String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

    let subkey: Vec<u16> = "SOFTWARE\\Microsoft\\Cryptography"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let value: Vec<u16> = "MachineGuid"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut size: u32 = 0;
        let status = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        );
        if status != ERROR_SUCCESS || size < 2 {
            return None;
        }
        let mut buf = vec![0u16; size.div_ceil(2) as usize];
        let status = RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        );
        if status != ERROR_SUCCESS {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let guid = String::from_utf16_lossy(&buf[..len]);
        if guid.is_empty() {
            None
        } else {
            Some(guid)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_guid_id_format() {
        let id = id_from_machine_guid("12345678-9abc-def0-1234-56789abcdef0");
        assert_eq!(id.len(), 32);
        assert!(id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn machine_guid_id_is_deterministic_and_distinct() {
        assert_eq!(id_from_machine_guid("guid-a"), id_from_machine_guid("guid-a"));
        assert_ne!(id_from_machine_guid("guid-a"), id_from_machine_guid("guid-b"));
    }

    #[test]
    fn machine_guid_id_matches_sha256_prefix() {
        let guid = "test-guid";
        assert_eq!(
            id_from_machine_guid(guid),
            crate::sha256_hex(guid.as_bytes())[..32]
        );
    }
}
