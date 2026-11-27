//! Dark modern color palette and reusable egui theme/widget helpers.

use egui::{Color32, CornerRadius, Margin, RichText, Stroke, Vec2, Visuals};

use crate::telemetry::BadgeTone;

pub const BG_CANVAS: Color32 = Color32::from_rgb(0x0B, 0x11, 0x20);
pub const BG_SURFACE: Color32 = Color32::from_rgb(0x11, 0x18, 0x27);
pub const BG_ELEVATED: Color32 = Color32::from_rgb(0x1E, 0x29, 0x3B);
pub const BG_HOVER: Color32 = Color32::from_rgb(0x28, 0x35, 0x48);
pub const BORDER_SUBTLE: Color32 = Color32::from_rgb(0x2A, 0x38, 0x4E);

pub const ACCENT_CYAN: Color32 = Color32::from_rgb(0x38, 0xBD, 0xF8);
pub const ACCENT_CYAN_BORDER: Color32 = Color32::from_rgb(0x1E, 0x60, 0x88);
pub const ACCENT_CYAN_BG: Color32 = Color32::from_rgb(0x13, 0x1D, 0x31);

pub const STATUS_EMERALD: Color32 = Color32::from_rgb(0x10, 0xB9, 0x81);
pub const STATUS_EMERALD_BG: Color32 = Color32::from_rgb(0x0D, 0x2D, 0x24);
pub const STATUS_EMERALD_BTN_BG: Color32 = Color32::from_rgb(0x0E, 0x35, 0x29);
pub const STATUS_EMERALD_TEXT: Color32 = Color32::from_rgb(0xD1, 0xFA, 0xE5);

pub const STATUS_AMBER: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);
pub const STATUS_AMBER_BG: Color32 = Color32::from_rgb(0x2D, 0x21, 0x0B);
pub const STATUS_AMBER_TEXT: Color32 = Color32::from_rgb(0xFD, 0xE6, 0x8A);

pub const STATUS_STOPPED: Color32 = Color32::from_rgb(0xEF, 0x44, 0x44);
pub const STATUS_STOPPED_BG: Color32 = Color32::from_rgb(0x2A, 0x15, 0x1B);
pub const STATUS_STOPPED_BTN_BG: Color32 = Color32::from_rgb(0x33, 0x16, 0x1E);
pub const STATUS_STOPPED_TEXT: Color32 = Color32::from_rgb(0xFE, 0xCA, 0xCA);

pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xF8, 0xFA, 0xFC);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0x94, 0xA3, 0xB8);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x64, 0x74, 0x8B);

pub fn apply_dark_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 7.0);
    style.spacing.slider_width = 240.0;
    style.spacing.interact_size.y = 24.0;

    let mut visuals = Visuals::dark();
    visuals.panel_fill = BG_CANVAS;
    visuals.window_fill = BG_SURFACE;
    visuals.extreme_bg_color = BG_ELEVATED;
    visuals.faint_bg_color = BG_SURFACE;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER_SUBTLE);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.slider_trailing_fill = true;

    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(0x38, 0xBD, 0xF8, 56);
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT_CYAN);

    visuals.widgets.noninteractive.bg_fill = BG_SURFACE;
    visuals.widgets.noninteractive.weak_bg_fill = BG_SURFACE;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER_SUBTLE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_PRIMARY);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);

    visuals.widgets.inactive.bg_fill = BG_ELEVATED;
    visuals.widgets.inactive.weak_bg_fill = BG_ELEVATED;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER_SUBTLE);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.5_f32, TEXT_PRIMARY);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);

    visuals.widgets.hovered.bg_fill = BG_HOVER;
    visuals.widgets.hovered.weak_bg_fill = BG_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT_CYAN);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, TEXT_PRIMARY);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);

    visuals.widgets.active.bg_fill = ACCENT_CYAN_BG;
    visuals.widgets.active.weak_bg_fill = ACCENT_CYAN_BG;
    visuals.widgets.active.bg_stroke = Stroke::new(1.2_f32, ACCENT_CYAN);
    visuals.widgets.active.fg_stroke = Stroke::new(1.5_f32, TEXT_PRIMARY);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);

    visuals.widgets.open.bg_fill = BG_ELEVATED;
    visuals.widgets.open.weak_bg_fill = BG_ELEVATED;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, ACCENT_CYAN_BORDER);
    visuals.widgets.open.fg_stroke = Stroke::new(1.5_f32, TEXT_PRIMARY);
    visuals.widgets.open.corner_radius = CornerRadius::same(8);

    style.visuals = visuals;
    ctx.set_style(style);
}

pub fn card_frame(border_color: Color32, fill_color: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill_color)
        .stroke(Stroke::new(1.0_f32, border_color))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(18, 14))
}

pub fn tile_frame(border_color: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(BG_ELEVATED)
        .stroke(Stroke::new(1.0_f32, border_color))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 10))
}

pub fn tone_colors(tone: BadgeTone) -> (Color32, Color32, Color32, Color32) {
    match tone {
        BadgeTone::Emerald => (
            STATUS_EMERALD_BG,
            STATUS_EMERALD,
            STATUS_EMERALD_TEXT,
            STATUS_EMERALD,
        ),
        BadgeTone::Cyan => (ACCENT_CYAN_BG, ACCENT_CYAN_BORDER, ACCENT_CYAN, ACCENT_CYAN),
        BadgeTone::Amber => (STATUS_AMBER_BG, STATUS_AMBER, STATUS_AMBER_TEXT, STATUS_AMBER),
        BadgeTone::Red => (
            STATUS_STOPPED_BG,
            STATUS_STOPPED,
            STATUS_STOPPED_TEXT,
            STATUS_STOPPED,
        ),
    }
}

pub fn pill_badge(
    ui: &mut egui::Ui,
    label: &str,
    dot_color: Option<Color32>,
    bg: Color32,
    border: Color32,
    text: Color32,
) {
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0_f32, border))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if let Some(dot) = dot_color {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(9.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.2, dot);
                }
                ui.label(RichText::new(label).size(12.0).strong().color(text));
            });
        });
}

pub fn action_button<'a>(
    text: &'a str,
    bg: Color32,
    border: Color32,
    fg: Color32,
) -> egui::Button<'a> {
    egui::Button::new(RichText::new(text).size(12.5).strong().color(fg))
        .fill(bg)
        .stroke(Stroke::new(1.0_f32, border))
        .corner_radius(CornerRadius::same(9))
}
