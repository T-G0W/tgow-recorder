use anyhow::{anyhow, bail, Result};
use libpulse_binding as pa;
use pa::context::{Context, FlagSet as ContextFlags, State as ContextState};
use pa::def::BufferAttr;
use pa::mainloop::standard::{IterateResult, Mainloop};
use pa::sample::{Format, Spec};
use pa::stream::{FlagSet as StreamFlags, PeekResult, State as StreamState, Stream};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::sources::{SourceBuffer, TrackInputs, Want, CHANNELS, RATE};

fn spec() -> Spec {
    Spec { format: Format::F32le, rate: RATE, channels: CHANNELS as u8 }
}

type Shared<T> = Rc<RefCell<T>>;

fn connect(name: &str) -> Result<(Shared<Mainloop>, Shared<Context>)> {
    let mainloop = Rc::new(RefCell::new(Mainloop::new().ok_or_else(|| anyhow!("pulse mainloop failed"))?));
    let context =
        Rc::new(RefCell::new(Context::new(&*mainloop.borrow(), name).ok_or_else(|| anyhow!("pulse context failed"))?));
    context
        .borrow_mut()
        .connect(None, ContextFlags::NOFLAGS, None)
        .map_err(|e| anyhow!("cannot reach the sound server: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match mainloop.borrow_mut().iterate(false) {
            IterateResult::Quit(_) | IterateResult::Err(_) => bail!("sound server connection failed"),
            IterateResult::Success(_) => {}
        }
        match context.borrow().get_state() {
            ContextState::Ready => break,
            ContextState::Failed | ContextState::Terminated => bail!("sound server refused the connection"),
            _ => {}
        }
        if Instant::now() > deadline {
            bail!("timed out connecting to the sound server");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok((mainloop, context))
}

fn wait_flag(mainloop: &Rc<RefCell<Mainloop>>, done: &Rc<RefCell<bool>>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !*done.borrow() && Instant::now() < deadline {
        let _ = mainloop.borrow_mut().iterate(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub id: String,
    pub description: String,
    pub output: bool,
}

pub fn list_devices() -> Result<Vec<AudioDevice>> {
    let (mainloop, context) = connect("tgow-recorder-list")?;
    let devices = Rc::new(RefCell::new(Vec::new()));
    let done = Rc::new(RefCell::new(false));
    {
        let d = devices.clone();
        let f = done.clone();
        let introspect = context.borrow().introspect();
        introspect.get_source_info_list(move |r| match r {
            pa::callbacks::ListResult::Item(info) => {
                let id = info.name.as_deref().unwrap_or_default().to_string();
                let description = info.description.as_deref().unwrap_or(&id).to_string();
                d.borrow_mut().push(AudioDevice { id, description, output: info.monitor_of_sink.is_some() });
            }
            _ => *f.borrow_mut() = true,
        });
    }
    wait_flag(&mainloop, &done);
    context.borrow_mut().disconnect();
    let mut list = devices.borrow().clone();
    list.sort_by_key(|d| !d.output);
    Ok(list)
}

#[derive(Debug, Clone)]
struct SinkInput {
    index: u32,
    app: String,
    sink: u32,
}

fn app_name(info: &pa::context::introspect::SinkInputInfo) -> String {
    let props = &info.proplist;
    props
        .get_str("application.name")
        .or_else(|| props.get_str("application.process.binary"))
        .or_else(|| info.name.as_deref().map(|s| s.to_string()))
        .unwrap_or_default()
}

pub fn list_applications() -> Result<Vec<String>> {
    let (mainloop, context) = connect("tgow-recorder-list")?;
    let names = Rc::new(RefCell::new(Vec::<String>::new()));
    let done = Rc::new(RefCell::new(false));
    {
        let n = names.clone();
        let f = done.clone();
        context.borrow().introspect().get_sink_input_info_list(move |r| match r {
            pa::callbacks::ListResult::Item(info) => {
                let name = app_name(info);
                if !name.is_empty() && !n.borrow().iter().any(|x: &String| x.eq_ignore_ascii_case(&name)) {
                    n.borrow_mut().push(name);
                }
            }
            _ => *f.borrow_mut() = true,
        });
    }
    wait_flag(&mainloop, &done);
    context.borrow_mut().disconnect();
    let mut v = names.borrow().clone();
    v.sort_by_key(|s| s.to_lowercase());
    Ok(v)
}

struct Live {
    stream: Stream,
    buffer: Arc<SourceBuffer>,
    track: usize,
    sink_input: Option<u32>,
    follow: Option<Follow>,
    read_bytes: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Follow {
    DefaultSink,
    DefaultSource,
}

struct Device {
    sink: String,
    source: String,
}

fn open_stream(
    context: &Rc<RefCell<Context>>,
    device: Option<&str>,
    monitor_of: Option<u32>,
    label: &str,
) -> Result<Stream> {
    let mut stream = Stream::new(&mut context.borrow_mut(), label, &spec(), None)
        .ok_or_else(|| anyhow!("could not create an audio stream"))?;
    if let Some(idx) = monitor_of {
        stream.set_monitor_stream(idx).map_err(|e| anyhow!("monitor stream failed: {e}"))?;
    }
    let frag = (RATE as usize * CHANNELS * 4 / 100) as u32;
    let attr =
        BufferAttr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize: frag };
    stream
        .connect_record(device, Some(&attr), StreamFlags::ADJUST_LATENCY)
        .map_err(|e| anyhow!("could not record from {}: {e}", device.unwrap_or("default")))?;
    Ok(stream)
}

fn drain(live: &mut Live) {
    loop {
        let Some(n) = live.stream.readable_size() else { return };
        if n == 0 {
            return;
        }
        match live.stream.peek() {
            Ok(PeekResult::Data(bytes)) => {
                live.read_bytes += bytes.len();
                let mut q = live.buffer.samples.lock().unwrap();
                for c in bytes.as_chunks::<4>().0 {
                    q.push_back(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
                }
                let max = RATE as usize * CHANNELS / 2;
                if q.len() > max {
                    let drop = q.len() - RATE as usize * CHANNELS / 10;
                    q.drain(..drop);
                }
                drop(q);
                let _ = live.stream.discard();
            }
            Ok(PeekResult::Hole(len)) => {
                let mut q = live.buffer.samples.lock().unwrap();
                q.extend(std::iter::repeat_n(0.0, len / 4));
                drop(q);
                let _ = live.stream.discard();
            }
            Ok(PeekResult::Empty) | Err(_) => return,
        }
    }
}

pub fn spawn(
    tracks: Vec<Vec<Want>>,
    inputs: Vec<Arc<TrackInputs>>,
    stop: Arc<AtomicBool>,
) -> Result<std::thread::JoinHandle<()>> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<()>>();
    let handle = std::thread::Builder::new().name("audio-capture".into()).spawn(move || {
        let (mainloop, context) = match connect("tgow-recorder") {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let mut live: Vec<Live> = Vec::new();
        for (ti, wants) in tracks.iter().enumerate() {
            for w in wants {
                if let Want::Device(dev) = w {
                    let dev = &match dev.as_str() {
                        "default_output" => "@DEFAULT_MONITOR@".to_string(),
                        "default_input" => "@DEFAULT_SOURCE@".to_string(),
                        other => other.to_string(),
                    };
                    let follow = match dev.as_str() {
                        "@DEFAULT_MONITOR@" => Some(Follow::DefaultSink),
                        "@DEFAULT_SOURCE@" => Some(Follow::DefaultSource),
                        _ => None,
                    };
                    match open_stream(&context, Some(dev), None, "tgow-recorder-device") {
                        Ok(stream) => {
                            let buffer = SourceBuffer::new();
                            inputs[ti].sources.lock().unwrap().push(buffer.clone());
                            live.push(Live { stream, buffer, track: ti, sink_input: None, follow, read_bytes: 0 });
                        }
                        Err(e) => crate::report::warning(&format!("{e:#}")),
                    }
                }
            }
        }
        let _ = tx.send(Ok(()));

        let wants_apps = tracks.iter().any(|w| w.iter().any(|x| matches!(x, Want::Apps { .. })));
        let sink_inputs: Rc<RefCell<Option<Vec<SinkInput>>>> = Rc::new(RefCell::new(None));
        let monitors: Rc<RefCell<HashMap<u32, String>>> = Rc::new(RefCell::new(HashMap::new()));
        let mut last_scan = Instant::now() - Duration::from_secs(10);
        let follows = live.iter().any(|l| l.follow.is_some());
        let defaults: Rc<RefCell<Option<Device>>> = Rc::new(RefCell::new(None));
        let mut known: Option<(String, String)> = None;
        let mut last_defaults = Instant::now();
        let mut last_report = Instant::now();

        while !stop.load(Ordering::SeqCst) {
            match mainloop.borrow_mut().iterate(false) {
                IterateResult::Quit(_) | IterateResult::Err(_) => break,
                IterateResult::Success(_) => {}
            }
            for l in live.iter_mut() {
                drain(l);
            }
            if crate::report::verbose() && last_report.elapsed() >= Duration::from_secs(1) {
                let secs = last_report.elapsed().as_secs_f64();
                last_report = Instant::now();
                for (i, l) in live.iter_mut().enumerate() {
                    let queued = l.buffer.samples.lock().unwrap().len();
                    crate::report::info(&format!(
                        "audio source {i} (track {}): {:.0} KB/s, {:?}, {} ms queued",
                        l.track,
                        l.read_bytes as f64 / 1024.0 / secs,
                        l.stream.get_state(),
                        queued * 1000 / (RATE as usize * CHANNELS)
                    ));
                    l.read_bytes = 0;
                }
            }

            if follows && last_defaults.elapsed() > Duration::from_millis(1000) {
                last_defaults = Instant::now();
                let slot = defaults.clone();
                context.borrow().introspect().get_server_info(move |info| {
                    *slot.borrow_mut() = Some(Device {
                        sink: info.default_sink_name.as_deref().unwrap_or_default().to_string(),
                        source: info.default_source_name.as_deref().unwrap_or_default().to_string(),
                    });
                });
            }
            if let Some(now) = defaults.borrow_mut().take() {
                let now = (now.sink, now.source);
                if let Some(before) = known.as_ref().filter(|b| **b != now) {
                    for l in live.iter_mut() {
                        let target = match l.follow {
                            Some(Follow::DefaultSink) if before.0 != now.0 && !now.0.is_empty() => {
                                format!("{}.monitor", now.0)
                            }
                            Some(Follow::DefaultSource) if before.1 != now.1 && !now.1.is_empty() => now.1.clone(),
                            _ => continue,
                        };
                        match open_stream(&context, Some(&target), None, "tgow-recorder-device") {
                            Ok(stream) => {
                                let _ = l.stream.disconnect();
                                l.stream = stream;
                                crate::report::info(&format!("audio moved to {target}"));
                            }
                            Err(e) => crate::report::warning(&format!("{e:#}")),
                        }
                    }
                }
                known = Some(now);
            }

            if wants_apps && last_scan.elapsed() > Duration::from_millis(1000) {
                last_scan = Instant::now();
                let introspect = context.borrow().introspect();
                let target = sink_inputs.clone();
                let found = Rc::new(RefCell::new(Vec::new()));
                introspect.get_sink_input_info_list(move |r| match r {
                    pa::callbacks::ListResult::Item(info) => {
                        let app = app_name(info);
                        if !app.eq_ignore_ascii_case("tgow-recorder") {
                            found.borrow_mut().push(SinkInput { index: info.index, app, sink: info.sink });
                        }
                    }
                    _ => *target.borrow_mut() = Some(found.borrow().clone()),
                });
                let mons = monitors.clone();
                introspect.get_sink_info_list(move |r| {
                    if let pa::callbacks::ListResult::Item(info) = r {
                        if let Some(m) = info.monitor_source_name.as_deref() {
                            mons.borrow_mut().insert(info.index, m.to_string());
                        }
                    }
                });
            }

            if let Some(current) = sink_inputs.borrow_mut().take() {
                live.retain(|l| match l.sink_input {
                    Some(idx) => {
                        let alive = current.iter().any(|s| s.index == idx)
                            && !matches!(l.stream.get_state(), StreamState::Failed | StreamState::Terminated);
                        if !alive {
                            let mut srcs = inputs[l.track].sources.lock().unwrap();
                            srcs.retain(|b| !Arc::ptr_eq(b, &l.buffer));
                        }
                        alive
                    }
                    None => true,
                });
                for (ti, wants) in tracks.iter().enumerate() {
                    for w in wants {
                        let Want::Apps { names, inverse } = w else { continue };
                        for si in &current {
                            let hit = names.iter().any(|n| si.app.to_lowercase() == *n);
                            if hit == *inverse {
                                continue;
                            }
                            if live.iter().any(|l| l.track == ti && l.sink_input == Some(si.index)) {
                                continue;
                            }
                            let monitor = monitors.borrow().get(&si.sink).cloned();
                            match open_stream(&context, monitor.as_deref(), Some(si.index), "tgow-recorder-app") {
                                Ok(stream) => {
                                    let buffer = SourceBuffer::new();
                                    inputs[ti].sources.lock().unwrap().push(buffer.clone());
                                    live.push(Live {
                                        stream,
                                        buffer,
                                        track: ti,
                                        sink_input: Some(si.index),
                                        follow: None,
                                        read_bytes: 0,
                                    });
                                }
                                Err(e) => crate::report::info(&format!("app audio {}: {e:#}", si.app)),
                            }
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        for l in live.iter_mut() {
            let _ = l.stream.disconnect();
        }
        context.borrow_mut().disconnect();
    })?;
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow!("audio setup timed out")),
    }
}
