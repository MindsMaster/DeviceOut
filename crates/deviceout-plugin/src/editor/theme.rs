use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use deviceout_i18n::Script;
use nih_plug_egui::egui::{
    self, Color32, CornerRadius, FontId, Margin, Stroke, TextStyle, Vec2, Visuals,
};

pub(crate) const BG: Color32 = Color32::from_rgb(9, 9, 11);
pub(crate) const CARD: Color32 = Color32::from_rgb(16, 16, 20);
pub(crate) const TRACK: Color32 = Color32::from_rgb(39, 39, 42);
pub(crate) const WIDGET: Color32 = Color32::from_rgb(24, 24, 27);
pub(crate) const WIDGET_HOVER: Color32 = Color32::from_rgb(39, 39, 42);
pub(crate) const BORDER: Color32 = Color32::from_rgb(35, 35, 41);
pub(crate) const OUTLINE: Color32 = Color32::from_rgb(39, 39, 42);

pub(crate) const TEXT: Color32 = Color32::from_rgb(244, 244, 245);
pub(crate) const TEXT_DIM: Color32 = Color32::from_rgb(161, 161, 170);
pub(crate) const TEXT_FAINT: Color32 = Color32::from_rgb(113, 113, 122);

pub(crate) const PRIMARY: Color32 = Color32::from_rgb(250, 250, 250);
pub(crate) const PRIMARY_HOVER: Color32 = Color32::from_rgb(228, 228, 231);
pub(crate) const PRIMARY_TEXT: Color32 = Color32::from_rgb(9, 9, 11);

pub(crate) const GREEN: Color32 = Color32::from_rgb(52, 211, 153);
pub(crate) const AMBER: Color32 = Color32::from_rgb(251, 191, 36);
pub(crate) const ORANGE: Color32 = Color32::from_rgb(251, 146, 60);
pub(crate) const RED: Color32 = Color32::from_rgb(248, 113, 113);

pub(crate) const CORNER: CornerRadius = CornerRadius::same(10);
pub(crate) const CORNER_SMALL: CornerRadius = CornerRadius::same(6);
pub(crate) const CORNER_BUTTON: CornerRadius = CornerRadius::same(8);

pub(crate) fn install(ctx: &egui::Context) {
    install_fonts(ctx, deviceout_i18n::current().script());
    egui_extras::install_image_loaders(ctx);

    let mut style = (*ctx.style()).clone();

    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(16.0)),
        (TextStyle::Body, FontId::proportional(13.5)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();

    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(12.0, 7.0);
    style.spacing.interact_size.y = 32.0;

    let mut scroll = egui::style::ScrollStyle::floating();
    scroll.bar_width = 8.0;
    scroll.floating_width = 6.0;
    scroll.bar_inner_margin = 3.0;
    scroll.handle_min_length = 32.0;
    scroll.foreground_color = true;
    scroll.active_handle_opacity = 0.45;
    scroll.interact_handle_opacity = 0.8;
    scroll.active_background_opacity = 0.0;
    scroll.interact_background_opacity = 0.35;
    style.spacing.scroll = scroll;

    let mut visuals = Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = WIDGET;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_corner_radius = CORNER;
    visuals.override_text_color = Some(TEXT);
    visuals.hyperlink_color = TEXT;
    visuals.selection.bg_fill = Color32::from_rgb(63, 63, 70);
    visuals.selection.stroke = Stroke::new(1.0_f32, TEXT_DIM);

    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.noninteractive.corner_radius = CORNER_SMALL;
    visuals.widgets.inactive.weak_bg_fill = WIDGET;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.corner_radius = CORNER_SMALL;
    visuals.widgets.hovered.weak_bg_fill = WIDGET_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.hovered.corner_radius = CORNER_SMALL;
    visuals.widgets.active.weak_bg_fill = WIDGET_HOVER;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.active.corner_radius = CORNER_SMALL;
    visuals.widgets.open = visuals.widgets.hovered;

    style.visuals = visuals;
    ctx.set_style(style);
}

pub(crate) fn root_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(BG)
        .inner_margin(Margin::symmetric(18, 16))
}

pub(crate) fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CORNER)
        .inner_margin(Margin::symmetric(14, 12))
}

type LoadedFont = Option<(String, Arc<egui::FontData>)>;

pub(crate) fn install_fonts(ctx: &egui::Context, script: Script) {
    let mut fonts = egui::FontDefinitions::default();
    let mut names = Vec::new();
    for candidates in [script_fonts(script), script_fonts(Script::Hans)] {
        if let Some((name, data)) = cached_font(candidates) {
            if !names.contains(&name) {
                fonts.font_data.insert(name.clone(), data);
                names.push(name);
            }
        }
    }
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .extend(names.iter().cloned());
    }
    ctx.set_fonts(fonts);
}

fn script_fonts(script: Script) -> &'static [&'static str] {
    match script {
        Script::Hans => &["msyh.ttc", "msyh.ttf", "deng.ttf", "simhei.ttf", "simsun.ttc"],
        Script::Hant => &["msjh.ttc", "msjh.ttf", "mingliu.ttc"],
        Script::Japanese => &["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"],
        Script::Korean => &["malgun.ttf", "gulim.ttc"],
        Script::Thai => &["LeelawUI.ttf", "leelawad.ttf", "tahoma.ttf"],
        Script::Latin => &[],
    }
}

fn cached_font(candidates: &'static [&'static str]) -> LoadedFont {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, LoadedFont>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = candidates.first()?;
    let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
    cache
        .entry(key)
        .or_insert_with(|| load_first_font(candidates))
        .clone()
}

fn load_first_font(candidates: &[&str]) -> LoadedFont {
    let root =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let fonts_dir = root.join("Fonts");
    for file in candidates {
        let Ok(bytes) = std::fs::read(fonts_dir.join(file)) else {
            continue;
        };
        return Some(((*file).to_string(), Arc::new(egui::FontData::from_owned(bytes))));
    }
    None
}
