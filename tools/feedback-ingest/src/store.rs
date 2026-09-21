use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::util::{day_key, iso_ts, parse_day_key};

pub const PING_KEEP_SECS: u64 = 30 * 86400;

#[derive(Deserialize)]
pub struct Payload {
    pub feedback_id: String,
    pub message: String,
    #[serde(default)]
    pub contact: Option<String>,
    #[serde(default)]
    pub diag: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub version: String,
}

fn default_kind() -> String {
    "bug".into()
}

#[derive(Deserialize)]
pub struct Stored {
    #[serde(default)]
    pub ticket: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub feedback_id: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub contact: Option<String>,
    #[serde(default)]
    pub diag: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub handled_at: Option<u64>,
}

#[derive(Deserialize)]
pub struct PingPayload {
    pub telemetry_id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub locale: String,
    #[serde(default)]
    pub tz: String,
}

pub fn list_tickets(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json").map(|s| s.to_string())
        })
        .filter(|s| safe_ticket(s))
        .collect()
}

pub fn ticket_count(dir: &Path) -> u32 {
    list_tickets(dir).len() as u32
}

pub fn load_ticket(dir: &Path, ticket: &str) -> Option<Stored> {
    let bytes = std::fs::read(dir.join(format!("{ticket}.json"))).ok()?;
    let mut item: Stored = serde_json::from_slice(&bytes).ok()?;
    if item.ticket.is_empty() {
        item.ticket = ticket.to_string();
    }
    Some(item)
}

pub fn safe_ticket(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("DO-") else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn valid_ticket_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("DO-") else {
        return false;
    };
    !rest.is_empty()
        && rest.len() <= 32
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

static LAST_TICKET_MS: Mutex<u128> = Mutex::new(0);

pub fn ticket_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut last = LAST_TICKET_MS.lock().unwrap_or_else(|p| p.into_inner());
    let n = now.max(*last + 1);
    *last = n;
    format!("DO-{n:x}")
}

pub fn ticket_ms(ticket: &str) -> u64 {
    ticket
        .strip_prefix("DO-")
        .and_then(|h| u128::from_str_radix(h, 16).ok())
        .map(|ms| ms as u64)
        .unwrap_or(0)
}

pub fn forward(dir: &Path, payload: &Payload, ticket: &str, ip: &str) -> bool {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(format!("{ticket}.json"));
    let record = serde_json::json!({
        "ticket": ticket,
        "ip": ip,
        "feedback_id": payload.feedback_id,
        "message": payload.message,
        "contact": payload.contact,
        "diag": payload.diag,
        "kind": payload.kind,
        "version": payload.version,
        "handled_at": serde_json::Value::Null,
    });
    let Ok(bytes) = serde_json::to_vec_pretty(&record) else {
        return false;
    };
    if std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .is_err()
    {
        return false;
    }
    if write_replace(&path, &bytes).is_err() {
        let _ = std::fs::remove_file(&path);
        return false;
    }
    eprintln!(
        "ticket {ticket} id={} kind={} ip={ip}",
        payload.feedback_id, payload.kind
    );
    true
}

pub fn set_handled(dir: &Path, ticket: &str, handled: bool, now: u64) -> std::io::Result<bool> {
    let path = dir.join(format!("{ticket}.json"));
    let Ok(bytes) = std::fs::read(&path) else {
        return Ok(false);
    };
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(false);
    };
    let Some(map) = value.as_object_mut() else {
        return Ok(false);
    };
    let stamp = if handled {
        serde_json::json!(now)
    } else {
        serde_json::Value::Null
    };
    map.insert("handled_at".into(), stamp);
    let out = serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?;
    write_replace(&path, &out)?;
    Ok(true)
}

pub fn write_replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

pub fn pings_file(dir: &Path, epoch: u64) -> PathBuf {
    dir.join("stats")
        .join(format!("pings-{}.jsonl", day_key(epoch)))
}

pub fn count_lines(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

pub fn store_ping(dir: &Path, ping: &PingPayload, ip: &str, now: u64) -> std::io::Result<()> {
    std::fs::create_dir_all(dir.join("stats"))?;
    let line = serde_json::json!({
        "ts": iso_ts(now),
        "id": ping.telemetry_id,
        "version": ping.version,
        "os": ping.os,
        "arch": ping.arch,
        "locale": ping.locale,
        "tz": ping.tz,
        "ip": ip,
    })
    .to_string();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(pings_file(dir, now))?;
    writeln!(file, "{line}")?;
    Ok(())
}

pub fn prune_old_pings(dir: &Path, now: u64) {
    let cutoff = now.saturating_sub(PING_KEEP_SECS);
    let Ok(entries) = std::fs::read_dir(dir.join("stats")) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(date) = name
            .strip_prefix("pings-")
            .and_then(|s| s.strip_suffix(".jsonl"))
        else {
            continue;
        };
        let Some(day_epoch) = parse_day_key(date) else {
            continue;
        };
        if day_epoch < cutoff && std::fs::remove_file(entry.path()).is_ok() {
            eprintln!("pruned old ping log {name}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_id_validation_strict() {
        assert!(valid_ticket_id("DO-1991bf2a3c8"));
        assert!(valid_ticket_id(&ticket_id()));
        assert!(!valid_ticket_id("DO-"));
        assert!(!valid_ticket_id("DO-1991BF2A3C8"));
        assert!(!valid_ticket_id("XX-1991bf2a3c8"));
        assert!(!valid_ticket_id("DO-1991bf2a3c8/extra"));
        assert!(!valid_ticket_id("../DO-1991bf2a3c8"));
        assert!(!valid_ticket_id("DO-zz"));
        assert!(!valid_ticket_id(&format!("DO-{}", "a".repeat(33))));
    }

    #[test]
    fn ticket_ids_never_repeat_within_a_process() {
        let ids: Vec<String> = (0..64).map(|_| ticket_id()).collect();
        for pair in ids.windows(2) {
            let a = u128::from_str_radix(&pair[0][3..], 16).unwrap();
            let b = u128::from_str_radix(&pair[1][3..], 16).unwrap();
            assert!(b > a, "{} then {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn handling_a_ticket_is_reversible_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("fb-handled-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let payload = Payload {
            feedback_id: "abc".into(),
            message: "hello".into(),
            contact: Some("me@example.com".into()),
            diag: "diag".into(),
            kind: "feature".into(),
            version: "1.1.1".into(),
        };
        assert!(forward(&dir, &payload, "DO-1", "1.1.1.1"));
        assert!(load_ticket(&dir, "DO-1").unwrap().handled_at.is_none());

        assert!(set_handled(&dir, "DO-1", true, 1_700).unwrap());
        let item = load_ticket(&dir, "DO-1").unwrap();
        assert_eq!(item.handled_at, Some(1_700));
        assert_eq!(item.message, "hello");
        assert_eq!(item.kind, "feature");
        assert_eq!(item.contact.as_deref(), Some("me@example.com"));

        assert!(set_handled(&dir, "DO-1", false, 1_800).unwrap());
        assert!(load_ticket(&dir, "DO-1").unwrap().handled_at.is_none());
        assert!(!set_handled(&dir, "DO-missing", true, 1_900).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forward_refuses_to_overwrite_an_existing_ticket() {
        let dir = std::env::temp_dir().join(format!("fb-forward-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let payload = Payload {
            feedback_id: "abc".into(),
            message: "first".into(),
            contact: None,
            diag: String::new(),
            kind: "bug".into(),
            version: "1.0.0".into(),
        };
        assert!(forward(&dir, &payload, "DO-1", "1.1.1.1"));
        assert!(!forward(&dir, &payload, "DO-1", "1.1.1.1"));
        assert_eq!(load_ticket(&dir, "DO-1").unwrap().message, "first");
        assert!(!dir.join("DO-1.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
