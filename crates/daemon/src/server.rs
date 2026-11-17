use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use tgow_common::ipc::{self, Event, Listener, Request, Stream};

use crate::daemon::Msg;

pub fn bind() -> anyhow::Result<Listener> {
    if ipc::connect().is_ok() {
        anyhow::bail!("another instance is already running");
    }
    Ok(ipc::bind()?)
}

pub fn serve(listener: Listener, tx: Sender<Msg>) {
    let _ = std::thread::Builder::new().name("ipc".into()).spawn(move || {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let tx = tx.clone();
            let _ = std::thread::Builder::new().name("ipc-client".into()).spawn(move || handle(stream, tx));
        }
    });
}

fn handle(stream: Stream, tx: Sender<Msg>) {
    let Ok(read_half) = stream.try_clone() else { return };
    let mut writer = stream;
    let mut subscribed = false;
    let reader = BufReader::new(read_half);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Some(request) = ipc::decode::<Request>(&line) else {
            if !subscribed {
                let reply = Event::Error { message: "unknown request".into() };
                if writer.write_all(ipc::encode(&reply).as_bytes()).is_err() {
                    break;
                }
            }
            continue;
        };
        if request == Request::Subscribe {
            if subscribed {
                continue;
            }
            let Ok(clone) = writer.try_clone() else { break };
            subscribed = true;
            if tx.send(Msg::Subscribe(clone)).is_err() {
                break;
            }
            continue;
        }
        let (reply_tx, reply_rx) = mpsc::channel();
        if tx.send(Msg::Request(request, reply_tx)).is_err() {
            break;
        }
        let reply = reply_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or(Event::Error { message: "daemon did not answer".into() });
        if !subscribed && writer.write_all(ipc::encode(&reply).as_bytes()).is_err() {
            break;
        }
    }
}
