use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::index::Device;
use crate::util::{date_prefix, parse_day_key};

pub fn daily_path(dir: &Path) -> PathBuf {
    dir.join("stats").join("daily.jsonl")
}

pub fn rollup(dir: &Path, devices: &HashMap<String, Device>, now: u64) -> usize {
    let done = written_dates(dir);
    let mut pending: Vec<(u64, PathBuf)> = ping_log_days(dir)
        .into_iter()
        .filter(|(start, _)| start + 86_400 <= now)
        .filter(|(start, _)| !done.contains(&date_prefix(*start)))
        .collect();
    pending.sort();
    if pending.is_empty() {
        return 0;
    }
    let mut lines = Vec::new();
    for (start, path) in &pending {
        let ids = distinct_ids(path);
        let fresh = ids
            .iter()
            .filter(|id| {
                devices
                    .get(*id)
                    .is_some_and(|d| d.first_seen >= *start && d.first_seen < start + 86_400)
            })
            .count();
        lines.push(
            serde_json::json!({
                "date": date_prefix(*start),
                "dau": ids.len(),
                "new": fresh,
                "total": devices.len(),
            })
            .to_string(),
        );
    }
    let written = lines.len();
    if let Err(e) = append(&daily_path(dir), &lines) {
        eprintln!("daily rollup error: {e}");
        return 0;
    }
    eprintln!("daily: rolled up {written} day(s)");
    written
}

fn append(path: &Path, lines: &[String]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for line in lines {
        writeln!(file, "{line}")?;
    }
    file.sync_all()
}

fn written_dates(dir: &Path) -> HashSet<String> {
    let Ok(text) = std::fs::read_to_string(daily_path(dir)) else {
        return HashSet::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|v| v.get("date").and_then(|d| d.as_str()).map(str::to_string))
        .collect()
}

fn ping_log_days(dir: &Path) -> Vec<(u64, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir.join("stats")) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let key = name.strip_prefix("pings-")?.strip_suffix(".jsonl")?;
            Some((parse_day_key(key)?, entry.path()))
        })
        .collect()
}

fn distinct_ids(path: &Path) -> HashSet<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashSet::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|v| v.get("id").and_then(|x| x.as_str()).map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{store_ping, PingPayload};

    fn ping(id: &str) -> PingPayload {
        PingPayload {
            telemetry_id: id.into(),
            version: "1.1.1".into(),
            os: "Windows 11".into(),
            arch: "x86_64".into(),
            locale: "zh-CN".into(),
            tz: "China Standard Time".into(),
        }
    }

    fn device(first_seen: u64) -> Device {
        Device {
            first_seen,
            last_seen: first_seen,
            ..Device::default()
        }
    }

    #[test]
    fn only_finished_days_roll_up_and_never_twice() {
        let dir = std::env::temp_dir().join(format!("fb-daily-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let now = 1_787_140_800u64;
        let yesterday = now - 86_400;
        store_ping(&dir, &ping("a"), "1.2.3.4", yesterday).unwrap();
        store_ping(&dir, &ping("a"), "1.2.3.4", yesterday + 60).unwrap();
        store_ping(&dir, &ping("b"), "5.6.7.8", yesterday + 120).unwrap();
        store_ping(&dir, &ping("c"), "9.9.9.9", now).unwrap();

        let devices = HashMap::from([
            ("a".to_string(), device(yesterday)),
            ("b".to_string(), device(10)),
            ("c".to_string(), device(now)),
        ]);

        assert_eq!(rollup(&dir, &devices, now), 1);
        assert_eq!(rollup(&dir, &devices, now), 0);

        let text = std::fs::read_to_string(daily_path(&dir)).unwrap();
        assert_eq!(text.lines().count(), 1);
        let row: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(row["date"], date_prefix(yesterday));
        assert_eq!(row["dau"], 2);
        assert_eq!(row["new"], 1);
        assert_eq!(row["total"], 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
