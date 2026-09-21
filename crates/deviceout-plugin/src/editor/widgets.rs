use nih_plug_egui::egui::{
    self, Color32, CornerRadius, FontId, Pos2, Rect, Response, RichText, Sense, Shape, Stroke,
    StrokeKind, Vec2,
};

use deviceout_engine::EngineState;
use deviceout_i18n::Lang;

use super::theme;

fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    egui::lerp(egui::Rgba::from(a)..=egui::Rgba::from(b), t).into()
}

fn hover_amount(ui: &egui::Ui, response: &Response) -> f32 {
    let lit = response.hovered() && ui.is_enabled();
    ease(
        ui.ctx()
            .animate_bool_with_time(response.id.with("lit"), lit, 0.12),
    )
}

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
                ui.add(egui::Label::new(text).truncate());
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

fn styled_button(ui: &mut egui::Ui, label: &str, spec: ButtonSpec, stretch: bool) -> Response {
    let font = FontId::proportional(12.5);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font.clone(), spec.text);
    let natural = galley.size().x + 24.0;
    let width = if stretch {
        ui.available_width()
    } else {
        natural.min(ui.available_width().max(48.0))
    };
    let size = Vec2::new(width, 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    let enabled = ui.is_enabled();
    let lit = hover_amount(ui, &response);
    let held = enabled && response.is_pointer_button_down_on();
    let fade = |c: Color32| c.gamma_multiply(0.45);
    let fill = if enabled {
        let blended = mix(spec.fill, spec.hover_fill, lit);
        if held {
            blended.gamma_multiply(0.86)
        } else {
            blended
        }
    } else {
        fade(spec.fill)
    };
    let text_color = if enabled {
        mix(spec.text, spec.hover_text, lit)
    } else {
        fade(spec.text)
    };

    let rect = if held { rect.shrink(0.5) } else { rect };
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
    let text_pos = Pos2::new(
        rect.min.x + ((rect.width() - galley.size().x) / 2.0).max(6.0),
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter()
        .with_clip_rect(rect)
        .galley(text_pos, galley, text_color);

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
        false,
    )
}

pub(crate) fn outline_button(ui: &mut egui::Ui, label: &str) -> Response {
    outline_button_sized(ui, label, false)
}

pub(crate) fn outline_button_fill(ui: &mut egui::Ui, label: &str) -> Response {
    outline_button_sized(ui, label, true)
}

fn outline_button_sized(ui: &mut egui::Ui, label: &str, stretch: bool) -> Response {
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
        stretch,
    )
}

pub(crate) fn switch(ui: &mut egui::Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(42.0, 24.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }

    let t = ease(ui.ctx().animate_bool_with_time(response.id, *on, 0.22));
    let lit = hover_amount(ui, &response);
    let track = mix(theme::TRACK, theme::GREEN, t).gamma_multiply(1.0 + 0.16 * lit);
    let border = mix(theme::BORDER, theme::GREEN, t);

    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(12),
        track,
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );

    let radius = 9.0;
    let cx = egui::lerp((rect.left() + radius + 3.0)..=(rect.right() - radius - 3.0), t);
    let squash = if response.is_pointer_button_down_on() {
        2.0
    } else {
        0.0
    };
    let knob = Rect::from_center_size(
        Pos2::new(cx, rect.center().y),
        Vec2::new(radius * 2.0 + squash * 2.0, radius * 2.0),
    );
    painter.rect_filled(
        knob.translate(Vec2::new(0.0, 1.0)),
        CornerRadius::same(9),
        Color32::from_black_alpha(80),
    );
    painter.rect_filled(
        knob,
        CornerRadius::same(9),
        mix(Color32::from_rgb(0xd4, 0xd4, 0xd8), Color32::WHITE, t),
    );

    response
}

pub(crate) fn status_indicator(ui: &mut egui::Ui, state: Option<EngineState>) {
    let t = deviceout_i18n::t();
    let (label, color) = match state {
        Some(EngineState::Running) => (t.state_running, theme::GREEN),
        Some(EngineState::Priming) => (t.state_priming, theme::AMBER),
        Some(EngineState::Stopped) => (t.state_stopped, theme::TEXT_FAINT),
        Some(EngineState::Failed) => (t.state_error, theme::RED),
        Some(EngineState::Reconnecting) => (t.state_reconnecting, theme::ORANGE),
        None => (t.state_idle, theme::TEXT_FAINT),
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

pub(crate) struct NumberCombo<'a> {
    pub value: u32,
    pub floor: u32,
    pub ceiling: u32,
    pub steps: &'a [u32],
    pub unit: &'a str,
    pub blocked: &'a str,
    pub scrub: bool,
}

const PX_PER_STEP: f32 = 4.0;

pub(crate) fn number_combo(
    ui: &mut egui::Ui,
    id: &str,
    combo: NumberCombo<'_>,
    drag: &mut Option<u32>,
) -> Option<u32> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::hover());
    let popup_id = ui.make_persistent_id(id);

    let painter = ui.painter().clone();
    let frame = painter.add(Shape::Noop);

    let arrow_rect = Rect::from_min_max(Pos2::new(rect.right() - 32.0, rect.top()), rect.max);
    let hit = if combo.scrub { arrow_rect } else { rect };
    let arrow = ui.interact(hit, popup_id.with("arrow"), Sense::click());
    if arrow.clicked() {
        ui.memory_mut(|memory| memory.toggle_popup(popup_id));
    }
    let open = ui.memory(|memory| memory.is_popup_open(popup_id));

    let unit_width = painter
        .layout_no_wrap(
            combo.unit.to_string(),
            FontId::monospace(11.0),
            theme::TEXT_FAINT,
        )
        .size()
        .x;
    let edit_rect = Rect::from_min_max(
        Pos2::new(rect.left() + 12.0, rect.top()),
        Pos2::new(arrow_rect.left() - unit_width - 12.0, rect.bottom()),
    );
    let mut picked = None;
    let field = if combo.scrub {
        let field = ui.interact(edit_rect, popup_id.with("value"), Sense::click_and_drag());
        let acc_id = popup_id.with("acc");
        if field.dragged() {
            let shown = drag.unwrap_or(combo.value);
            let mut acc = ui.data_mut(|data| data.get_temp::<f32>(acc_id).unwrap_or(0.0))
                + field.drag_delta().x;
            let moved = (acc / PX_PER_STEP).trunc();
            if moved != 0.0 {
                acc -= moved * PX_PER_STEP;
                let next = (shown as f32 + moved).max(0.0) as u32;
                *drag = Some(next.clamp(combo.floor.max(1), combo.ceiling.max(1)));
            }
            ui.data_mut(|data| data.insert_temp(acc_id, acc));
        } else {
            ui.data_mut(|data| data.insert_temp(acc_id, 0.0_f32));
            if field.drag_stopped() {
                picked = drag.take();
            } else {
                *drag = None;
            }
        }
        if field.clicked() {
            ui.memory_mut(|memory| memory.toggle_popup(popup_id));
        }
        if field.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        Some(field)
    } else {
        None
    };

    painter.text(
        Pos2::new(edit_rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        drag.unwrap_or(combo.value),
        FontId::monospace(14.0),
        theme::TEXT,
    );

    let busy = field.as_ref().is_some_and(|f| f.dragged());
    let hovered = field.as_ref().is_some_and(|f| f.hovered());
    let border = if busy {
        theme::PRIMARY
    } else if open || arrow.hovered() || hovered {
        theme::MENU_BORDER
    } else {
        theme::OUTLINE
    };
    painter.set(
        frame,
        egui::epaint::RectShape::new(
            rect,
            theme::CORNER_BUTTON,
            theme::WIDGET,
            Stroke::new(1.0_f32, border),
            StrokeKind::Inside,
        ),
    );
    painter.text(
        Pos2::new(arrow_rect.left() - 10.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        combo.unit,
        FontId::monospace(11.0),
        theme::TEXT_FAINT,
    );
    let center = Pos2::new(arrow_rect.center().x, rect.center().y);
    let direction = if open { -1.0 } else { 1.0 };
    painter.add(Shape::line(
        vec![
            center + Vec2::new(-3.5, -1.75 * direction),
            center + Vec2::new(0.0, 1.75 * direction),
            center + Vec2::new(3.5, -1.75 * direction),
        ],
        Stroke::new(1.5_f32, theme::TEXT_DIM),
    ));

    if open {
        let screen = ui.ctx().screen_rect();
        let wanted = combo.steps.len() as f32 * 32.0 + 12.0;
        let below = (screen.bottom() - rect.bottom() - 13.0).max(0.0);
        let above = (rect.top() - screen.top() - 13.0).max(0.0);
        let placement = if below >= wanted || below >= above {
            egui::AboveOrBelow::Below
        } else {
            egui::AboveOrBelow::Above
        };
        let room = match placement {
            egui::AboveOrBelow::Below => below,
            egui::AboveOrBelow::Above => above,
        };
        let mut anchor = arrow.clone();
        anchor.rect = rect.expand2(Vec2::new(0.0, 5.0));
        ui.scope(|ui| {
            let style = ui.style_mut();
            style.spacing.menu_margin = egui::Margin::same(6);
            style.visuals.window_fill = theme::MENU;
            style.visuals.window_stroke = Stroke::new(1.0_f32, theme::MENU_BORDER);
            style.visuals.menu_corner_radius = theme::CORNER;
            style.visuals.popup_shadow = egui::epaint::Shadow {
                offset: [0, 6],
                blur: 20,
                spread: 2,
                color: Color32::from_black_alpha(100),
            };
            egui::popup::popup_above_or_below_widget(
                ui,
                popup_id,
                &anchor,
                placement,
                egui::PopupCloseBehavior::CloseOnClickOutside,
                |ui| {
                    ui.set_width((width - 14.0).max(1.0));
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let mut scroll = egui::style::ScrollStyle::floating();
                    scroll.bar_width = 6.0;
                    scroll.floating_width = 3.0;
                    scroll.floating_allocated_width = 8.0;
                    scroll.handle_min_length = 32.0;
                    scroll.dormant_handle_opacity = 0.25;
                    scroll.active_handle_opacity = 0.35;
                    scroll.interact_handle_opacity = 0.65;
                    scroll.active_background_opacity = 0.0;
                    scroll.interact_background_opacity = 0.0;
                    ui.spacing_mut().scroll = scroll;
                    egui::ScrollArea::vertical()
                        .max_height((room - 14.0).clamp(1.0, 280.0))
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for &step in combo.steps {
                                let hint = if step < combo.floor {
                                    Some(combo.blocked)
                                } else {
                                    None
                                };
                                let label = format!("{step} {}", combo.unit);
                                let here = step == drag.unwrap_or(combo.value);
                                if combo_option(ui, &label, hint, here).clicked()
                                    && hint.is_none()
                                {
                                    picked = Some(step);
                                    ui.memory_mut(|memory| memory.close_popup());
                                }
                            }
                        });
                },
            );
        });
    }

    picked.filter(|&v| v != combo.value)
}

fn combo_option(ui: &mut egui::Ui, label: &str, hint: Option<&str>, selected: bool) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 30.0), Sense::click());
    let dim = hint.is_some();
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if dim {
            Color32::TRANSPARENT
        } else if response.hovered() || response.is_pointer_button_down_on() {
            theme::MENU_HOVER
        } else if selected {
            theme::MENU_SELECTED
        } else {
            Color32::TRANSPARENT
        };
        painter.rect_filled(rect, theme::CORNER_SMALL, fill);
        painter.text(
            Pos2::new(rect.left() + 10.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::monospace(12.5),
            if dim { theme::TEXT_FAINT } else { theme::TEXT },
        );
        if let Some(hint) = hint {
            painter.text(
                Pos2::new(rect.right() - 10.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                hint,
                FontId::proportional(10.5),
                theme::TEXT_FAINT,
            );
        } else if selected {
            let center = Pos2::new(rect.right() - 14.0, rect.center().y);
            painter.add(Shape::line(
                vec![
                    center + Vec2::new(-4.0, 0.0),
                    center + Vec2::new(-1.0, 3.0),
                    center + Vec2::new(5.0, -4.0),
                ],
                Stroke::new(1.6_f32, theme::TEXT),
            ));
        }
    }
    response
}

pub(crate) fn progress_bar(ui: &mut egui::Ui, fraction: f64, target: f64) {
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
    let mark = target.clamp(0.0, 1.0) as f32;
    if mark > 0.002 {
        let x = rect.min.x + rect.width() * mark;
        let tick = Rect::from_min_max(
            Pos2::new(x - 1.0, rect.min.y - 2.0),
            Pos2::new(x + 1.0, rect.max.y + 2.0),
        );
        painter.rect_filled(tick, 1.0, theme::GREEN);
    }
}

const SPINNER_ROOM: f32 = 12.0;

fn stat_label(ui: &mut egui::Ui, label: &str) {
    ui.vertical_centered(|ui| {
        ui.add(
            egui::Label::new(
                RichText::new(label)
                    .font(FontId::proportional(10.0))
                    .color(theme::TEXT_FAINT)
                    .strong(),
            )
            .truncate(),
        );
    });
}

fn stat_value(ui: &mut egui::Ui, value: &str, unit: &str, color: Color32, busy: bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let total = value_width(ui, value, unit) + if busy { SPINNER_ROOM } else { 0.0 };
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
        if busy {
            ui.add_space(3.0);
            ui.add(egui::Spinner::new().size(9.0).color(theme::AMBER));
        }
    });
}

pub(crate) fn stat_cell(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    unit: &str,
    color: Color32,
    busy: bool,
) -> Response {
    ui.vertical(|ui| {
        ui.set_min_width(ui.available_width());
        stat_label(ui, label);
        ui.add_space(3.0);
        stat_value(ui, value, unit, color, busy);
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

pub(crate) fn device_picker(
    ui: &mut egui::Ui,
    label: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 34.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, ui.is_enabled(), label)
    });
    let popup_id = ui.make_persistent_id("device_menu");
    if response.clicked() {
        ui.memory_mut(|memory| memory.toggle_popup(popup_id));
    }
    let open = ui.memory(|memory| memory.is_popup_open(popup_id));
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_string(),
        FontId::proportional(13.0),
        theme::TEXT,
    );
    job.wrap = egui::text::TextWrapping {
        max_width: (width - 44.0).max(1.0),
        max_rows: 1,
        break_anywhere: true,
        ..Default::default()
    };
    let galley = ui.painter().layout_job(job);
    let truncated = galley.elided;
    let fill = if response.hovered() {
        theme::WIDGET
    } else {
        theme::CARD
    };
    let border = if response.has_focus() {
        theme::TEXT_DIM
    } else if open || response.hovered() {
        theme::MENU_BORDER
    } else {
        theme::OUTLINE
    };
    ui.painter().rect(
        rect,
        theme::CORNER_BUTTON,
        fill,
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );
    let text_pos = Pos2::new(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley, theme::TEXT);
    let center = Pos2::new(rect.right() - 16.0, rect.center().y);
    let direction = if open { -1.0 } else { 1.0 };
    ui.painter().add(Shape::line(
        vec![
            center + Vec2::new(-3.5, -1.75 * direction),
            center + Vec2::new(0.0, 1.75 * direction),
            center + Vec2::new(3.5, -1.75 * direction),
        ],
        Stroke::new(1.5_f32, theme::TEXT_DIM),
    ));

    if open {
        let gap = 5.0;
        let screen = ui.ctx().screen_rect();
        let below = (screen.bottom() - rect.bottom() - gap - 8.0).max(0.0);
        let above = (rect.top() - screen.top() - gap - 8.0).max(0.0);
        let placement = if below >= 294.0 || below >= above {
            egui::AboveOrBelow::Below
        } else {
            egui::AboveOrBelow::Above
        };
        let available_height = match placement {
            egui::AboveOrBelow::Below => below,
            egui::AboveOrBelow::Above => above,
        };
        let mut anchor = response.clone();
        anchor.rect = rect.expand2(Vec2::new(0.0, gap));
        ui.scope(|ui| {
            let style = ui.style_mut();
            style.spacing.menu_margin = egui::Margin::same(6);
            style.visuals.window_fill = theme::MENU;
            style.visuals.window_stroke = Stroke::new(1.0_f32, theme::MENU_BORDER);
            style.visuals.menu_corner_radius = theme::CORNER;
            style.visuals.popup_shadow = egui::epaint::Shadow {
                offset: [0, 6],
                blur: 20,
                spread: 2,
                color: Color32::from_black_alpha(100),
            };
            egui::popup::popup_above_or_below_widget(
                ui,
                popup_id,
                &anchor,
                placement,
                egui::PopupCloseBehavior::CloseOnClickOutside,
                |ui| {
                    ui.set_width((width - 14.0).max(1.0));
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let mut scroll = egui::style::ScrollStyle::floating();
                    scroll.bar_width = 6.0;
                    scroll.floating_width = 3.0;
                    scroll.floating_allocated_width = 8.0;
                    scroll.handle_min_length = 32.0;
                    scroll.dormant_handle_opacity = 0.25;
                    scroll.active_handle_opacity = 0.35;
                    scroll.interact_handle_opacity = 0.65;
                    scroll.active_background_opacity = 0.0;
                    scroll.interact_background_opacity = 0.0;
                    ui.spacing_mut().scroll = scroll;
                    egui::ScrollArea::vertical()
                        .max_height((available_height - 14.0).clamp(1.0, 280.0))
                        .auto_shrink([false, true])
                        .show(ui, add_contents);
                },
            );
        });
    } else if truncated {
        return response.on_hover_ui(|ui| {
            ui.set_max_width(width);
            ui.add(egui::Label::new(label).wrap());
        });
    }

    response
}

pub(crate) fn device_option(ui: &mut egui::Ui, label: &str, selected: bool) -> Response {
    let width = ui.available_width();
    let text_offset = 10.0;
    let galley = ui.painter().layout(
        label.to_string(),
        FontId::proportional(13.0),
        theme::TEXT,
        (width - text_offset - 30.0).max(1.0),
    );
    let height = (galley.size().y + 16.0).max(34.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if response.hovered() || response.is_pointer_button_down_on() {
            theme::MENU_HOVER
        } else if selected {
            theme::MENU_SELECTED
        } else {
            Color32::TRANSPARENT
        };
        painter.rect_filled(rect, theme::CORNER_SMALL, fill);
        if response.has_focus() {
            painter.rect_stroke(
                rect.shrink(1.0),
                theme::CORNER_SMALL,
                Stroke::new(1.0_f32, theme::TEXT_DIM),
                StrokeKind::Inside,
            );
        }
        if selected {
            let center = Pos2::new(rect.right() - 14.0, rect.center().y);
            painter.add(Shape::line(
                vec![
                    center + Vec2::new(-4.0, 0.0),
                    center + Vec2::new(-1.0, 3.0),
                    center + Vec2::new(5.0, -4.0),
                ],
                Stroke::new(1.6_f32, theme::TEXT),
            ));
        }
        let text_pos = Pos2::new(
            rect.left() + text_offset,
            rect.center().y - galley.size().y / 2.0,
        );
        painter.galley(text_pos, galley, theme::TEXT);
    }

    response
}

pub(crate) fn refresh_button(ui: &mut egui::Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
    let lit = hover_amount(ui, &response);
    let held = response.is_pointer_button_down_on();
    let fill = mix(theme::CARD, theme::WIDGET, lit);
    let border = if response.has_focus() {
        theme::TEXT_DIM
    } else {
        mix(theme::OUTLINE, theme::MENU_BORDER, lit)
    };
    let rect = if held { rect.shrink(0.5) } else { rect };
    let painter = ui.painter();
    painter.rect(
        rect,
        theme::CORNER_BUTTON,
        fill,
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );

    let color = mix(theme::TEXT_DIM, theme::TEXT, lit);
    refresh_icon(painter, rect.center(), 6.5, color);

    response.on_hover_text(deviceout_i18n::t().refresh_devices)
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

pub(crate) fn alert_line(ui: &mut egui::Ui, color: Color32, text: &str) -> Response {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 14.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 2.5, color);
        ui.add(egui::Label::new(RichText::new(text).size(11.5).color(theme::TEXT_DIM)).wrap())
    })
    .inner
}

pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    source: egui::ImageSource<'_>,
    tooltip: &str,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let lit = hover_amount(ui, &response);

    if lit > 0.0 {
        ui.painter().rect_filled(
            rect,
            theme::CORNER_SMALL,
            theme::WIDGET_HOVER.gamma_multiply(lit),
        );
    }

    let tint = mix(theme::TEXT_FAINT, theme::TEXT, lit);
    let size = if response.is_pointer_button_down_on() {
        14.0
    } else {
        15.0
    };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(size));
    egui::Image::new(source).tint(tint).paint_at(ui, icon_rect);

    response.on_hover_text(tooltip)
}

pub(crate) fn language_menu(ui: &mut egui::Ui) -> Option<deviceout_i18n::Preference> {
    use deviceout_i18n::Preference;

    let t = deviceout_i18n::t();
    let pref = deviceout_i18n::preference();
    let current = deviceout_i18n::current();
    let mut chosen = None;
    egui::ComboBox::from_id_salt("language")
        .selected_text(native_name_text(current, 11.0, theme::TEXT_DIM))
        .width(0.0)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(pref == Preference::System, t.follow_system)
                .clicked()
            {
                chosen = Some(Preference::System);
            }
            ui.separator();
            for lang in Lang::ALL {
                if ui
                    .selectable_label(
                        pref == Preference::Fixed(lang),
                        native_name_text(lang, 13.0, theme::TEXT),
                    )
                    .clicked()
                {
                    chosen = Some(Preference::Fixed(lang));
                }
            }
        })
        .response
        .on_hover_text(t.language);
    chosen
}

fn native_name_text(lang: Lang, size: f32, color: Color32) -> RichText {
    RichText::new(lang.native_name())
        .font(FontId::new(size, theme::script_family(lang.script())))
        .color(color)
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
