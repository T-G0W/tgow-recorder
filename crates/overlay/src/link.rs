use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use tgow_common::ipc::{self, Event, Request, Status, ToastKind};

pub struct Toast {
    pub kind: ToastKind,
    pub text: String,
    pub born: Instant,
}

pub struct Shared {
    pub connected: bool,
    pub status: Status,
    pub status_at: Instant,
    pub toasts: Vec<Toast>,
    pub close: bool,
}

#[derive(Clone)]
pub struct Link {
    shared: Arc<Mutex<Shared>>,
    ctx: egui::Context,
}

impl Link {
    pub fn start(ctx: egui::Context) -> Link {
        let shared = Arc::new(Mutex::new(Shared {
            connected: false,
            status: Status::default(),
            status_at: Instant::now(),
            toasts: Vec::new(),
            close: false,
        }));
        let link = Link { shared, ctx };
        let worker = link.clone();
        std::thread::spawn(move || worker.listen());
        link
    }

    fn listen(&self) {
        loop {
            if let Ok(mut stream) = ipc::connect() {
                if stream.write_all(ipc::encode(&Request::Subscribe).as_bytes()).is_ok() {
                    self.with(|s| s.connected = true);
                    let reader = BufReader::new(stream);
                    for line in reader.lines() {
                        let Ok(line) = line else { break };
                        if let Some(event) = ipc::decode::<Event>(&line) {
                            self.handle(event);
                        }
                    }
                }
                self.with(|s| s.connected = false);
            }
            std::thread::sleep(Duration::from_millis(1500));
        }
    }

    fn handle(&self, event: Event) {
        self.with(|s| match event {
            Event::Status { status } => {
                s.status = status;
                s.status_at = Instant::now();
            }
            Event::Toast { kind, text, .. } => s.toasts.push(Toast { kind, text, born: Instant::now() }),
            Event::Close => s.close = true,
            Event::Error { message } => {
                s.toasts.push(Toast { kind: ToastKind::Error, text: message, born: Instant::now() })
            }
            Event::Ok => {}
        });
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut Shared) -> R) -> R {
        let mut guard = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let out = f(&mut guard);
        drop(guard);
        self.ctx.request_repaint();
        out
    }

    pub fn read<R>(&self, f: impl FnOnce(&Shared) -> R) -> R {
        let guard = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        f(&guard)
    }

    pub fn send(&self, request: Request) {
        let link = self.clone();
        std::thread::spawn(move || match ipc::send(&request) {
            Ok(Event::Error { message }) => link.toast(ToastKind::Error, message),
            Ok(Event::Toast { kind, text, .. }) => link.toast(kind, text),
            _ => {}
        });
    }

    pub fn send_blocking(&self, request: Request) {
        let _ = ipc::send(&request);
    }

    pub fn toast(&self, kind: ToastKind, text: String) {
        self.with(|s| s.toasts.push(Toast { kind, text, born: Instant::now() }));
    }
}
