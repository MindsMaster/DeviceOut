mod admin;
mod api;
mod assets;
mod config;
mod stats;
mod store;
mod util;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tiny_http::{Method, Response, Server, StatusCode};

use config::{env_nonempty, env_u32, load_dotenv, load_or_create_admin_path, Config, Limits};
use store::{count_lines, load_seen, pings_file, prune_old_pings};
use util::{normalize_path, now_epoch, secure};

const PING_PRUNE_INTERVAL: Duration = Duration::from_secs(3600);

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
    let server = Server::http(&bind).expect("bind");
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

    let delete_path = format!("/{}/delete", cfg.admin_path);
    for mut req in server.incoming_requests() {
        if cfg.kill || std::env::var_os("FEEDBACK_DISABLED").is_some() {
            let _ = req.respond(secure(
                Response::from_string("disabled").with_status_code(StatusCode(503)),
            ));
            continue;
        }
        {
            let mut st = state.lock().unwrap();
            if st.last_ping_prune.elapsed() > PING_PRUNE_INTERVAL {
                st.last_ping_prune = Instant::now();
                prune_old_pings(&cfg.dir, now_epoch());
            }
        }
        let method = req.method().clone();
        let path = normalize_path(req.url());
        let response = match method {
            Method::Get => admin::handle_get(&req, &path, &cfg, &state),
            Method::Post if path == "/ping" => api::handle_ping(&mut req, &cfg, &state),
            Method::Post if path == delete_path => admin::handle_admin_delete(&mut req, &cfg, &state),
            Method::Post => api::handle_post(&mut req, &cfg, &state),
            _ => Response::from_string("method").with_status_code(StatusCode(405)),
        };
        let _ = req.respond(secure(response));
    }
}
