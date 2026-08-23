use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::{atomic_write, paths, sha256_file};

struct ReadyCache {
    setup_len: u64,
    setup_mtime: Option<SystemTime>,
    manifest_mtime: Option<SystemTime>,
    ready: Option<PendingManifest>,
}

static READY: Mutex<Option<ReadyCache>> = Mutex::new(None);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PendingManifest {
    pub version: String,
    pub sha256: String,
    pub bundle_path: String,
}

pub fn write_manifest(m: &PendingManifest) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(m).map_err(|e| std::io::Error::other(e.to_string()))?;
    atomic_write(&paths::pending_manifest_path(), &bytes)
}

pub fn read_manifest() -> Option<PendingManifest> {
    let mut bytes = std::fs::read(paths::pending_manifest_path()).ok()?;
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        bytes = bytes[3..].to_vec();
    }
    serde_json::from_slice(&bytes).ok()
}

pub fn pending_ready() -> Option<PendingManifest> {
    let setup = paths::pending_setup_path();
    let man = paths::pending_manifest_path();
    let setup_meta = std::fs::metadata(&setup).ok()?;
    let man_meta = std::fs::metadata(&man).ok()?;
    let stamp = (
        setup_meta.len(),
        setup_meta.modified().ok(),
        man_meta.modified().ok(),
    );
    if let Ok(guard) = READY.lock() {
        if let Some(cache) = guard.as_ref() {
            if cache.setup_len == stamp.0
                && cache.setup_mtime == stamp.1
                && cache.manifest_mtime == stamp.2
            {
                return cache.ready.clone();
            }
        }
    }
    let ready = verify_pending();
    if let Ok(mut guard) = READY.lock() {
        *guard = Some(ReadyCache {
            setup_len: stamp.0,
            setup_mtime: stamp.1,
            manifest_mtime: stamp.2,
            ready: ready.clone(),
        });
    }
    ready
}

fn verify_pending() -> Option<PendingManifest> {
    let manifest = read_manifest()?;
    let setup = paths::pending_setup_path();
    if !setup.is_file() {
        return None;
    }
    let Ok(actual) = sha256_file(&setup) else {
        return None;
    };
    if actual != manifest.sha256.to_ascii_lowercase() {
        return None;
    }
    if semver::Version::parse(&manifest.version).is_err() {
        return None;
    }
    let _ = PathBuf::from(&manifest.bundle_path);
    Some(manifest)
}
