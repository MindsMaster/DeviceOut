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
    enumerate_devices, DeviceOutParams, EngineController, UiState, QUEUE_PERIOD_STEPS,
    TARGET_MS_STEPS,
};

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
    target_drag: Option<u32>,
    queue_drag: Option<u32>,
    send_watch: Option<(String, Instant)>,
    send_note: Option<(Instant, String, egui::Color32)>,
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
            target_drag: None,
            queue_drag: None,
            send_watch: None,
            send_note: None,
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

            egui::CentralPanel::default()
                .frame(theme::root_frame())
                .show(ctx, |ui| {
                    let height = ui.available_height();
                    egui::ScrollArea::vertical()
                        .id_salt("deviceout_root")
                        .auto_shrink([false, false])
                        .max_height(height)
                        .min_scrolled_height(height)
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                        )
                        .show(ui, |ui| {
                            egui::Frame::new()
                                .inner_margin(egui::Margin {
                                    right: 12,
                                    ..egui::Margin::ZERO
                                })
                                .show(ui, |ui| {
                                    let snap = snapshot(&w.engine);

                                    header(ui, snap.as_ref());
                                    ui.add_space(12.0);
                                    device_card(ui, &w, snap.as_ref());
                                    ui.add_space(12.0);

                                    match &snap {
                                        Some(s) => {
                                            fill_card(ui, s);
                                            ui.add_space(12.0);
                                            stats_card(ui, Some(s));
                                            ui.add_space(12.0);
                                            tuning_card(ui, state, &w, snap.as_ref());
                                        }
                                        None => {
                                            stats_card(ui, None);
                                            ui.add_space(12.0);
                                            tuning_card(ui, state, &w, snap.as_ref());
                                            ui.add_space(12.0);
                                            idle_card(ui);
                                        }
                                    }

                                    ui.add_space(12.0);
                                    let device_line = {
                                        let id = w.params.device_id.read().clone();
                                        w.devices
                                            .read()
                                            .iter()
                                            .find(|d| d.id == id)
                                            .map(|d| format!("{} | {}", d.name, d.mix_format))
                                            .unwrap_or_else(|| {
                                                if id.is_empty() {
                                                    "none".into()
                                                } else {
                                                    id
                                                }
                                            })
                                    };
                                    footer_card(
                                        ui,
                                        state,
                                        bundle.as_ref(),
                                        snap.as_ref(),
                                        &device_line,
                                    );
                                });
                        });
                    if let Some((at, text, color)) = state.send_note.as_ref() {
                        let keep =
                            state.send_watch.is_some() || at.elapsed() < Duration::from_secs(4);
                        if keep {
                            widgets::toast(ctx, text, *color);
                        }
                    }
                });
        },
    )
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

            if let Some(pref) = widgets::language_menu(ui) {
                deviceout_i18n::set_preference(pref);
                theme::install_fonts(ui.ctx(), deviceout_i18n::current().script());
            }
        });
    });
}

fn device_card(ui: &mut egui::Ui, w: &Wiring, snap: Option<&UiState>) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());

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
    });
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

fn fill_card(ui: &mut egui::Ui, s: &UiState) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_heading(
            ui,
            t().heading_buffer,
            Some(
                egui::RichText::new(format!("{:.0}%", s.fill_fraction * 100.0))
                    .font(egui::FontId::monospace(12.0))
                    .color(theme::TEXT_DIM),
            ),
        );
        ui.add_space(6.0);
        widgets::progress_bar(ui, s.fill_fraction, s.target_fraction);
    });
}

fn target_step_index(steps: &[u32], ms: u32) -> usize {
    steps
        .iter()
        .enumerate()
        .min_by_key(|(_, step)| step.abs_diff(ms))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn allowed_target_steps(floor: u32) -> &'static [u32] {
    let first = TARGET_MS_STEPS
        .iter()
        .position(|&s| s >= floor)
        .unwrap_or(TARGET_MS_STEPS.len() - 1);
    &TARGET_MS_STEPS[first..]
}

fn stats_card(ui: &mut egui::Ui, snap: Option<&UiState>) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.columns(3, |cols| match snap {
            Some(s) => {
                widgets::stat_cell(
                    &mut cols[0],
                    t().heading_latency,
                    &format!("{:.0}", s.latency_ms),
                    "ms",
                    theme::TEXT,
                );
                let (drift, drift_color) = match s.drift_ppm {
                    Some(ppm) => (format!("{ppm:+.2}"), theme::TEXT),
                    None => (format!("{:+.0}…", s.raw_drift_ppm), theme::TEXT_DIM),
                };
                widgets::stat_cell(&mut cols[1], t().heading_drift, &drift, "ppm", drift_color);
                widgets::stat_cell(
                    &mut cols[2],
                    t().heading_format,
                    &format!("{:.1}", s.sink_rate_hz / 1000.0),
                    "kHz",
                    theme::TEXT,
                );
            }
            None => {
                widgets::stat_cell(
                    &mut cols[0],
                    t().heading_latency,
                    "—",
                    "",
                    theme::TEXT_FAINT,
                );
                widgets::stat_cell(&mut cols[1], t().heading_drift, "—", "", theme::TEXT_FAINT);
                widgets::stat_cell(&mut cols[2], t().heading_format, "—", "", theme::TEXT_FAINT);
            }
        });
    });
}

fn tuning_card(ui: &mut egui::Ui, state: &mut EditorUi, w: &Wiring, snap: Option<&UiState>) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());

        let steps = allowed_target_steps(w.engine.target_floor_ms());
        let committed = w.engine.target_ms();
        let shown = state.target_drag.unwrap_or(committed);
        widgets::section_heading(
            ui,
            t().heading_target_latency,
            Some(
                egui::RichText::new(format!("{shown} ms"))
                    .font(egui::FontId::monospace(12.0))
                    .color(theme::TEXT),
            ),
        );
        ui.add_space(8.0);

        let mut idx = target_step_index(steps, shown);
        let slider = widgets::stepped_slider(ui, &mut idx, steps.len());
        let next = steps[idx.min(steps.len() - 1)];
        if slider.dragged() {
            state.target_drag = Some(next);
        }
        let commit = slider.drag_stopped() || (slider.clicked() && !slider.dragged());
        if commit {
            state.target_drag = None;
            if next != committed {
                *w.params.target_ms.write() = next;
                w.engine.set_target_ms_async(next);
            }
        }

        ui.add_space(14.0);
        queue_row(ui, state, w);
        ui.add_space(14.0);
        exclusive_row(ui, w, snap);
    });
}

fn exclusive_row(ui: &mut egui::Ui, w: &Wiring, snap: Option<&UiState>) {
    let mut wanted = w.engine.exclusive();
    widgets::section_heading(ui, t().heading_exclusive, None);
    ui.add_space(6.0);
    if widgets::switch(ui, &mut wanted).changed() {
        *w.params.exclusive.write() = wanted;
        w.engine.set_exclusive_async(wanted);
    }
    if wanted && snap.is_some_and(|s| !s.exclusive) {
        ui.add_space(6.0);
        widgets::alert_line(ui, theme::AMBER, t().exclusive_fell_back);
    }
}

fn queue_row(ui: &mut egui::Ui, state: &mut EditorUi, w: &Wiring) {
    let committed = w.engine.queue_periods();
    let shown = state.queue_drag.unwrap_or(committed);
    widgets::section_heading(
        ui,
        t().heading_device_queue,
        Some(
            egui::RichText::new(format!("{shown}×"))
                .font(egui::FontId::monospace(12.0))
                .color(theme::TEXT),
        ),
    );
    ui.add_space(8.0);

    let mut idx = target_step_index(QUEUE_PERIOD_STEPS, shown);
    let slider = widgets::stepped_slider(ui, &mut idx, QUEUE_PERIOD_STEPS.len());
    let next = QUEUE_PERIOD_STEPS[idx.min(QUEUE_PERIOD_STEPS.len() - 1)];
    if slider.dragged() {
        state.queue_drag = Some(next);
    }
    if slider.drag_stopped() || (slider.clicked() && !slider.dragged()) {
        state.queue_drag = None;
        if next != committed {
            *w.params.queue_periods.write() = next;
            w.engine.set_queue_periods_async(next);
        }
    }
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

fn idle_card(ui: &mut egui::Ui) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(t().engine_idle)
                    .size(13.0)
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(16.0);
        });
    });
}

fn footer_card(
    ui: &mut egui::Ui,
    state: &mut EditorUi,
    bundle: Option<&PathBuf>,
    snap: Option<&UiState>,
    device_line: &str,
) {
    poll_prompt(state);
    poll_send(state);
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        update_row(ui, state, bundle);
        ui.add_space(8.0);
        feedback_row(ui, state, bundle.map(|p| p.as_path()), snap, device_line);
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
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if widgets::switch(ui, &mut state.telemetry_opt_in).changed() {
            deviceout_update::telemetry::set_enabled(state.telemetry_opt_in);
        }
        ui.add(
            egui::Label::new(
                egui::RichText::new(t().usage_stats)
                    .size(10.5)
                    .color(theme::TEXT_FAINT),
            )
            .truncate(),
        );
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
            state.send_note = Some((
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
            state.send_note = Some((Instant::now(), t().sending.into(), theme::TEXT_DIM));
        }
        Err(msg) => {
            state.send_watch = None;
            state.send_note = Some((Instant::now(), msg, theme::RED));
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
        state.send_note = Some((Instant::now(), t().sent.into(), theme::GREEN));
        return;
    }
    if outbox_file(&deviceout_update::failed_dir(), &id).is_file() {
        state.send_watch = None;
        state.send_note = Some((Instant::now(), t().send_failed.into(), theme::RED));
        return;
    }
    if elapsed >= Duration::from_secs(25) {
        state.send_watch = None;
        state.send_note = Some((Instant::now(), t().saved_will_retry.into(), theme::AMBER));
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

