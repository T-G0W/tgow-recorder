use std::sync::{Arc, Mutex};

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use tgow_common::{tr, Config};

use crate::backdrop::{Backdrop, Shot};
use crate::theme::{self, Theme};

pub struct RegionApp {
    start: Option<Pos2>,
    current: Option<Pos2>,
    result: Arc<Mutex<Option<String>>>,
    theme: Theme,
    done: bool,
    backdrop: Backdrop,
    focused: bool,
}

impl RegionApp {
    pub fn new(cc: &eframe::CreationContext<'_>, result: Arc<Mutex<Option<String>>>, shot: Option<Shot>) -> Self {
        let config = Config::load();
        tgow_common::i18n::set_language(&config.main.language);
        let theme = Theme::from_config(&config.main);
        theme.apply(&cc.egui_ctx);
        let backdrop = Backdrop::new(&cc.egui_ctx, shot);
        Self { start: None, current: None, result, theme, done: false, backdrop, focused: false }
    }

    fn selection(&self) -> Option<Rect> {
        Some(Rect::from_two_pos(self.start?, self.current?))
    }

    fn finish(&mut self, ctx: &egui::Context, rect: Option<Rect>) {
        if self.done {
            return;
        }
        self.done = true;
        if let Some(rect) = rect.filter(|r| r.width() >= 4.0 && r.height() >= 4.0) {
            let ppp = ctx.pixels_per_point();
            let origin = ctx.input(|i| i.viewport().outer_rect.map(|r| r.min)).unwrap_or(Pos2::ZERO);
            let x = ((rect.min.x + origin.x) * ppp).round() as i64;
            let y = ((rect.min.y + origin.y) * ppp).round() as i64;
            let w = (rect.width() * ppp).round() as i64;
            let h = (rect.height() * ppp).round() as i64;
            if let Ok(mut out) = self.result.lock() {
                *out = Some(format!("{w}x{h}+{x}+{y}"));
            }
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for RegionApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if crate::transparent() {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.focused {
            self.focused = true;
            crate::platform::bring_to_front(frame);
        }
        let screen = ui.max_rect();
        let response = ui.interact(screen, ui.scope_id().with("region"), Sense::drag());
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);

        if response.drag_started() {
            self.start = response.interact_pointer_pos();
            self.current = self.start;
        }
        if response.dragged() {
            self.current = response.interact_pointer_pos().or(self.current);
        }
        if response.drag_stopped() {
            let sel = self.selection();
            self.finish(&ctx, sel);
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.finish(&ctx, None);
            return;
        }
        if ctx.input(|i| i.key_pressed(Key::Enter)) {
            let sel = self.selection();
            self.finish(&ctx, sel);
            return;
        }

        let painter = ui.painter();
        self.backdrop.paint(painter, screen);
        let dim = Color32::from_rgba_premultiplied(0, 0, 0, 120);
        match self.selection() {
            Some(sel) => {
                for part in [
                    Rect::from_min_max(screen.min, Pos2::new(screen.max.x, sel.min.y)),
                    Rect::from_min_max(Pos2::new(screen.min.x, sel.max.y), screen.max),
                    Rect::from_min_max(Pos2::new(screen.min.x, sel.min.y), Pos2::new(sel.min.x, sel.max.y)),
                    Rect::from_min_max(Pos2::new(sel.max.x, sel.min.y), Pos2::new(screen.max.x, sel.max.y)),
                ] {
                    painter.rect_filled(part, CornerRadius::ZERO, dim);
                }
                painter.rect_stroke(sel, CornerRadius::ZERO, Stroke::new(2.0, self.theme.accent), StrokeKind::Outside);
                let ppp = ctx.pixels_per_point();
                let label = format!("{} x {}", (sel.width() * ppp).round() as i64, (sel.height() * ppp).round() as i64);
                let pos = Pos2::new(sel.min.x, (sel.min.y - 8.0).max(screen.min.y + 22.0));
                let bg = painter.text(pos, Align2::LEFT_BOTTOM, &label, theme::font(16.0), Color32::TRANSPARENT);
                painter.rect_filled(bg.expand(5.0), self.theme.radius(4), Color32::from_rgb(16, 16, 18));
                painter.text(pos, Align2::LEFT_BOTTOM, &label, theme::font(16.0), theme::TEXT);
            }
            None => {
                painter.rect_filled(screen, CornerRadius::ZERO, dim);
                let hint = tr("Drag to select a region. Esc cancels.");
                let center = screen.center_top() + Vec2::new(0.0, 60.0);
                let bg = painter.text(center, Align2::CENTER_CENTER, &hint, theme::font(19.0), Color32::TRANSPARENT);
                painter.rect_filled(
                    bg.expand2(Vec2::new(18.0, 10.0)),
                    self.theme.radius(8),
                    Color32::from_rgb(16, 16, 18),
                );
                painter.text(center, Align2::CENTER_CENTER, &hint, theme::font(19.0), theme::TEXT);
            }
        }
        if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
            let guide = Stroke::new(1.0, Color32::from_white_alpha(70));
            painter.line_segment([Pos2::new(screen.min.x, pos.y), Pos2::new(screen.max.x, pos.y)], guide);
            painter.line_segment([Pos2::new(pos.x, screen.min.y), Pos2::new(pos.x, screen.max.y)], guide);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}
