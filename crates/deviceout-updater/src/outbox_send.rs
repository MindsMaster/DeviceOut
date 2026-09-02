use std::time::Duration;

use anyhow::Result;
use serde::Deserialize;

use deviceout_update::outbox::{self, MAX_ATTEMPTS};
use deviceout_update::paths;

use crate::logutil;

pub fn send_all() -> Result<()> {
    outbox::cleanup_capped_dir(&paths::sent_dir(), deviceout_update::SENT_CAP);
    outbox::cleanup_capped_dir(&paths::failed_dir(), deviceout_update::FAILED_CAP);

    let files = outbox::list_json(&paths::outbox_dir());
    for path in files {
        let Some(mut item) = outbox::load_item(&path) else {
            continue;
        };
        if let Some(last) = item.last_attempt_ms {
            let wait = backoff_ms(item.attempts);
            let now = now_ms();
            if now.saturating_sub(last) < wait {
                continue;
            }
        }
        item.attempts = item.attempts.saturating_add(1);
        item.last_attempt_ms = Some(now_ms());
        match post_item(&item) {
            Ok(ticket) => {
                item.ticket = Some(ticket);
                item.error = None;
                let _ = outbox::move_item(&path, &paths::sent_dir(), &item);
                outbox::cleanup_capped_dir(&paths::sent_dir(), deviceout_update::SENT_CAP);
                logutil::log(&format!("sent {} ticket={}", item.id, item.ticket.as_deref().unwrap_or("-")));
            }
            Err(SendErr::Permanent(msg)) => {
                item.error = Some(msg);
                let _ = outbox::move_item(&path, &paths::failed_dir(), &item);
                outbox::cleanup_capped_dir(&paths::failed_dir(), deviceout_update::FAILED_CAP);
                logutil::log(&format!("permanent fail {}: {}", item.id, item.error.as_deref().unwrap_or("-")));
            }
            Err(SendErr::Retry(msg)) => {
                item.error = Some(msg);
                if item.attempts >= MAX_ATTEMPTS {
                    let _ = outbox::move_item(&path, &paths::failed_dir(), &item);
                    outbox::cleanup_capped_dir(&paths::failed_dir(), deviceout_update::FAILED_CAP);
                    logutil::log(&format!("retry exhausted {}", item.id));
                } else {
                    let _ = outbox::save_item(&paths::outbox_dir(), &item);
                    logutil::log(&format!("retry {} attempt {}", item.id, item.attempts));
                }
            }
        }
    }
    Ok(())
}

enum SendErr {
    Permanent(String),
    Retry(String),
}

#[derive(Deserialize)]
struct Reply {
    #[serde(default)]
    ticket: Option<String>,
}

fn post_item(item: &deviceout_update::OutboxItem) -> Result<String, SendErr> {
    let url = std::env::var("DEVICEOUT_FEEDBACK_URL")
        .ok()
        .filter(|s| s.starts_with("https://"))
        .unwrap_or_else(|| paths::BUILTIN_FEEDBACK_URL.to_string());

    let mut body = serde_json::json!({
        "feedback_id": item.feedback_id,
        "kind": item.kind,
        "message": item.message,
        "contact": item.contact,
        "version": item.version,
    });
    if let (Some(map), Some(diag)) = (body.as_object_mut(), &item.diag) {
        map.insert("diag".to_string(), serde_json::Value::String(diag.clone()));
    }
    let encoded = serde_json::to_vec(&body).map_err(|e| SendErr::Permanent(e.to_string()))?;
    let agent = crate::http::agent_lenient(Duration::from_secs(20));
    let mut resp = agent
        .post(&url)
        .header("Content-Type", "application/json")
        .header("X-DeviceOut-Token", paths::BUILTIN_FEEDBACK_TOKEN)
        .send(&encoded)
        .map_err(|e| SendErr::Retry(e.to_string()))?;
    let code = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    if (200..300).contains(&code) {
        let ticket = serde_json::from_str::<Reply>(&text)
            .ok()
            .and_then(|r| r.ticket)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("DO-{}", &item.id.chars().rev().take(8).collect::<String>()));
        return Ok(ticket);
    }
    let msg = short_http_error(code, &text);
    if is_transient(code) {
        Err(SendErr::Retry(msg))
    } else {
        Err(SendErr::Permanent(msg))
    }
}

fn is_transient(code: u16) -> bool {
    matches!(code, 404 | 405 | 408 | 425 | 429) || !(400..500).contains(&code)
}

fn short_http_error(code: u16, body: &str) -> String {
    let lower = body.to_ascii_lowercase();
    if lower.contains("<html") || lower.contains("<!doctype") || lower.contains("nexus") {
        if code == 404 || code == 405 {
            return format!("{code} 反馈接收端未部署");
        }
        return code.to_string();
    }
    let snippet: String = body.chars().filter(|c| !c.is_control()).take(80).collect();
    if snippet.is_empty() {
        code.to_string()
    } else {
        format!("{code} {snippet}")
    }
}

fn backoff_ms(attempts: u32) -> u64 {
    let mins = 1u64 << attempts.min(6);
    mins.saturating_mul(60_000)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limits_and_server_errors_are_retried() {
        for code in [408, 425, 429, 500, 502, 503, 504] {
            assert!(is_transient(code), "{code}");
        }
    }

    #[test]
    fn undeployed_endpoint_is_retried() {
        assert!(is_transient(404));
        assert!(is_transient(405));
    }

    #[test]
    fn client_faults_are_permanent() {
        for code in [400, 401, 403, 413, 422] {
            assert!(!is_transient(code), "{code}");
        }
    }

    #[test]
    fn backoff_caps_at_64_minutes() {
        assert_eq!(backoff_ms(0), 60_000);
        assert_eq!(backoff_ms(3), 8 * 60_000);
        assert_eq!(backoff_ms(6), 64 * 60_000);
        assert_eq!(backoff_ms(40), 64 * 60_000);
    }
}
