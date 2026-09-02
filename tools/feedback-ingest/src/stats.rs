use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::store::{load_seen, pings_file, ticket_count};
use crate::util::{date_prefix, hour_prefix, utc_parts};

const ONLINE_WINDOW_SECS: u64 = 30 * 60;

pub struct DashStats {
    pub total_users: usize,
    pub online: usize,
    pub today_active: usize,
    pub total_tickets: usize,
    pub trend_labels: Vec<String>,
    pub trend_values: Vec<u32>,
    pub versions: Vec<(String, u32)>,
    pub locales: Vec<(String, u32)>,
    pub regions: Vec<(String, u32)>,
    pub os: Vec<(String, u32)>,
}

pub fn compute_stats(dir: &Path, now: u64) -> DashStats {
    let seen = load_seen(dir);
    let online = seen
        .values()
        .filter(|e| now.saturating_sub(e.last_seen) < ONLINE_WINDOW_SECS)
        .count();

    let today_start = now - now % 86_400;
    let mut today_ids: HashSet<String> = seen
        .iter()
        .filter(|(_, e)| e.last_seen >= today_start)
        .map(|(id, _)| id.clone())
        .collect();

    let hour_start = now - now % 3600;
    let mut buckets: Vec<HashSet<String>> = (0..24).map(|_| HashSet::new()).collect();
    let prefix_of: Vec<String> = (0..24)
        .map(|i| hour_prefix(hour_start - (23 - i) as u64 * 3600))
        .collect();
    let today = date_prefix(now);
    let mut os_latest: HashMap<String, String> = HashMap::new();
    for day in [now - 86_400, now] {
        let Ok(text) = std::fs::read_to_string(pings_file(dir, day)) else {
            continue;
        };
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let (Some(ts), Some(id)) = (
                v.get("ts").and_then(|x| x.as_str()),
                v.get("id").and_then(|x| x.as_str()),
            ) else {
                continue;
            };
            let (Some(hour), Some(date)) = (ts.get(..13), ts.get(..10)) else {
                continue;
            };
            if let Some(idx) = prefix_of.iter().position(|p| p == hour) {
                buckets[idx].insert(id.to_string());
                if let Some(o) = v.get("os").and_then(|x| x.as_str()) {
                    os_latest.insert(id.to_string(), o.to_string());
                }
                if date == today {
                    today_ids.insert(id.to_string());
                }
            }
        }
    }
    let trend_labels: Vec<String> = (0..24)
        .map(|i| format!("{:02}:00", utc_parts(hour_start - (23 - i) as u64 * 3600).3))
        .collect();
    let trend_values: Vec<u32> = buckets.iter().map(|b| b.len() as u32).collect();

    let mut versions: HashMap<String, u32> = HashMap::new();
    let mut locales: HashMap<String, u32> = HashMap::new();
    let mut regions: HashMap<String, u32> = HashMap::new();
    let mut os: HashMap<String, u32> = HashMap::new();
    for e in seen.values() {
        let v = if e.version.is_empty() {
            "未知"
        } else {
            e.version.as_str()
        };
        *versions.entry(v.to_string()).or_default() += 1;
        let l = if e.locale.is_empty() {
            "未知"
        } else {
            e.locale.as_str()
        };
        *locales.entry(l.to_string()).or_default() += 1;
        *regions.entry(region_of(&e.tz)).or_default() += 1;
    }
    for o in os_latest.values() {
        *os.entry(os_family(o)).or_default() += 1;
    }

    DashStats {
        total_users: seen.len(),
        online,
        today_active: today_ids.len(),
        total_tickets: ticket_count(dir) as usize,
        trend_labels,
        trend_values,
        versions: top_n(versions, 8),
        locales: top_n(locales, 10),
        regions: top_n(regions, 8),
        os: top_n(os, 8),
    }
}

fn top_n(map: HashMap<String, u32>, n: usize) -> Vec<(String, u32)> {
    let mut v: Vec<(String, u32)> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if v.len() > n {
        let rest: u32 = v.drain(n..).map(|x| x.1).sum();
        v.push(("其他".into(), rest));
    }
    v
}

pub fn region_of(tz: &str) -> String {
    for k in [
        "Shanghai",
        "Urumqi",
        "Chongqing",
        "Harbin",
        "Kashgar",
        "China Standard",
    ] {
        if tz.contains(k) {
            return "中国大陆".into();
        }
    }
    if tz.contains("Taipei") {
        return "中国台湾".into();
    }
    if tz.contains("Hong_Kong") || tz.contains("Hong Kong") || tz.contains("Macau") {
        return "中国港澳".into();
    }
    if tz.trim().is_empty() {
        return "未知".into();
    }
    "其他/海外".into()
}

pub fn os_family(os: &str) -> String {
    let o = os.trim();
    if o.is_empty() {
        return "未知".into();
    }
    if o.to_ascii_lowercase().starts_with("windows") {
        let mut it = o.split_whitespace();
        let a = it.next().unwrap_or("");
        let b = it.next().unwrap_or("");
        return format!("{a} {b}").trim().to_string();
    }
    o.split_whitespace().next().unwrap_or(o).to_string()
}

pub fn pairs_json(v: &[(String, u32)]) -> serde_json::Value {
    serde_json::json!({
        "labels": v.iter().map(|x| &x.0).collect::<Vec<_>>(),
        "values": v.iter().map(|x| x.1).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{store_ping, PingPayload};

    fn ping(id: &str) -> PingPayload {
        PingPayload {
            telemetry_id: id.into(),
            version: "1.0.0".into(),
            os: "Windows 11".into(),
            arch: "x86_64".into(),
            locale: "zh-CN".into(),
            tz: "Asia/Shanghai".into(),
        }
    }

    #[test]
    fn today_and_trend_come_from_ping_logs() {
        let dir = std::env::temp_dir().join(format!("fb-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let now = 1_787_140_800u64;
        store_ping(&dir, &ping("a"), "1.1.1.1", now - 600).unwrap();
        store_ping(&dir, &ping("b"), "1.1.1.1", now - 5 * 3600).unwrap();
        store_ping(&dir, &ping("c"), "1.1.1.1", now - 20 * 3600).unwrap();
        store_ping(&dir, &ping("d"), "1.1.1.1", now - 30 * 3600).unwrap();

        let stats = compute_stats(&dir, now);
        assert_eq!(stats.today_active, 2);
        assert_eq!(stats.trend_values.iter().sum::<u32>(), 3);
        assert_eq!(stats.trend_values[22], 1);
        assert_eq!(stats.trend_values[18], 1);
        assert_eq!(stats.trend_values[3], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn regions_and_os() {
        assert_eq!(region_of("China Standard Time"), "中国大陆");
        assert_eq!(region_of("Asia/Shanghai"), "中国大陆");
        assert_eq!(region_of("Asia/Taipei"), "中国台湾");
        assert_eq!(region_of("America/New_York"), "其他/海外");
        assert_eq!(region_of(""), "未知");
        assert_eq!(os_family("Windows 11 23H2 x86_64"), "Windows 11");
        assert_eq!(os_family("macOS 15.1 arm64"), "macOS");
        assert_eq!(os_family(""), "未知");
    }
}
