use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::{Config, Hit, Limits};
use crate::http::Request;
use crate::store::{forward, store_ping, ticket_count, ticket_id, update_seen, Payload, PingPayload};
use crate::util::{
    client_ip, ct_eq, header_value, json_ok, now_epoch, ok_json, over_limit, prune, sanitize_field,
    text, valid_telemetry_id, Resp,
};

pub const MAX_BODY: usize = 48 * 1024;
const DIAG_CAP: usize = 32 * 1024;
const PING_MAX_BODY: usize = 8 * 1024;
const PING_MIN_INTERVAL: Duration = Duration::from_secs(15 * 60);

pub fn handle_post(req: &Request, cfg: &Config, state: &Mutex<Limits>) -> Resp {
    let Some(expected) = cfg.token.as_deref() else {
        return text(503, "disabled");
    };

    let ip = client_ip(req);

    {
        let mut st = state.lock().unwrap();
        prune(&mut st);
        if over_limit(&st.by_ip, &ip, cfg.rate_ip_hour) {
            return text(429, "rate");
        }
        st.by_ip
            .entry(ip.clone())
            .or_default()
            .push(Hit { at: Instant::now() });
    }

    let got = header_value(req, "X-DeviceOut-Token");
    if !ct_eq(got.as_deref().unwrap_or(""), expected) {
        return text(401, "token");
    }

    if req.body.len() > MAX_BODY {
        return text(413, "too large");
    }
    let Ok(mut payload) = serde_json::from_slice::<Payload>(&req.body) else {
        return text(400, "json");
    };
    payload.message = payload.message.trim().to_string();
    if payload.message.is_empty() {
        return text(400, "empty");
    }
    if payload.feedback_id.trim().is_empty() || payload.feedback_id.len() > 128 {
        return text(400, "id");
    }
    if !payload
        .feedback_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return text(400, "id");
    }
    payload.kind = if payload.kind == "feature" {
        "feature".into()
    } else {
        "bug".into()
    };
    payload.version = sanitize_field(&payload.version, 64);
    if payload.diag.len() > DIAG_CAP {
        payload.diag = payload.diag.chars().take(DIAG_CAP).collect();
    }
    if ticket_count(&cfg.dir) >= cfg.max_files {
        return text(503, "full").header("Retry-After", "3600");
    }

    {
        let mut st = state.lock().unwrap();
        prune(&mut st);
        if over_limit(&st.by_id, &payload.feedback_id, cfg.rate_id_day) {
            return text(429, "rate");
        }
        st.by_id
            .entry(payload.feedback_id.clone())
            .or_default()
            .push(Hit { at: Instant::now() });
    }

    let ticket = ticket_id();
    if !forward(&cfg.dir, &payload, &ticket, &ip) {
        return text(500, "store");
    }
    json_ok(&serde_json::json!({ "ticket": ticket }).to_string())
}

pub fn handle_ping(req: &Request, cfg: &Config, state: &Mutex<Limits>) -> Resp {
    let Some(expected) = cfg.token.as_deref() else {
        return text(503, "disabled");
    };
    let got = header_value(req, "X-DeviceOut-Token");
    if !ct_eq(got.as_deref().unwrap_or(""), expected) {
        return text(401, "token");
    }

    let ip = client_ip(req);
    {
        let mut st = state.lock().unwrap();
        prune(&mut st);
        if over_limit(&st.ping_by_ip, &ip, cfg.ping_rate_ip_hour) {
            return text(429, "rate");
        }
        st.ping_by_ip
            .entry(ip.clone())
            .or_default()
            .push(Hit { at: Instant::now() });
    }

    if req.body.len() > PING_MAX_BODY {
        return text(413, "too large");
    }
    let Ok(mut ping) = serde_json::from_slice::<PingPayload>(&req.body) else {
        return text(400, "json");
    };
    if !valid_telemetry_id(&ping.telemetry_id) {
        return text(400, "id");
    }
    ping.version = sanitize_field(&ping.version, 32);
    ping.os = sanitize_field(&ping.os, 128);
    ping.arch = sanitize_field(&ping.arch, 128);
    ping.locale = sanitize_field(&ping.locale, 128);
    ping.tz = sanitize_field(&ping.tz, 128);

    {
        let mut st = state.lock().unwrap();
        if let Some(last) = st.ping_last.get(&ping.telemetry_id) {
            if last.elapsed() < PING_MIN_INTERVAL {
                return ok_json();
            }
        }
        st.ping_last
            .insert(ping.telemetry_id.clone(), Instant::now());
    }

    let now = now_epoch();
    if let Err(e) = store_ping(&cfg.dir, &ping, &ip, now) {
        eprintln!("ping store error: {e}");
        return text(500, "store");
    }
    if let Err(e) = update_seen(&cfg.dir, &ping, &ip, now) {
        eprintln!("seen update error: {e}");
    }
    eprintln!(
        "ping id={} v={} os={} ip={ip}",
        ping.telemetry_id, ping.version, ping.os
    );
    ok_json()
}
