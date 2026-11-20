#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod app_icon;
mod backdrop;
mod gallery;
mod global;
mod icons;
mod link;
mod platform;
mod region;
mod settings;
mod sources;
mod strings;
mod theme;
mod toast;
mod widgets;

use std::sync::{Arc, Mutex};

use eframe::egui;
use tgow_common::ipc::{self, Request};

const STAGE: &str = "TGOW_OVERLAY_STAGE";

fn stage() -> u32 {
    std::env::var(STAGE).ok().and_then(|s| s.parse().ok()).unwrap_or(0)
}

pub fn transparent() -> bool {
    cfg!(unix) && stage() == 0
}

#[cfg(windows)]
fn renderer() -> eframe::Renderer {
    if stage() == 0 {
        eframe::Renderer::Wgpu
    } else {
        eframe::Renderer::Glow
    }
}

#[cfg(unix)]
fn renderer() -> eframe::Renderer {
    eframe::Renderer::Glow
}

const LAST_STAGE: u32 = 1;

fn finish_options(viewport: egui::ViewportBuilder) -> eframe::NativeOptions {
    #[cfg(windows)]
    {
        let mut options = eframe::NativeOptions { viewport, renderer: renderer(), ..Default::default() };
        options.glow_options.vsync = false;
        options.wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoNoVsync;
        options
    }
    #[cfg(unix)]
    eframe::NativeOptions { viewport, renderer: renderer(), ..Default::default() }
}

fn options(title: &str) -> eframe::NativeOptions {
    let viewport = egui::ViewportBuilder::default()
        .with_title(title)
        .with_app_id("tgow-recorder")
        .with_icon(app_icon::icon())
        .with_transparent(transparent())
        .with_decorations(false)
        .with_always_on_top();
    #[cfg(windows)]
    let viewport = viewport
        .with_position([-32000.0, -32000.0])
        .with_inner_size([1280.0, 720.0])
        .with_resizable(false)
        .with_taskbar(false);
    #[cfg(unix)]
    let viewport = viewport.with_fullscreen(true);
    finish_options(viewport)
}

fn run_toast(args: &[String]) -> i32 {
    let at = args.iter().position(|a| a == "--toast").unwrap_or(0);
    let kind = tgow_common::ipc::ToastKind::from_id(args.get(at + 1).map(String::as_str).unwrap_or("info"));
    let hold = args.get(at + 2).and_then(|v| v.parse().ok()).unwrap_or(3000);
    let text = args.get(at + 3..).map(|rest| rest.join(" ")).unwrap_or_default();
    let viewport = egui::ViewportBuilder::default()
        .with_title("TGOW Recorder notification")
        .with_app_id("tgow-recorder")
        .with_icon(app_icon::icon())
        .with_inner_size([toast::WIDTH, toast::HEIGHT])
        .with_position([-32000.0, -32000.0])
        .with_resizable(false)
        .with_decorations(false)
        .with_always_on_top()
        .with_taskbar(false)
        .with_active(false);
    let result = eframe::run_native(
        "TGOW Recorder notification",
        finish_options(viewport),
        Box::new(move |cc| Ok(Box::new(toast::ToastApp::new(cc, kind, text, hold)))),
    );
    i32::from(result.is_err())
}

fn run_overlay(page: Option<String>) -> i32 {
    let pending: app::Pending = Arc::new(Mutex::new(Vec::new()));
    let shared = pending.clone();
    let shot = if transparent() { None } else { platform::capture_screen() };
    let result = eframe::run_native(
        "TGOW Recorder",
        options("TGOW Recorder"),
        Box::new(move |cc| Ok(Box::new(app::OverlayApp::new(cc, shared, page.as_deref(), shot)))),
    );
    let _ = ipc::send(&Request::OverlayClosed);
    let queued: Vec<Request> = pending.lock().map(|mut p| p.drain(..).collect()).unwrap_or_default();
    if !queued.is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(120));
        for request in queued {
            let _ = ipc::send(&request);
        }
    }
    match result {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("tgow-overlay: {err}");
            1
        }
    }
}

fn run_region_select() -> i32 {
    let result = Arc::new(Mutex::new(None));
    let shared = result.clone();
    let shot = if transparent() { None } else { platform::capture_screen() };
    let run = eframe::run_native(
        "TGOW Recorder region",
        options("TGOW Recorder region"),
        Box::new(move |cc| Ok(Box::new(region::RegionApp::new(cc, shared, shot)))),
    );
    if let Err(err) = run {
        eprintln!("tgow-overlay: {err}");
        return 1;
    }
    match result.lock().ok().and_then(|r| r.clone()) {
        Some(geometry) => {
            println!("{geometry}");
            0
        }
        None => 1,
    }
}

fn retry_next_stage(args: &[String]) -> i32 {
    let next = stage() + 1;
    if next > LAST_STAGE {
        return 1;
    }
    let Ok(exe) = std::env::current_exe() else { return 1 };
    let mut command = std::process::Command::new(exe);
    command.args(args).env(STAGE, next.to_string());
    platform::hide_console(&mut command);
    command.status().ok().and_then(|s| s.code()).unwrap_or(1)
}

fn main() {
    tgow_common::i18n::register_german(strings::GERMAN);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = std::panic::catch_unwind(|| run(&args)).unwrap_or_else(|_| retry_next_stage(&args));
    std::process::exit(code);
}

fn run(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--toast") {
        run_toast(args)
    } else if args.iter().any(|a| a == "--select-region") {
        run_region_select()
    } else if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: tgow-overlay [--select-region] [--toast <kind> <ms> <text>] [--page replay|record|stream|screenshot|settings|gallery]");
        0
    } else if args.iter().any(|a| a == "--version") {
        println!("tgow-overlay {}", tgow_common::VERSION);
        0
    } else {
        let page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).cloned();
        run_overlay(page)
    }
}
