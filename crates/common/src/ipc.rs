use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

use crate::config::Action;
use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Run { action: Action },
    SaveReplay { seconds: u32 },
    StartReplay,
    StopReplay,
    StartRecord,
    StopRecord,
    StartStream,
    StopStream,
    ReloadConfig,
    ResetPortal,
    Subscribe,
    Status,
    OverlayClosed,
    Quit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToastKind {
    #[default]
    Info,
    Record,
    Replay,
    Stream,
    Screenshot,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ModeStatus {
    pub active: bool,
    pub elapsed_secs: f64,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Status {
    pub replay: ModeStatus,
    pub record: ModeStatus,
    pub stream: ModeStatus,
    pub paused: bool,
    pub mic_muted: bool,
    pub replay_buffer_secs: f64,
    pub last_saved: Option<String>,
    pub free_space_bytes: Option<u64>,
    pub gpu_vendor: String,
    pub version: String,
    pub wayland: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Status { status: Status },
    Toast { kind: ToastKind, text: String, path: Option<String> },
    Close,
    Ok,
    Error { message: String },
}

pub fn encode<T: Serialize>(value: &T) -> String {
    let mut s = serde_json::to_string(value).unwrap_or_default();
    s.push('\n');
    s
}

pub fn decode<T: for<'de> Deserialize<'de>>(line: &str) -> Option<T> {
    serde_json::from_str(line.trim()).ok()
}

#[cfg(unix)]
pub type Stream = std::os::unix::net::UnixStream;
#[cfg(unix)]
pub type Listener = std::os::unix::net::UnixListener;

#[cfg(windows)]
pub type Stream = std::net::TcpStream;
#[cfg(windows)]
pub type Listener = std::net::TcpListener;

#[cfg(unix)]
pub fn connect() -> std::io::Result<Stream> {
    Stream::connect(paths::daemon_socket())
}

#[cfg(windows)]
pub fn connect() -> std::io::Result<Stream> {
    let text = std::fs::read_to_string(paths::daemon_port_file())?;
    let port: u16 = text.trim().parse().map_err(|_| std::io::Error::other("bad port file"))?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    Stream::connect_timeout(&addr, Duration::from_millis(500))
}

#[cfg(unix)]
pub fn bind() -> std::io::Result<Listener> {
    let path = paths::daemon_socket();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(&path);
    Listener::bind(path)
}

#[cfg(windows)]
pub fn bind() -> std::io::Result<Listener> {
    let listener = Listener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], 0)))?;
    let port = listener.local_addr()?.port();
    let file = paths::daemon_port_file();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, port.to_string())?;
    Ok(listener)
}

pub fn unbind() {
    let _ = std::fs::remove_file(paths::daemon_socket());
    let _ = std::fs::remove_file(paths::daemon_port_file());
}

pub fn send(request: &Request) -> std::io::Result<Event> {
    let mut stream = connect()?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(encode(request).as_bytes())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    decode(&line).ok_or_else(|| std::io::Error::other("bad reply from daemon"))
}

pub fn daemon_running() -> bool {
    connect().is_ok()
}

pub mod recorder {
    use std::path::PathBuf;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Command {
        Save,
        SaveSeconds(u32),
        Pause,
        Resume,
        TogglePause,
        Mute(usize, bool),
        Marker,
        Stop,
    }

    impl Command {
        pub fn line(&self) -> String {
            match self {
                Command::Save => "save\n".into(),
                Command::SaveSeconds(s) => format!("save {s}\n"),
                Command::Pause => "pause\n".into(),
                Command::Resume => "resume\n".into(),
                Command::TogglePause => "toggle-pause\n".into(),
                Command::Mute(track, on) => format!("mute {track} {}\n", if *on { 1 } else { 0 }),
                Command::Marker => "marker\n".into(),
                Command::Stop => "stop\n".into(),
            }
        }

        pub fn parse(line: &str) -> Option<Command> {
            let mut parts = line.split_whitespace();
            let head = parts.next()?;
            match head {
                "save" => match parts.next() {
                    Some(n) => n.parse().ok().map(Command::SaveSeconds),
                    None => Some(Command::Save),
                },
                "pause" => Some(Command::Pause),
                "resume" => Some(Command::Resume),
                "toggle-pause" => Some(Command::TogglePause),
                "mute" => {
                    let track = parts.next()?.parse().ok()?;
                    let on = parts.next()? == "1";
                    Some(Command::Mute(track, on))
                }
                "marker" => Some(Command::Marker),
                "stop" | "quit" => Some(Command::Stop),
                _ => None,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum Output {
        Started,
        Saved { kind: String, path: PathBuf, seconds: f64 },
        Paused(bool),
        Error(String),
        Warning(String),
        Finished,
    }

    pub const PREFIX: &str = "@tgow ";

    impl Output {
        pub fn line(&self) -> String {
            let body = match self {
                Output::Started => "started".to_string(),
                Output::Saved { kind, path, seconds } => {
                    format!("saved {kind} {seconds:.2} {}", path.display())
                }
                Output::Paused(p) => format!("paused {}", if *p { 1 } else { 0 }),
                Output::Error(m) => format!("error {}", m.replace('\n', " ")),
                Output::Warning(m) => format!("warning {}", m.replace('\n', " ")),
                Output::Finished => "finished".to_string(),
            };
            format!("{PREFIX}{body}")
        }

        pub fn parse(line: &str) -> Option<Output> {
            let body = line.strip_prefix(PREFIX)?;
            let (head, rest) = body.split_once(' ').unwrap_or((body, ""));
            match head {
                "started" => Some(Output::Started),
                "finished" => Some(Output::Finished),
                "paused" => Some(Output::Paused(rest.trim() == "1")),
                "error" => Some(Output::Error(rest.to_string())),
                "warning" => Some(Output::Warning(rest.to_string())),
                "saved" => {
                    let mut it = rest.splitn(3, ' ');
                    let kind = it.next()?.to_string();
                    let seconds = it.next()?.parse().ok()?;
                    let path = PathBuf::from(it.next()?);
                    Some(Output::Saved { kind, path, seconds })
                }
                _ => None,
            }
        }
    }
}
