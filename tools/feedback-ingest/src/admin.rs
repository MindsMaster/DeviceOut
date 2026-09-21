use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use crate::assets;
use crate::config::{Config, Hit, Limits};
use crate::http::{Request, Response};
use crate::index::Index;
use crate::stats::{compute_stats, pairs_json, Window};
use crate::store::{
    list_tickets, load_ticket, safe_ticket, set_handled, ticket_ms, valid_ticket_id, Stored,
};
use crate::util::{
    b64, client_ip, ct_eq, esc, header_value, html, json_ok, now_epoch, over_limit, parse_offset,
    prune, query_of, query_param, text, Resp,
};

const DELETE_MAX_BODY: usize = 4096;

pub fn handle_get(
    req: &Request,
    path: &str,
    cfg: &Config,
    state: &Mutex<Limits>,
    index: &Mutex<Index>,
) -> Resp {
    if path == "/" {
        return text(200, "ok");
    }
    let Some(rest) = path.strip_prefix(&format!("/{}", cfg.admin_path)) else {
        return text(404, "not found");
    };
    handle_admin(req, rest, cfg, state, index)
}

fn handle_admin(
    req: &Request,
    rest: &str,
    cfg: &Config,
    state: &Mutex<Limits>,
    index: &Mutex<Index>,
) -> Resp {
    if let Some(resp) = admin_gate(req, cfg, state) {
        return resp;
    }
    let rest = rest.trim_start_matches('/');
    let prefix = format!("/deviceout-feedback/{}", cfg.admin_path);
    let view = View::from_target(&req.target);
    if rest.is_empty() {
        return html(admin_index(cfg, index, &prefix, view));
    }
    if rest == "data" {
        return json_ok(&dashboard_json(cfg, index, view));
    }
    if !safe_ticket(rest) {
        return text(404, "not found");
    }
    match load_ticket(&cfg.dir, rest) {
        Some(item) => html(admin_detail(&item, &prefix)),
        None => text(404, "not found"),
    }
}

fn admin_gate(req: &Request, cfg: &Config, state: &Mutex<Limits>) -> Option<Resp> {
    let Some(password) = cfg.admin_password.as_deref() else {
        return Some(text(404, "not found"));
    };
    let ip = client_ip(req);
    {
        let mut st = state.lock().unwrap();
        prune(&mut st);
        if over_limit(&st.admin_fail_ip, &ip, cfg.admin_fail_hour) {
            return Some(text(429, "rate"));
        }
    }
    match admin_auth(req, password) {
        Auth::Ok => None,
        Auth::Missing => Some(challenge()),
        Auth::Wrong => {
            let mut st = state.lock().unwrap();
            prune(&mut st);
            st.admin_fail_ip
                .entry(ip)
                .or_default()
                .push(Hit { at: Instant::now() });
            Some(challenge())
        }
    }
}

fn challenge() -> Resp {
    text(401, "auth").header("WWW-Authenticate", "Basic realm=\"DeviceOut\"")
}

enum Auth {
    Ok,
    Missing,
    Wrong,
}

pub fn handle_admin_handle(req: &Request, cfg: &Config, state: &Mutex<Limits>) -> Resp {
    if let Some(resp) = admin_gate(req, cfg, state) {
        return resp;
    }
    if req.body.len() > DELETE_MAX_BODY {
        return text(413, "too large");
    }
    let body = String::from_utf8_lossy(&req.body);
    let ticket = form_field(&body, "ticket");
    if !valid_ticket_id(ticket) {
        return text(404, "not found");
    }
    let handled = form_field(&body, "handled") == "1";
    let prefix = format!("/deviceout-feedback/{}", cfg.admin_path);
    match set_handled(&cfg.dir, ticket, handled, now_epoch()) {
        Ok(true) => {
            eprintln!("ticket {ticket} handled={handled}");
            Response::empty(303).header("Location", &format!("{prefix}/"))
        }
        Ok(false) => text(404, "not found"),
        Err(e) => {
            eprintln!("handle ticket {ticket} error: {e}");
            text(500, "store")
        }
    }
}

fn form_field<'a>(body: &'a str, key: &str) -> &'a str {
    body.split('&')
        .find_map(|p| p.strip_prefix(&format!("{key}=")))
        .unwrap_or("")
        .trim()
}

pub fn handle_admin_delete(req: &Request, cfg: &Config, state: &Mutex<Limits>) -> Resp {
    if let Some(resp) = admin_gate(req, cfg, state) {
        return resp;
    }
    let prefix = format!("/deviceout-feedback/{}", cfg.admin_path);
    if req.body.len() > DELETE_MAX_BODY {
        return text(413, "too large");
    }
    let body = String::from_utf8_lossy(&req.body);
    let ticket = form_field(&body, "ticket");
    if !valid_ticket_id(ticket) {
        return text(404, "not found");
    }
    match std::fs::remove_file(cfg.dir.join(format!("{ticket}.json"))) {
        Ok(()) => {
            eprintln!("deleted ticket {ticket}");
            Response::empty(303).header("Location", &format!("{prefix}/"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => text(404, "not found"),
        Err(e) => {
            eprintln!("delete ticket {ticket} error: {e}");
            text(500, "store")
        }
    }
}

fn admin_auth(req: &Request, password: &str) -> Auth {
    let Some(header) = header_value(req, "Authorization") else {
        return Auth::Missing;
    };
    let expected = format!("Basic {}", b64(&format!("admin:{password}")));
    if ct_eq(&header, &expected) {
        Auth::Ok
    } else {
        Auth::Wrong
    }
}

fn kind_label(kind: &str) -> &'static str {
    if kind == "feature" {
        "建议"
    } else {
        "问题"
    }
}

fn kind_pill(kind: &str) -> String {
    let cls = if kind == "feature" { "feature" } else { "bug" };
    format!("<span class=\"pill {cls}\">{}</span>", kind_label(kind))
}

fn status_pill(handled: bool) -> &'static str {
    if handled {
        "<span class=\"pill done\">已处理</span>"
    } else {
        "<span class=\"pill todo\">待处理</span>"
    }
}

fn handle_form(prefix: &str, ticket: &str, handled: bool) -> String {
    format!(
        "<form class=\"del\" method=\"post\" action=\"{}/handle\">\
         <input type=\"hidden\" name=\"ticket\" value=\"{}\">\
         <input type=\"hidden\" name=\"handled\" value=\"{}\">\
         <button type=\"submit\" class=\"okbtn\">{}</button></form>",
        esc(prefix),
        esc(ticket),
        if handled { "0" } else { "1" },
        if handled { "撤销" } else { "处理" },
    )
}

fn delete_form(prefix: &str, ticket: &str) -> String {
    format!(
        "<form class=\"del\" method=\"post\" action=\"{}/delete\" \
         onsubmit=\"return confirm('删除？')\">\
         <input type=\"hidden\" name=\"ticket\" value=\"{}\">\
         <button type=\"submit\" class=\"delbtn\">删除</button></form>",
        esc(prefix),
        esc(ticket),
    )
}

fn ticket_rows(dir: &Path) -> Vec<serde_json::Value> {
    let mut items: Vec<Stored> = list_tickets(dir)
        .into_iter()
        .filter_map(|name| load_ticket(dir, &name))
        .collect();
    items.sort_by(|a, b| {
        a.handled_at
            .is_some()
            .cmp(&b.handled_at.is_some())
            .then_with(|| ticket_ms(&b.ticket).cmp(&ticket_ms(&a.ticket)))
    });
    items
        .into_iter()
        .map(|item| {
            let preview: String = item.message.chars().take(80).collect();
            serde_json::json!({
                "ticket": item.ticket,
                "kind": item.kind,
                "version": item.version,
                "ts": ticket_ms(&item.ticket),
                "ip": item.ip,
                "contact": item.contact.unwrap_or_default(),
                "message": item.message,
                "preview": preview,
                "handled": item.handled_at.is_some(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub struct View {
    pub offset_min: i32,
    pub window: Window,
}

impl View {
    fn from_target(target: &str) -> Self {
        let query = query_of(target);
        Self {
            offset_min: parse_offset(query_param(query, "tzoff")),
            window: Window::parse(query_param(query, "window")),
        }
    }
}

fn dashboard_json(cfg: &Config, index: &Mutex<Index>, view: View) -> String {
    let stats = {
        let idx = index.lock().unwrap();
        compute_stats(&idx, &cfg.dir, now_epoch(), view.offset_min, view.window)
    };
    serde_json::json!({
        "users": stats.total_users,
        "active": stats.active,
        "online": stats.online,
        "today": stats.today_active,
        "tickets": stats.total_tickets,
        "cohort": stats.cohort,
        "window": view.window.key(),
        "windowLabel": view.window.label(),
        "trend": {"labels": stats.trend_labels, "values": stats.trend_values},
        "versions": pairs_json(&stats.versions),
        "locales": pairs_json(&stats.locales),
        "regions": pairs_json(&stats.regions),
        "os": pairs_json(&stats.os),
        "rows": ticket_rows(&cfg.dir),
    })
    .to_string()
}

fn admin_index(cfg: &Config, index: &Mutex<Index>, prefix: &str, view: View) -> String {
    let data = dashboard_json(cfg, index, view);
    let stats: serde_json::Value = serde_json::from_str(&data).unwrap_or_else(|_| serde_json::json!({}));
    let num = |key: &str| stats.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    let users = num("users");
    let active = num("active");
    let online = num("online");
    let today = num("today");
    let tickets = num("tickets");
    let data = data.replace("</", "<\\/");

    let mut out = String::with_capacity(48 * 1024);
    out.push_str("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\">");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    out.push_str("<title>DeviceOut</title>");
    out.push_str("<script src=\"https://cdn.jsdelivr.net/npm/chart.js@4\"></script>");
    out.push_str("<style>");
    out.push_str(assets::css());
    out.push_str("</style></head><body><div class=\"wrap\">");
    out.push_str("<header class=\"top\"><h1>DeviceOut</h1><div class=\"tools\">");
    out.push_str("<select id=\"tz\" class=\"sel\"></select>");
    out.push_str("<select id=\"win\" class=\"sel\"></select>");
    out.push_str(&format!(
        "<div class=\"live\"><span class=\"dot\"></span><span id=\"n-live\">{online}</span></div>"
    ));
    out.push_str("</div></header>");
    out.push_str("<section class=\"cards\">");
    for (id, label, cls, value) in [
        ("n-users", "累计用户", "", users),
        ("n-active", "活跃 30 天", "", active),
        ("n-online", "在线", "green", online),
        ("n-today", "今日", "", today),
        ("n-tickets", "工单", "", tickets),
    ] {
        out.push_str(&format!(
            "<div class=\"card\"><div class=\"label\">{label}</div>\
             <div class=\"num {cls}\" id=\"{id}\">{value}</div></div>"
        ));
    }
    out.push_str("</section>");
    out.push_str(
        "<section class=\"card wide\"><h2>24 小时</h2>\
         <div class=\"chart tall\"><canvas id=\"trend\"></canvas></div></section>",
    );
    out.push_str("<section class=\"grid2\">");
    for (title, id) in [("版本", "ver"), ("系统", "os"), ("语言", "loc"), ("地区", "reg")] {
        out.push_str(&format!(
            "<div class=\"card\"><h2>{title}<span class=\"sub cohort\"></span></h2>\
             <div class=\"chart\"><canvas id=\"{id}\"></canvas></div></div>"
        ));
    }
    out.push_str("</section>");
    out.push_str(
        "<section class=\"card wide\"><h2>工单</h2>\
         <table><thead><tr><th>编号</th><th>状态</th><th>类型</th><th>版本</th><th>时间</th><th>IP</th><th>联系</th><th>摘要</th><th></th></tr></thead>\
         <tbody id=\"ticket-body\"></tbody></table></section></div>",
    );
    out.push_str("<script>const PREFIX = ");
    out.push_str(&serde_json::to_string(prefix).unwrap_or_else(|_| "\"\"".into()));
    out.push_str(";const DATA = ");
    out.push_str(&data);
    out.push_str(";</script><script>");
    out.push_str(assets::ts_js());
    out.push_str("</script><script>");
    out.push_str(assets::js());
    out.push_str("</script></body></html>");
    out
}

fn admin_detail(item: &Stored, prefix: &str) -> String {
    let version = if item.version.is_empty() {
        "-".to_string()
    } else {
        esc(&item.version)
    };
    let diag_section = if item.diag.trim().is_empty() {
        String::new()
    } else {
        format!(
            "<div class=\"card\"><h2>诊断</h2><pre class=\"block\">{}</pre></div>",
            esc(&item.diag)
        )
    };
    let mut out = String::with_capacity(16 * 1024);
    out.push_str("<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\">");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    out.push_str(&format!("<title>{} · DeviceOut</title>", esc(&item.ticket)));
    out.push_str("<style>");
    out.push_str(assets::css());
    out.push_str("</style></head><body><div class=\"wrap narrow stack\">");
    out.push_str(&format!(
        "<a class=\"back\" href=\"{}\">返回</a>",
        esc(prefix)
    ));
    out.push_str(&format!(
        "<div class=\"card\"><div class=\"dhead\"><h1 class=\"mono\">{}</h1>{}{}\
         <span style=\"margin-left:auto\" class=\"actions\">{}{}</span></div>\
         <div class=\"meta\">\
         <div><div class=\"k\">时间</div><div class=\"v ts\" data-ts=\"{}\">-</div></div>\
         <div><div class=\"k\">IP</div><div class=\"v mono\">{}</div></div>\
         <div><div class=\"k\">ID</div><div class=\"v mono\">{}</div></div>\
         <div><div class=\"k\">版本</div><div class=\"v mono\">{}</div></div>\
         <div><div class=\"k\">联系</div><div class=\"v\">{}</div></div>\
         </div></div>",
        esc(&item.ticket),
        kind_pill(&item.kind),
        status_pill(item.handled_at.is_some()),
        handle_form(prefix, &item.ticket, item.handled_at.is_some()),
        delete_form(prefix, &item.ticket),
        ticket_ms(&item.ticket),
        esc(&item.ip),
        esc(&item.feedback_id),
        version,
        esc(item.contact.as_deref().unwrap_or("-")),
    ));
    out.push_str(&format!(
        "<div class=\"card\"><h2>描述</h2><pre class=\"block\">{}</pre></div>",
        esc(&item.message)
    ));
    out.push_str(&diag_section);
    out.push_str("</div><script>");
    out.push_str(assets::ts_js());
    out.push_str("</script></body></html>");
    out
}
