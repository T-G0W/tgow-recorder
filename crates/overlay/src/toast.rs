use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Vec2};
use tgow_common::ipc::ToastKind;
use tgow_common::Config;

use crate::icons;
use crate::theme::{self, Theme};

pub const WIDTH: f32 = 420.0;
pub const HEIGHT: f32 = 88.0;
pub const RADIUS: f32 = 10.0;
const SLIDE: f32 = 0.3;
const BACKGROUND: Color32 = Color32::from_rgb(16, 16, 18);

pub struct ToastApp {
    kind: ToastKind,
    text: String,
    theme: Theme,
    born: Instant,
    hold: f32,
    placed: bool,
}

impl ToastApp {
    pub fn new(cc: &eframe::CreationContext<'_>, kind: ToastKind, text: String, hold_ms: u64) -> Self {
        let config = Config::load();
        let theme = Theme::from_config(&config.main);
        theme.apply(&cc.egui_ctx);
        Self {
            kind,
            text,
            theme,
            born: Instant::now(),
            hold: (hold_ms.clamp(500, 15_000) as f32) / 1000.0,
            placed: false,
        }
    }

    fn color(&self) -> Color32 {
        if self.kind == ToastKind::Error {
            theme::DANGER
        } else {
            self.theme.accent
        }
    }

    fn icon(&self, painter: &egui::Painter, center: Pos2) {
        let color = self.color();
        painter.circle_filled(center, 23.0, color.gamma_multiply(0.18));
        match self.kind {
            ToastKind::Record => icons::record(painter, center, 30.0, color),
            ToastKind::Replay => icons::replay(painter, center, 30.0, color),
            ToastKind::Stream => icons::stream(painter, center, 30.0, color),
            ToastKind::Screenshot => icons::camera(painter, center, 28.0, color, BACKGROUND),
            ToastKind::Error => icons::close(painter, center, 18.0, color),
            ToastKind::Info => icons::logo(painter, Rect::from_center_size(center, Vec2::splat(26.0)), color),
        }
    }
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

impl eframe::App for ToastApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        BACKGROUND.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let age = self.born.elapsed().as_secs_f32();
        let total = SLIDE * 2.0 + self.hold;
        if age >= total {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let shown = if age < SLIDE {
            ease(age / SLIDE)
        } else if age > SLIDE + self.hold {
            1.0 - ease((age - SLIDE - self.hold) / SLIDE)
        } else {
            1.0
        };
        crate::platform::place_toast(frame, 1.0 - shown, !self.placed);
        self.placed = true;

        let screen = ui.max_rect();
        let offset = (1.0 - shown) * screen.width();
        let panel = screen.translate(Vec2::new(offset, 0.0));
        let painter = ui.painter();
        painter.rect_filled(panel, CornerRadius::same(RADIUS as u8), BACKGROUND);
        painter.rect_filled(
            Rect::from_min_size(panel.left_top(), Vec2::new(4.0, panel.height())),
            CornerRadius::ZERO,
            self.color(),
        );
        self.icon(painter, Pos2::new(panel.left() + 46.0, panel.center().y));
        let left = panel.left() + 84.0;
        painter.text(
            Pos2::new(left, panel.top() + 14.0),
            Align2::LEFT_TOP,
            "TGOW Recorder",
            theme::font(13.0),
            self.color(),
        );
        let galley = painter.layout(self.text.clone(), theme::font(16.0), theme::TEXT, panel.right() - left - 18.0);
        painter.galley(Pos2::new(left, panel.top() + 34.0), galley, theme::TEXT);

        let still = age > SLIDE && age < SLIDE + self.hold;
        if still {
            let left_in_hold = (SLIDE + self.hold - age).max(0.0);
            ctx.request_repaint_after(Duration::from_secs_f32(left_in_hold.min(0.25)));
        } else {
            ctx.request_repaint_after(Duration::from_millis(15));
        }
    }
}
