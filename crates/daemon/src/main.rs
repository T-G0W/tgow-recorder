#![cfg_attr(windows, windows_subsystem = "windows")]

mod args;
mod ctl;
mod daemon;
mod games;
#[cfg(unix)]
mod linux;
mod platform;
mod server;
mod state;
mod strings;
#[cfg(windows)]
mod windows;

#[cfg(windows)]
fn attach_console() {
    use ::windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(unix)]
fn attach_console() {}

fn main() {
    tgow_common::i18n::register_german(strings::GERMAN);
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !matches!(args.first().map(String::as_str), None | Some("daemon") | Some("--show")) {
        attach_console();
    }
    match args.first().map(String::as_str) {
        None | Some("daemon") | Some("--show") => {
            let show = args.iter().any(|a| a == "--show");
            if let Err(err) = daemon::run(show) {
                eprintln!("tgow-recorder: {err}");
                std::process::exit(1);
            }
        }
        Some("ctl") => std::process::exit(ctl::run(&args[1..])),
        Some("--version") | Some("-v") => println!("tgow-recorder {}", tgow_common::VERSION),
        Some("--help") | Some("-h") | Some("help") => {
            println!("usage: tgow-recorder [daemon] [--show]\n       tgow-recorder ctl <command>\n       tgow-recorder --version");
        }
        Some(other) => {
            eprintln!("tgow-recorder: unknown argument '{other}', try --help");
            std::process::exit(2);
        }
    }
}
