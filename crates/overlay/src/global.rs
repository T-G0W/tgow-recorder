use eframe::egui::{self, Align2, Color32, Event, Key, Modifiers, Rect, RichText, Sense, Slider, Ui, Vec2};
use tgow_common::config::{AccentPreset, Action, Config, Hotkey, HotkeyMode, Hotkeys, NotificationSpeed};
use tgow_common::i18n::{self, LANGUAGES};
use tgow_common::ipc::Status;
use tgow_common::{system, tr, trf, VERSION};

use crate::theme::{self, Theme};
use crate::widgets::{self, field_label, info_line, section};

#[derive(Default)]
pub struct GlobalState {
    pub capturing: Option<Action>,
    pub capture_error: Option<String>,
}

pub enum GlobalAction {
    None,
    ResetPortal,
    Quit,
}

pub fn page(ui: &mut Ui, theme: &Theme, config: &mut Config, state: &mut GlobalState, status: &Status) -> GlobalAction {
    let mut result = GlobalAction::None;
    let main = &mut config.main;

    section(ui, theme, &tr("Appearance"), |ui| {
        field_label(ui, &tr("Accent color"));
        ui.horizontal(|ui| {
            for (preset, label, rgb) in [
                (AccentPreset::Nvidia, "NVIDIA", [118u8, 185, 0]),
                (AccentPreset::Amd, "AMD", [221, 0, 49]),
                (AccentPreset::Intel, "Intel", [8, 109, 183]),
                (AccentPreset::Custom, "Custom", main.accent_rgb),
            ] {
                let selected = main.accent == preset;
                let text = tr(label);
                let text_width = ui.painter().layout_no_wrap(text.clone(), theme::font(15.0), theme::TEXT).size().x;
                let width = (text_width + 52.0).max(120.0);
                let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 40.0), Sense::click());
                let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                ui.painter().rect_filled(rect, theme.radius(6), theme::FIELD_FILL);
                ui.painter().rect_filled(
                    Rect::from_min_size(rect.left_top() + Vec2::new(8.0, 10.0), Vec2::splat(20.0)),
                    theme.radius(4),
                    color,
                );
                ui.painter().text(
                    rect.left_center() + Vec2::new(36.0, 0.0),
                    Align2::LEFT_CENTER,
                    text,
                    theme::font(15.0),
                    theme::TEXT,
                );
                if selected || response.hovered() {
                    ui.painter().rect_stroke(
                        rect,
                        theme.radius(6),
                        egui::Stroke::new(if selected { 2.0 } else { 1.0 }, color),
                        egui::StrokeKind::Inside,
                    );
                }
                if response.clicked() {
                    main.accent = preset;
                }
            }
        });
        if main.accent == AccentPreset::Custom {
            for (i, name) in ["Red", "Green", "Blue"].iter().enumerate() {
                ui.add(Slider::new(&mut main.accent_rgb[i], 0..=255).text(tr(name)));
            }
        }
        ui.checkbox(&mut main.rounded_corners, tr("Use rounded corners"));
        field_label(ui, &tr("Overlay background opacity:"));
        ui.add(Slider::new(&mut main.overlay_opacity, 0.0..=0.95).fixed_decimals(2));
    });

    section(ui, theme, &tr("Startup"), |ui| {
        widgets::labeled_combo(
            ui,
            &tr("Start program on system startup?"),
            &mut main.start_on_login,
            &[(true, tr("Yes")), (false, tr("No"))],
        );
        ui.checkbox(&mut main.show_tray_icon, tr("Show system tray icon"));
    });

    section(ui, theme, &tr("Updates"), |ui| {
        ui.checkbox(&mut main.auto_update, tr("Install updates automatically"));
        ui.label(
            RichText::new(tr("Updates are downloaded from GitHub and installed when nothing is being recorded."))
                .color(theme::TEXT_DIM),
        );
        ui.label(
            RichText::new(tgow_common::trf("Installed version: %s", &[tgow_common::VERSION])).color(theme::TEXT_DIM),
        );
    });

    section(ui, theme, &tr("Keyboard hotkeys"), |ui| {
        widgets::labeled_combo(
            ui,
            &tr("Enable keyboard hotkeys?"),
            &mut main.hotkey_mode,
            &[
                (HotkeyMode::Enabled, tr("Yes")),
                (
                    HotkeyMode::VirtualOnly,
                    tr("Yes, but only grab virtual devices (supports some input remapping software)"),
                ),
                (HotkeyMode::NoGrab, tr("Yes, but don't grab devices (supports all input remapping software)")),
                (HotkeyMode::Disabled, tr("No")),
            ],
        );
        if main.hotkey_mode != HotkeyMode::Disabled {
            ui.add_space(4.0);
            egui::Grid::new("hotkeys").num_columns(2).spacing([24.0, 8.0]).show(ui, |ui| {
                for action in Action::ALL {
                    ui.label(RichText::new(format!("{}:", tr(action.label_key()))).color(theme::TEXT_DIM));
                    let hotkey = main.hotkeys.get(action);
                    let text = if hotkey.is_set() { hotkey.label() } else { tr("None") };
                    if widgets::plain_button(ui, theme, &text, 220.0).clicked() {
                        state.capturing = Some(action);
                        state.capture_error = None;
                    }
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if widgets::plain_button(ui, theme, &tr("Clear hotkeys"), 220.0).clicked() {
                    main.hotkeys.clear();
                }
                if widgets::plain_button(ui, theme, &tr("Reset hotkeys to default"), 260.0).clicked() {
                    main.hotkeys = Hotkeys::default();
                }
            });
            info_line(ui, &tr("Press ESC to go back to the previous page/close the UI."));
        }
    });

    section(ui, theme, &tr("Controller hotkeys"), |ui| {
        widgets::labeled_combo(
            ui,
            &tr("Enable controller hotkeys?"),
            &mut main.controller_hotkeys,
            &[(true, tr("Yes")), (false, tr("No"))],
        );
        if main.controller_hotkeys {
            for (combo, what) in [
                ("Home + Start", "to show/hide the UI"),
                ("Home + Y", "to take a screenshot"),
                ("Home + D-pad up", "to save a replay"),
                ("Home + D-pad down", "to start/stop recording"),
                ("Home + D-pad left", "to turn replay on/off"),
                ("Home + D-pad right", "to save a 1 minute replay"),
            ] {
                ui.label(format!("{} {} {}", tr("Press"), combo, tr(what)));
            }
        }
    });

    section(ui, theme, &tr("Other settings"), |ui| {
        widgets::labeled_combo(
            ui,
            &tr("Notification speed:"),
            &mut main.notification_speed,
            &[(NotificationSpeed::Normal, tr("Normal")), (NotificationSpeed::Fast, tr("Fast"))],
        );
        let mut languages = vec![(String::new(), tr("System language"))];
        for (code, name) in LANGUAGES {
            languages.push((code.to_string(), name.to_string()));
        }
        if widgets::labeled_combo(ui, &tr("Language:"), &mut main.language, &languages) {
            i18n::set_language(&main.language);
        }
        ui.checkbox(
            &mut main.exclude_metadata,
            tr("Exclude personal metadata from the video (such as audio track title)"),
        );
        widgets::labeled_number(ui, &tr("Countdown before recording (seconds):"), &mut main.countdown_seconds, 0, 10);
        widgets::labeled_number(
            ui,
            &tr("Warn when free disk space is below (GB):"),
            &mut main.low_disk_warning_gb,
            0,
            2000,
        );
        ui.checkbox(&mut main.play_sounds, tr("Play a sound when saving"));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if widgets::plain_button(ui, theme, &tr("Reset portal selection"), 280.0).clicked() {
                result = GlobalAction::ResetPortal;
            }
        });
    });

    section(ui, theme, &tr("Application info"), |ui| {
        ui.label(trf("Version: %s", &[VERSION]));
        let vendor = if status.gpu_vendor.is_empty() { system::gpu_vendor() } else { status.gpu_vendor.clone() };
        ui.label(trf("GPU vendor: %s", &[&vendor]));
        let session = if system::is_wayland() { "Wayland" } else { "X11" };
        ui.label(trf("Session: %s", &[session]));
        ui.label("T-GOW");
        ui.add_space(6.0);
        if widgets::plain_button(ui, theme, &tr("Exit program"), 220.0).clicked() {
            result = GlobalAction::Quit;
        }
    });

    result
}

fn key_name(key: Key) -> String {
    key.name().to_string()
}

fn is_alphanumeric(key: Key) -> bool {
    let name = key.name();
    name.chars().count() == 1 && name.chars().all(|c| c.is_ascii_alphanumeric())
}

fn modifier_list(m: Modifiers) -> Vec<String> {
    let mut list = Vec::new();
    if m.ctrl {
        list.push("Ctrl".to_string());
    }
    if m.shift {
        list.push("Shift".to_string());
    }
    if m.alt {
        list.push("Alt".to_string());
    }
    list
}

pub fn capture_dialog(ctx: &egui::Context, ui: &mut Ui, theme: &Theme, hotkeys: &mut Hotkeys, state: &mut GlobalState) {
    let Some(action) = state.capturing else { return };
    let screen = ui.max_rect();
    let blocker = ui.interact(screen, ui.scope_id().with("hotkey-blocker"), Sense::click());
    let _ = blocker;
    theme::dim_background(ui.painter(), screen, 0.6);
    let rect = Rect::from_center_size(screen.center(), Vec2::new(760.0, 230.0));
    ui.painter().rect_filled(rect, theme.radius(10), Color32::from_rgb(20, 20, 23));
    ui.painter().rect_filled(
        Rect::from_min_size(rect.left_top(), Vec2::new(rect.width(), 4.0)),
        theme.radius(2),
        theme.accent,
    );
    theme::bold_text(
        ui.painter(),
        rect.center_top() + Vec2::new(0.0, 52.0),
        Align2::CENTER_CENTER,
        &trf("Press a key combination to use for the hotkey: \"%s\"", &[&tr(action.label_key())]),
        19.0,
        theme::TEXT,
    );
    ui.painter().text(
        rect.center_top() + Vec2::new(0.0, 100.0),
        Align2::CENTER_CENTER,
        tr("Alpha-numerical keys can't be used alone in hotkeys, they have to be used one or more of these keys: Alt, Ctrl, Shift."),
        theme::font(14.0),
        theme::TEXT_DIM,
    );
    ui.painter().text(
        rect.center_top() + Vec2::new(0.0, 126.0),
        Align2::CENTER_CENTER,
        tr("Press Esc to cancel or Backspace to remove the hotkey."),
        theme::font(14.0),
        theme::TEXT_DIM,
    );
    let mods = ctx.input(|i| i.modifiers);
    let held = modifier_list(mods).join(" + ");
    ui.painter().text(
        rect.center_top() + Vec2::new(0.0, 168.0),
        Align2::CENTER_CENTER,
        if held.is_empty() { "...".to_string() } else { format!("{held} + ...") },
        theme::font(20.0),
        theme.accent,
    );
    if let Some(err) = &state.capture_error {
        ui.painter().text(
            rect.center_bottom() - Vec2::new(0.0, 22.0),
            Align2::CENTER_CENTER,
            err,
            theme::font(15.0),
            theme::DANGER,
        );
    }

    let events = ctx.input(|i| i.events.clone());
    for event in events {
        let Event::Key { key, pressed: true, modifiers, repeat: false, .. } = event else { continue };
        if key == Key::Escape {
            state.capturing = None;
            state.capture_error = None;
            break;
        }
        if key == Key::Backspace && !modifiers.any() {
            hotkeys.set(action, Hotkey::none());
            state.capturing = None;
            state.capture_error = None;
            break;
        }
        let mods = modifier_list(modifiers);
        if is_alphanumeric(key) && mods.is_empty() {
            state.capture_error = Some(tr("Alpha-numerical keys can't be used alone in hotkeys"));
            continue;
        }
        let candidate = Hotkey { key: key_name(key), modifiers: mods };
        if let Some(other) = hotkeys.conflict(action, &candidate) {
            state.capture_error = Some(format!(
                "{} ({})",
                trf("The hotkey %s is already used for something else", &[&candidate.label()]),
                tr(other.label_key())
            ));
            continue;
        }
        hotkeys.set(action, candidate);
        state.capturing = None;
        state.capture_error = None;
        break;
    }
}
