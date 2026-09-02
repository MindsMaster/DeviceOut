use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui};
use parking_lot::{Mutex, RwLock};

use deviceout_sink::DeviceInfo;
use deviceout_update::outbox::FeedbackKind;

use crate::i18n;
use crate::{enumerate_devices, DeviceOutParams, EngineController, UiState, RING_FRAME_STEPS};

mod theme;
mod widgets;
#[cfg(windows)]
mod win_prompt;

type SharedDevices = Arc<RwLock<Vec<DeviceInfo>>>;

const INSTALL_BUTTON_COOLDOWN: Duration = Duration::from_secs(30);

const REPO_URL: &str = "https://github.com/MindsMaster/DeviceOut";
const AUTHOR_EMAIL: &str = "an5w1r@163.com";
const QQ_GROUP: &str = "1046048297";
const QQ_GROUP_URL: &str = "https://qun.qq.com/universal-share/share?ac=1&authKey=QEsKQUh0Z2dhNgn2lQDrnRRGD0MALQvTfF3FPr5ZYOv8i/lKdNp8G6gjTgYrD/Rw&busi_data=eyJncm91cENvZGUiOiIxMDQ2MDQ4Mjk3IiwidG9rZW4iOiJRR3lINzRUTUt0U2M0bDRkeTRhTERCT0ZscFlsTUxBNm9TdjUwelYxZXN2dzdUWUlyWnY0Mi9oOHI3c2JBOEYrIiwidWluIjoiMTE3MzM0ODYxMCJ9&data=EuNu7PewjDdMH0GF2GuEOYyIXT8iT25RJajcGVD3qL5LXkiATxSyzB-gdxqgbbFkbRvkyzoiJQ_sJ8-tBfFbSuW4NYFdjzdIr5Z1E8reAiM&svctype=5&tempid=h5_group_info";

pub(crate) struct Wiring {
    pub params: Arc<DeviceOutParams>,
    pub engine: Arc<EngineController>,
    pub devices: SharedDevices,
}

struct PromptWait {
    kind: FeedbackKind,
    diag: Option<String>,
    done: Arc<Mutex<Option<Option<(String, String)>>>>,
}

struct EditorUi {
    install_started: Option<Instant>,
    check_busy_since: Option<Instant>,
    check_mark_attempt: Option<i64>,
    prompt_wait: Option<PromptWait>,
    telemetry_opt_in: bool,
    last_ping: Option<Instant>,
    ring_drag: Option<u32>,
    send_watch: Option<(String, Instant)>,
    send_note: Option<(Instant, String, egui::Color32)>,
}

impl Default for EditorUi {
    fn default() -> Self {
        Self {
            install_started: None,
            check_busy_since: None,
            check_mark_attempt: None,
            prompt_wait: None,
            telemetry_opt_in: deviceout_update::telemetry::is_enabled(),
            last_ping: None,
            ring_drag: None,
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

            let ping_due = state
                .last_ping
                .map_or(true, |t| t.elapsed() >= Duration::from_secs(15 * 60));
            if ping_due {
                state.last_ping = Some(Instant::now());
                deviceout_update::telemetry::spawn_ping(env!("CARGO_PKG_VERSION"));
            }

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
                                            size_card(ui, state, &w);
                                            alerts(ui, s);
                                        }
                                        None => {
                                            stats_card(ui, None);
                                            ui.add_space(12.0);
                                            size_card(ui, state, &w);
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
                        let keep = state.send_watch.is_some() || at.elapsed() < Duration::from_secs(4);
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
        capacity_frames: m.capacity_frames(),
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
                let _ = open::that(REPO_URL);
            }
            if widgets::icon_button(
                ui,
                egui::include_image!("../../assets/icons/tencentqq.svg"),
                &format!("{} {QQ_GROUP}", i18n::pick("QQ群", "QQ group")),
            )
            .clicked()
            {
                let _ = open::that(QQ_GROUP_URL);
            }
            if widgets::icon_button(
                ui,
                egui::include_image!("../../assets/icons/mail.svg"),
                AUTHOR_EMAIL,
            )
            .clicked()
            {
                let _ = open::that(format!("mailto:{AUTHOR_EMAIL}"));
            }

            let label = match i18n::lang() {
                i18n::Lang::Zh => "EN",
                i18n::Lang::En => "中",
            };
            if widgets::lang_button(ui, label, i18n::pick("切换到 English", "切换到中文"))
                .clicked()
            {
                i18n::set_lang(match i18n::lang() {
                    i18n::Lang::Zh => i18n::Lang::En,
                    i18n::Lang::En => i18n::Lang::Zh,
                });
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
                egui::RichText::new(d.mix_format.to_string())
                    .font(egui::FontId::proportional(11.0))
                    .color(theme::TEXT_FAINT)
            });

        widgets::section_heading(ui, "OUTPUT DEVICE", format_text);
        ui.add_space(4.0);

        let current_name = w
            .devices
            .read()
            .iter()
            .find(|d| d.id == current_id)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| {
                if current_id.is_empty() {
                    i18n::pick("未选择设备", "No device selected").to_string()
                } else {
                    i18n::pick("已保存的设备不可用", "Saved device unavailable").to_string()
                }
            });

        ui.horizontal(|ui| {
            let button_width = 34.0;
            let combo_width = (ui.available_width() - button_width - 8.0).max(120.0);

            let mut chosen: Option<String> = None;
            egui::ComboBox::from_id_salt("device")
                .selected_text(egui::RichText::new(current_name).size(13.0))
                .width(combo_width)
                .show_ui(ui, |ui| {
                    for device in w.devices.read().iter() {
                        let label = if device.is_default {
                            format!(
                                "{} · {}",
                                device.name,
                                i18n::pick("系统默认", "System default")
                            )
                        } else {
                            device.name.clone()
                        };
                        if ui
                            .selectable_label(device.id == current_id, label)
                            .clicked()
                        {
                            chosen = Some(device.id.clone());
                        }
                    }
                });

            if widgets::refresh_button(ui).clicked() {
                *w.devices.write() = enumerate_devices();
            }

            if let Some(id) = chosen {
                if id != current_id {
                    *w.params.device_id.write() = id.clone();
                    w.engine.set_device_async(id);
                }
            }
        });

        if let Some(error) = snap.and_then(|s| s.error.as_ref()) {
            ui.add_space(6.0);
            widgets::alert_line(ui, theme::RED, error.clone());
        }
    });
}

fn fill_card(ui: &mut egui::Ui, s: &UiState) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_heading(
            ui,
            "BUFFER",
            Some(
                egui::RichText::new(format!("{:.0}%", s.fill_fraction * 100.0))
                    .font(egui::FontId::monospace(12.0))
                    .color(theme::TEXT_DIM),
            ),
        );
        ui.add_space(6.0);
        widgets::progress_bar(ui, s.fill_fraction);
    });
}

fn ring_step_index(steps: &[u32], frames: u32) -> usize {
    steps
        .iter()
        .enumerate()
        .min_by_key(|(_, step)| step.abs_diff(frames))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn allowed_ring_steps(floor: u32) -> &'static [u32] {
    let first = RING_FRAME_STEPS
        .iter()
        .position(|&s| s >= floor)
        .unwrap_or(RING_FRAME_STEPS.len() - 1);
    &RING_FRAME_STEPS[first..]
}

fn stats_card(ui: &mut egui::Ui, snap: Option<&UiState>) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.columns(3, |cols| {
            match snap {
                Some(s) => {
                    widgets::stat_cell(
                        &mut cols[0],
                        "LATENCY",
                        &format!("{:.0}", s.latency_ms),
                        "ms",
                        theme::TEXT,
                    );
                    let (drift, drift_color) = match s.drift_ppm {
                        Some(ppm) => (format!("{ppm:+.2}"), theme::TEXT),
                        None => (format!("{:+.0}…", s.raw_drift_ppm), theme::TEXT_DIM),
                    };
                    widgets::stat_cell(&mut cols[1], "DRIFT", &drift, "ppm", drift_color);
                    widgets::stat_cell(
                        &mut cols[2],
                        "FORMAT",
                        &format!("{:.1}", s.sink_rate_hz / 1000.0),
                        "kHz",
                        theme::TEXT,
                    );
                }
                None => {
                    widgets::stat_cell(&mut cols[0], "LATENCY", "—", "", theme::TEXT_FAINT);
                    widgets::stat_cell(&mut cols[1], "DRIFT", "—", "", theme::TEXT_FAINT);
                    widgets::stat_cell(&mut cols[2], "FORMAT", "—", "", theme::TEXT_FAINT);
                }
            }
        });
    });
}

fn size_card(ui: &mut egui::Ui, state: &mut EditorUi, w: &Wiring) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());

        let steps = allowed_ring_steps(w.engine.ring_floor());
        let committed = w.engine.ring_frames();
        let shown = state.ring_drag.unwrap_or(committed);
        widgets::section_heading(
            ui,
            "BUFFER SIZE",
            Some(
                egui::RichText::new(thousands(u64::from(shown)))
                    .font(egui::FontId::monospace(12.0))
                    .color(theme::TEXT),
            ),
        );
        ui.add_space(8.0);

        let mut idx = ring_step_index(steps, shown);
        let slider = widgets::stepped_slider(ui, &mut idx, steps.len());
        let next = steps[idx.min(steps.len() - 1)];
        if slider.dragged() {
            state.ring_drag = Some(next);
        }
        let commit = slider.drag_stopped() || (slider.clicked() && !slider.dragged());
        if commit {
            state.ring_drag = None;
            if next != committed {
                *w.params.ring_frames.write() = next;
                w.engine.set_ring_frames_async(next);
            }
        }
    });
}

fn alerts(ui: &mut egui::Ui, s: &UiState) {
    let mut lines: Vec<(egui::Color32, String)> = Vec::new();

    if s.underruns > 0 || s.overruns > 0 {
        let text = match i18n::lang() {
            i18n::Lang::Zh => format!(
                "断流：欠载 {} 次，溢出 {} 次，静音 {:.2} s",
                s.underruns, s.overruns, s.dropout_seconds
            ),
            i18n::Lang::En => format!(
                "Dropouts: {} underruns, {} overruns, {:.2} s muted",
                s.underruns, s.overruns, s.dropout_seconds
            ),
        };
        lines.push((theme::RED, text));
    }

    if s.reconnects > 0 {
        let discarded_s = if s.sink_rate_hz > 0.0 {
            s.frames_discarded as f64 / s.sink_rate_hz
        } else {
            0.0
        };
        let text = match i18n::lang() {
            i18n::Lang::Zh => format!(
                "掉线 {} 次，已重连；丢弃 {} 帧（约 {:.2} s）",
                s.reconnects, s.frames_discarded, discarded_s
            ),
            i18n::Lang::En => format!(
                "{} reconnects; dropped {} frames (~{:.2} s)",
                s.reconnects, s.frames_discarded, discarded_s
            ),
        };
        lines.push((theme::ORANGE, text));
    }

    if s.clamp_events > 0 {
        let text = match i18n::lang() {
            i18n::Lang::Zh => format!("重采样限幅 {} 次，检查两端采样率", s.clamp_events),
            i18n::Lang::En => format!(
                "Resampler clamped {} times; check both sample rates",
                s.clamp_events
            ),
        };
        lines.push((theme::AMBER, text));
    }

    if lines.is_empty() {
        return;
    }

    ui.add_space(12.0);
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_heading(ui, "EVENTS", None);
        ui.add_space(2.0);
        for (color, text) in lines {
            widgets::alert_line(ui, color, text);
        }
    });
}

fn idle_card(ui: &mut egui::Ui) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(i18n::pick("链路未启动", "Engine idle"))
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

fn update_row(ui: &mut egui::Ui, state: &mut EditorUi, bundle: Option<&PathBuf>) {
    let current = env!("CARGO_PKG_VERSION");
    let pending = deviceout_update::pending_ready();
    let st = deviceout_update::load_state();
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
    );

    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(10.0, 14.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.0, color);
        let label = ui.label(
            egui::RichText::new(&status)
                .size(11.5)
                .color(theme::TEXT_DIM),
        );
        if let Some(tip) = tip {
            label.on_hover_text(tip);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if pending.is_some() {
                ui.add_enabled_ui(state.install_started.is_none(), |ui| {
                    if widgets::primary_button(ui, i18n::pick("安装", "Install")).clicked() {
                        state.install_started = Some(Instant::now());
                        deviceout_update::spawn_updater(&[OsStr::new("--apply-pending")]);
                    }
                });
            }
            let checking = state.check_busy_since.is_some();
            ui.add_enabled_ui(!checking, |ui| {
                if widgets::outline_button(
                    ui,
                    i18n::pick(
                        if checking {
                            "检查中…"
                        } else {
                            "检查更新"
                        },
                        if checking {
                            "Checking…"
                        } else {
                            "Check for updates"
                        },
                    ),
                )
                .clicked()
                {
                    state.check_mark_attempt = st.last_attempt;
                    state.check_busy_since = Some(Instant::now());
                    spawn_check_now(bundle);
                }
            });
        });
    });
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
) -> (String, egui::Color32, Option<String>) {
    if busy {
        return (
            i18n::pick("正在检查更新…", "Checking for updates…").into(),
            theme::TEXT_DIM,
            None,
        );
    }
    if let Some(pending) = pending {
        let text = match i18n::lang() {
            i18n::Lang::Zh => format!("v{} 已就绪", pending.version),
            i18n::Lang::En => format!("Update v{} ready", pending.version),
        };
        return (
            text,
            theme::AMBER,
            Some(
                i18n::pick(
                    "点击安装，随后按提示关闭宿主",
                    "Click Install, then close the host when prompted",
                )
                .into(),
            ),
        );
    }
    if let Some(err) = st.last_install_error.as_deref() {
        return (
            i18n::pick("安装失败", "Install failed").into(),
            theme::RED,
            Some(error_tip(err)),
        );
    }
    if let Some(err) = st.last_error.as_deref() {
        return (
            i18n::pick("检查失败", "Check failed").into(),
            theme::RED,
            Some(error_tip(err)),
        );
    }
    if let Some(latest) = st.last_latest.as_deref() {
        match deviceout_update::cmp_latest(latest, current) {
            deviceout_update::Cmp::Newer => {
                let text = match i18n::lang() {
                    i18n::Lang::Zh => format!("发现新版本 v{latest}"),
                    i18n::Lang::En => format!("Update v{latest} available"),
                };
                return (text, theme::AMBER, None);
            }
            deviceout_update::Cmp::EqualOrOlder => {
                let text = match i18n::lang() {
                    i18n::Lang::Zh => format!("已是最新 v{current}"),
                    i18n::Lang::En => format!("Up to date v{current}"),
                };
                return (text, theme::TEXT_DIM, None);
            }
            deviceout_update::Cmp::Invalid => {}
        }
    }
    (format!("v{current}"), theme::TEXT_DIM, None)
}

fn error_tip(err: &str) -> String {
    err.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect()
}

fn feedback_row(
    ui: &mut egui::Ui,
    state: &mut EditorUi,
    bundle: Option<&Path>,
    snap: Option<&UiState>,
    device_line: &str,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let waiting = state.prompt_wait.is_some();
        ui.add_enabled_ui(!waiting, |ui| {
            if widgets::outline_button(ui, i18n::pick("问题反馈", "Report a bug")).clicked() {
                open_feedback(state, FeedbackKind::Bug, bundle, snap, device_line);
            }
            if widgets::outline_button(ui, i18n::pick("功能建议", "Feature request")).clicked()
            {
                open_feedback(state, FeedbackKind::Feature, bundle, snap, device_line);
            }
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(i18n::pick("匿名统计", "Usage stats"))
                    .size(10.5)
                    .color(theme::TEXT_FAINT),
            );
            if widgets::switch(ui, &mut state.telemetry_opt_in).changed() {
                deviceout_update::telemetry::set_enabled(state.telemetry_opt_in);
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
        let is_bug = kind == FeedbackKind::Bug;
        let _ = std::thread::Builder::new()
            .name("deviceout-feedback".into())
            .spawn(move || {
                let result = win_prompt::run_feedback(is_bug);
                *slot.lock() = Some(result);
            });
        state.prompt_wait = Some(PromptWait { kind, diag, done });
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
    let wait = state.prompt_wait.take().expect("checked above");
    let Some((message, contact)) = result else {
        return;
    };
    let message = message.trim();
    if message.is_empty() {
        return;
    }
    match queue_feedback(wait.kind, message, &contact, wait.diag) {
        Ok(id) => {
            deviceout_update::spawn_updater(&[OsStr::new("--send-outbox")]);
            state.send_watch = Some((id, Instant::now()));
            state.send_note = Some((
                Instant::now(),
                i18n::pick("发送中", "Sending").into(),
                theme::TEXT_DIM,
            ));
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
        state.send_note = Some((
            Instant::now(),
            i18n::pick("已发送", "Sent").into(),
            theme::GREEN,
        ));
        return;
    }
    if outbox_file(&deviceout_update::failed_dir(), &id).is_file() {
        state.send_watch = None;
        state.send_note = Some((
            Instant::now(),
            i18n::pick("发送失败", "Send failed").into(),
            theme::RED,
        ));
        return;
    }
    if elapsed >= Duration::from_secs(25) {
        state.send_watch = None;
        state.send_note = Some((
            Instant::now(),
            i18n::pick("已保存，稍后重试", "Saved, will retry").into(),
            theme::AMBER,
        ));
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
        "output_device={device_line}\nengine_state={:?}\nfill={:.1}%\nunderruns={}\noverruns={}\ndevice_starvations={}\ndropout_s={:.2}\nreconnects={}\nframes_discarded={}\nclamp={}\nlatency_ms={:.1}\ndrift_ppm={}\nring_frames={}\nperiod_frames={}\nsink_hz={:.0}",
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
        s.period_frames,
        s.sink_rate_hz,
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
        return Err(i18n::pick(
            "待发送反馈过多，请稍后再试",
            "Too many queued feedback items; try again later",
        )
        .into());
    }
    let contact = Some(contact.trim().to_string()).filter(|c| !c.is_empty());
    let item = deviceout_update::new_outbox_item(
        kind,
        message.to_string(),
        contact,
        diag,
        env!("CARGO_PKG_VERSION").to_string(),
    )
    .map_err(|e| {
        format!(
            "{}{e}",
            i18n::pick("无法写入反馈：", "Failed to save feedback: ")
        )
    })?;
    deviceout_update::save_item(&deviceout_update::outbox_dir(), &item).map_err(|e| {
        format!(
            "{}{e}",
            i18n::pick("无法写入反馈：", "Failed to save feedback: ")
        )
    })?;
    Ok(item.id)
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
