use eframe::egui::{
    self, Align2, Color32, CornerRadius, DragValue, Margin, Rect, Response, RichText, Sense, Stroke, TextEdit, Ui, Vec2,
};
use tgow_common::tr;

use crate::theme::{self, Theme};

pub fn section(ui: &mut Ui, theme: &Theme, title: &str, body: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::SECTION_FILL)
        .corner_radius(theme.radius(10))
        .inner_margin(Margin::symmetric(22, 18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
            let bar = Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 4.0), Vec2::new(4.0, 22.0));
            ui.painter().rect_filled(bar, CornerRadius::same(2), theme.accent);
            theme::bold_text(
                ui.painter(),
                rect.left_center() + Vec2::new(14.0, 0.0),
                Align2::LEFT_CENTER,
                title,
                20.0,
                theme::TEXT,
            );
            ui.add_space(6.0);
            body(ui);
        });
    ui.add_space(12.0);
}

pub fn field_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::TEXT_DIM));
}

pub fn combo<T: PartialEq + Clone>(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut T,
    options: &[(T, String)],
) -> bool {
    let current = options.iter().find(|(v, _)| v == value).map(|(_, l)| l.clone()).unwrap_or_else(|| "-".to_string());
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).selected_text(current).width(360.0).height(420.0).show_ui(ui, |ui| {
        for (v, label) in options {
            if ui.selectable_label(v == value, label).clicked() && v != value {
                *value = v.clone();
                changed = true;
            }
        }
    });
    changed
}

pub fn labeled_combo<T: PartialEq + Clone>(ui: &mut Ui, label: &str, value: &mut T, options: &[(T, String)]) -> bool {
    field_label(ui, label);
    combo(ui, label, value, options)
}

pub fn number(ui: &mut Ui, value: &mut u32, min: u32, max: u32) -> Response {
    ui.add(DragValue::new(value).range(min..=max).speed(1.0))
}

pub fn labeled_number(ui: &mut Ui, label: &str, value: &mut u32, min: u32, max: u32) -> Response {
    field_label(ui, label);
    number(ui, value, min, max)
}

pub fn size_pair(ui: &mut Ui, w: &mut u32, h: &mut u32) {
    ui.horizontal(|ui| {
        number(ui, w, 0, 16384);
        ui.label("x");
        number(ui, h, 0, 16384);
    });
}

pub fn path_field(ui: &mut Ui, label: &str, value: &mut String) {
    field_label(ui, label);
    ui.horizontal(|ui| {
        ui.add(TextEdit::singleline(value).desired_width(ui.available_width() - 110.0));
        if ui.button(tr("Open")).clicked() {
            let path = tgow_common::paths::expand_tilde(value);
            let _ = tgow_common::paths::ensure_dir(&path);
            tgow_common::system::open_path(&path);
        }
    });
}

pub fn text_field(ui: &mut Ui, label: &str, value: &mut String, secret: bool) {
    field_label(ui, label);
    ui.add(TextEdit::singleline(value).password(secret).desired_width(ui.available_width().min(560.0)));
}

pub fn yes_no_games(ui: &mut Ui, label: &str, value: &mut tgow_common::config::YesNoGames, games_label: &str) -> bool {
    use tgow_common::config::YesNoGames;
    labeled_combo(
        ui,
        label,
        value,
        &[(YesNoGames::No, tr("No")), (YesNoGames::Yes, tr("Yes")), (YesNoGames::GamesOnly, tr(games_label))],
    )
}

pub fn accent_button(ui: &mut Ui, theme: &Theme, text: &str, width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 42.0), Sense::click());
    let fill = if response.hovered() { theme.accent } else { theme.accent.gamma_multiply(0.82) };
    ui.painter().rect_filled(rect, theme.radius(6), fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, theme::font(17.0), Color32::WHITE);
    response
}

pub fn plain_button(ui: &mut Ui, theme: &Theme, text: &str, width: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 42.0), Sense::click());
    let fill = if response.hovered() { theme::FIELD_HOVER } else { theme::FIELD_FILL };
    ui.painter().rect_filled(rect, theme.radius(6), fill);
    if response.hovered() {
        ui.painter().rect_stroke(rect, theme.radius(6), Stroke::new(1.0, theme.accent), egui::StrokeKind::Inside);
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, theme::font(17.0), theme::TEXT);
    response
}

pub fn small_icon_button(
    ui: &mut Ui,
    theme: &Theme,
    size: f32,
    draw: impl FnOnce(&egui::Painter, Rect, Color32),
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let fill = if response.hovered() { theme::FIELD_HOVER } else { theme::FIELD_FILL };
    ui.painter().rect_filled(rect, theme.radius(5), fill);
    let color = if response.hovered() { theme.accent } else { theme::TEXT };
    draw(ui.painter(), rect, color);
    response
}

pub fn info_line(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::TEXT_DIM).size(14.0));
}
