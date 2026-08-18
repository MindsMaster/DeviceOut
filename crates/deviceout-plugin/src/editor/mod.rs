use std::sync::Arc;

use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui};
use parking_lot::{Mutex, RwLock};

use deviceout_engine::EngineMetrics;
use deviceout_sink::DeviceInfo;

use crate::{enumerate_devices, DeviceOut, DeviceOutParams, EngineSlot, UiState, RING_MS};

mod theme;
mod widgets;

type SharedMetrics = Arc<RwLock<Option<Arc<EngineMetrics>>>>;
type SharedDevices = Arc<RwLock<Vec<DeviceInfo>>>;

const REPO_URL: &str = "https://github.com/MindsMaster/DeviceOut";
const AUTHOR_EMAIL: &str = "an5w1r@163.com";
const QQ_GROUP: &str = "1046048297";
const QQ_GROUP_URL: &str = "https://qun.qq.com/universal-share/share?ac=1&authKey=QEsKQUh0Z2dhNgn2lQDrnRRGD0MALQvTfF3FPr5ZYOv8i/lKdNp8G6gjTgYrD/Rw&busi_data=eyJncm91cENvZGUiOiIxMDQ2MDQ4Mjk3IiwidG9rZW4iOiJRR3lINzRUTUt0U2M0bDRkeTRhTERCT0ZscFlsTUxBNm9TdjUwelYxZXN2dzdUWUlyWnY0Mi9oOHI3c2JBOEYrIiwidWluIjoiMTE3MzM0ODYxMCJ9&data=EuNu7PewjDdMH0GF2GuEOYyIXT8iT25RJajcGVD3qL5LXkiATxSyzB-gdxqgbbFkbRvkyzoiJQ_sJ8-tBfFbSuW4NYFdjzdIr5Z1E8reAiM&svctype=5&tempid=h5_group_info";

pub(crate) struct Wiring {
    pub params: Arc<DeviceOutParams>,
    pub slot: Arc<Mutex<EngineSlot>>,
    pub metrics: SharedMetrics,
    pub devices: SharedDevices,
}

pub(crate) fn create(w: Wiring) -> Option<Box<dyn Editor>> {
    let editor_state = Arc::clone(&w.params.editor_state);

    create_egui_editor(
        editor_state,
        (),
        |ctx, _| theme::install(ctx),
        move |ctx, _setter, _state| {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));

            egui::CentralPanel::default()
                .frame(theme::root_frame())
                .show(ctx, |ui| {
                    let snap = snapshot(&w.metrics);

                    header(ui, snap.as_ref());
                    ui.add_space(12.0);
                    device_card(ui, &w, snap.as_ref());
                    ui.add_space(12.0);

                    match &snap {
                        Some(s) => {
                            buffer_card(ui, s);
                            ui.add_space(12.0);
                            stats_card(ui, s);
                            alerts(ui, s);
                        }
                        None => idle_card(ui),
                    }
                });
        },
    )
}

fn snapshot(metrics: &SharedMetrics) -> Option<UiState> {
    let guard = metrics.read();
    let m = guard.as_ref()?;
    let stats = m.stats();

    let latency_ms = if m.sink_rate_hz() > 0.0 {
        m.smoothed_fill() / m.sink_rate_hz() * 1000.0
    } else {
        0.0
    };

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
        latency_ms,
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
                &format!("QQ群 {QQ_GROUP}"),
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
                    "未选择设备".to_string()
                } else {
                    "已保存的设备不可用".to_string()
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
                            format!("{} · 系统默认", device.name)
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
                    let mut guard = w.slot.lock();
                    DeviceOut::restart_locked(&mut guard, id);
                    *w.metrics.write() = guard.handle.as_ref().map(|h| Arc::clone(h.metrics()));
                }
            }
        });

        if let Some(error) = snap.and_then(|s| s.error.as_ref()) {
            ui.add_space(6.0);
            widgets::alert_line(ui, theme::RED, error.clone());
        }
    });
}

fn buffer_card(ui: &mut egui::Ui, s: &UiState) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());

        widgets::section_heading(
            ui,
            "BUFFER",
            Some(
                egui::RichText::new(format!("{:>3.0}%", s.fill_fraction * 100.0))
                    .font(egui::FontId::monospace(12.0))
                    .color(theme::TEXT_DIM),
            ),
        );
        ui.add_space(6.0);
        widgets::progress_bar(ui, s.fill_fraction);
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!(
                "{} frames · {} f/period · {:.0} ms ring",
                thousands(s.capacity_frames),
                s.period_frames,
                RING_MS,
            ))
                .font(egui::FontId::monospace(10.5))
                .color(theme::TEXT_FAINT),
        );
    });
}

fn stats_card(ui: &mut egui::Ui, s: &UiState) {
    theme::card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());

        let (drift, drift_color) = match s.drift_ppm {
            Some(ppm) => (format!("{ppm:+.2}"), theme::TEXT),
            None => (format!("{:+.0}…", s.raw_drift_ppm), theme::TEXT_DIM),
        };

        ui.columns(3, |cols| {
            widgets::stat_cell(
                &mut cols[0],
                "LATENCY",
                &format!("{:.0}", s.latency_ms),
                "ms",
                theme::TEXT,
            );
            widgets::stat_cell(&mut cols[1], "DRIFT", &drift, "ppm", drift_color);
            widgets::stat_cell(
                &mut cols[2],
                "FORMAT",
                &format!("{:.1}", s.sink_rate_hz / 1000.0),
                "kHz",
                theme::TEXT,
            );
        });
    });
}

fn alerts(ui: &mut egui::Ui, s: &UiState) {
    let mut lines: Vec<(egui::Color32, String)> = Vec::new();

    if s.underruns > 0 || s.overruns > 0 {
        lines.push((
            theme::RED,
            format!(
                "断流：欠载 {} 次，溢出 {} 次，静音 {:.2} s",
                s.underruns, s.overruns, s.dropout_seconds
            ),
        ));
    }

    if s.reconnects > 0 {
        let discarded_s = if s.sink_rate_hz > 0.0 {
            s.frames_discarded as f64 / s.sink_rate_hz
        } else {
            0.0
        };
        lines.push((
            theme::ORANGE,
            format!(
                "掉线 {} 次，已重连；丢弃 {} 帧（约 {:.2} s）",
                s.reconnects, s.frames_discarded, discarded_s
            ),
        ));
    }

    if s.clamp_events > 0 {
        lines.push((
            theme::AMBER,
            format!("重采样限幅 {} 次，检查两端采样率", s.clamp_events),
        ));
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
                egui::RichText::new("链路未启动")
                    .size(13.0)
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(16.0);
        });
    });
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
