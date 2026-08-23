use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use deviceout_update::paths;

static FEEDBACK: AtomicBool = AtomicBool::new(false);

pub fn set_feedback() {
    FEEDBACK.store(true, Ordering::Relaxed);
}

pub fn set_update() {
    FEEDBACK.store(false, Ordering::Relaxed);
}

pub fn log(msg: &str) {
    let path = if FEEDBACK.load(Ordering::Relaxed) {
        paths::feedback_log_path()
    } else {
        paths::update_log_path()
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{ts} {msg}");
    }
}
