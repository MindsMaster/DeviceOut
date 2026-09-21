use std::time::Duration;

use deviceout_update::{os_short, paths, telemetry};

use crate::logutil;

pub fn run(plugin_version: Option<&str>) {
    if !telemetry::is_enabled() {
        return;
    }
    let Some(telemetry_id) = telemetry::ensure_telemetry_id() else {
        return;
    };
    let version = plugin_version
        .filter(|s| !s.is_empty())
        .unwrap_or(env!("CARGO_PKG_VERSION"));
    let (locale, tz) = locale_and_tz();
    let body = serde_json::json!({
        "telemetry_id": telemetry_id,
        "version": version,
        "os": os_short(),
        "arch": std::env::consts::ARCH,
        "locale": locale,
        "tz": tz,
        "interval": telemetry::ping_interval(),
    });
    let Ok(encoded) = serde_json::to_vec(&body) else {
        return;
    };
    let url = format!("{}/ping", paths::BUILTIN_FEEDBACK_URL);
    let agent = crate::http::agent(Duration::from_secs(8));
    match agent
        .post(&url)
        .header("Content-Type", "application/json")
        .header("X-DeviceOut-Token", paths::BUILTIN_FEEDBACK_TOKEN)
        .send(&encoded)
    {
        Ok(mut resp) => telemetry::record_ping_ok(next_interval(&mut resp)),
        Err(e) => logutil::log(&format!("ping: {e}")),
    }
}

fn next_interval(resp: &mut ureq::http::Response<ureq::Body>) -> Option<i64> {
    let text = resp.body_mut().read_to_string().ok()?;
    let body: serde_json::Value = serde_json::from_str(&text).ok()?;
    body.get("next").and_then(|v| v.as_i64())
}

#[cfg(windows)]
fn locale_and_tz() -> (String, String) {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
    use windows_sys::Win32::System::Time::{
        GetDynamicTimeZoneInformation, DYNAMIC_TIME_ZONE_INFORMATION, TIME_ZONE_ID_INVALID,
    };

    let mut buf = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    let locale = if len > 0 {
        String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1).min(buf.len())])
    } else {
        String::new()
    };

    let mut info: DYNAMIC_TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
    let rc = unsafe { GetDynamicTimeZoneInformation(&mut info) };
    let tz = if rc != TIME_ZONE_ID_INVALID {
        let end = info
            .TimeZoneKeyName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.TimeZoneKeyName.len());
        String::from_utf16_lossy(&info.TimeZoneKeyName[..end])
    } else {
        String::new()
    };

    (locale, tz)
}

#[cfg(not(windows))]
fn locale_and_tz() -> (String, String) {
    (String::new(), String::new())
}
