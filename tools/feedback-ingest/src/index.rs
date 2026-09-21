use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::store::{pings_file, write_replace, PingPayload};
use crate::util::parse_iso_ts;

pub const DEVICE_CAP: usize = 200_000;
const FLUSH_DEBOUNCE: Duration = Duration::from_secs(10);
const REPLAY_DAYS: u64 = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Device {
    #[serde(default)]
    pub first_seen: u64,
    #[serde(default)]
    pub last_seen: u64,
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
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub pings: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Snapshot {
    #[serde(default)]
    saved_at: u64,
    #[serde(default)]
    devices: HashMap<String, Device>,
}

#[derive(Debug)]
pub struct Index {
    devices: HashMap<String, Device>,
    saved_at: u64,
    dirty: bool,
    last_flush: Instant,
}

pub fn snapshot_path(dir: &Path) -> std::path::PathBuf {
    dir.join("stats").join("devices.json")
}

fn legacy_path(dir: &Path) -> std::path::PathBuf {
    dir.join("stats").join("seen.json")
}

impl Index {
    pub fn load(dir: &Path, now: u64) -> Self {
        let snapshot = read_snapshot(dir);
        let mut index = Self {
            devices: snapshot.devices,
            saved_at: snapshot.saved_at,
            dirty: false,
            last_flush: Instant::now(),
        };
        let replayed = index.replay(dir, now);
        if replayed > 0 {
            eprintln!("index: replayed {replayed} pings after the last snapshot");
        }
        index
    }

    pub fn devices(&self) -> &HashMap<String, Device> {
        &self.devices
    }

    pub fn len(&self) -> usize {
        self.devices.len()
    }

    pub fn record(&mut self, ping: &PingPayload, ip: &str, at: u64) {
        let entry = self.devices.entry(ping.telemetry_id.clone()).or_default();
        if entry.first_seen == 0 || at < entry.first_seen {
            entry.first_seen = at;
        }
        entry.last_seen = entry.last_seen.max(at);
        entry.pings += 1;
        keep(&mut entry.version, &ping.version);
        keep(&mut entry.os, &ping.os);
        keep(&mut entry.arch, &ping.arch);
        keep(&mut entry.locale, &ping.locale);
        keep(&mut entry.tz, &ping.tz);
        keep(&mut entry.ip, ip);
        self.saved_at = self.saved_at.max(at);
        self.dirty = true;
        self.evict();
    }

    pub fn flush(&mut self, dir: &Path, force: bool) {
        if !self.dirty {
            return;
        }
        if !force && self.last_flush.elapsed() < FLUSH_DEBOUNCE {
            return;
        }
        let stats = dir.join("stats");
        if std::fs::create_dir_all(&stats).is_err() {
            return;
        }
        let snapshot = Snapshot {
            saved_at: self.saved_at,
            devices: self.devices.clone(),
        };
        let Ok(bytes) = serde_json::to_vec(&snapshot) else {
            return;
        };
        match write_replace(&snapshot_path(dir), &bytes) {
            Ok(()) => {
                self.dirty = false;
                self.last_flush = Instant::now();
            }
            Err(e) => eprintln!("index flush error: {e}"),
        }
    }

    fn replay(&mut self, dir: &Path, now: u64) -> usize {
        let cutoff = self.saved_at;
        let mut applied = 0;
        for back in (0..REPLAY_DAYS).rev() {
            let day = now.saturating_sub(back * 86_400);
            let Ok(text) = std::fs::read_to_string(pings_file(dir, day)) else {
                continue;
            };
            for line in text.lines() {
                let Some((ping, ip, at)) = parse_line(line) else {
                    continue;
                };
                if at <= cutoff {
                    continue;
                }
                self.record(&ping, &ip, at);
                applied += 1;
            }
        }
        self.dirty = applied > 0;
        applied
    }

    fn evict(&mut self) {
        if self.devices.len() <= DEVICE_CAP {
            return;
        }
        let mut by_age: Vec<(u64, String)> = self
            .devices
            .iter()
            .map(|(id, d)| (d.last_seen, id.clone()))
            .collect();
        by_age.sort();
        let drop = self.devices.len() - DEVICE_CAP;
        for (_, id) in by_age.into_iter().take(drop) {
            self.devices.remove(&id);
        }
    }
}

fn keep(slot: &mut String, value: &str) {
    if !value.trim().is_empty() {
        *slot = value.to_string();
    }
}

fn read_snapshot(dir: &Path) -> Snapshot {
    if let Some(snapshot) = std::fs::read(snapshot_path(dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<Snapshot>(&b).ok())
    {
        return snapshot;
    }
    let Some(devices) = std::fs::read(legacy_path(dir))
        .ok()
        .and_then(|b| serde_json::from_slice::<HashMap<String, Device>>(&b).ok())
    else {
        return Snapshot::default();
    };
    eprintln!("index: migrated {} devices from seen.json", devices.len());
    let saved_at = devices.values().map(|d| d.last_seen).max().unwrap_or(0);
    Snapshot { saved_at, devices }
}

fn parse_line(line: &str) -> Option<(PingPayload, String, u64)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let at = parse_iso_ts(v.get("ts")?.as_str()?)?;
    let field = |key: &str| {
        v.get(key)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let ping = PingPayload {
        telemetry_id: v.get("id")?.as_str()?.to_string(),
        version: field("version"),
        os: field("os"),
        arch: field("arch"),
        locale: field("locale"),
        tz: field("tz"),
    };
    Some((ping, field("ip"), at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::store_ping;

    fn ping(id: &str) -> PingPayload {
        PingPayload {
            telemetry_id: id.into(),
            version: "1.1.1".into(),
            os: "Windows 11 26100 x86_64".into(),
            arch: "x86_64".into(),
            locale: "zh-CN".into(),
            tz: "China Standard Time".into(),
        }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fb-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn repeated_pings_never_drop_the_last_seen() {
        let dir = temp("index-last-seen");
        let mut index = Index::load(&dir, 1_000);
        index.record(&ping("a"), "1.2.3.4", 1_000);
        index.record(&ping("a"), "1.2.3.4", 1_060);
        index.record(&ping("a"), "1.2.3.4", 1_120);
        let d = &index.devices()["a"];
        assert_eq!(d.first_seen, 1_000);
        assert_eq!(d.last_seen, 1_120);
        assert_eq!(d.pings, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_fields_never_overwrite_what_is_known() {
        let dir = temp("index-keep");
        let mut index = Index::load(&dir, 1_000);
        index.record(&ping("a"), "1.2.3.4", 1_000);
        let mut blank = ping("a");
        blank.locale = String::new();
        blank.os = "   ".into();
        index.record(&blank, "", 1_060);
        let d = &index.devices()["a"];
        assert_eq!(d.locale, "zh-CN");
        assert_eq!(d.os, "Windows 11 26100 x86_64");
        assert_eq!(d.ip, "1.2.3.4");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_round_trips_through_disk() {
        let dir = temp("index-snapshot");
        let mut index = Index::load(&dir, 1_000);
        index.record(&ping("a"), "1.2.3.4", 1_000);
        index.record(&ping("b"), "5.6.7.8", 1_100);
        index.flush(&dir, true);
        let reloaded = Index::load(&dir, 1_100);
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded.devices()["b"].last_seen, 1_100);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_crash_after_the_snapshot_is_recovered_from_the_ping_log() {
        let dir = temp("index-replay");
        let now = 1_787_140_800u64;
        let mut index = Index::load(&dir, now);
        index.record(&ping("a"), "1.2.3.4", now - 600);
        store_ping(&dir, &ping("a"), "1.2.3.4", now - 600).unwrap();
        index.flush(&dir, true);
        store_ping(&dir, &ping("a"), "1.2.3.4", now - 60).unwrap();
        store_ping(&dir, &ping("b"), "5.6.7.8", now - 30).unwrap();

        let reloaded = Index::load(&dir, now);
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded.devices()["a"].last_seen, now - 60);
        assert_eq!(reloaded.devices()["a"].pings, 2);
        assert_eq!(reloaded.devices()["b"].first_seen, now - 30);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_legacy_seen_file_is_migrated_once() {
        let dir = temp("index-legacy");
        std::fs::create_dir_all(dir.join("stats")).unwrap();
        let legacy = serde_json::json!({
            "old": {"first_seen": 10, "last_seen": 20, "version": "1.0.0", "locale": "en-US", "pings": 4}
        });
        std::fs::write(legacy_path(&dir), legacy.to_string()).unwrap();
        let mut index = Index::load(&dir, 1_000);
        assert_eq!(index.devices()["old"].version, "1.0.0");
        assert_eq!(index.devices()["old"].pings, 4);
        index.record(&ping("old"), "1.2.3.4", 1_000);
        index.flush(&dir, true);
        let reloaded = Index::load(&dir, 1_000);
        assert_eq!(reloaded.devices()["old"].version, "1.1.1");
        assert_eq!(reloaded.devices()["old"].first_seen, 10);
        assert_eq!(reloaded.devices()["old"].pings, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
