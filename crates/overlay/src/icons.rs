use std::f32::consts::{PI, TAU};

use eframe::egui::{epaint::PathShape, Color32, CornerRadius, Painter, Pos2, Rect, Stroke, Vec2};

fn arc_points(center: Pos2, radius: f32, from: f32, to: f32, steps: usize) -> Vec<Pos2> {
    (0..=steps)
        .map(|i| {
            let t = from + (to - from) * i as f32 / steps as f32;
            center + Vec2::new(t.cos(), t.sin()) * radius
        })
        .collect()
}

fn arc(painter: &Painter, center: Pos2, radius: f32, from: f32, to: f32, stroke: Stroke) {
    let steps = ((to - from).abs() * radius / 4.0).max(8.0) as usize;
    painter.add(PathShape::line(arc_points(center, radius, from, to, steps), stroke));
}

fn triangle(painter: &Painter, a: Pos2, b: Pos2, c: Pos2, color: Color32) {
    painter.add(PathShape::convex_polygon(vec![a, b, c], color, Stroke::NONE));
}

pub fn replay(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let r = size * 0.5;
    let width = r * 0.13;
    painter.circle_filled(center, r * 0.42, color);
    let gap = 0.38;
    for half in [0.0, PI] {
        let start = -PI / 2.0 + half + gap;
        let end = -PI / 2.0 + half + PI - 0.05;
        arc(painter, center, r * 0.82, start, end, Stroke::new(width, color));
        let tip_angle = start;
        let tip = center + Vec2::new(tip_angle.cos(), tip_angle.sin()) * r * 0.82;
        let tangent = Vec2::new(tip_angle.sin(), -tip_angle.cos());
        let normal = Vec2::new(tip_angle.cos(), tip_angle.sin());
        triangle(painter, tip + tangent * width * 1.9, tip + normal * width * 1.5, tip - normal * width * 1.5, color);
    }
}

pub fn record(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let r = size * 0.5;
    painter.circle_filled(center, r * 0.45, color);
    arc(painter, center, r * 0.82, -PI / 2.0 + 0.55, -PI / 2.0 + TAU - 0.05, Stroke::new(r * 0.13, color));
}

pub fn stream(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let r = size * 0.5;
    let width = r * 0.11;
    let head = center + Vec2::new(0.0, -r * 0.08);
    painter.circle_filled(head, r * 0.22, color);
    let base_top = head + Vec2::new(0.0, r * 0.1);
    painter.add(PathShape::convex_polygon(
        vec![
            base_top + Vec2::new(-r * 0.12, 0.0),
            base_top + Vec2::new(r * 0.12, 0.0),
            base_top + Vec2::new(r * 0.2, r * 0.75),
            base_top + Vec2::new(-r * 0.2, r * 0.75),
        ],
        color,
        Stroke::NONE,
    ));
    for (i, radius) in [0.5f32, 0.78].iter().enumerate() {
        let spread = 0.95 - i as f32 * 0.08;
        arc(painter, head, r * radius, -PI / 2.0 - PI * spread, -PI / 2.0 - PI * 0.3, Stroke::new(width, color));
        arc(painter, head, r * radius, -PI / 2.0 + PI * 0.3, -PI / 2.0 + PI * spread, Stroke::new(width, color));
    }
}

pub fn camera(painter: &Painter, center: Pos2, size: f32, color: Color32, background: Color32) {
    let w = size * 0.95;
    let h = size * 0.68;
    let body = Rect::from_center_size(center + Vec2::new(0.0, size * 0.06), Vec2::new(w, h));
    painter.rect_filled(body, CornerRadius::same((size * 0.12) as u8), color);
    let bump = Rect::from_center_size(Pos2::new(center.x, body.top() - size * 0.05), Vec2::new(w * 0.4, size * 0.16));
    painter.rect_filled(bump, CornerRadius::same((size * 0.05) as u8), color);
    painter.circle_filled(body.center(), h * 0.33, background);
    painter.circle_filled(body.center(), h * 0.2, color);
}

pub fn gear(painter: &Painter, center: Pos2, size: f32, color: Color32, background: Color32) {
    let r = size * 0.5;
    let teeth = 8;
    for i in 0..teeth {
        let a = i as f32 * TAU / teeth as f32;
        let dir = Vec2::new(a.cos(), a.sin());
        let side = Vec2::new(-a.sin(), a.cos());
        let inner = center + dir * r * 0.55;
        let outer = center + dir * r * 0.95;
        painter.add(PathShape::convex_polygon(
            vec![inner - side * r * 0.2, inner + side * r * 0.2, outer + side * r * 0.14, outer - side * r * 0.14],
            color,
            Stroke::NONE,
        ));
    }
    painter.circle_filled(center, r * 0.68, color);
    painter.circle_filled(center, r * 0.3, background);
}

pub fn gallery(painter: &Painter, center: Pos2, size: f32, color: Color32, background: Color32) {
    let s = size * 0.5;
    let back = Rect::from_center_size(center + Vec2::new(s * 0.18, -s * 0.18), Vec2::splat(s * 1.4));
    painter.rect_stroke(
        back,
        CornerRadius::same((s * 0.12) as u8),
        Stroke::new(s * 0.1, color),
        eframe::egui::StrokeKind::Middle,
    );
    let front = Rect::from_center_size(center + Vec2::new(-s * 0.18, s * 0.18), Vec2::splat(s * 1.4));
    painter.rect_filled(front.expand(s * 0.08), CornerRadius::same((s * 0.14) as u8), background);
    painter.rect_filled(front, CornerRadius::same((s * 0.12) as u8), color);
    let base = front.bottom() - s * 0.25;
    triangle(
        painter,
        Pos2::new(front.left() + s * 0.2, base),
        Pos2::new(front.left() + s * 0.65, front.top() + s * 0.55),
        Pos2::new(front.left() + s * 1.05, base),
        background,
    );
    painter.circle_filled(Pos2::new(front.right() - s * 0.4, front.top() + s * 0.4), s * 0.14, background);
}

pub fn close(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let h = size * 0.5;
    let stroke = Stroke::new(size * 0.11, color);
    painter.line_segment([center + Vec2::new(-h, -h), center + Vec2::new(h, h)], stroke);
    painter.line_segment([center + Vec2::new(h, -h), center + Vec2::new(-h, h)], stroke);
}

pub fn logo(painter: &Painter, rect: Rect, accent: Color32) {
    let radius = CornerRadius::same((rect.width() * 0.22) as u8);
    painter.rect_filled(rect, radius, accent);
    let inner = rect.shrink(rect.width() * 0.16);
    painter.rect_filled(inner, CornerRadius::same((rect.width() * 0.14) as u8), Color32::from_rgb(16, 16, 18));
    painter.circle_filled(inner.center(), inner.width() * 0.26, Color32::WHITE);
    painter.circle_stroke(inner.center(), inner.width() * 0.38, Stroke::new(rect.width() * 0.05, Color32::WHITE));
}

pub fn pause(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let w = size * 0.22;
    let h = size * 0.8;
    for dx in [-size * 0.2, size * 0.2] {
        painter.rect_filled(
            Rect::from_center_size(center + Vec2::new(dx, 0.0), Vec2::new(w, h)),
            CornerRadius::same(2),
            color,
        );
    }
}
