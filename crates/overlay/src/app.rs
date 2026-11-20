use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Pos2, Rect, ScrollArea, Sense, Ui, UiBuilder, Vec2};
use tgow_common::config::{Action, NotificationSpeed};
use tgow_common::i18n;
use tgow_common::ipc::{Request, ToastKind};
use tgow_common::{system, tr, trf, Config};

use crate::backdrop::{Backdrop, Shot};
use crate::gallery::Gallery;
use crate::global::{self, GlobalAction, GlobalState};
use crate::icons;
use crate::link::Link;
use crate::settings::{self, Mode};
use crate::sources::{self, SharedSources};
use crate::theme::{self, Theme, PANEL_GAP, PANEL_SIZE, SIDE_BUTTON, TOP_BAR_HEIGHT};
use crate::widgets;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Main,
    Settings(Mode),
    Global,
    Gallery,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Replay,
    Record,
    Stream,
}

enum MenuAction {
    Send(Request),
    SendAndClose(Request),
    Open(Page),
}

struct MenuItem {
    label: String,
    action: MenuAction,
    enabled: bool,
}

pub type Pending = Arc<Mutex<Vec<Request>>>;

pub struct OverlayApp {
    config: Config,
    draft: Config,
    theme: Theme,
    page: Page,
    dropdown: Option<Panel>,
    link: Link,
    sources: SharedSources,
    global: GlobalState,
    gallery: Gallery,
    pending: Pending,
    closing: bool,
    backdrop: Backdrop,
    focused: bool,
}

impl OverlayApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        pending: Pending,
        start_page: Option<&str>,
        shot: Option<Shot>,
    ) -> Self {
        let config = Config::load();
        i18n::set_language(&config.main.language);
        let theme = Theme::from_config(&config.main);
        theme.apply(&cc.egui_ctx);
        let link = Link::start(cc.egui_ctx.clone());
        let sources = sources::load(cc.egui_ctx.clone());
        let page = match start_page {
            Some("replay") => Page::Settings(Mode::Replay),
            Some("record") => Page::Settings(Mode::Record),
            Some("stream") => Page::Settings(Mode::Stream),
            Some("screenshot") => Page::Settings(Mode::Screenshot),
            Some("settings") => Page::Global,
            Some("gallery") => Page::Gallery,
            _ => Page::Main,
        };
        let backdrop = Backdrop::new(&cc.egui_ctx, shot);
        Self {
            backdrop,
            focused: false,
            draft: config.clone(),
            config,
            theme,
            page,
            dropdown: None,
            link,
            sources,
            global: GlobalState::default(),
            gallery: Gallery::default(),
            pending,
            closing: false,
        }
    }

    fn close(&mut self, ctx: &egui::Context) {
        if !self.closing {
            self.closing = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn close_then(&mut self, ctx: &egui::Context, request: Request) {
        if let Ok(mut p) = self.pending.lock() {
            p.push(request);
        }
        self.close(ctx);
    }

    fn open_page(&mut self, page: Page) {
        self.dropdown = None;
        self.draft = self.config.clone();
        if page == Page::Gallery {
            self.gallery.loaded = false;
        }
        self.page = page;
    }

    fn go_back(&mut self) {
        self.global.capturing = None;
        self.draft = self.config.clone();
        i18n::set_language(&self.config.main.language);
        self.page = Page::Main;
    }

    fn save_draft(&mut self) {
        self.config = self.draft.clone();
        i18n::set_language(&self.config.main.language);
        match self.config.save() {
            Ok(()) => self.link.send(Request::ReloadConfig),
            Err(err) => self.link.toast(ToastKind::Error, err.to_string()),
        }
        self.page = Page::Main;
    }

    fn sync_theme(&mut self, ctx: &egui::Context) {
        let wanted = match self.page {
            Page::Global => Theme::from_config(&self.draft.main),
            _ => Theme::from_config(&self.config.main),
        };
        if wanted.accent != self.theme.accent || wanted.rounded != self.theme.rounded || wanted.dim != self.theme.dim {
            self.theme = wanted;
            self.theme.apply(ctx);
        }
    }

    fn top_bar(&mut self, ui: &mut Ui, ctx: &egui::Context, screen: Rect) {
        let bar = Rect::from_min_size(screen.min, Vec2::new(screen.width(), TOP_BAR_HEIGHT));
        let painter = ui.painter();
        painter.rect_filled(bar, CornerRadius::ZERO, Color32::from_rgba_premultiplied(6, 6, 7, 215));
        icons::logo(
            painter,
            Rect::from_center_size(Pos2::new(bar.left() + 34.0, bar.center().y), Vec2::splat(42.0)),
            self.theme.accent,
        );
        theme::bold_text(painter, bar.center(), Align2::CENTER_CENTER, "TGOW Recorder", 26.0, theme::TEXT);

        let close_rect = Rect::from_center_size(Pos2::new(bar.right() - 32.0, bar.center().y), Vec2::splat(42.0));
        let response = ui.interact(close_rect, ui.scope_id().with("close"), Sense::click());
        let color = if response.hovered() { self.theme.accent } else { theme::TEXT };
        icons::close(ui.painter(), close_rect.center(), 22.0, color);
        if response.clicked() {
            self.close(ctx);
        }

        let connected = self.link.read(|s| s.connected);
        if !connected {
            ui.painter().text(
                Pos2::new(bar.center().x, bar.bottom() + 18.0),
                Align2::CENTER_CENTER,
                tr("The background service is not running. Start tgow-recorder to record and use hotkeys."),
                theme::font(14.0),
                Color32::from_rgb(230, 180, 60),
            );
        }
    }

    fn main_page(&mut self, ui: &mut Ui, ctx: &egui::Context, screen: Rect) {
        let (status, status_age) = self.link.read(|s| (s.status.clone(), s.status_at.elapsed().as_secs_f64()));
        let group_w = PANEL_SIZE * 3.0 + PANEL_GAP * 2.0;
        let top = screen.top() + TOP_BAR_HEIGHT + (screen.height() * 0.09).max(48.0);
        let left = screen.center().x - group_w / 2.0;

        let live = |m: &tgow_common::ipc::ModeStatus, paused: bool| {
            if m.active && !paused {
                m.elapsed_secs + status_age
            } else {
                m.elapsed_secs
            }
        };

        let replay_status = if status.replay.active { tr("On") } else { tr("Off") };
        let record_status = if status.record.active {
            let clock = i18n::clock_text(live(&status.record, status.paused));
            if status.paused {
                format!("{}  {}", tr("Paused"), clock)
            } else {
                format!("{}  {}", tr("Recording"), clock)
            }
        } else {
            tr("Not recording")
        };
        let stream_status = if status.stream.active {
            format!("{}  {}", tr("Streaming"), i18n::clock_text(live(&status.stream, false)))
        } else {
            tr("Not streaming")
        };

        let panels = [
            (Panel::Replay, tr("Instant Replay"), replay_status, status.replay.active),
            (Panel::Record, tr("Record"), record_status, status.record.active),
            (Panel::Stream, tr("Livestream"), stream_status, status.stream.active),
        ];

        let mut panel_rects = Vec::new();
        let mut clicked_inside_menu = false;
        for (i, (panel, title, text, active)) in panels.iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(left + i as f32 * (PANEL_SIZE + PANEL_GAP), top),
                Vec2::splat(PANEL_SIZE),
            );
            panel_rects.push((*panel, rect));
            let response = ui.interact(rect, ui.scope_id().with(("panel", i)), Sense::click());
            let open = self.dropdown == Some(*panel);
            let hovered = response.hovered();
            let painter = ui.painter();
            let fill = if hovered || open { theme::PANEL_HOVER } else { theme::PANEL_FILL };
            painter.rect_filled(rect, self.theme.radius(8), fill);
            theme::bold_text(
                painter,
                rect.center_top() + Vec2::new(0.0, 28.0),
                Align2::CENTER_CENTER,
                title,
                18.0,
                theme::TEXT,
            );
            let icon_color = if *active { self.theme.accent } else { Color32::WHITE };
            let icon_center = rect.center() - Vec2::new(0.0, 2.0);
            match panel {
                Panel::Replay => icons::replay(painter, icon_center, 98.0, icon_color),
                Panel::Record => {
                    if *active && status.paused {
                        icons::pause(painter, icon_center, 70.0, icon_color);
                    } else {
                        icons::record(painter, icon_center, 98.0, icon_color);
                    }
                }
                Panel::Stream => icons::stream(painter, icon_center, 98.0, icon_color),
            }
            painter.text(
                rect.center_bottom() - Vec2::new(0.0, 26.0),
                Align2::CENTER_CENTER,
                text,
                theme::font(14.0),
                if *active { theme::TEXT } else { theme::TEXT_DIM },
            );
            if hovered || open || *active {
                let bar = Rect::from_min_size(rect.left_bottom() - Vec2::new(0.0, 4.0), Vec2::new(rect.width(), 4.0));
                let color = if hovered || open { self.theme.accent } else { self.theme.accent_soft() };
                painter.rect_filled(bar, CornerRadius::ZERO, color);
            }
            if response.clicked() {
                clicked_inside_menu = true;
                self.dropdown = if open { None } else { Some(*panel) };
            }
        }

        let side_x = left + group_w + 25.0;
        let side_total = SIDE_BUTTON * 3.0 + 8.0 * 2.0;
        let side_top = top + (PANEL_SIZE - side_total) / 2.0;
        let side = [(tr("Take a screenshot"), 0), (tr("Settings"), 1), (tr("Gallery"), 2)];
        for (label, index) in side {
            let rect = Rect::from_min_size(
                Pos2::new(side_x, side_top + index as f32 * (SIDE_BUTTON + 8.0)),
                Vec2::splat(SIDE_BUTTON),
            );
            let response = ui.interact(rect, ui.scope_id().with(("side", index)), Sense::click()).on_hover_text(label);
            let painter = ui.painter();
            let fill = if response.hovered() { theme::PANEL_HOVER } else { theme::PANEL_FILL };
            painter.rect_filled(rect, self.theme.radius(8), fill);
            let color = if response.hovered() { self.theme.accent } else { Color32::WHITE };
            let bg = Color32::from_rgb(10, 10, 11);
            match index {
                0 => icons::camera(painter, rect.center(), 36.0, color, bg),
                1 => icons::gear(painter, rect.center(), 38.0, color, bg),
                _ => icons::gallery(painter, rect.center(), 36.0, color, bg),
            }
            if response.clicked() {
                clicked_inside_menu = true;
                match index {
                    0 => self.close_then(ctx, Request::Run { action: Action::Screenshot }),
                    1 => self.open_page(Page::Global),
                    _ => self.open_page(Page::Gallery),
                }
            }
            if response.secondary_clicked() && index == 0 {
                self.open_page(Page::Settings(Mode::Screenshot));
            }
        }

        if let Some(open) = self.dropdown {
            if let Some((_, rect)) = panel_rects.iter().find(|(p, _)| *p == open) {
                let items = self.menu_items(open, &status);
                if self.dropdown_menu(ui, ctx, *rect, items) {
                    clicked_inside_menu = true;
                }
            }
        }

        if !clicked_inside_menu && self.dropdown.is_some() && ctx.input(|i| i.pointer.any_click()) {
            self.dropdown = None;
        }

        let hotkey = self.config.main.hotkeys.get(Action::ShowHide);
        let mut footer = Vec::new();
        if hotkey.is_set() {
            footer.push(trf("Press %s to open the overlay", &[&hotkey.label()]));
        }
        if let Some(bytes) = status.free_space_bytes {
            footer.push(trf("Free space: %s", &[&system::human_bytes(bytes)]));
        }
        if !footer.is_empty() {
            ui.painter().text(
                screen.center_bottom() - Vec2::new(0.0, 80.0),
                Align2::CENTER_CENTER,
                footer.join("     "),
                theme::font(14.0),
                theme::TEXT_DIM,
            );
        }

        if status.record.active || status.stream.active || status.replay.active {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn menu_items(&self, panel: Panel, status: &tgow_common::ipc::Status) -> Vec<MenuItem> {
        let item = |label: String, action: MenuAction, enabled: bool| MenuItem { label, action, enabled };
        match panel {
            Panel::Replay => {
                let on = status.replay.active;
                let duration = self.config.replay.duration_seconds;
                let mut items = vec![if on {
                    item(tr("Turn off"), MenuAction::Send(Request::Run { action: Action::ToggleReplay }), true)
                } else {
                    item(tr("Turn on"), MenuAction::SendAndClose(Request::Run { action: Action::ToggleReplay }), true)
                }];
                items.push(item(tr("Save"), MenuAction::SendAndClose(Request::Run { action: Action::SaveReplay }), on));
                let short = self.config.replay.short_save_seconds.max(2);
                let short_label =
                    if short == 30 { tr("Save 30 sec") } else { trf("Save %s", &[&i18n::duration_text(short as f64)]) };
                for (seconds, label) in
                    [(short, short_label), (60, tr("Save 1 min")), (300, tr("Save 5 min")), (600, tr("Save 10 min"))]
                {
                    if duration > seconds {
                        items.push(item(label, MenuAction::SendAndClose(Request::SaveReplay { seconds }), on));
                    }
                }
                items.push(item(tr("Settings"), MenuAction::Open(Page::Settings(Mode::Replay)), true));
                items
            }
            Panel::Record => {
                let on = status.record.active;
                vec![
                    if on {
                        item(tr("Stop and save"), MenuAction::Send(Request::Run { action: Action::ToggleRecord }), true)
                    } else {
                        item(tr("Start"), MenuAction::SendAndClose(Request::Run { action: Action::ToggleRecord }), true)
                    },
                    item(
                        if status.paused { tr("Unpause") } else { tr("Pause") },
                        MenuAction::Send(Request::Run { action: Action::TogglePause }),
                        on,
                    ),
                    item(tr("Settings"), MenuAction::Open(Page::Settings(Mode::Record)), true),
                ]
            }
            Panel::Stream => {
                let on = status.stream.active;
                vec![
                    if on {
                        item(tr("Stop"), MenuAction::Send(Request::Run { action: Action::ToggleStream }), true)
                    } else {
                        item(tr("Start"), MenuAction::SendAndClose(Request::Run { action: Action::ToggleStream }), true)
                    },
                    item(tr("Settings"), MenuAction::Open(Page::Settings(Mode::Stream)), true),
                ]
            }
        }
    }

    fn dropdown_menu(&mut self, ui: &mut Ui, ctx: &egui::Context, anchor: Rect, items: Vec<MenuItem>) -> bool {
        let item_h = 46.0;
        let menu = Rect::from_min_size(
            anchor.left_bottom() + Vec2::new(0.0, 4.0),
            Vec2::new(anchor.width(), item_h * items.len() as f32 + 8.0),
        );
        ui.painter().rect_filled(menu, self.theme.radius(8), Color32::from_rgba_premultiplied(0, 0, 0, 225));
        let mut clicked = false;
        let mut chosen = None;
        for (i, item) in items.into_iter().enumerate() {
            let rect = Rect::from_min_size(
                menu.left_top() + Vec2::new(0.0, 4.0 + i as f32 * item_h),
                Vec2::new(menu.width(), item_h),
            );
            let response = ui.interact(rect, ui.scope_id().with(("menu", i)), Sense::click());
            let painter = ui.painter();
            if response.hovered() && item.enabled {
                painter.rect_filled(rect.shrink2(Vec2::new(4.0, 1.0)), self.theme.radius(4), theme::FIELD_FILL);
                painter.rect_filled(
                    Rect::from_min_size(rect.left_top() + Vec2::new(4.0, 6.0), Vec2::new(4.0, item_h - 12.0)),
                    CornerRadius::same(2),
                    self.theme.accent,
                );
            }
            let color = if item.enabled { theme::TEXT } else { Color32::from_rgb(95, 95, 100) };
            painter.text(
                rect.left_center() + Vec2::new(22.0, 0.0),
                Align2::LEFT_CENTER,
                &item.label,
                theme::font(16.0),
                color,
            );
            if response.clicked() {
                clicked = true;
                if item.enabled {
                    chosen = Some(item.action);
                }
            }
        }
        if ctx.input(|i| i.pointer.any_click())
            && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|p| menu.contains(p))
        {
            clicked = true;
        }
        if let Some(action) = chosen {
            self.dropdown = None;
            match action {
                MenuAction::Send(request) => self.link.send(request),
                MenuAction::SendAndClose(request) => self.close_then(ctx, request),
                MenuAction::Open(page) => self.open_page(page),
            }
        }
        clicked
    }

    fn side_column(&mut self, ui: &mut Ui, rect: Rect, title: &str, editable: bool) -> SideResult {
        let mut result = SideResult::None;
        let painter = ui.painter();
        let icon_box = Rect::from_min_size(rect.min, Vec2::new(rect.width(), PANEL_SIZE));
        painter.rect_filled(icon_box, self.theme.radius(8), theme::PANEL_FILL);
        let bg = Color32::from_rgb(10, 10, 11);
        let c = icon_box.center() - Vec2::new(0.0, 12.0);
        match self.page {
            Page::Settings(Mode::Replay) => icons::replay(painter, c, 96.0, Color32::WHITE),
            Page::Settings(Mode::Record) => icons::record(painter, c, 96.0, Color32::WHITE),
            Page::Settings(Mode::Stream) => icons::stream(painter, c, 96.0, Color32::WHITE),
            Page::Settings(Mode::Screenshot) => icons::camera(painter, c, 90.0, Color32::WHITE, bg),
            Page::Gallery => icons::gallery(painter, c, 88.0, Color32::WHITE, bg),
            _ => icons::gear(painter, c, 92.0, Color32::WHITE, bg),
        }
        theme::bold_text(
            painter,
            icon_box.center_bottom() - Vec2::new(0.0, 28.0),
            Align2::CENTER_CENTER,
            title,
            19.0,
            theme::TEXT,
        );
        painter.rect_filled(
            Rect::from_min_size(icon_box.left_bottom() - Vec2::new(0.0, 4.0), Vec2::new(icon_box.width(), 4.0)),
            CornerRadius::ZERO,
            self.theme.accent,
        );

        let below = Rect::from_min_max(Pos2::new(rect.left(), icon_box.bottom() + 16.0), rect.max);
        ui.scope_builder(UiBuilder::new().max_rect(below), |ui| {
            let width = rect.width();
            if let Page::Settings(mode) = self.page {
                if let Some(flag) = settings::advanced_flag(&mut self.draft, mode) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (value, label) in [(false, tr("Simple view")), (true, tr("Advanced view"))] {
                            let (r, resp) = ui.allocate_exact_size(Vec2::new(width / 2.0 - 2.0, 38.0), Sense::click());
                            let selected = *flag == value;
                            let fill = if selected {
                                self.theme.accent.gamma_multiply(0.85)
                            } else if resp.hovered() {
                                theme::FIELD_HOVER
                            } else {
                                theme::FIELD_FILL
                            };
                            ui.painter().rect_filled(r, self.theme.radius(6), fill);
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, label, theme::font(15.0), theme::TEXT);
                            if resp.clicked() {
                                *flag = value;
                            }
                        }
                    });
                    ui.add_space(12.0);
                }
            }
            if editable {
                if widgets::accent_button(ui, &self.theme, &tr("Save"), width).clicked() {
                    result = SideResult::Save;
                }
                ui.add_space(6.0);
                if widgets::plain_button(ui, &self.theme, &tr("Cancel"), width).clicked() {
                    result = SideResult::Back;
                }
            } else if widgets::plain_button(ui, &self.theme, &tr("Back"), width).clicked() {
                result = SideResult::Back;
            }
        });
        result
    }

    fn sub_page(&mut self, ui: &mut Ui, ctx: &egui::Context, screen: Rect) {
        let side_w = 280.0;
        let gap = 28.0;
        let content_w = (screen.width() - side_w - gap - 80.0).clamp(420.0, 960.0);
        let total = side_w + gap + content_w;
        let left = (screen.center().x - total / 2.0).max(screen.left() + 20.0);
        let top = screen.top() + TOP_BAR_HEIGHT + 40.0;
        let bottom = screen.bottom() - 30.0;
        let side_rect = Rect::from_min_max(Pos2::new(left, top), Pos2::new(left + side_w, bottom));
        let content_rect = Rect::from_min_max(Pos2::new(left + side_w + gap, top), Pos2::new(left + total, bottom));

        let (title, editable) = match self.page {
            Page::Settings(mode) => (mode.title(), true),
            Page::Global => (tr("Settings"), true),
            Page::Gallery => (tr("Gallery"), false),
            Page::Main => (String::new(), false),
        };
        match self.side_column(ui, side_rect, &title, editable) {
            SideResult::Save => {
                self.save_draft();
                return;
            }
            SideResult::Back => {
                self.go_back();
                return;
            }
            SideResult::None => {}
        }

        let status = self.link.read(|s| s.status.clone());
        let sources = self.sources.lock().map(|s| s.clone()).unwrap_or_default();
        let mut global_action = GlobalAction::None;
        let mut close_after = false;
        ui.scope_builder(UiBuilder::new().max_rect(content_rect), |ui| {
            ScrollArea::vertical().id_salt(("page", page_key(self.page))).auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(content_rect.width() - 16.0);
                match self.page {
                    Page::Settings(mode) => {
                        if !sources.loaded {
                            widgets::info_line(ui, &tr("Loading devices..."));
                        }
                        settings::page(ui, &self.theme, &mut self.draft, mode, &sources);
                    }
                    Page::Global => {
                        global_action = global::page(ui, &self.theme, &mut self.draft, &mut self.global, &status);
                    }
                    Page::Gallery => {
                        let config = self.config.clone();
                        if self.gallery.ui(ui, &self.theme, &config, status.free_space_bytes) {
                            close_after = true;
                        }
                    }
                    Page::Main => {}
                }
                ui.add_space(30.0);
            });
        });

        match global_action {
            GlobalAction::ResetPortal => self.link.send(Request::ResetPortal),
            GlobalAction::Quit => {
                self.link.send_blocking(Request::Quit);
                self.close(ctx);
            }
            GlobalAction::None => {}
        }
        if close_after {
            self.close(ctx);
        }
    }

    fn toasts(&mut self, ui: &mut Ui, ctx: &egui::Context, screen: Rect) {
        let life = match self.config.main.notification_speed {
            NotificationSpeed::Normal => 4.5,
            NotificationSpeed::Fast => 2.5,
        };
        let toasts: Vec<(ToastKind, String, f32)> = self.link.with(|s| {
            s.toasts.retain(|t| t.born.elapsed().as_secs_f32() < life);
            s.toasts.iter().map(|t| (t.kind, t.text.clone(), t.born.elapsed().as_secs_f32())).collect()
        });
        if toasts.is_empty() {
            return;
        }
        let width = 440.0;
        let mut y = screen.top() + TOP_BAR_HEIGHT + 16.0;
        for (kind, text, age) in toasts.iter().rev().take(4) {
            let fade = ((life - age) / 0.35).clamp(0.0, 1.0) * (age / 0.15).clamp(0.0, 1.0);
            let painter = ui.painter();
            let galley =
                painter.layout(text.clone(), theme::font(16.0), theme::TEXT.gamma_multiply(fade), width - 46.0);
            let height = galley.size().y + 28.0;
            let slide = (1.0 - (age / 0.2).clamp(0.0, 1.0)) * 40.0;
            let rect =
                Rect::from_min_size(Pos2::new(screen.right() - width - 20.0 + slide, y), Vec2::new(width, height));
            painter.rect_filled(
                rect,
                self.theme.radius(8),
                Color32::from_rgba_premultiplied(8, 8, 9, (235.0 * fade) as u8),
            );
            let stripe = if *kind == ToastKind::Error { theme::DANGER } else { self.theme.accent };
            painter.rect_filled(
                Rect::from_min_size(rect.left_top(), Vec2::new(5.0, rect.height())),
                CornerRadius::same(2),
                stripe.gamma_multiply(fade),
            );
            painter.galley(rect.left_top() + Vec2::new(24.0, 14.0), galley, theme::TEXT);
            y += height + 10.0;
        }
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}

enum SideResult {
    None,
    Save,
    Back,
}

fn page_key(page: Page) -> u8 {
    match page {
        Page::Main => 0,
        Page::Settings(Mode::Replay) => 1,
        Page::Settings(Mode::Record) => 2,
        Page::Settings(Mode::Stream) => 3,
        Page::Settings(Mode::Screenshot) => 4,
        Page::Global => 5,
        Page::Gallery => 6,
    }
}

impl eframe::App for OverlayApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if crate::transparent() {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        }
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.focused {
            self.focused = true;
            crate::platform::bring_to_front(frame);
        }
        if self.link.read(|s| s.close) {
            self.close(&ctx);
        }
        self.sync_theme(&ctx);

        let screen = ui.max_rect();
        self.backdrop.paint(ui.painter(), screen);
        theme::dim_background(ui.painter(), screen, self.theme.dim);
        if self.page != Page::Main {
            theme::dim_background(ui.painter(), screen, 0.25);
        }

        self.top_bar(ui, &ctx, screen);
        match self.page {
            Page::Main => self.main_page(ui, &ctx, screen),
            _ => self.sub_page(ui, &ctx, screen),
        }
        self.toasts(ui, &ctx, screen);

        if self.global.capturing.is_some() {
            let theme = self.theme;
            global::capture_dialog(&ctx, ui, &theme, &mut self.draft.main.hotkeys, &mut self.global);
        } else if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if self.dropdown.is_some() {
                self.dropdown = None;
            } else if self.page != Page::Main {
                self.go_back();
            } else {
                self.close(&ctx);
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            self.closing = true;
        }
    }
}
