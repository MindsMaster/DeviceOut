mod diag;
mod feed;
mod lock;
pub mod outbox;
pub mod paths;
mod pending;
pub mod schedule;
mod sign;
mod spawn;
mod state;
pub mod telemetry;
mod ui_json;
mod version;

pub use diag::{build_diagnostics, os_short, sanitize_user_paths, write_panic};
pub use feed::{parse_feed, Feed};
pub use outbox::{
    cleanup_capped_dir, ensure_feedback_id, list_json, load_item, move_item, new_outbox_item,
    save_item, FeedbackKind, OutboxItem, FAILED_CAP, MAX_ATTEMPTS, OUTBOX_CAP, SENT_CAP,
};
pub use paths::{
    appdata_dir, bundle_dll, failed_dir, feedback_id_path, feedback_log_path,
    outbox_dir, panic_log_path, pending_dir, pending_manifest_path, pending_setup_path,
    sent_dir, state_path, ui_json_path, update_log_path, updater_exe, BUILTIN_FEED_URL,
    BUILTIN_FEEDBACK_URL, BUILTIN_FEEDBACK_TOKEN, BUNDLE_NAME, SETUP_PREFIX,
};
pub use pending::{pending_ready, read_manifest, write_manifest, PendingManifest};
pub use sign::{sign, verify, PUBLIC_KEYS};
pub use spawn::spawn_updater;
pub use state::{load_state, save_state, State};
pub use ui_json::{load_ui_json, save_ui_json, UiJson};
pub use version::{
    bundle_dll_locked, bundle_writable, cmp_latest, file_version_string, loaded_bundle_path,
    release_version, scope_label, valid_bundle_path, Cmp,
};

pub fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    replace_file(&tmp, path)
}

pub fn replace_file(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    to_hex(&h.finalize())
}

pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(to_hex(&h.finalize()))
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

pub fn parse_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = from_hex_digit(bytes[i])?;
        let lo = from_hex_digit(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

fn from_hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let b = [0x01, 0xab, 0xff];
        assert_eq!(parse_hex(&to_hex(&b)).as_deref(), Some(b.as_slice()));
    }

    #[test]
    fn atomic_write_replaces_existing() {
        let dir = std::env::temp_dir().join(format!("deviceout-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_replaces_while_a_reader_holds_the_file_open() {
        let dir = std::env::temp_dir().join(format!("deviceout-atomic-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        atomic_write(&path, b"one").unwrap();
        let reader = std::fs::File::open(&path).unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert!(!path.with_extension("tmp").exists());
        drop(reader);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
