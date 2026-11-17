use anyhow::{anyhow, Context, Result};
use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType, StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode};
use pipewire as pw;
use pw::spa;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crate::frame::{Frame, FrameSlot, PixFmt};

#[derive(Debug, Clone)]
pub struct PortalRequest {
    pub window: bool,
    pub cursor: bool,
    pub restore: bool,
    pub token_file: PathBuf,
    pub fps: u32,
    pub crop: Option<(i32, i32, u32, u32)>,
}

struct Granted {
    fd: OwnedFd,
    node: u32,
    origin: (i32, i32),
}

async fn negotiate(req: &PortalRequest) -> Result<(Granted, Box<dyn std::any::Any>)> {
    let proxy = Screencast::new().await.context("desktop portal is not available")?;
    let session =
        proxy.create_session(CreateSessionOptions::default()).await.context("could not create a screencast session")?;

    let token = if req.restore {
        std::fs::read_to_string(&req.token_file).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
    } else {
        None
    };
    let cursor = if req.cursor { CursorMode::EMBEDDED } else { CursorMode::HIDDEN };
    let sources = if req.window { SourceType::WINDOW } else { SourceType::MONITOR | SourceType::WINDOW };
    let mut select = SelectSourcesOptions::default()
        .set_cursor_mode(cursor)
        .set_sources(sources)
        .set_multiple(false)
        .set_persist_mode(PersistMode::ExplicitlyRevoked);
    if let Some(t) = token.as_deref() {
        select = select.set_restore_token(t);
    }
    proxy.select_sources(&session, select).await?.response()?;

    let started = proxy
        .start(&session, None, StartCastOptions::default())
        .await?
        .response()
        .map_err(|_| anyhow!("the desktop portal was canceled"))?;

    if let Some(new_token) = started.restore_token() {
        if let Some(dir) = req.token_file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&req.token_file, new_token);
    }
    let stream = started.streams().first().ok_or_else(|| anyhow!("the portal returned no stream"))?;
    let node = stream.pipe_wire_node_id();
    let origin = stream.position().unwrap_or((0, 0));
    let fd = proxy
        .open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default())
        .await
        .context("could not open the pipewire remote")?;
    let keep: Box<dyn std::any::Any> = Box::new((proxy, session));
    Ok((Granted { fd, node, origin }, keep))
}

pub fn spawn(req: PortalRequest, slot: Arc<FrameSlot>, stop: Arc<AtomicBool>) -> Result<std::thread::JoinHandle<()>> {
    let (tx, rx) = mpsc::channel::<Result<()>>();
    let handle = std::thread::Builder::new().name("portal".into()).spawn(move || {
        let negotiated = futures_lite::future::block_on(negotiate(&req));
        let (granted, _keep) = match negotiated {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let _ = tx.send(Ok(()));
        if let Err(e) = run_stream(granted, &req, &slot, &stop) {
            slot.close(format!("{e:#}"));
        } else {
            slot.close("capture stopped");
        }
    })?;
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(())) => return Ok(handle),
            Ok(Err(e)) => return Err(e),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(anyhow!("the desktop portal went away")),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if crate::control::cancelled() {
                    crate::report::finished();
                    std::process::exit(0);
                }
                if std::time::Instant::now() > deadline {
                    return Err(anyhow!("timed out waiting for the desktop portal"));
                }
            }
        }
    }
}

struct StreamState {
    width: u32,
    height: u32,
    format: Option<PixFmt>,
}

fn video_format_pod(fps: u32) -> Result<Vec<u8>> {
    use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    use spa::param::video::VideoFormat;
    let obj = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle { width: 1920, height: 1080 },
            spa::utils::Rectangle { width: 1, height: 1 },
            spa::utils::Rectangle { width: 16384, height: 16384 }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction { num: fps, denom: 1 },
            spa::utils::Fraction { num: 0, denom: 1 },
            spa::utils::Fraction { num: 1000, denom: 1 }
        ),
    );
    let bytes =
        spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &spa::pod::Value::Object(obj))
            .map_err(|e| anyhow!("pod serialize failed: {e:?}"))?
            .0
            .into_inner();
    Ok(bytes)
}

fn run_stream(granted: Granted, req: &PortalRequest, slot: &Arc<FrameSlot>, stop: &Arc<AtomicBool>) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_fd_rc(granted.fd, None)?;

    let stream = pw::stream::StreamBox::new(
        &core,
        "tgow-recorder-capture",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;

    let crop = req.crop.map(|(x, y, w, h)| (x - granted.origin.0, y - granted.origin.1, w, h));
    let slot_cb = slot.clone();
    let slot_state = slot.clone();
    let failed = Arc::new(AtomicBool::new(false));
    let failed_cb = failed.clone();
    let min_gap = Duration::from_secs_f64(1.0 / (req.fps.max(1) as f64 * 1.05));
    let mut last_publish = Instant::now() - Duration::from_secs(1);

    let _listener = stream
        .add_local_listener_with_user_data(StreamState { width: 0, height: 0, format: None })
        .state_changed(move |_, _, _, new| {
            if let pw::stream::StreamState::Error(msg) = new {
                slot_state.close(format!("pipewire stream error: {msg}"));
                failed_cb.store(true, Ordering::SeqCst);
            }
        })
        .param_changed(|_, state, id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let mut info = spa::param::video::VideoInfoRaw::default();
            if info.parse(param).is_err() {
                return;
            }
            state.width = info.size().width;
            state.height = info.size().height;
            state.format = match info.format() {
                spa::param::video::VideoFormat::BGRx | spa::param::video::VideoFormat::BGRA => Some(PixFmt::Bgrx),
                spa::param::video::VideoFormat::RGBx | spa::param::video::VideoFormat::RGBA => Some(PixFmt::Rgbx),
                _ => None,
            };
        })
        .process(move |stream, state| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let Some(format) = state.format else { return };
            if state.width == 0 || state.height == 0 {
                return;
            }
            let now = Instant::now();
            if now.duration_since(last_publish) < min_gap {
                return;
            }
            let datas = buffer.datas_mut();
            let Some(d) = datas.first_mut() else { return };
            let chunk_size = d.chunk().size() as usize;
            let offset = d.chunk().offset() as usize;
            let mut stride = d.chunk().stride().unsigned_abs() as usize;
            if stride == 0 {
                stride = state.width as usize * 4;
            }
            if chunk_size == 0 {
                return;
            }
            let Some(bytes) = d.data() else { return };
            let need = stride * state.height as usize;
            if bytes.len() < offset + need {
                return;
            }
            let mut frame = Frame {
                data: bytes[offset..offset + need].to_vec(),
                width: state.width,
                height: state.height,
                stride,
                format,
                captured: now,
            };
            if let Some((x, y, w, h)) = crop {
                frame = frame.crop(x.max(0) as u32, y.max(0) as u32, w, h);
            }
            last_publish = now;
            slot_cb.publish(frame);
        })
        .register()?;

    let pod_bytes = video_format_pod(req.fps)?;
    let mut params = [spa::pod::Pod::from_bytes(&pod_bytes).ok_or_else(|| anyhow!("bad format pod"))?];
    stream.connect(
        spa::utils::Direction::Input,
        Some(granted.node),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;

    let ml = mainloop.clone();
    let stop_flag = stop.clone();
    let failed_flag = failed.clone();
    let timer = mainloop.loop_().add_timer(move |_| {
        if stop_flag.load(Ordering::SeqCst) || failed_flag.load(Ordering::SeqCst) {
            ml.quit();
        }
    });
    timer
        .update_timer(Some(Duration::from_millis(50)), Some(Duration::from_millis(50)))
        .into_result()
        .map_err(|e| anyhow!("timer setup failed: {e:?}"))?;

    mainloop.run();
    let _ = stream.disconnect();
    if failed.load(Ordering::SeqCst) {
        return Err(anyhow!("the screencast stream failed"));
    }
    Ok(())
}
