use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::index::{Device, Index};
use crate::store::{pings_file, ticket_count};
use crate::util::{day_key, day_start, hour_label, hour_start, parse_iso_ts, OTHER, UNKNOWN};

pub const LEGACY_PING_INTERVAL_SECS: u64 = 15 * 60;
pub const ACTIVE_WINDOW_SECS: u64 = 30 * 86_400;
const ONLINE_FACTOR: u64 = 3;
const ONLINE_MIN_SECS: u64 = 15 * 60;
const ONLINE_MAX_SECS: u64 = 60 * 60;
const TREND_HOURS: u64 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    All,
    Days(u64),
}

impl Window {
    pub fn parse(value: Option<&str>) -> Self {
        let raw = value.unwrap_or("30d").trim();
        if raw == "all" {
            return Window::All;
        }
        raw.strip_suffix('d')
            .and_then(|n| n.parse::<u64>().ok())
            .filter(|d| (1..=3650).contains(d))
            .map(Window::Days)
            .unwrap_or(Window::Days(30))
    }

    pub fn key(&self) -> String {
        match self {
            Window::All => "all".into(),
            Window::Days(d) => format!("{d}d"),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Window::All => "全部".into(),
            Window::Days(d) => format!("{d} 天"),
        }
    }

    fn holds(&self, device: &Device, now: u64) -> bool {
        match self {
            Window::All => true,
            Window::Days(d) => now.saturating_sub(device.last_seen) <= d * 86_400,
        }
    }
}

#[derive(Debug)]
pub struct DashStats {
    pub total_users: usize,
    pub active: usize,
    pub online: usize,
    pub today_active: usize,
    pub total_tickets: usize,
    pub cohort: usize,
    pub trend_labels: Vec<String>,
    pub trend_values: Vec<u32>,
    pub versions: Vec<(String, u32)>,
    pub locales: Vec<(String, u32)>,
    pub regions: Vec<(String, u32)>,
    pub os: Vec<(String, u32)>,
}

pub fn compute_stats(
    index: &Index,
    dir: &Path,
    now: u64,
    offset_min: i32,
    window: Window,
) -> DashStats {
    let devices = index.devices();
    let online = devices.values().filter(|d| is_online(d, now)).count();
    let active = devices
        .values()
        .filter(|d| now.saturating_sub(d.last_seen) <= ACTIVE_WINDOW_SECS)
        .count();
    let today = day_start(now, offset_min);
    let today_active = devices.values().filter(|d| d.last_seen >= today).count();

    let mut cohort = 0;
    let mut versions: HashMap<String, u32> = HashMap::new();
    let mut locales: HashMap<String, u32> = HashMap::new();
    let mut regions: HashMap<String, u32> = HashMap::new();
    let mut os: HashMap<String, u32> = HashMap::new();
    for device in devices.values().filter(|d| window.holds(d, now)) {
        cohort += 1;
        bump(&mut versions, &device.version);
        bump(&mut os, &os_family(&device.os));
        bump(&mut locales, &device.locale);
        bump(
            &mut regions,
            &crate::geo::country_of(&device.locale, &device.tz),
        );
    }

    let (trend_labels, trend_values) = trend(dir, now, offset_min);

    DashStats {
        total_users: devices.len(),
        active,
        online,
        today_active,
        total_tickets: ticket_count(dir) as usize,
        cohort,
        trend_labels,
        trend_values,
        versions: top_n(versions, 8),
        locales: top_n(locales, 10),
        regions: top_n(regions, 12),
        os: top_n(os, 8),
    }
}

pub fn online_window(device: &Device) -> u64 {
    let interval = match device.interval_secs {
        0 => LEGACY_PING_INTERVAL_SECS,
        secs => secs.clamp(60, 3_600),
    };
    (interval * ONLINE_FACTOR).clamp(ONLINE_MIN_SECS, ONLINE_MAX_SECS)
}

fn is_online(device: &Device, now: u64) -> bool {
    now.saturating_sub(device.last_seen) < online_window(device)
}

fn trend(dir: &Path, now: u64, offset_min: i32) -> (Vec<String>, Vec<u32>) {
    let current = hour_start(now, offset_min);
    let oldest = current.saturating_sub((TREND_HOURS - 1) * 3600);
    let mut buckets: Vec<HashSet<String>> = (0..TREND_HOURS).map(|_| HashSet::new()).collect();

    let mut read = HashSet::new();
    for day in [oldest, now] {
        if !read.insert(day_key(day)) {
            continue;
        }
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
            let Some(at) = parse_iso_ts(ts) else {
                continue;
            };
            let bucket = hour_start(at, offset_min);
            if bucket < oldest || bucket > current {
                continue;
            }
            buckets[((bucket - oldest) / 3600) as usize].insert(id.to_string());
        }
    }

    let labels = (0..TREND_HOURS)
        .map(|i| hour_label(oldest + i * 3600, offset_min))
        .collect();
    let values = buckets.iter().map(|b| b.len() as u32).collect();
    (labels, values)
}

fn bump(map: &mut HashMap<String, u32>, value: &str) {
    let key = if value.trim().is_empty() {
        UNKNOWN
    } else {
        value.trim()
    };
    *map.entry(key.to_string()).or_default() += 1;
}

fn top_n(mut map: HashMap<String, u32>, n: usize) -> Vec<(String, u32)> {
    let unknown = map.remove(UNKNOWN);
    let mut v: Vec<(String, u32)> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if v.len() > n {
        let rest: u32 = v.drain(n..).map(|x| x.1).sum();
        v.push((OTHER.into(), rest));
    }
    if let Some(count) = unknown {
        v.push((UNKNOWN.into(), count));
    }
    v
}

pub fn os_family(os: &str) -> String {
    let o = os.trim();
    if o.is_empty() {
        return UNKNOWN.into();
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

    const NOW: u64 = 1_787_140_800;

    fn ping(id: &str, locale: &str, os: &str) -> PingPayload {
        PingPayload {
            telemetry_id: id.into(),
            version: "1.1.1".into(),
            os: os.into(),
            arch: "x86_64".into(),
            locale: locale.into(),
            tz: String::new(),
            interval: 0,
        }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fb-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn seeded(dir: &Path) -> Index {
        let mut index = Index::load(dir, NOW);
        index.record(
            &ping("fresh", "zh-CN", "Windows 11 26100"),
            "1.1.1.1",
            NOW - 60,
        );
        index.record(
            &ping("today", "en-US", "Windows 10 19045"),
            "1.1.1.2",
            NOW - 3 * 3600,
        );
        index.record(
            &ping("week", "ru-RU", "Windows 11 22631"),
            "1.1.1.3",
            NOW - 6 * 86_400,
        );
        index.record(&ping("ghost", "", ""), "1.1.1.4", NOW - 200 * 86_400);
        index
    }

    #[test]
    fn every_chart_adds_up_to_the_cohort_it_claims() {
        let dir = temp("stats-cohort");
        let index = seeded(&dir);
        for window in [Window::All, Window::Days(30), Window::Days(7)] {
            let stats = compute_stats(&index, &dir, NOW, 0, window);
            let sum = |v: &Vec<(String, u32)>| v.iter().map(|x| x.1).sum::<u32>() as usize;
            assert_eq!(sum(&stats.versions), stats.cohort, "{window:?}");
            assert_eq!(sum(&stats.os), stats.cohort, "{window:?}");
            assert_eq!(sum(&stats.locales), stats.cohort, "{window:?}");
            assert_eq!(sum(&stats.regions), stats.cohort, "{window:?}");
        }
        let all = compute_stats(&index, &dir, NOW, 0, Window::All);
        assert_eq!(all.cohort, 4);
        assert_eq!(all.total_users, 4);
        assert_eq!(
            compute_stats(&index, &dir, NOW, 0, Window::Days(30)).cohort,
            3
        );
        assert_eq!(
            compute_stats(&index, &dir, NOW, 0, Window::Days(7)).cohort,
            3
        );
        assert_eq!(
            compute_stats(&index, &dir, NOW, 0, Window::Days(1)).cohort,
            2
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn online_follows_each_client_own_cadence() {
        let dir = temp("stats-online");
        let mut index = Index::load(&dir, NOW);
        let mut fast = ping("fast", "zh-CN", "Windows 11");
        fast.interval = 300;
        index.record(&fast, "1.1.1.1", NOW - 60);
        index.record(&ping("legacy", "zh-CN", "Windows 11"), "1.1.1.2", NOW - 60);

        assert_eq!(online_window(&index.devices()["fast"]), 900);
        assert_eq!(online_window(&index.devices()["legacy"]), 2_700);

        assert_eq!(compute_stats(&index, &dir, NOW, 0, Window::All).online, 2);
        assert_eq!(
            compute_stats(&index, &dir, NOW + 1_000, 0, Window::All).online,
            1
        );
        assert_eq!(
            compute_stats(&index, &dir, NOW + 3_000, 0, Window::All).online,
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_silly_interval_cannot_stretch_the_online_window() {
        let huge = Device {
            interval_secs: u64::MAX,
            ..Device::default()
        };
        assert_eq!(online_window(&huge), ONLINE_MAX_SECS);
        let tiny = Device {
            interval_secs: 1,
            ..Device::default()
        };
        assert_eq!(online_window(&tiny), ONLINE_MIN_SECS);
    }

    #[test]
    fn the_active_window_still_counts_a_month() {
        let dir = temp("stats-active");
        let index = seeded(&dir);
        assert_eq!(
            compute_stats(&index, &dir, NOW, 0, Window::Days(30)).active,
            3
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn today_follows_the_viewer_offset() {
        let dir = temp("stats-today");
        let index = seeded(&dir);
        assert_eq!(
            compute_stats(&index, &dir, NOW, 0, Window::All).today_active,
            2
        );
        assert_eq!(
            compute_stats(&index, &dir, NOW, 480, Window::All).today_active,
            2
        );
        assert_eq!(
            compute_stats(&index, &dir, NOW, -660, Window::All).today_active,
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_trend_buckets_pings_by_the_viewer_hour() {
        let dir = temp("stats-trend");
        let index = Index::load(&dir, NOW);
        store_ping(
            &dir,
            &ping("a", "zh-CN", "Windows 11"),
            "1.1.1.1",
            NOW - 600,
        )
        .unwrap();
        store_ping(
            &dir,
            &ping("a", "zh-CN", "Windows 11"),
            "1.1.1.1",
            NOW - 300,
        )
        .unwrap();
        store_ping(
            &dir,
            &ping("b", "zh-CN", "Windows 11"),
            "1.1.1.2",
            NOW - 5 * 3600,
        )
        .unwrap();
        store_ping(
            &dir,
            &ping("c", "zh-CN", "Windows 11"),
            "1.1.1.3",
            NOW - 30 * 3600,
        )
        .unwrap();

        let stats = compute_stats(&index, &dir, NOW, 0, Window::All);
        assert_eq!(stats.trend_values.iter().sum::<u32>(), 2);
        assert_eq!(stats.trend_values[22], 1);
        assert_eq!(stats.trend_values[18], 1);
        assert_eq!(stats.trend_labels[23], "12:00");
        assert_eq!(stats.trend_labels[0], "13:00");

        let shifted = compute_stats(&index, &dir, NOW, 480, Window::All);
        assert_eq!(shifted.trend_values.iter().sum::<u32>(), 2);
        assert_eq!(shifted.trend_labels[23], "20:00");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_never_hides_inside_the_tail() {
        let mut map = HashMap::new();
        for i in 0..5 {
            map.insert(format!("v{i}"), 5 - i as u32);
        }
        map.insert(UNKNOWN.into(), 1);
        let top = top_n(map, 2);
        assert_eq!(top[0].0, "v0");
        assert_eq!(top[2].0, OTHER);
        assert_eq!(top[2].1, 3 + 2 + 1);
        assert_eq!(top[3].0, UNKNOWN);
        assert_eq!(top[3].1, 1);
    }

    #[test]
    fn windows_parse_from_the_query() {
        assert_eq!(Window::parse(None), Window::Days(30));
        assert_eq!(Window::parse(Some("all")), Window::All);
        assert_eq!(Window::parse(Some("7d")), Window::Days(7));
        assert_eq!(Window::parse(Some("junk")), Window::Days(30));
        assert_eq!(Window::parse(Some("0d")), Window::Days(30));
        assert_eq!(Window::parse(Some("99999d")), Window::Days(30));
        assert_eq!(Window::Days(7).key(), "7d");
        assert_eq!(Window::All.label(), "全部");
    }

    #[test]
    fn os_families() {
        assert_eq!(os_family("Windows 11 23H2 x86_64"), "Windows 11");
        assert_eq!(os_family("macOS 15.1 arm64"), "macOS");
        assert_eq!(os_family(""), UNKNOWN);
    }
}
