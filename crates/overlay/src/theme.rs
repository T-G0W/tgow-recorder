use eframe::egui::{self, Align2, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Stroke, TextStyle, Vec2};
use tgow_common::config::MainConfig;

pub const TOP_BAR_HEIGHT: f32 = 64.0;
pub const PANEL_SIZE: f32 = 215.0;
pub const PANEL_GAP: f32 = 4.0;
pub const SIDE_BUTTON: f32 = 68.0;

pub const TEXT: Color32 = Color32::from_rgb(240, 240, 240);
pub const TEXT_DIM: Color32 = Color32::from_rgb(160, 160, 165);
pub const PANEL_FILL: Color32 = Color32::from_rgba_premultiplied(9, 9, 10, 236);
pub const PANEL_HOVER: Color32 = Color32::from_rgba_premultiplied(24, 24, 26, 242);
pub const SECTION_FILL: Color32 = Color32::from_rgba_premultiplied(14, 14, 16, 232);
pub const FIELD_FILL: Color32 = Color32::from_rgb(38, 38, 42);
pub const FIELD_HOVER: Color32 = Color32::from_rgb(56, 56, 62);
pub const DANGER: Color32 = Color32::from_rgb(220, 60, 60);

#[derive(Clone, Copy)]
pub struct Theme {
    pub accent: Color32,
    pub rounded: bool,
    pub dim: f32,
}

impl Theme {
    pub fn from_config(main: &MainConfig) -> Self {
        let [r, g, b] = main.accent_color();
        Self {
            accent: Color32::from_rgb(r, g, b),
            rounded: main.rounded_corners,
            dim: main.overlay_opacity.clamp(0.0, 0.95),
        }
    }

    pub fn radius(&self, r: u8) -> CornerRadius {
        if self.rounded {
            CornerRadius::same(r)
        } else {
            CornerRadius::ZERO
        }
    }

    pub fn accent_soft(&self) -> Color32 {
        self.accent.gamma_multiply(0.35)
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let accent = self.accent;
        let radius = self.radius(6);
        ctx.all_styles_mut(|style| {
            style.text_styles = [
                (TextStyle::Heading, FontId::new(24.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(17.0, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(17.0, FontFamily::Proportional)),
                (TextStyle::Small, FontId::new(13.0, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(15.0, FontFamily::Monospace)),
            ]
            .into();
            style.spacing.item_spacing = Vec2::new(10.0, 9.0);
            style.spacing.button_padding = Vec2::new(14.0, 7.0);
            style.spacing.interact_size = Vec2::new(40.0, 32.0);
            style.spacing.combo_width = 320.0;
            style.spacing.slider_width = 260.0;
            style.spacing.icon_width = 22.0;
            style.spacing.icon_width_inner = 12.0;

            let v = &mut style.visuals;
            *v = egui::Visuals::dark();
            v.override_text_color = Some(TEXT);
            v.panel_fill = Color32::TRANSPARENT;
            v.window_fill = Color32::from_rgb(24, 24, 27);
            v.window_stroke = Stroke::new(1.0, Color32::from_rgb(52, 52, 58));
            v.window_corner_radius = radius;
            v.menu_corner_radius = radius;
            v.extreme_bg_color = Color32::from_rgb(20, 20, 23);
            v.selection.bg_fill = accent.gamma_multiply(0.75);
            v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
            v.hyperlink_color = accent;
            for w in [
                &mut v.widgets.inactive,
                &mut v.widgets.noninteractive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
                &mut v.widgets.open,
            ] {
                w.corner_radius = radius;
                w.fg_stroke = Stroke::new(1.5, TEXT);
            }
            v.widgets.noninteractive.bg_fill = Color32::TRANSPARENT;
            v.widgets.noninteractive.weak_bg_fill = Color32::TRANSPARENT;
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(48, 48, 54));
            v.widgets.inactive.bg_fill = FIELD_FILL;
            v.widgets.inactive.weak_bg_fill = FIELD_FILL;
            v.widgets.inactive.bg_stroke = Stroke::NONE;
            v.widgets.hovered.bg_fill = FIELD_HOVER;
            v.widgets.hovered.weak_bg_fill = FIELD_HOVER;
            v.widgets.hovered.bg_stroke = Stroke::new(1.0, accent);
            v.widgets.active.bg_fill = accent.gamma_multiply(0.8);
            v.widgets.active.weak_bg_fill = accent.gamma_multiply(0.8);
            v.widgets.active.bg_stroke = Stroke::new(1.0, accent);
            v.widgets.open.bg_fill = FIELD_HOVER;
            v.widgets.open.weak_bg_fill = FIELD_HOVER;
            v.widgets.open.bg_stroke = Stroke::new(1.0, accent);
        });
    }
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold_text(painter: &Painter, pos: Pos2, anchor: Align2, text: &str, size: f32, color: Color32) {
    painter.text(pos + Vec2::new(0.45, 0.0), anchor, text, font(size), color);
    painter.text(pos, anchor, text, font(size), color);
}

pub fn dim_background(painter: &Painter, rect: egui::Rect, amount: f32) {
    let strength = 1.0 - (1.0 - amount.clamp(0.0, 1.0)).powf(2.2);
    let a = (strength * 255.0).round() as u8;
    painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgba_premultiplied(0, 0, 0, a));
}
