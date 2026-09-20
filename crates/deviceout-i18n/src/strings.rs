#[derive(Debug)]
pub struct Strings {
    pub language: &'static str,
    pub follow_system: &'static str,
    pub qq_group: &'static str,

    pub no_device: &'static str,
    pub saved_device_unavailable: &'static str,
    pub system_default: &'static str,
    pub refresh_devices: &'static str,

    pub heading_output: &'static str,
    pub heading_buffer: &'static str,
    pub heading_latency: &'static str,
    pub heading_drift: &'static str,
    pub heading_format: &'static str,
    pub heading_buffer_size: &'static str,
    pub mix_format: &'static str,
    pub sample_f32: &'static str,
    pub sample_i16: &'static str,
    pub sample_i32: &'static str,

    pub state_running: &'static str,
    pub state_priming: &'static str,
    pub state_stopped: &'static str,
    pub state_error: &'static str,
    pub state_reconnecting: &'static str,
    pub state_idle: &'static str,
    pub engine_idle: &'static str,

    pub fault_device_not_found: &'static str,
    pub fault_device_lost: &'static str,
    pub fault_channel_mismatch: &'static str,
    pub fault_unsupported_format: &'static str,
    pub fault_audio_system: &'static str,
    pub fault_stream_init: &'static str,
    pub fault_stream: &'static str,
    pub fault_resample: &'static str,
    pub fault_engine: &'static str,
    pub fault_retry: &'static str,

    pub alert_dropouts: &'static str,
    pub alert_reconnects: &'static str,
    pub alert_clamps: &'static str,

    pub install: &'static str,
    pub check_updates: &'static str,
    pub checking: &'static str,
    pub checking_status: &'static str,
    pub update_ready: &'static str,
    pub update_ready_tip: &'static str,
    pub install_failed: &'static str,
    pub install_failed_tip: &'static str,
    pub check_failed: &'static str,
    pub check_failed_tip: &'static str,
    pub update_available: &'static str,
    pub up_to_date: &'static str,

    pub report_bug: &'static str,
    pub feature_request: &'static str,
    pub usage_stats: &'static str,
    pub feedback_window_failed: &'static str,
    pub sending: &'static str,
    pub sent: &'static str,
    pub send_failed: &'static str,
    pub saved_will_retry: &'static str,
    pub feedback_queue_full: &'static str,
    pub feedback_save_failed: &'static str,

    pub prompt_bug_title: &'static str,
    pub prompt_feature_title: &'static str,
    pub prompt_describe_bug: &'static str,
    pub prompt_describe_feature: &'static str,
    pub prompt_contact: &'static str,
    pub prompt_contact_hint: &'static str,
    pub prompt_submit: &'static str,
    pub prompt_cancel: &'static str,

    pub updater_title: &'static str,
    pub updater_updated: &'static str,
    pub updater_close_host: &'static str,
    pub updater_failed: &'static str,
}

pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in args {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_replaces_named_placeholders_only() {
        assert_eq!(
            fill("v{version} ready {version}", &[("version", "1.2.3")]),
            "v1.2.3 ready 1.2.3"
        );
        assert_eq!(fill("{a} {b}", &[("a", "x")]), "x {b}");
        assert_eq!(fill("plain", &[("a", "x")]), "plain");
    }
}
