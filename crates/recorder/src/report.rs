use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use tgow_common::ipc::recorder::Output;

static VERBOSE: AtomicBool = AtomicBool::new(false);

pub fn set_verbose(v: bool) {
    VERBOSE.store(v, Ordering::Relaxed);
}

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

fn emit(out: Output) {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{}", out.line());
    let _ = stdout.flush();
}

pub fn started() {
    emit(Output::Started);
}

pub fn finished() {
    emit(Output::Finished);
}

pub fn paused(p: bool) {
    emit(Output::Paused(p));
}

pub fn saved(kind: &str, path: &Path, seconds: f64) {
    emit(Output::Saved { kind: kind.to_string(), path: path.to_path_buf(), seconds });
}

pub fn error(msg: &str) {
    eprintln!("tgow-rec: error: {msg}");
    emit(Output::Error(msg.to_string()));
}

pub fn warning(msg: &str) {
    eprintln!("tgow-rec: warning: {msg}");
    emit(Output::Warning(msg.to_string()));
}

pub fn info(msg: &str) {
    if verbose() {
        eprintln!("tgow-rec: {msg}");
    }
}
