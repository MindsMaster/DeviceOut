use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui};
use parking_lot::{Mutex, RwLock};

use deviceout_engine::{Fault, FaultKind};
use deviceout_i18n::{fill, t, Strings};
use deviceout_sink::{DeviceInfo, SampleFormat, StreamFormat};
use deviceout_update::outbox::FeedbackKind;

use crate::{
    enumerate_devices, DeviceOutParams, EngineController, UiState, DEFAULT_QUEUE_PERIODS,
    DEFAULT_TARGET_MS, MAX_TARGET_MS, QUEUE_PERIOD_STEPS, TARGET_MS_STEPS,
};
use deviceout_engine::ASSUMED_PERIOD_MS;

mod fonts;
#[cfg(test)]
mod tests;
mod theme;
mod widgets;
#[cfg(windows)]
mod win_prompt;

type SharedDevices = Arc<RwLock<Vec<DeviceInfo>>>;

const INSTALL_BUTTON_COOLDOWN: Duration = Duration::from_secs(30);
const UPDATE_VIEW_INTERVAL: Duration = Duration::from_secs(1);

const MS: &str = "ms";
const PPM: &str = "ppm";
const COMBO_WIDTH: f32 = 118.0;
const AT_TARGET_TOLERANCE: f64 = 0.03;
const MAX_ALERTS: usize = 1;
const TAB_TUNE: usize = 1;
const TAB_ABOUT: usize = 2;
const REPO_URL: &str = "https://github.com/MindsMaster/DeviceOut";
const AUTHOR_EMAIL: &str = "an5w1r@163.com";
const QQ_GROUP: &str = "1046048297";
const QQ_GROUP_URL: &str = "https://qun.qq.com/universal-share/share?ac=1&authKey=QEsKQUh0Z2dhNgn2lQDrnRRGD0MALQvTfF3FPr5ZYOv8i/lKdNp8G6gjTgYrD/Rw&busi_data=eyJncm91cENvZGUiOiIxMDQ2MDQ4Mjk3IiwidG9rZW4iOiJRR3lINzRUTUt0U2M0bDRkeTRhTERCT0ZscFlsTUxBNm9TdjUwelYxZXN2dzdUWUlyWnY0Mi9oOHI3c2JBOEYrIiwidWluIjoiMTE3MzM0ODYxMCJ9&data=EuNu7PewjDdMH0GF2GuEOYyIXT8iT25RJajcGVD3qL5LXkiATxSyzB-gdxqgbbFkbRvkyzoiJQ_sJ8-tBfFbSuW4NYFdjzdIr5Z1E8reAiM&svctype=5&tempid=h5_group_info";

pub(crate) struct Wiring {
    pub params: Arc<DeviceOutParams>,
    pub engine: Arc<EngineController>,
    pub devices: SharedDevices,
}

type PromptResult = Arc<Mutex<Option<Option<(String, String)>>>>;

struct PromptWait {
    kind: FeedbackKind,
    diag: Option<String>,
    done: PromptResult,
    #[cfg(windows)]
    window: win_prompt::PromptWindow,
    #[cfg(windows)]
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl Drop for PromptWait {
    fn drop(&mut self) {
        win_prompt::close_prompt(&self.window);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct EditorUi {
    update_view: Option<UpdateView>,
    install_started: Option<Instant>,
    check_busy_since: Option<Instant>,
    check_mark_attempt: Option<i64>,
    prompt_wait: Option<PromptWait>,
    telemetry_opt_in: bool,
    tab: usize,
    target_drag: Option<u32>,
    queue_drag: Option<u32>,
    send_watch: Option<(String, Instant)>,
    note: Option<(Instant, String, egui::Color32)>,
}

impl Default for EditorUi {
    fn default() -> Self {
        Self {
            update_view: None,
            install_started: None,
            check_busy_since: None,
            check_mark_attempt: None,
            prompt_wait: None,
            telemetry_opt_in: deviceout_update::telemetry::is_enabled(),
            tab: 0,
            target_drag: None,
            queue_drag: None,
            send_watch: None,
            note: None,
        }
    }
}

pub(crate) fn create(w: Wiring) -> Option<Box<dyn Editor>> {
    let editor_state = Arc::clone(&w.params.editor_state);
    let bundle = deviceout_update::loaded_bundle_path();

    create_egui_editor(
        editor_state,
        EditorUi::default(),
        |ctx, _| theme::install(ctx),
        move |ctx, _setter, state| {
            ctx.request_repaint_after(Duration::from_millis(100));

            let snap = snapshot(&w.engine);
            egui::CentralPanel::default()
                .frame(theme::root_frame())
                .show(ctx, |ui| {
                    body(ui, state, &w, bundle.as_ref(), snap.as_ref())
                });
        },
    )
}

fn body(
    ui: &mut egui::Ui,
    state: &mut EditorUi,
    w: &Wiring,
    bundle: Option<&PathBuf>,
    snap: Option<&UiState>,
) {
    header(ui, snap);
    ui.add_space(8.0);
    status_card(ui, snap);
    ui.add_space(8.0);

    let labels = [t().tab_run, t().tab_tune, t().tab_about];
    widgets::tab_bar(ui, &mut state.tab, &labels);
    ui.add_space(8.0);

    match state.tab {
        TAB_TUNE => tune_pane(ui, state, w, snap),
        TAB_ABOUT => about_pane(ui, state, bundle, snap, w),
        _ => run_pane(ui, w, snap),
    }

    if let Some((at, text, color)) = state.note.as_ref() {
        let keep = state.send_watch.is_some() || at.elapsed() < Duration::from_secs(4);
        if keep {
            widgets::toast(ui.ctx(), text, *color);
        }
    }
}

fn snapshot(engine: &EngineController) -> Option<UiState> {
    let m = engine.metrics()?;
    let stats = m.stats();

    Some(UiState {
        state: m.state(),
        error: m.last_error(),
        fill_fraction: m.fill_fraction(),
        target_fraction: m.target_fraction(),
        capacity_frames: m.capacity_frames(),
        target_frames: m.target_frames(),
        min_target_frames: m.min_target_frames(),
        exclusive: m.exclusive(),
        drift_ppm: m.drift_ppm_settled(),
        raw_drift_ppm: m.drift_ppm(),
        underruns: stats.underrun_events(),
        overruns: stats.overrun_events(),
        dropout_seconds: m.dropout_seconds(),
        clamp_events: m.clamp_events(),
        sink_rate_hz: m.sink_rate_hz(),
        period_frames: m.period_frames(),
        min_period_frames: m.min_period_frames(),
        latency_ms: m.latency_ms(),
        device_starvations: m.device_starvations(),
        reconnects: m.reconnects(),
        frames_discarded: m.frames_discarded(),
    })
}

fn header(ui: &mut egui::Ui, snap: Option<&UiState>) {
    ui.horizontal(|ui| {
        widgets::wordmark(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            widgets::status_indicator(ui, snap.map(|s| s.state));
            ui.add_space(4.0);

            if widgets::icon_button(
                ui,
                egui::include_image!("../../assets/icons/github.svg"),
                REPO_URL,
            )
            .clicked()
            {
                let _ = open::that_detached(REPO_URL);
            }
            if widgets::icon_button(
                ui,
                egui::include_image!("../../assets/icons/tencentqq.svg"),
                &format!("{} {QQ_GROUP}", t().qq_group),
            )
            .clicked()
            {
                let _ = open::that_detached(QQ_GROUP_URL);
            }
            if widgets::icon_button(
                ui,
                egui::include_image!("../../assets/icons/mail.svg"),
                AUTHOR_EMAIL,
            )
            .clicked()
            {
                let _ = open::that_detached(format!("mailto:{AUTHOR_EMAIL}"));
            }
        });
    });
}

fn device_section(ui: &mut egui::Ui, w: &Wiring, snap: Option<&UiState>) {
    ui.set_width(ui.available_width());

    {
        let current_id = w.params.device_id.read().clone();
        let format_text = w
            .devices
            .read()
            .iter()
            .find(|d| d.id == current_id)
            .map(|d| {
                egui::RichText::new(mix_format_text(&d.mix_format))
                    .font(egui::FontId::proportional(11.0))
                    .color(theme::TEXT_FAINT)
            });

        widgets::section_heading(ui, t().heading_output, format_text);
        ui.add_space(4.0);

        let current_name = w
            .devices
            .read()
            .iter()
            .find(|d| d.id == current_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| {
                if current_id.is_empty() {
                    t().no_device.to_string()
                } else {
                    t().saved_device_unavailable.to_string()
                }
            });

        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::refresh_button(ui).clicked() {
                    *w.devices.write() = enumerate_devices();
                }

                let mut chosen: Option<String> = None;
                widgets::device_picker(ui, &current_name, |ui| {
                    for device in w.devices.read().iter() {
                        let label = if device.is_default {
                            format!("{} · {}", device.name, t().system_default)
                        } else {
                            device.name.clone()
                        };
                        if widgets::device_option(ui, &label, device.id == current_id).clicked() {
                            chosen = Some(device.id.clone());
                            ui.memory_mut(|memory| memory.close_popup());
                        }
                    }
                });

                if let Some(id) = chosen {
                    if id != current_id {
                        *w.params.device_id.write() = id.clone();
                        w.engine.set_device_async(id);
                    }
                }
            });
        });

        if let Some(fault) = snap.and_then(|s| s.error.as_ref()) {
            ui.add_space(6.0);
            let message = fault_text(fault, t());
            let tooltip_width = ui.available_width();
            widgets::alert_line(ui, theme::RED, &message).on_hover_ui(|ui| {
                ui.set_max_width(tooltip_width);
                ui.add(egui::Label::new(&message).wrap());
            });
        }
    }
}

fn fault_text(fault: &Fault, t: &Strings) -> String {
    let base = match fault.kind {
        FaultKind::DeviceNotFound => t.fault_device_not_found.to_string(),
        FaultKind::DeviceLost => t.fault_device_lost.to_string(),
        FaultKind::ChannelMismatch { device, source } => fill(
            t.fault_channel_mismatch,
            &[
                ("device", &device.to_string()),
                ("source", &source.to_string()),
            ],
        ),
        FaultKind::UnsupportedFormat => t.fault_unsupported_format.to_string(),
        FaultKind::ComInit | FaultKind::Enumeration => t.fault_audio_system.to_string(),
        FaultKind::StreamInit => t.fault_stream_init.to_string(),
        FaultKind::Stream => t.fault_stream.to_string(),
        FaultKind::Resample => t.fault_resample.to_string(),
        FaultKind::Config | FaultKind::Thread => t.fault_engine.to_string(),
    };
    if fault.attempt > 0 {
        fill(
            t.fault_retry,
            &[("message", &base), ("attempt", &fault.attempt.to_string())],
        )
    } else {
        base
    }
}

fn status_card(ui: &mut egui::Ui, snap: Option<&UiState>) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        status_cells(ui, snap);
        ui.add_space(10.0);
        let (fill_fraction, target) =
            snap.map_or((0.0, 0.0), |s| (s.fill_fraction, s.target_fraction));
        widgets::progress_bar(ui, fill_fraction, target);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            widgets::heading_text(ui, t().heading_buffer);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let trailing = match snap {
                    None => "\u{2014}".to_string(),
                    Some(_) if (fill_fraction - target).abs() <= AT_TARGET_TOLERANCE => {
                        t().buffer_at_target.to_string()
                    }
                    Some(_) => format!("{:.0}%", fill_fraction * 100.0),
                };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(trailing)
                            .font(egui::FontId::monospace(11.0))
                            .color(theme::TEXT_DIM),
                    )
                    .truncate(),
                );
            });
        });
    });
}

fn status_cells(ui: &mut egui::Ui, snap: Option<&UiState>) {
    ui.columns(3, |cols| match snap {
        Some(s) => {
            widgets::stat_cell(
                &mut cols[0],
                t().heading_latency,
                &format!("{:.0}", s.latency_ms),
                MS,
                theme::TEXT,
                false,
            );
            let (drift, drift_color, settling) = match s.drift_ppm {
                Some(ppm) => (format!("{ppm:+.2}"), theme::TEXT, false),
                None => (format!("{:+.1}", s.raw_drift_ppm), theme::TEXT_DIM, true),
            };
            widgets::stat_cell(
                &mut cols[1],
                t().heading_drift,
                &drift,
                PPM,
                drift_color,
                settling,
            );
            let dropouts = dropout_count(s);
            let color = if dropouts == 0 {
                theme::TEXT
            } else {
                theme::AMBER
            };
            widgets::stat_cell(
                &mut cols[2],
                t().heading_dropouts,
                &dropouts.to_string(),
                "",
                color,
                false,
            );
        }
        None => {
            for (index, title) in [t().heading_latency, t().heading_drift, t().heading_dropouts]
                .into_iter()
                .enumerate()
            {
                widgets::stat_cell(
                    &mut cols[index],
                    title,
                    "\u{2014}",
                    "",
                    theme::TEXT_FAINT,
                    false,
                );
            }
        }
    });
}

fn dropout_count(s: &UiState) -> u64 {
    s.underruns + s.overruns + s.device_starvations
}

fn run_pane(ui: &mut egui::Ui, w: &Wiring, snap: Option<&UiState>) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        device_section(ui, w, snap);
        widgets::row_divider(ui);
        match snap.filter(|s| s.sink_rate_hz > 0.0) {
            Some(s) => stream_section(ui, s),
            None => idle_section(ui),
        }
    });
}

fn stream_section(ui: &mut egui::Ui, s: &UiState) {
    {
        widgets::kv_row(
            ui,
            t().heading_period,
            &format!("{:.1} {MS}", device_period_ms(Some(s))),
        );
        if let Some(ms) = min_period_ms(s) {
            widgets::kv_row(ui, t().heading_min_period, &format!("{ms:.1} {MS}"));
        }
        widgets::kv_row(
            ui,
            t().heading_mode,
            if s.exclusive {
                t().mode_exclusive
            } else {
                t().mode_shared
            },
        );
        let tooltip_width = ui.available_width();
        for alert in stream_alerts(s).into_iter().take(MAX_ALERTS) {
            ui.add_space(6.0);
            widgets::alert_line_compact(ui, theme::AMBER, &alert).on_hover_ui(|ui| {
                ui.set_max_width(tooltip_width);
                ui.add(egui::Label::new(&alert).wrap());
            });
        }
    }
}

fn stream_alerts(s: &UiState) -> Vec<String> {
    let t = t();
    let mut alerts = Vec::new();
    if dropout_count(s) > 0 {
        alerts.push(fill(
            t.alert_dropouts,
            &[
                ("underruns", &s.underruns.to_string()),
                ("overruns", &s.overruns.to_string()),
                ("seconds", &format!("{:.1}", s.dropout_seconds)),
            ],
        ));
    }
    if s.reconnects > 0 {
        let seconds = if s.sink_rate_hz > 0.0 {
            s.frames_discarded as f64 / s.sink_rate_hz
        } else {
            0.0
        };
        alerts.push(fill(
            t.alert_reconnects,
            &[
                ("count", &s.reconnects.to_string()),
                ("frames", &s.frames_discarded.to_string()),
                ("seconds", &format!("{seconds:.1}")),
            ],
        ));
    }
    if s.clamp_events > 0 {
        alerts.push(fill(
            t.alert_clamps,
            &[("count", &s.clamp_events.to_string())],
        ));
    }
    alerts
}

fn tune_pane(ui: &mut egui::Ui, state: &mut EditorUi, w: &Wiring, snap: Option<&UiState>) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        target_row(ui, state, w);
        widgets::row_divider(ui);
        queue_row(ui, state, w, snap);
        widgets::row_divider(ui);
        exclusive_row(ui, state, w, snap);
    });
    ui.add_space(8.0);
    if widgets::outline_button(ui, t().restore_defaults).clicked() {
        restore_defaults(state, w);
    }
}

fn restore_defaults(state: &mut EditorUi, w: &Wiring) {
    *w.params.queue_periods.write() = w.engine.request_queue_periods(DEFAULT_QUEUE_PERIODS);
    let applied = w.engine.request_target_ms(DEFAULT_TARGET_MS);
    *w.params.target_ms.write() = applied;
    w.engine.request_exclusive(false);
    *w.params.exclusive.write() = false;
    state.target_drag = None;
    state.queue_drag = None;
    if applied != DEFAULT_TARGET_MS {
        let text = fill(t().target_floor_hit, &[("ms", &applied.to_string())]);
        note(state, text, theme::AMBER);
    }
}

fn about_pane(
    ui: &mut egui::Ui,
    state: &mut EditorUi,
    bundle: Option<&PathBuf>,
    snap: Option<&UiState>,
    w: &Wiring,
) {
    poll_prompt(state);
    poll_send(state);
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        update_row(ui, state, bundle);
        widgets::row_divider(ui);
        language_row(ui);
        widgets::row_divider(ui);
        telemetry_row(ui, state);
    });
    ui.add_space(8.0);
    let line = device_line(w);
    feedback_row(ui, state, bundle.map(|p| p.as_path()), snap, &line);
}

fn language_row(ui: &mut egui::Ui) {
    widgets::settings_row(ui, t().language, |ui| {
        if let Some(pref) = widgets::language_menu(ui) {
            deviceout_i18n::set_preference(pref);
            theme::install_fonts(ui.ctx(), deviceout_i18n::current().script());
        }
    });
}

fn telemetry_row(ui: &mut egui::Ui, state: &mut EditorUi) {
    let mut on = state.telemetry_opt_in;
    let mut changed = false;
    widgets::settings_row(ui, t().usage_stats, |ui| {
        changed = widgets::switch(ui, &mut on).changed();
    });
    if changed {
        state.telemetry_opt_in = on;
        deviceout_update::telemetry::set_enabled(on);
    }
}

fn device_line(w: &Wiring) -> String {
    let id = w.params.device_id.read().clone();
    w.devices
        .read()
        .iter()
        .find(|d| d.id == id)
        .map(|d| format!("{} | {}", d.name, d.mix_format))
        .unwrap_or_else(|| if id.is_empty() { "none".into() } else { id })
}

fn note(state: &mut EditorUi, text: String, color: egui::Color32) {
    state.note = Some((Instant::now(), text, color));
}

fn target_row(ui: &mut egui::Ui, state: &mut EditorUi, w: &Wiring) {
    let floor = w.engine.target_floor_ms();
    let committed = w.engine.target_ms();
    if committed < floor {
        let raised = w.engine.request_target_ms(committed);
        *w.params.target_ms.write() = raised;
        let text = fill(t().target_floor_hit, &[("ms", &raised.to_string())]);
        note(state, text, theme::AMBER);
    }

    let mut picked = None;
    widgets::settings_row(ui, t().heading_target_latency, |ui| {
        picked = widgets::number_combo(
            ui,
            "target_ms",
            widgets::NumberCombo {
                width: COMBO_WIDTH,
                value: w.engine.target_ms(),
                floor,
                ceiling: MAX_TARGET_MS,
                steps: TARGET_MS_STEPS,
                unit: MS,
                blocked: t().step_unavailable,
                scrub: true,
            },
            &mut state.target_drag,
        );
    });
    if let Some(wanted) = picked {
        let applied = w.engine.request_target_ms(wanted);
        *w.params.target_ms.write() = applied;
        if applied != wanted {
            let text = fill(t().target_floor_hit, &[("ms", &applied.to_string())]);
            note(state, text, theme::AMBER);
        }
    }
}

fn queue_row(ui: &mut egui::Ui, state: &mut EditorUi, w: &Wiring, snap: Option<&UiState>) {
    let period_ms = device_period_ms(snap);
    let steps: Vec<u32> = QUEUE_PERIOD_STEPS
        .iter()
        .map(|&n| queue_ms(n, period_ms))
        .collect();

    let mut picked = None;
    widgets::settings_row(ui, t().heading_device_buffer, |ui| {
        picked = widgets::number_combo(
            ui,
            "queue_ms",
            widgets::NumberCombo {
                width: COMBO_WIDTH,
                value: queue_ms(w.engine.queue_periods(), period_ms),
                floor: 0,
                ceiling: u32::MAX,
                steps: &steps,
                unit: MS,
                blocked: t().step_unavailable,
                scrub: false,
            },
            &mut state.queue_drag,
        );
    });
    if let Some(ms) = picked {
        let periods = (f64::from(ms) / period_ms).round() as u32;
        *w.params.queue_periods.write() = w.engine.request_queue_periods(periods);
    }
}

fn exclusive_row(ui: &mut egui::Ui, state: &mut EditorUi, w: &Wiring, snap: Option<&UiState>) {
    let running = snap.filter(|s| s.sink_rate_hz > 0.0).map(|s| s.exclusive);
    let wanted = w.engine.exclusive();
    if wanted && running == Some(false) {
        w.engine.clear_exclusive();
        *w.params.exclusive.write() = false;
        note(state, t().exclusive_fell_back.into(), theme::AMBER);
    }

    let mut on = running.unwrap_or(wanted);
    let mut changed = false;
    widgets::settings_row(ui, t().heading_exclusive, |ui| {
        changed = widgets::switch(ui, &mut on).changed();
    });
    if changed {
        w.engine.request_exclusive(on);
        *w.params.exclusive.write() = on;
    }
}

fn min_period_ms(s: &UiState) -> Option<f64> {
    if s.min_period_frames == 0 || s.sink_rate_hz <= 0.0 {
        return None;
    }
    Some(s.min_period_frames as f64 * 1.0e3 / s.sink_rate_hz)
}

fn device_period_ms(snap: Option<&UiState>) -> f64 {
    snap.map(|s| s.period_frames as f64 * 1.0e3 / s.sink_rate_hz)
        .filter(|ms| ms.is_finite() && *ms > 0.0)
        .unwrap_or(ASSUMED_PERIOD_MS)
}

fn queue_ms(periods: u32, period_ms: f64) -> u32 {
    (f64::from(periods) * period_ms).round().max(1.0) as u32
}

fn mix_format_text(fmt: &StreamFormat) -> String {
    let t = t();
    let sample = match fmt.sample_format {
        SampleFormat::F32 => t.sample_f32,
        SampleFormat::I16 => t.sample_i16,
        SampleFormat::I32 => t.sample_i32,
    };
    fill(
        t.mix_format,
        &[
            ("rate", &fmt.sample_rate.to_string()),
            ("channels", &fmt.channels.to_string()),
            ("sample", sample),
        ],
    )
}

fn idle_section(ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            egui::RichText::new(t().engine_idle)
                .size(13.0)
                .color(theme::TEXT_DIM),
        );
        ui.add_space(12.0);
    });
}

struct UpdateView {
    pending: Option<deviceout_update::PendingManifest>,
    state: deviceout_update::State,
    read_at: Instant,
}

impl UpdateView {
    fn read() -> Self {
        Self {
            pending: deviceout_update::pending_ready(),
            state: deviceout_update::load_state(),
            read_at: Instant::now(),
        }
    }
}

fn refresh_update_view(state: &mut EditorUi) {
    let stale = state
        .update_view
        .as_ref()
        .is_none_or(|v| v.read_at.elapsed() >= UPDATE_VIEW_INTERVAL);
    if stale {
        state.update_view = Some(UpdateView::read());
    }
}

fn update_row(ui: &mut egui::Ui, state: &mut EditorUi, bundle: Option<&PathBuf>) {
    let current = env!("CARGO_PKG_VERSION");
    refresh_update_view(state);
    let view = state.update_view.take().expect("refreshed above");
    let pending = view.pending.clone();
    let st = view.state.clone();
    state.update_view = Some(view);
    poll_check_busy(state, pending.is_some(), &st);
    if pending.is_none()
        || state
            .install_started
            .is_some_and(|t| t.elapsed() >= INSTALL_BUTTON_COOLDOWN)
    {
        state.install_started = None;
    }

    let (status, color, tip) = update_status_text(
        current,
        pending.as_ref(),
        &st,
        state.check_busy_since.is_some(),
        t(),
    );

    let tooltip_width = ui.available_width();
    ui.horizontal(|ui| {
        ui.set_min_height(44.0);
        ui.spacing_mut().item_spacing.x = 8.0;
        let checking = state.check_busy_since.is_some();
        let action = if checking {
            t().checking
        } else {
            t().check_updates
        };
        let action_w = button_width(ui, action);
        let install_w = if pending.is_some() {
            button_width(ui, t().install) + 8.0
        } else {
            0.0
        };
        let left_w = (ui.available_width() - action_w - install_w).max(48.0);

        ui.allocate_ui_with_layout(
            egui::Vec2::new(left_w, 30.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(left_w);
                ui.spacing_mut().item_spacing.x = 6.0;
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::new(10.0, 14.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 3.0, color);
                let label = ui.add(
                    egui::Label::new(
                        egui::RichText::new(&status)
                            .size(11.5)
                            .color(theme::TEXT_DIM),
                    )
                    .truncate(),
                );
                if let Some(tip) = tip {
                    label.on_hover_ui(|ui| {
                        ui.set_max_width(tooltip_width);
                        ui.add(egui::Label::new(tip).wrap());
                    });
                }
            },
        );

        ui.add_enabled_ui(!checking, |ui| {
            if widgets::outline_button(ui, action).clicked() {
                state.check_mark_attempt = st.last_attempt;
                state.check_busy_since = Some(Instant::now());
                state.update_view = None;
                spawn_check_now(bundle);
            }
        });
        if pending.is_some() {
            ui.add_enabled_ui(state.install_started.is_none(), |ui| {
                if widgets::primary_button(ui, t().install).clicked() {
                    state.install_started = Some(Instant::now());
                    state.update_view = None;
                    deviceout_update::spawn_updater(&[OsStr::new("--apply-pending")]);
                }
            });
        }
    });
}

fn button_width(ui: &egui::Ui, label: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            label.to_string(),
            egui::FontId::proportional(12.5),
            theme::TEXT,
        )
        .size()
        .x
        + 24.0
}

fn spawn_check_now(bundle: Option<&PathBuf>) {
    if let Some(path) = bundle {
        deviceout_update::spawn_updater(&[path.as_os_str(), OsStr::new("--check-now")]);
    }
}

fn poll_check_busy(state: &mut EditorUi, has_pending: bool, st: &deviceout_update::State) {
    let Some(started) = state.check_busy_since else {
        return;
    };
    let elapsed = started.elapsed();
    let ran = st.last_attempt.is_some() && st.last_attempt != state.check_mark_attempt;
    let finished = has_pending
        || (ran && st.last_error.is_some())
        || (ran
            && st
                .last_check
                .zip(st.last_attempt)
                .is_some_and(|(check, attempt)| check >= attempt));
    if (finished && elapsed >= Duration::from_millis(400)) || elapsed >= Duration::from_secs(60) {
        state.check_busy_since = None;
    }
}

fn update_status_text(
    current: &str,
    pending: Option<&deviceout_update::PendingManifest>,
    st: &deviceout_update::State,
    busy: bool,
    t: &Strings,
) -> (String, egui::Color32, Option<String>) {
    if busy {
        return (t.checking_status.into(), theme::TEXT_DIM, None);
    }
    if let Some(pending) = pending {
        return (
            fill(t.update_ready, &[("version", &pending.version)]),
            theme::AMBER,
            Some(t.update_ready_tip.into()),
        );
    }
    if st.last_install_error.is_some() {
        return (
            t.install_failed.into(),
            theme::RED,
            Some(t.install_failed_tip.into()),
        );
    }
    if st.last_error.is_some() {
        return (
            t.check_failed.into(),
            theme::RED,
            Some(t.check_failed_tip.into()),
        );
    }
    if let Some(latest) = st.last_latest.as_deref() {
        match deviceout_update::cmp_latest(latest, current) {
            deviceout_update::Cmp::Newer => {
                return (
                    fill(t.update_available, &[("version", latest)]),
                    theme::AMBER,
                    None,
                );
            }
            deviceout_update::Cmp::EqualOrOlder => {
                return (
                    fill(t.up_to_date, &[("version", current)]),
                    theme::TEXT_DIM,
                    None,
                );
            }
            deviceout_update::Cmp::Invalid => {}
        }
    }
    (format!("v{current}"), theme::TEXT_DIM, None)
}

fn feedback_row(
    ui: &mut egui::Ui,
    state: &mut EditorUi,
    bundle: Option<&Path>,
    snap: Option<&UiState>,
    device_line: &str,
) {
    let waiting = state.prompt_wait.is_some();
    ui.add_enabled_ui(!waiting, |ui| {
        ui.columns(2, |cols| {
            if widgets::outline_button_fill(&mut cols[0], t().report_bug).clicked() {
                open_feedback(state, FeedbackKind::Bug, bundle, snap, device_line);
            }
            if widgets::outline_button_fill(&mut cols[1], t().feature_request).clicked() {
                open_feedback(state, FeedbackKind::Feature, bundle, snap, device_line);
            }
        });
    });
}

fn open_feedback(
    state: &mut EditorUi,
    kind: FeedbackKind,
    bundle: Option<&Path>,
    snap: Option<&UiState>,
    device_line: &str,
) {
    #[cfg(windows)]
    {
        let diag = match kind {
            FeedbackKind::Bug => Some(compose_diag(bundle, snap, device_line)),
            FeedbackKind::Feature => None,
        };
        let done = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&done);
        let window: win_prompt::PromptWindow = Arc::default();
        let window_for_thread = Arc::clone(&window);
        let is_bug = kind == FeedbackKind::Bug;
        let thread = std::thread::Builder::new()
            .name("deviceout-feedback".into())
            .spawn(move || {
                let result = win_prompt::run_feedback(is_bug, &window_for_thread);
                *slot.lock() = Some(result);
            });
        let Ok(thread) = thread else {
            state.note = Some((
                Instant::now(),
                t().feedback_window_failed.into(),
                theme::RED,
            ));
            return;
        };
        state.prompt_wait = Some(PromptWait {
            kind,
            diag,
            done,
            window,
            thread: Some(thread),
        });
    }
    #[cfg(not(windows))]
    {
        let _ = (state, kind, bundle, snap, device_line);
    }
}

fn poll_prompt(state: &mut EditorUi) {
    let Some(wait) = state.prompt_wait.as_ref() else {
        return;
    };
    let Some(result) = wait.done.lock().take() else {
        return;
    };
    let mut wait = state.prompt_wait.take().expect("checked above");
    let Some((message, contact)) = result else {
        return;
    };
    let message = message.trim();
    if message.is_empty() {
        return;
    }
    match queue_feedback(wait.kind, message, &contact, wait.diag.take()) {
        Ok(id) => {
            deviceout_update::spawn_updater(&[OsStr::new("--send-outbox")]);
            state.send_watch = Some((id, Instant::now()));
            state.note = Some((Instant::now(), t().sending.into(), theme::TEXT_DIM));
        }
        Err(msg) => {
            state.send_watch = None;
            state.note = Some((Instant::now(), msg, theme::RED));
        }
    }
}

fn poll_send(state: &mut EditorUi) {
    let Some((id, since)) = state.send_watch.as_ref() else {
        return;
    };
    let id = id.clone();
    let elapsed = since.elapsed();
    if outbox_file(&deviceout_update::sent_dir(), &id).is_file() {
        state.send_watch = None;
        state.note = Some((Instant::now(), t().sent.into(), theme::GREEN));
        return;
    }
    if outbox_file(&deviceout_update::failed_dir(), &id).is_file() {
        state.send_watch = None;
        state.note = Some((Instant::now(), t().send_failed.into(), theme::RED));
        return;
    }
    if elapsed >= Duration::from_secs(25) {
        state.send_watch = None;
        state.note = Some((Instant::now(), t().saved_will_retry.into(), theme::AMBER));
    }
}

fn outbox_file(dir: &std::path::Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

fn session_diag(snap: Option<&UiState>, device_line: &str) -> String {
    let Some(s) = snap else {
        return format!("engine=idle\noutput_device={device_line}");
    };
    let drift = s
        .drift_ppm
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| format!("raw {:.1}", s.raw_drift_ppm));
    format!(
        "output_device={device_line}\nengine_state={:?}\nfill={:.1}%\nunderruns={}\noverruns={}\ndevice_starvations={}\ndropout_s={:.2}\nreconnects={}\nframes_discarded={}\nclamp={}\nlatency_ms={:.1}\ndrift_ppm={}\nring_frames={}\ntarget_frames={:.0}\nmin_target_frames={}\nexclusive={}\nperiod_frames={}\nsink_hz={:.0}\nengine_error={}",
        s.state,
        s.fill_fraction * 100.0,
        s.underruns,
        s.overruns,
        s.device_starvations,
        s.dropout_seconds,
        s.reconnects,
        s.frames_discarded,
        s.clamp_events,
        s.latency_ms,
        drift,
        s.capacity_frames,
        s.target_frames,
        s.min_target_frames,
        s.exclusive,
        s.period_frames,
        s.sink_rate_hz,
        deviceout_update::sanitize_user_paths(
            s.error.as_ref().map_or("none", |fault| fault.detail.as_str())
        ),
    )
}

fn compose_diag(bundle: Option<&Path>, snap: Option<&UiState>, device_line: &str) -> String {
    let base = deviceout_update::build_diagnostics(bundle, env!("CARGO_PKG_VERSION"));
    format!(
        "{base}\n--- session ---\n{}",
        session_diag(snap, device_line)
    )
}

fn queue_feedback(
    kind: FeedbackKind,
    message: &str,
    contact: &str,
    diag: Option<String>,
) -> Result<String, String> {
    let pending = deviceout_update::list_json(&deviceout_update::outbox_dir()).len();
    if pending >= deviceout_update::OUTBOX_CAP {
        return Err(t().feedback_queue_full.into());
    }
    let contact = Some(contact.trim().to_string()).filter(|c| !c.is_empty());
    let item = deviceout_update::new_outbox_item(
        kind,
        message.to_string(),
        contact,
        diag,
        env!("CARGO_PKG_VERSION").to_string(),
    )
    .map_err(|e| format!("{}{e}", t().feedback_save_failed))?;
    deviceout_update::save_item(&deviceout_update::outbox_dir(), &item)
        .map_err(|e| format!("{}{e}", t().feedback_save_failed))?;
    Ok(item.id)
}
