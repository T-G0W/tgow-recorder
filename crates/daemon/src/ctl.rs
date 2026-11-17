use tgow_common::config::Action;
use tgow_common::i18n::clock_text;
use tgow_common::ipc::{self, Event, ModeStatus, Request};

fn usage() -> String {
    let mut text = String::from(
        "usage: tgow-recorder ctl <command>\n\ncommands:\n  status\n  reload-config\n  reset-portal\n  quit\n  save-replay [seconds]\n  start-replay | stop-replay\n  start-record | stop-record\n  start-stream | stop-stream\n",
    );
    for action in Action::ALL {
        text.push_str("  ");
        text.push_str(&action.id().replace('_', "-"));
        text.push('\n');
    }
    text
}

fn parse(args: &[String]) -> Result<Request, String> {
    let Some(cmd) = args.first() else { return Err(usage()) };
    let request = match cmd.as_str() {
        "status" => Request::Status,
        "reload-config" | "reload" => Request::ReloadConfig,
        "reset-portal" => Request::ResetPortal,
        "quit" | "exit" => Request::Quit,
        "show" | "toggle-show" => Request::Run { action: Action::ShowHide },
        "start-replay" => Request::StartReplay,
        "stop-replay" => Request::StopReplay,
        "start-record" => Request::StartRecord,
        "stop-record" => Request::StopRecord,
        "start-stream" => Request::StartStream,
        "stop-stream" => Request::StopStream,
        "save-replay" if args.len() > 1 => {
            let seconds = args[1].parse().map_err(|_| format!("invalid number of seconds: {}", args[1]))?;
            Request::SaveReplay { seconds }
        }
        "help" | "-h" | "--help" => return Err(usage()),
        other => match Action::from_id(other) {
            Some(action) => Request::Run { action },
            None => return Err(format!("unknown command: {other}\n\n{}", usage())),
        },
    };
    Ok(request)
}

fn mode_line(name: &str, m: &ModeStatus) -> String {
    if m.active {
        format!("{name}: on ({}, {})", m.target, clock_text(m.elapsed_secs))
    } else {
        format!("{name}: off")
    }
}

pub fn run(args: &[String]) -> i32 {
    let request = match parse(args) {
        Ok(r) => r,
        Err(text) => {
            eprintln!("{text}");
            return 2;
        }
    };
    match ipc::send(&request) {
        Ok(Event::Status { status }) => {
            println!("{}", mode_line("replay", &status.replay));
            println!("{}", mode_line("record", &status.record));
            println!("{}", mode_line("stream", &status.stream));
            if status.paused {
                println!("recording paused");
            }
            if status.mic_muted {
                println!("microphone muted");
            }
            if let Some(path) = status.last_saved {
                println!("last saved: {path}");
            }
            0
        }
        Ok(Event::Error { message }) => {
            eprintln!("tgow-recorder: {message}");
            1
        }
        Ok(_) => 0,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound || err.kind() == std::io::ErrorKind::ConnectionRefused {
                eprintln!("tgow-recorder: the daemon is not running, start it with `tgow-recorder`");
            } else {
                eprintln!("tgow-recorder: could not talk to the daemon: {err}");
            }
            1
        }
    }
}
