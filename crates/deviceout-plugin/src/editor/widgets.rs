use nih_plug_egui::egui::{
    self, Color32, FontId, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, Vec2,
};

use deviceout_engine::EngineState;

use super::theme;
use crate::i18n;

pub(crate) fn heading_text(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title)
            .font(FontId::proportional(10.5))
            .color(theme::TEXT_FAINT)
            .strong(),
    );
}

pub(crate) fn section_heading(ui: &mut egui::Ui, title: &str, trailing: Option<RichText>) {
    ui.horizontal(|ui| {
        heading_text(ui, title);
        if let Some(text) = trailing {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(text);
            });
        }
    });
}

struct ButtonSpec {
    fill: Color32,
    hover_fill: Color32,
    stroke: Stroke,
    text: Color32,
    hover_text: Color32,
}

fn styled_button(ui: &mut egui::Ui, label: &str, spec: ButtonSpec) -> Response {
    let font = FontId::proportional(12.5);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font.clone(), spec.text);
    let size = Vec2::new(galley.size().x + 24.0, 30.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    let enabled = ui.is_enabled();
    let hovered = enabled && response.hovered();
    let fade = |c: Color32| c.gamma_multiply(0.45);
    let fill = match (enabled, hovered) {
        (false, _) => fade(spec.fill),
        (true, true) => spec.hover_fill,
        (true, false) => spec.fill,
    };
    let text_color = match (enabled, hovered) {
        (false, _) => fade(spec.text),
        (true, true) => spec.hover_text,
        (true, false) => spec.text,
    };

    let painter = ui.painter();
    if fill != Color32::TRANSPARENT {
        painter.rect_filled(rect, theme::CORNER_BUTTON, fill);
    }
    if spec.stroke != Stroke::NONE {
        let stroke = if enabled {
            spec.stroke
        } else {
            Stroke::new(spec.stroke.width, fade(spec.stroke.color))
        };
        painter.rect_stroke(rect, theme::CORNER_BUTTON, stroke, StrokeKind::Inside);
    }
    let galley = painter.layout_no_wrap(label.to_string(), font, text_color);
    painter.galley(rect.center() - galley.size() / 2.0, galley, text_color);

    response
}

pub(crate) fn primary_button(ui: &mut egui::Ui, label: &str) -> Response {
    styled_button(
        ui,
        label,
        ButtonSpec {
            fill: theme::PRIMARY,
            hover_fill: theme::PRIMARY_HOVER,
            stroke: Stroke::NONE,
            text: theme::PRIMARY_TEXT,
            hover_text: theme::PRIMARY_TEXT,
        },
    )
}

pub(crate) fn outline_button(ui: &mut egui::Ui, label: &str) -> Response {
    styled_button(
        ui,
        label,
        ButtonSpec {
            fill: Color32::TRANSPARENT,
            hover_fill: theme::WIDGET_HOVER,
            stroke: Stroke::new(1.0_f32, theme::OUTLINE),
            text: theme::TEXT_DIM,
            hover_text: theme::TEXT,
        },
    )
}

pub(crate) fn switch(ui: &mut egui::Ui, on: &mut bool) -> Response {
    let size = Vec2::new(30.0, 16.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }

    let track_off = Color32::from_rgb(0x27, 0x27, 0x2a);
    let knob_off = Color32::from_rgb(0xa1, 0xa1, 0xaa);
    let track_on = Color32::from_rgb(0xfa, 0xfa, 0xfa);
    let knob_on = Color32::from_rgb(0x09, 0x09, 0x0b);

    let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.15);
    let lerp = |a: Color32, b: Color32| -> Color32 {
        egui::lerp(egui::Rgba::from(a)..=egui::Rgba::from(b), t).into()
    };
    let mut track = lerp(track_off, track_on);
    let knob = lerp(knob_off, knob_on);
    if response.hovered() {
        track = track.gamma_multiply(1.2);
    }

    let painter = ui.painter();
    painter.rect_filled(rect, rect.height() / 2.0, track);

    let knob_radius = 6.0;
    let knob_x = egui::lerp(
        (rect.min.x + knob_radius + 2.0)..=(rect.max.x - knob_radius - 2.0),
        t,
    );
    painter.circle_filled(Pos2::new(knob_x, rect.center().y), knob_radius, knob);

    response
}

pub(crate) fn status_indicator(ui: &mut egui::Ui, state: Option<EngineState>) {
    let (label, color) = match state {
        Some(EngineState::Running) => (i18n::pick("运行中", "Running"), theme::GREEN),
        Some(EngineState::Priming) => (i18n::pick("预填充", "Priming"), theme::AMBER),
        Some(EngineState::Stopped) => (i18n::pick("已停止", "Stopped"), theme::TEXT_FAINT),
        Some(EngineState::Failed) => (i18n::pick("出错", "Error"), theme::RED),
        Some(EngineState::Reconnecting) => (i18n::pick("重连中", "Reconnecting"), theme::ORANGE),
        None => (i18n::pick("未启动", "Idle"), theme::TEXT_FAINT),
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(8.0, 8.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.0, color);
        ui.label(
            RichText::new(label)
                .font(FontId::proportional(12.0))
                .color(theme::TEXT_DIM),
        );
    });
}

pub(crate) fn stepped_slider(ui: &mut egui::Ui, index: &mut usize, steps: usize) -> Response {
    let steps = steps.max(2);
    let last = steps - 1;
    let (rect, mut response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 16.0),
        Sense::click_and_drag(),
    );

    let pad = 5.0;
    let x0 = rect.left() + pad;
    let x1 = rect.right() - pad;
    let span = (x1 - x0).max(1.0);
    let cy = rect.center().y;

    if let Some(pos) = response.interact_pointer_pos() {
        if response.clicked() || response.dragged() {
            let t = ((pos.x - x0) / span).clamp(0.0, 1.0);
            let next = (t * last as f32).round() as usize;
            if next != *index {
                *index = next;
                response.mark_changed();
            }
        }
    }

    let t = (*index).min(last) as f32 / last as f32;
    let painter = ui.painter();
    let track = Rect::from_min_max(Pos2::new(x0, cy - 1.5), Pos2::new(x1, cy + 1.5));
    painter.rect_filled(track, 1.5, theme::TRACK);
    if t > 0.002 {
        let filled = Rect::from_min_max(track.min, Pos2::new(x0 + span * t, track.max.y));
        painter.rect_filled(filled, 1.5, theme::TEXT);
    }

    let thumb = Pos2::new(x0 + span * t, cy);
    let r = if response.hovered() || response.dragged() {
        5.0
    } else {
        4.5
    };
    painter.circle_filled(thumb, r, theme::PRIMARY);

    response
}

pub(crate) fn progress_bar(ui: &mut egui::Ui, fraction: f64) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, theme::TRACK);
    let fill = fraction.clamp(0.0, 1.0) as f32;
    if fill > 0.002 {
        let filled = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.min.x + rect.width() * fill, rect.max.y),
        );
        painter.rect_filled(filled, 3.0, theme::TEXT);
    }
}

fn stat_label(ui: &mut egui::Ui, label: &str) {
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(label)
                .font(FontId::proportional(10.0))
                .color(theme::TEXT_FAINT)
                .strong(),
        );
    });
}

fn stat_value(ui: &mut egui::Ui, value: &str, unit: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let total = value_width(ui, value, unit);
        ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
        ui.label(
            RichText::new(value)
                .font(FontId::monospace(15.0))
                .color(color)
                .strong(),
        );
        if !unit.is_empty() {
            ui.label(
                RichText::new(unit)
                    .font(FontId::proportional(11.0))
                    .color(theme::TEXT_FAINT),
            );
        }
    });
}

pub(crate) fn stat_cell(ui: &mut egui::Ui, label: &str, value: &str, unit: &str, color: Color32) -> Response {
    ui.vertical(|ui| {
        ui.set_min_width(ui.available_width());
        stat_label(ui, label);
        ui.add_space(3.0);
        stat_value(ui, value, unit, color);
    })
    .response
}

fn value_width(ui: &egui::Ui, value: &str, unit: &str) -> f32 {
    let v = ui
        .painter()
        .layout_no_wrap(value.to_string(), FontId::monospace(15.0), theme::TEXT)
        .size();
    let u = if unit.is_empty() {
        0.0
    } else {
        ui.painter()
            .layout_no_wrap(
                unit.to_string(),
                FontId::proportional(11.0),
                theme::TEXT_FAINT,
            )
            .size()
            .x
            + 3.0
    };
    v.x + u
}

pub(crate) fn refresh_button(ui: &mut egui::Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(30.0, 30.0), Sense::click());
    let painter = ui.painter();

    if response.hovered() {
        painter.rect_filled(rect, theme::CORNER_SMALL, theme::WIDGET_HOVER);
    }

    let color = if response.hovered() {
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };
    refresh_icon(painter, rect.center(), 6.5, color);

    response.on_hover_text(i18n::pick("刷新设备列表", "Refresh device list"))
}

fn refresh_icon(painter: &egui::Painter, center: Pos2, radius: f32, color: Color32) {
    let start = (-70.0_f32).to_radians();
    let sweep = 300.0_f32.to_radians();
    let stroke = Stroke::new(1.4_f32, color);

    let points: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = start + sweep * i as f32 / 24.0;
            center + radius * Vec2::new(a.cos(), a.sin())
        })
        .collect();
    painter.add(Shape::line(points.clone(), stroke));

    let end = *points.last().unwrap();
    let angle = start + sweep;
    let tangent = Vec2::new(-angle.sin(), angle.cos());
    let back = -tangent;
    for sign in [30.0_f32, -30.0] {
        let r = sign.to_radians();
        let dir = Vec2::new(
            back.x * r.cos() - back.y * r.sin(),
            back.x * r.sin() + back.y * r.cos(),
        );
        painter.line_segment([end, end + dir * 3.6], stroke);
    }
}

pub(crate) fn alert_line(ui: &mut egui::Ui, color: Color32, text: String) -> Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 14.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 2.5, color);
        ui.label(RichText::new(text).size(11.5).color(theme::TEXT_DIM))
    })
    .inner
}

pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    source: egui::ImageSource<'_>,
    tooltip: &str,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());

    if response.hovered() {
        ui.painter()
            .rect_filled(rect, theme::CORNER_SMALL, theme::WIDGET_HOVER);
    }

    let tint = if response.hovered() {
        theme::TEXT
    } else {
        theme::TEXT_FAINT
    };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(15.0));
    egui::Image::new(source).tint(tint).paint_at(ui, icon_rect);

    response.on_hover_text(tooltip)
}

pub(crate) fn lang_button(ui: &mut egui::Ui, label: &str, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());

    if response.hovered() {
        ui.painter()
            .rect_filled(rect, theme::CORNER_SMALL, theme::WIDGET_HOVER);
    }

    let color = if response.hovered() {
        theme::TEXT
    } else {
        theme::TEXT_FAINT
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.0),
        color,
    );

    response.on_hover_text(tooltip)
}

pub(crate) fn wordmark(ui: &mut egui::Ui) {
    ui.label(
        RichText::new("DeviceOut")
            .font(FontId::proportional(15.0))
            .color(theme::TEXT)
            .strong(),
    );
}

pub(crate) fn toast(ctx: &egui::Context, text: &str, color: Color32) {
    egui::Area::new(egui::Id::new("deviceout_toast"))
        .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 14.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::CARD)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .corner_radius(theme::CORNER_BUTTON)
                .inner_margin(egui::Margin::symmetric(22, 10))
                .show(ui, |ui| {
                    ui.set_min_width(168.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(text).size(13.0).color(color).strong());
                    });
                });
        });
}
