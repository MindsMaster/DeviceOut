use std::io::Read;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tiny_http::{Response, StatusCode};

use crate::config::{Config, Hit, Limits};
use crate::store::{forward, store_ping, ticket_count, ticket_id, update_seen, Payload, PingPayload};
use crate::util::{
    client_ip, ct_eq, header_value, json_ok, now_epoch, ok_json, over_limit, prune, sanitize_field,
    valid_telemetry_id, Resp,
};

const MAX_BODY: usize = 48 * 1024;
const DIAG_CAP: usize = 32 * 1024;
const PING_MAX_BODY: usize = 8 * 1024;
const PING_MIN_INTERVAL: Duration = Duration::from_secs(15 * 60);

pub fn handle_post(req: &mut tiny_http::Request, cfg: &Config, state: &Mutex<Limits>) -> Resp {
    let Some(expected) = cfg.token.as_deref() else {
        return Response::from_string("disabled").with_status_code(StatusCode(503));
    };

    let ip = client_ip(req);

    {
        let mut st = state.lock().unwrap();
        prune(&mut st);
        if over_limit(&st.by_ip, &ip, cfg.rate_ip_hour) {
            return Response::from_string("rate").with_status_code(StatusCode(429));
        }
        st.by_ip
            .entry(ip.clone())
            .or_default()
            .push(Hit { at: Instant::now() });
    }

    if let Some(len) = header_value(req, "Content-Length").and_then(|s| s.trim().parse::<usize>().ok())
    {
        if len > MAX_BODY {
            return Response::from_string("too large").with_status_code(StatusCode(413));
        }
    }

    let got = header_value(req, "X-DeviceOut-Token");
    if !ct_eq(got.as_deref().unwrap_or(""), expected) {
        return Response::from_string("token").with_status_code(StatusCode(401));
    }

    let mut body = Vec::new();
    let mut limited = req.as_reader().take(MAX_BODY as u64 + 1);
    if limited.read_to_end(&mut body).is_err() {
        return Response::from_string("read").with_status_code(StatusCode(400));
    }
    if body.len() > MAX_BODY {
        return Response::from_string("too large").with_status_code(StatusCode(413));
    }
    let Ok(mut payload) = serde_json::from_slice::<Payload>(&body) else {
        return Response::from_string("json").with_status_code(StatusCode(400));
    };
    payload.message = payload.message.trim().to_string();
    if payload.message.is_empty() {
        return Response::from_string("empty").with_status_code(StatusCode(400));
    }
    if payload.feedback_id.trim().is_empty() || payload.feedback_id.len() > 128 {
        return Response::from_string("id").with_status_code(StatusCode(400));
    }
    if !payload
        .feedback_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Response::from_string("id").with_status_code(StatusCode(400));
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
        return Response::from_string("full").with_status_code(StatusCode(429));
    }

    {
        let mut st = state.lock().unwrap();
        prune(&mut st);
        if over_limit(&st.by_id, &payload.feedback_id, cfg.rate_id_day) {
            return Response::from_string("rate").with_status_code(StatusCode(429));
        }
        st.by_id
            .entry(payload.feedback_id.clone())
            .or_default()
            .push(Hit { at: Instant::now() });
    }

    let ticket = ticket_id();
    if !forward(&cfg.dir, &payload, &ticket, &ip) {
        return Response::from_string("store").with_status_code(StatusCode(500));
    }
    json_ok(&serde_json::json!({ "ticket": ticket }).to_string())
}

pub fn handle_ping(req: &mut tiny_http::Request, cfg: &Config, state: &Mutex<Limits>) -> Resp {
    let Some(expected) = cfg.token.as_deref() else {
        return Response::from_string("disabled").with_status_code(StatusCode(503));
    };
    let got = header_value(req, "X-DeviceOut-Token");
    if !ct_eq(got.as_deref().unwrap_or(""), expected) {
        return Response::from_string("token").with_status_code(StatusCode(401));
    }

    let ip = client_ip(req);
    {
        let mut st = state.lock().unwrap();
        prune(&mut st);
        if over_limit(&st.ping_by_ip, &ip, cfg.ping_rate_ip_hour) {
            return Response::from_string("rate").with_status_code(StatusCode(429));
        }
        st.ping_by_ip
            .entry(ip.clone())
            .or_default()
            .push(Hit { at: Instant::now() });
    }

    if let Some(len) = header_value(req, "Content-Length").and_then(|s| s.trim().parse::<usize>().ok())
    {
        if len > PING_MAX_BODY {
            return Response::from_string("too large").with_status_code(StatusCode(413));
        }
    }
    let mut body = Vec::new();
    let mut limited = req.as_reader().take(PING_MAX_BODY as u64 + 1);
    if limited.read_to_end(&mut body).is_err() {
        return Response::from_string("read").with_status_code(StatusCode(400));
    }
    if body.len() > PING_MAX_BODY {
        return Response::from_string("too large").with_status_code(StatusCode(413));
    }
    let Ok(mut ping) = serde_json::from_slice::<PingPayload>(&body) else {
        return Response::from_string("json").with_status_code(StatusCode(400));
    };
    if !valid_telemetry_id(&ping.telemetry_id) {
        return Response::from_string("id").with_status_code(StatusCode(400));
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
        return Response::from_string("store").with_status_code(StatusCode(500));
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
