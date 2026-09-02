use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{atomic_write, paths};

pub const OUTBOX_CAP: usize = 8;
pub const SENT_CAP: usize = 20;
pub const FAILED_CAP: usize = 20;
pub const MAX_ATTEMPTS: u32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackKind {
    #[default]
    Bug,
    Feature,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutboxItem {
    pub id: String,
    pub created_ms: u64,
    pub feedback_id: String,
    #[serde(default)]
    pub kind: FeedbackKind,
    pub message: String,
    #[serde(default)]
    pub contact: Option<String>,
    #[serde(default)]
    pub diag: Option<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub last_attempt_ms: Option<u64>,
    #[serde(default)]
    pub ticket: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn ensure_feedback_id() -> std::io::Result<String> {
    let path = paths::feedback_id_path();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let id = existing.trim();
        if !id.is_empty() {
            return Ok(id.to_string());
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let id = random_id();
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut f) => {
            use std::io::Write;
            f.write_all(id.as_bytes())?;
            Ok(id)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = std::fs::read_to_string(&path)?;
            let id = existing.trim();
            if id.is_empty() {
                Err(std::io::Error::other("empty feedback.id"))
            } else {
                Ok(id.to_string())
            }
        }
        Err(e) => Err(e),
    }
}

pub fn new_outbox_item(
    kind: FeedbackKind,
    message: String,
    contact: Option<String>,
    diag: Option<String>,
    version: String,
) -> std::io::Result<OutboxItem> {
    let feedback_id = ensure_feedback_id()?;
    Ok(OutboxItem {
        id: format!("{}-{}", now_ms(), &random_id()[..8]),
        created_ms: now_ms(),
        feedback_id,
        kind,
        message,
        contact: contact.filter(|s| !s.trim().is_empty()),
        diag,
        version,
        attempts: 0,
        last_attempt_ms: None,
        ticket: None,
        error: None,
    })
}

pub fn save_item(dir: &Path, item: &OutboxItem) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.json", item.id));
    let bytes = serde_json::to_vec_pretty(item).map_err(|e| std::io::Error::other(e.to_string()))?;
    atomic_write(&path, &bytes)?;
    Ok(path)
}

pub fn load_item(path: &Path) -> Option<OutboxItem> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn list_json(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    files
}

pub fn move_item(from: &Path, to_dir: &Path, item: &OutboxItem) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(to_dir)?;
    let dest = save_item(to_dir, item)?;
    if from != dest {
        let _ = std::fs::remove_file(from);
    }
    Ok(dest)
}

pub fn cleanup_capped_dir(dir: &Path, cap: usize) {
    let mut files = list_json(dir);
    if files.len() <= cap {
        return;
    }
    files.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let extra = files.len() - cap;
    for path in files.into_iter().take(extra) {
        let _ = std::fs::remove_file(path);
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn random_id() -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(now_ms().to_le_bytes());
    if let Ok(exe) = std::env::current_exe() {
        h.update(exe.to_string_lossy().as_bytes());
    }
    h.update(format!("{:?}", std::thread::current().id()).as_bytes());
    let bytes = h.finalize();
    crate::to_hex(&bytes[..16])
}
