use eframe::egui::{self, Color32, Painter, Pos2, Rect};

pub struct Shot {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct Backdrop {
    texture: Option<egui::TextureHandle>,
}

impl Backdrop {
    pub fn new(ctx: &egui::Context, shot: Option<Shot>) -> Backdrop {
        let texture = shot.filter(|s| s.rgba.len() == s.width * s.height * 4).map(|s| {
            let image = egui::ColorImage::from_rgba_unmultiplied([s.width, s.height], &s.rgba);
            ctx.load_texture("backdrop", image, egui::TextureOptions::LINEAR)
        });
        Backdrop { texture }
    }

    pub fn paint(&self, painter: &Painter, rect: Rect) {
        if let Some(texture) = &self.texture {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            painter.image(texture.id(), rect, uv, Color32::WHITE);
        }
    }
}
