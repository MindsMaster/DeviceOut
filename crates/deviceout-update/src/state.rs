use serde::{Deserialize, Serialize};

use crate::{atomic_write, paths};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct State {
    #[serde(default)]
    pub last_check: Option<i64>,
    #[serde(default)]
    pub last_attempt: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_install_error: Option<String>,
    #[serde(default)]
    pub feed_etag: Option<String>,
    #[serde(default)]
    pub last_latest: Option<String>,
}

pub fn load_state() -> State {
    let path = paths::state_path();
    let Ok(bytes) = std::fs::read(&path) else {
        return State::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save_state(state: &State) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(state).unwrap_or_else(|_| b"{}".to_vec());
    atomic_write(&paths::state_path(), &bytes)
}
