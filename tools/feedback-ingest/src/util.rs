use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::{Hit, Limits};
use crate::http::{Request, Response};

pub type Resp = Response;

pub fn normalize_path(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let path = path.trim_end_matches('/');
    let stripped = path.strip_prefix("/deviceout-feedback").unwrap_or(path);
    if stripped.is_empty() {
        "/".into()
    } else {
        stripped.to_string()
    }
}

pub fn text(status: u16, body: &str) -> Resp {
    Response::text(status, body)
}

pub fn ok_json() -> Resp {
    json_ok("{\"ok\":true}")
}

pub fn json_ok(body: &str) -> Resp {
    Response::bytes(200, body.as_bytes().to_vec(), "application/json")
}

pub fn html(body: String) -> Resp {
    Response::bytes(200, body.into_bytes(), "text/html; charset=utf-8")
}

pub fn secure(mut resp: Resp) -> Resp {
    for (k, v) in [
        ("X-Content-Type-Options", "nosniff"),
        ("X-Frame-Options", "DENY"),
        ("Referrer-Policy", "no-referrer"),
        ("Cache-Control", "no-store"),
        (
            "Content-Security-Policy",
            "default-src 'none'; script-src 'unsafe-inline' https://cdn.jsdelivr.net; style-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; form-action 'self'",
        ),
    ] {
        resp = resp.header(k, v);
    }
    resp
}

pub fn valid_telemetry_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn sanitize_field(s: &str, max: usize) -> String {
    s.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect()
}

pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y } as i32, m, d)
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + (d - 1) as u64;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

pub fn utc_parts(epoch: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (epoch / 86_400) as i64;
    let rem = epoch % 86_400;
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

pub fn day_key(epoch: u64) -> String {
    let (y, m, d, _, _, _) = utc_parts(epoch);
    format!("{y:04}{m:02}{d:02}")
}

pub fn parse_day_key(key: &str) -> Option<u64> {
    if key.len() != 8 || !key.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let y: i32 = key[..4].parse().ok()?;
    let m: u32 = key[4..6].parse().ok()?;
    let d: u32 = key[6..8].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((days_from_civil(y, m, d) * 86_400) as u64)
}

pub fn date_prefix(epoch: u64) -> String {
    let (y, m, d, _, _, _) = utc_parts(epoch);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn hour_prefix(epoch: u64) -> String {
    let (_, _, _, h, _, _) = utc_parts(epoch);
    format!("{}T{h:02}", date_prefix(epoch))
}

pub fn iso_ts(epoch: u64) -> String {
    let (y, m, d, h, min, s) = utc_parts(epoch);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}Z")
}

pub fn fmt_epoch(epoch: u64) -> String {
    let (y, m, d, h, min, _) = utc_parts(epoch);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}")
}

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn client_ip(req: &Request) -> String {
    let peer = req.peer.ip().to_string();
    if peer == "127.0.0.1" || peer == "::1" {
        let xff = header_value(req, "X-Forwarded-For");
        let real = header_value(req, "X-Real-IP");
        return pick_client_ip(&peer, xff.as_deref(), real.as_deref());
    }
    peer
}

pub fn pick_client_ip(peer: &str, xff: Option<&str>, real_ip: Option<&str>) -> String {
    if let Some(first) = xff.and_then(|v| v.split(',').next()).map(str::trim) {
        if plausible_ip(first) {
            return first.to_string();
        }
    }
    if let Some(real) = real_ip.map(str::trim) {
        if plausible_ip(real) {
            return real.to_string();
        }
    }
    peer.to_string()
}

fn plausible_ip(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 45
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() || c == '.' || c == ':')
}

pub fn header_value(req: &Request, name: &str) -> Option<String> {
    req.header(name).map(str::to_string)
}

pub fn over_limit(map: &HashMap<String, Vec<Hit>>, key: &str, max: u32) -> bool {
    map.get(key).map(|v| v.len() as u32 >= max).unwrap_or(false)
}

pub fn prune(st: &mut Limits) {
    let hour = Duration::from_secs(3600);
    let day = Duration::from_secs(86400);
    let now = Instant::now();
    st.by_ip.retain(|_, v| {
        v.retain(|h| now.saturating_duration_since(h.at) < hour);
        !v.is_empty()
    });
    st.by_id.retain(|_, v| {
        v.retain(|h| now.saturating_duration_since(h.at) < day);
        !v.is_empty()
    });
    st.admin_fail_ip.retain(|_, v| {
        v.retain(|h| now.saturating_duration_since(h.at) < hour);
        !v.is_empty()
    });
    st.ping_by_ip.retain(|_, v| {
        v.retain(|h| now.saturating_duration_since(h.at) < hour);
        !v.is_empty()
    });
    st.ping_last
        .retain(|_, at| now.saturating_duration_since(*at) < day);
}

pub fn ct_eq(a: &str, b: &str) -> bool {
    let aa = a.as_bytes();
    let bb = b.as_bytes();
    let n = aa.len().max(bb.len()).max(1);
    let mut diff = aa.len() ^ bb.len();
    for i in 0..n {
        let x = *aa.get(i).unwrap_or(&0);
        let y = *bb.get(i).unwrap_or(&0);
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

pub fn b64(input: &str) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] } else { 0 };
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(TABLE[(((b1 & 15) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < bytes.len() {
            out.push(TABLE[(b2 & 63) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_key_roundtrip() {
        let epoch = 1_787_097_600u64;
        assert_eq!(day_key(epoch), "20260819");
        assert_eq!(parse_day_key("20260819"), Some(epoch));
        assert_eq!(iso_ts(epoch), "2026-08-19T00:00:00Z");
        assert_eq!(hour_prefix(epoch + 3661), "2026-08-19T01");
    }

    #[test]
    fn day_key_leap() {
        let epoch = 1_709_208_000u64;
        assert_eq!(day_key(epoch), "20240229");
        assert_eq!(parse_day_key("20240229"), Some(epoch - 12 * 3600));
    }

    #[test]
    fn telemetry_id_validation() {
        assert!(valid_telemetry_id("abc-123"));
        assert!(valid_telemetry_id("a"));
        assert!(!valid_telemetry_id(""));
        assert!(!valid_telemetry_id("ABC"));
        assert!(!valid_telemetry_id("has space"));
        assert!(!valid_telemetry_id(&"x".repeat(65)));
    }

    #[test]
    fn sanitize_truncates_and_strips_controls() {
        assert_eq!(sanitize_field("  hello\t", 32), "hello");
        assert_eq!(sanitize_field("a\nb", 32), "ab");
        let long = "v".repeat(100);
        assert_eq!(sanitize_field(&long, 32).len(), 32);
    }

    #[test]
    fn client_ip_prefers_first_xff_entry() {
        assert_eq!(
            pick_client_ip("127.0.0.1", Some(" 203.0.113.7 , 10.0.0.1"), None),
            "203.0.113.7"
        );
        assert_eq!(
            pick_client_ip("127.0.0.1", Some("2001:db8::1"), None),
            "2001:db8::1"
        );
    }

    #[test]
    fn client_ip_falls_back_in_order() {
        assert_eq!(
            pick_client_ip("127.0.0.1", None, Some("198.51.100.2")),
            "198.51.100.2"
        );
        assert_eq!(
            pick_client_ip("127.0.0.1", Some(""), Some("")),
            "127.0.0.1"
        );
        assert_eq!(
            pick_client_ip("127.0.0.1", Some("not an ip!"), Some("<script>")),
            "127.0.0.1"
        );
        assert_eq!(
            pick_client_ip("127.0.0.1", Some(&"1".repeat(46)), None),
            "127.0.0.1"
        );
        assert_eq!(
            pick_client_ip("127.0.0.1", Some("junk junk"), Some(" 8.8.8.8 ")),
            "8.8.8.8"
        );
    }

    #[test]
    fn plausible_ip_accepts_ip_literals_only() {
        assert!(plausible_ip("192.168.1.1"));
        assert!(plausible_ip("2001:db8::ff:1"));
        assert!(!plausible_ip(""));
        assert!(!plausible_ip("1.2.3.4;drop"));
        assert!(!plausible_ip("example.com"));
        assert!(!plausible_ip(&"a".repeat(46)));
    }
}
