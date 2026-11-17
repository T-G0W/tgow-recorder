use eframe::egui::IconData;

fn coverage(x: f32) -> f32 {
    (x + 0.5).clamp(0.0, 1.0)
}

fn rounded_rect_distance(x: f32, y: f32, min: f32, max: f32, radius: f32) -> f32 {
    let cx = x.clamp(min + radius, max - radius);
    let cy = y.clamp(min + radius, max - radius);
    let outside = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
    outside - radius
}

pub fn icon() -> IconData {
    let size = 64u32;
    let s = size as f32 / 128.0;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let x = (px as f32 + 0.5) / s;
            let y = (py as f32 + 0.5) / s;
            let body = coverage(-rounded_rect_distance(x, y, 6.0, 122.0, 26.0) * s);
            let t = ((y - 6.0) / 116.0).clamp(0.0, 1.0);
            let top = [143.0, 212.0, 17.0];
            let bottom = [94.0, 148.0, 0.0];
            let mut color = [0.0f32; 3];
            for i in 0..3 {
                color[i] = top[i] + (bottom[i] - top[i]) * t;
            }
            let d = ((x - 64.0).powi(2) + (y - 64.0).powi(2)).sqrt();
            let ring = coverage((4.5 - (d - 38.0).abs()) * s);
            let dot = coverage((22.0 - d) * s);
            let white = ring.max(dot);
            for c in color.iter_mut() {
                *c = *c * (1.0 - white) + 255.0 * white;
            }
            rgba.extend_from_slice(&[color[0] as u8, color[1] as u8, color[2] as u8, (body * 255.0) as u8]);
        }
    }
    IconData { rgba, width: size, height: size }
}
