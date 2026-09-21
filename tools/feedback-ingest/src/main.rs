mod admin;
mod api;
mod assets;
mod config;
mod daily;
mod geo;
mod http;
mod index;
mod stats;
mod store;
mod util;

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use config::{env_nonempty, env_u32, load_dotenv, load_or_create_admin_path, Config, Limits};
use http::{Handler, Method, Request};
use index::Index;
use store::{count_lines, pings_file, prune_old_pings};
use util::{normalize_path, now_epoch, secure, text};

const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(3600);
const WORKERS: usize = 8;

fn main() {
    load_dotenv();
    assets::warmup();
    let bind = std::env::var("FEEDBACK_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    if !bind.starts_with("127.0.0.1") && !bind.starts_with("localhost") && bind != "::1" {
        eprintln!("warning: bind {bind} is not loopback; put this behind Caddy and do not expose the port");
    }
    let dir =
        PathBuf::from(std::env::var("FEEDBACK_DIR").unwrap_or_else(|_| "feedback-inbox".into()));
    let _ = std::fs::create_dir_all(dir.join("stats"));
    let admin_path = load_or_create_admin_path(&dir);
    let cfg = Config {
        token: env_nonempty("FEEDBACK_TOKEN"),
        admin_password: env_nonempty("FEEDBACK_ADMIN_PASSWORD"),
        admin_path,
        dir,
        kill: std::env::var_os("FEEDBACK_DISABLED").is_some() || Path::new("feedback.off").exists(),
        rate_ip_hour: env_u32("FEEDBACK_RATE_IP_HOUR", 8),
        rate_id_day: env_u32("FEEDBACK_RATE_ID_DAY", 12),
        admin_fail_hour: env_u32("FEEDBACK_ADMIN_FAIL_HOUR", 8),
        ping_rate_ip_hour: env_u32("FEEDBACK_PING_RATE_IP_HOUR", 240),
        max_files: env_u32("FEEDBACK_MAX_FILES", 500),
    };
    if cfg.token.is_none() {
        eprintln!("warning: FEEDBACK_TOKEN unset; POST will be rejected");
    }
    if cfg.admin_password.is_none() {
        eprintln!("warning: FEEDBACK_ADMIN_PASSWORD unset; admin page disabled");
    }
    let now = now_epoch();
    prune_old_pings(&cfg.dir, now);
    let mut index = Index::load(&cfg.dir, now);
    daily::rollup(&cfg.dir, index.devices(), now);
    index.flush(&cfg.dir, true);
    let today_pings = count_lines(&pings_file(&cfg.dir, now));
    eprintln!(
        "stats: {} known devices, {} pings today",
        index.len(),
        today_pings
    );
    let listener = TcpListener::bind(&bind).expect("bind");
    eprintln!(
        "deviceout-feedback-ingest {bind} token={} admin=/deviceout-feedback/{}/",
        if cfg.token.is_some() { "on" } else { "off" },
        cfg.admin_path
    );
    let state = Arc::new(Mutex::new(Limits {
        by_ip: HashMap::new(),
        by_id: HashMap::new(),
        admin_fail_ip: HashMap::new(),
        ping_by_ip: HashMap::new(),
        last_maintenance: Instant::now(),
    }));
    let index = Arc::new(Mutex::new(index));

    let cfg = Arc::new(cfg);
    let handler: Handler = Arc::new(move |req: Request| route(&req, &cfg, &state, &index));
    let limits = http::Limits {
        body_bytes: api::MAX_BODY,
        ..http::Limits::default()
    };
    if let Err(e) = http::serve(listener, WORKERS, limits, handler) {
        eprintln!("server stopped: {e}");
    }
}

fn route(
    req: &Request,
    cfg: &Config,
    state: &Mutex<Limits>,
    index: &Mutex<Index>,
) -> http::Response {
    if cfg.kill {
        return secure(text(503, "disabled"));
    }
    maintain(cfg, state, index);
    let path = normalize_path(&req.target);
    let delete_path = format!("/{}/delete", cfg.admin_path);
    let handle_path = format!("/{}/handle", cfg.admin_path);
    let response = match req.method {
        Method::Get | Method::Head => admin::handle_get(req, &path, cfg, state, index),
        Method::Post if path == "/ping" => api::handle_ping(req, cfg, state, index),
        Method::Post if path == delete_path => admin::handle_admin_delete(req, cfg, state),
        Method::Post if path == handle_path => admin::handle_admin_handle(req, cfg, state),
        Method::Post => api::handle_post(req, cfg, state),
        Method::Other => text(405, "method"),
    };
    secure(response)
}

fn maintain(cfg: &Config, state: &Mutex<Limits>, index: &Mutex<Index>) {
    {
        let mut st = state.lock().unwrap();
        if st.last_maintenance.elapsed() <= MAINTENANCE_INTERVAL {
            return;
        }
        st.last_maintenance = Instant::now();
    }
    let now = now_epoch();
    let mut idx = index.lock().unwrap();
    idx.flush(&cfg.dir, true);
    daily::rollup(&cfg.dir, idx.devices(), now);
    prune_old_pings(&cfg.dir, now);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};

    use util::b64;

    const ADMIN: &str = "abcdefghijklmnop";
    const LEGACY_PING: &str = r#"{"telemetry_id":"abc123","version":"1.0.0","os":"Windows 11 26100","arch":"x86_64","locale":"zh-CN","tz":"China Standard Time"}"#;
    const MODERN_PING: &str = r#"{"telemetry_id":"def456","version":"1.1.2","os":"Windows 11 26100","arch":"x86_64","locale":"zh-CN","tz":"China Standard Time","interval":300}"#;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fb-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn start(dir: &Path, ping_rate: u32) -> SocketAddr {
        let cfg = Arc::new(Config {
            token: Some("tok".into()),
            admin_password: Some("pw".into()),
            admin_path: ADMIN.into(),
            dir: dir.to_path_buf(),
            kill: false,
            rate_ip_hour: 100,
            rate_id_day: 100,
            admin_fail_hour: 100,
            ping_rate_ip_hour: ping_rate,
            max_files: 100,
        });
        let state = Arc::new(Mutex::new(Limits {
            by_ip: HashMap::new(),
            by_id: HashMap::new(),
            admin_fail_ip: HashMap::new(),
            ping_by_ip: HashMap::new(),
            last_maintenance: Instant::now(),
        }));
        let index = Arc::new(Mutex::new(Index::load(dir, now_epoch())));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handler: Handler = Arc::new(move |req: Request| route(&req, &cfg, &state, &index));
        std::thread::spawn(move || http::serve(listener, 2, http::Limits::default(), handler));
        addr
    }

    fn talk(addr: SocketAddr, raw: &str) -> String {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    fn post_ping(addr: SocketAddr, token: &str) -> String {
        post_body(addr, token, LEGACY_PING)
    }

    fn post_body(addr: SocketAddr, token: &str, body: &str) -> String {
        talk(
            addr,
            &format!(
                "POST /ping HTTP/1.1\r\nHost: a\r\nX-DeviceOut-Token: {token}\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        )
    }

    fn dashboard(addr: SocketAddr, query: &str) -> serde_json::Value {
        let reply = talk(
            addr,
            &format!(
                "GET /deviceout-feedback/{ADMIN}/data?{query} HTTP/1.1\r\nHost: a\r\nAuthorization: Basic {}\r\n\r\n",
                b64("admin:pw")
            ),
        );
        let body = reply.split("\r\n\r\n").nth(1).unwrap_or_default();
        serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {reply}"))
    }

    #[test]
    fn every_heartbeat_reaches_the_dashboard() {
        let dir = temp("route-ping");
        let addr = start(&dir, 100);
        for _ in 0..3 {
            let reply = post_ping(addr, "tok");
            assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
            assert!(reply.contains("\"next\":300"), "{reply}");
        }
        let data = dashboard(addr, "tzoff=480&window=30d");
        assert_eq!(data["users"], 1);
        assert_eq!(data["active"], 1);
        assert_eq!(data["online"], 1);
        assert_eq!(data["today"], 1);
        assert_eq!(data["cohort"], 1);
        assert_eq!(data["window"], "30d");
        assert_eq!(data["os"]["labels"][0], "Windows 11");
        assert_eq!(data["os"]["values"][0], 1);

        let log = std::fs::read_to_string(pings_file(&dir, now_epoch())).unwrap();
        assert_eq!(log.lines().count(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_client_from_before_this_change_is_still_counted() {
        let dir = temp("route-legacy");
        let addr = start(&dir, 100);
        assert!(post_body(addr, "tok", LEGACY_PING).starts_with("HTTP/1.1 200 OK"));
        assert!(post_body(addr, "tok", MODERN_PING).starts_with("HTTP/1.1 200 OK"));

        let data = dashboard(addr, "");
        assert_eq!(data["users"], 2);
        assert_eq!(data["online"], 2);
        assert_eq!(data["versions"]["labels"][0], "1.0.0");
        assert_eq!(data["versions"]["labels"][1], "1.1.2");

        let snapshot: serde_json::Value = {
            let mut idx = Index::load(&dir, now_epoch());
            idx.flush(&dir, true);
            serde_json::from_slice(&std::fs::read(index::snapshot_path(&dir)).unwrap()).unwrap()
        };
        assert_eq!(snapshot["devices"]["abc123"]["interval_secs"], 0);
        assert_eq!(snapshot["devices"]["def456"]["interval_secs"], 300);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_flood_is_refused_with_a_retry_hint() {
        let dir = temp("route-rate");
        let addr = start(&dir, 1);
        assert!(post_ping(addr, "tok").starts_with("HTTP/1.1 200 OK"));
        let reply = post_ping(addr, "tok");
        assert!(reply.starts_with("HTTP/1.1 429 "), "{reply}");
        assert!(reply.contains("Retry-After: 300\r\n"), "{reply}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn post_ticket(addr: SocketAddr, message: &str) -> String {
        let body = format!(
            r#"{{"feedback_id":"abc","message":"{message}","kind":"bug","version":"1.1.1"}}"#
        );
        let reply = talk(
            addr,
            &format!(
                "POST /report HTTP/1.1\r\nHost: a\r\nX-DeviceOut-Token: tok\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        );
        let json: serde_json::Value =
            serde_json::from_str(reply.split("\r\n\r\n").nth(1).unwrap_or_default())
                .unwrap_or_else(|e| panic!("{e}: {reply}"));
        json["ticket"].as_str().unwrap().to_string()
    }

    fn mark_handled(addr: SocketAddr, ticket: &str, handled: bool) -> String {
        let body = format!("ticket={ticket}&handled={}", u8::from(handled));
        talk(
            addr,
            &format!(
                "POST /deviceout-feedback/{ADMIN}/handle HTTP/1.1\r\nHost: a\r\nAuthorization: Basic {}\r\nContent-Length: {}\r\n\r\n{body}",
                b64("admin:pw"),
                body.len()
            ),
        )
    }

    #[test]
    fn handled_tickets_are_flagged_and_sink_to_the_bottom() {
        let dir = temp("route-handled");
        let addr = start(&dir, 100);
        let first = post_ticket(addr, "older");
        let second = post_ticket(addr, "newer");

        let rows = dashboard(addr, "")["rows"].clone();
        assert_eq!(rows[0]["ticket"], second);
        assert_eq!(rows[0]["handled"], false);

        let reply = mark_handled(addr, &second, true);
        assert!(reply.starts_with("HTTP/1.1 303 "), "{reply}");
        let rows = dashboard(addr, "")["rows"].clone();
        assert_eq!(rows[0]["ticket"], first);
        assert_eq!(rows[1]["ticket"], second);
        assert_eq!(rows[1]["handled"], true);

        assert!(mark_handled(addr, &second, false).starts_with("HTTP/1.1 303 "));
        assert_eq!(dashboard(addr, "")["rows"][0]["ticket"], second);

        let reply = talk(
            addr,
            &format!(
                "POST /deviceout-feedback/{ADMIN}/handle HTTP/1.1\r\nHost: a\r\nContent-Length: 0\r\n\r\n"
            ),
        );
        assert!(reply.starts_with("HTTP/1.1 401 "), "{reply}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_ping_without_the_token_is_turned_away() {
        let dir = temp("route-token");
        let addr = start(&dir, 100);
        let reply = post_ping(addr, "wrong");
        assert!(reply.starts_with("HTTP/1.1 401 "), "{reply}");
        assert!(!pings_file(&dir, now_epoch()).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
