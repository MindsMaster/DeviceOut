mod admin;
mod api;
mod assets;
mod config;
mod http;
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
use store::{count_lines, load_seen, pings_file, prune_old_pings};
use util::{normalize_path, now_epoch, secure, text};

const PING_PRUNE_INTERVAL: Duration = Duration::from_secs(3600);
const WORKERS: usize = 8;

fn main() {
    load_dotenv();
    assets::warmup();
    let bind = std::env::var("FEEDBACK_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    if !bind.starts_with("127.0.0.1") && !bind.starts_with("localhost") && bind != "::1" {
        eprintln!("warning: bind {bind} is not loopback; put this behind Caddy and do not expose the port");
    }
    let dir = PathBuf::from(std::env::var("FEEDBACK_DIR").unwrap_or_else(|_| "feedback-inbox".into()));
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
        ping_rate_ip_hour: env_u32("FEEDBACK_PING_RATE_IP_HOUR", 30),
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
    let seen = load_seen(&cfg.dir);
    let today_pings = count_lines(&pings_file(&cfg.dir, now));
    eprintln!(
        "stats: {} known telemetry ids, {} pings today",
        seen.len(),
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
        ping_last: HashMap::new(),
        last_ping_prune: Instant::now(),
    }));

    let cfg = Arc::new(cfg);
    let handler: Handler = Arc::new(move |req: Request| route(&req, &cfg, &state));
    let limits = http::Limits {
        body_bytes: api::MAX_BODY,
        ..http::Limits::default()
    };
    if let Err(e) = http::serve(listener, WORKERS, limits, handler) {
        eprintln!("server stopped: {e}");
    }
}

fn route(req: &Request, cfg: &Config, state: &Mutex<Limits>) -> http::Response {
    if cfg.kill {
        return secure(text(503, "disabled"));
    }
    {
        let mut st = state.lock().unwrap();
        if st.last_ping_prune.elapsed() > PING_PRUNE_INTERVAL {
            st.last_ping_prune = Instant::now();
            prune_old_pings(&cfg.dir, now_epoch());
        }
    }
    let path = normalize_path(&req.target);
    let delete_path = format!("/{}/delete", cfg.admin_path);
    let response = match req.method {
        Method::Get | Method::Head => admin::handle_get(req, &path, cfg, state),
        Method::Post if path == "/ping" => api::handle_ping(req, cfg, state),
        Method::Post if path == delete_path => admin::handle_admin_delete(req, cfg, state),
        Method::Post => api::handle_post(req, cfg, state),
        Method::Other => text(405, "method"),
    };
    secure(response)
}
