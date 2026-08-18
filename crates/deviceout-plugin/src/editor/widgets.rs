use nih_plug_egui::egui::{
    self, Color32, FontId, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, Vec2,
};

use deviceout_engine::EngineState;

use super::theme;

pub(crate) fn section_heading(ui: &mut egui::Ui, title: &str, trailing: Option<RichText>) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .font(FontId::proportional(10.5))
                .color(theme::TEXT_FAINT)
                .strong(),
        );
        if let Some(text) = trailing {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(text);
            });
        }
    });
}

pub(crate) fn status_indicator(ui: &mut egui::Ui, state: Option<EngineState>) {
    let (label, color) = match state {
        Some(EngineState::Running) => ("运行中", theme::GREEN),
        Some(EngineState::Priming) => ("预填充", theme::AMBER),
        Some(EngineState::Stopped) => ("已停止", theme::TEXT_FAINT),
        Some(EngineState::Failed) => ("出错", theme::RED),
        Some(EngineState::Reconnecting) => ("重连中", theme::ORANGE),
        None => ("未启动", theme::TEXT_FAINT),
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

pub(crate) fn stat_cell(ui: &mut egui::Ui, label: &str, value: &str, unit: &str, color: Color32) {
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(label)
                .font(FontId::proportional(10.0))
                .color(theme::TEXT_FAINT)
                .strong(),
        );
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let total = value_width(ui, value, unit);
            ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
            ui.label(RichText::new(value).font(FontId::monospace(15.0)).color(color).strong());
            if !unit.is_empty() {
                ui.label(
                    RichText::new(unit)
                        .font(FontId::proportional(11.0))
                        .color(theme::TEXT_FAINT),
                );
            }
        });
    });
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
            .layout_no_wrap(unit.to_string(), FontId::proportional(11.0), theme::TEXT_FAINT)
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

    response.on_hover_text("刷新设备列表")
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

pub(crate) fn alert_line(ui: &mut egui::Ui, color: Color32, text: String) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 14.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 2.5, color);
        ui.label(RichText::new(text).size(11.5).color(theme::TEXT_DIM));
    });
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

pub(crate) fn wordmark(ui: &mut egui::Ui) {
    ui.label(
        RichText::new("DeviceOut")
            .font(FontId::proportional(15.0))
            .color(theme::TEXT)
            .strong(),
    );
}
