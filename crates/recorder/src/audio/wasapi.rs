use anyhow::{anyhow, Context, Result};
use std::collections::HashSet;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::{implement, Interface, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, PROPERTYKEY};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, ActivateAudioInterfaceAsync, EDataFlow, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient,
    IAudioClient, IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, IMMEndpoint,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    DEVICE_STATE_ACTIVE, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantToStringAlloc, PROPVARIANT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, IAgileObject, IAgileObject_Impl, CLSCTX_ALL, COINIT_MULTITHREADED,
    STGM_READ,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::Variant::VT_BLOB;

use super::sources::{SourceBuffer, TrackInputs, Want, CHANNELS, RATE};

const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const FRIENDLY_NAME: PROPERTYKEY =
    PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 14 };

fn com_init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).context("audio device enumerator unavailable") }
}

fn take_pwstr(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { p.to_string().unwrap_or_default() };
    unsafe { CoTaskMemFree(Some(p.0 as *const c_void)) };
    s
}

fn device_id(device: &IMMDevice) -> String {
    unsafe { device.GetId().map(take_pwstr).unwrap_or_default() }
}

fn device_name(device: &IMMDevice) -> String {
    unsafe {
        let Ok(store) = device.OpenPropertyStore(STGM_READ) else { return String::new() };
        let Ok(value) = store.GetValue(&FRIENDLY_NAME) else { return String::new() };
        PropVariantToStringAlloc(&value).map(take_pwstr).unwrap_or_default()
    }
}

fn is_render(device: &IMMDevice) -> bool {
    unsafe { device.cast::<IMMEndpoint>().and_then(|e| e.GetDataFlow()).map(|f| f == eRender).unwrap_or(false) }
}

#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub id: String,
    pub description: String,
    pub output: bool,
}

pub fn list_devices() -> Result<Vec<AudioDevice>> {
    com_init();
    let en = enumerator()?;
    let mut out = Vec::new();
    for (flow, output) in [(eRender, true), (eCapture, false)] {
        let collection = unsafe { en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)? };
        let count = unsafe { collection.GetCount()? };
        for i in 0..count {
            let device = unsafe { collection.Item(i)? };
            let id = device_id(&device);
            let name = device_name(&device);
            let description = if output { format!("{name} (loopback)") } else { name };
            out.push(AudioDevice { id, description, output });
        }
    }
    Ok(out)
}

fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let file = path.rsplit('\\').next()?.to_string();
        Some(file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(&file).to_string())
    }
}

fn audio_sessions() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let Ok(en) = enumerator() else { return out };
    let Ok(collection) = (unsafe { en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }) else { return out };
    let count = unsafe { collection.GetCount().unwrap_or(0) };
    let own = std::process::id();
    for i in 0..count {
        let Ok(device) = (unsafe { collection.Item(i) }) else { continue };
        let Ok(manager) = (unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }) else { continue };
        let Ok(sessions) = (unsafe { manager.GetSessionEnumerator() }) else { continue };
        let n = unsafe { sessions.GetCount().unwrap_or(0) };
        for j in 0..n {
            let Ok(control) = (unsafe { sessions.GetSession(j) }) else { continue };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };
            let pid = unsafe { control2.GetProcessId().unwrap_or(0) };
            if pid == 0 || pid == own || out.iter().any(|(p, _)| *p == pid) {
                continue;
            }
            if let Some(name) = process_name(pid) {
                out.push((pid, name));
            }
        }
    }
    out
}

pub fn list_applications() -> Result<Vec<String>> {
    com_init();
    let mut names: Vec<String> = Vec::new();
    for (_, name) in audio_sessions() {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    }
    names.sort_by_key(|s| s.to_lowercase());
    Ok(names)
}

fn format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
        nChannels: CHANNELS as u16,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * CHANNELS as u32 * 4,
        nBlockAlign: (CHANNELS * 4) as u16,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

fn start_client(client: &IAudioClient, loopback: bool) -> Result<IAudioCaptureClient> {
    let mut flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    if loopback {
        flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    }
    let fmt = format();
    unsafe {
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &fmt, None)?;
        let capture = client.GetService::<IAudioCaptureClient>()?;
        client.Start()?;
        Ok(capture)
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct Activated(mpsc::SyncSender<()>);

impl IAgileObject_Impl for Activated_Impl {}

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(
        &self,
        _op: windows::core::Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let _ = self.0.try_send(());
        Ok(())
    }
}

fn process_client(pid: u32, exclude: bool) -> Result<IAudioClient> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: if exclude {
                    PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE
                } else {
                    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE
                },
            },
        },
    };
    let mut prop = ManuallyDrop::new(PROPVARIANT::default());
    unsafe {
        let inner = &mut *prop.Anonymous.Anonymous;
        inner.vt = VT_BLOB;
        inner.Anonymous.blob.cbSize = std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32;
        inner.Anonymous.blob.pBlobData = &mut params as *mut _ as *mut u8;
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(tx).into();
    let op = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*prop as *const PROPVARIANT),
            &handler,
        )?
    };
    rx.recv_timeout(Duration::from_secs(5)).map_err(|_| anyhow!("application audio activation timed out"))?;
    let mut hr = windows::core::HRESULT(0);
    let mut unknown = None;
    unsafe { op.GetActivateResult(&mut hr, &mut unknown)? };
    hr.ok()?;
    let unknown = unknown.ok_or_else(|| anyhow!("no audio client for process {pid}"))?;
    Ok(unknown.cast::<IAudioClient>()?)
}

#[derive(Clone, PartialEq, Eq)]
enum Origin {
    DefaultOutput,
    DefaultInput,
    Device,
    App(u32),
}

struct Live {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    buffer: Arc<SourceBuffer>,
    track: usize,
    origin: Origin,
    device: String,
}

impl Live {
    fn drain(&mut self) -> bool {
        loop {
            let packet = match unsafe { self.capture.GetNextPacketSize() } {
                Ok(n) => n,
                Err(_) => return false,
            };
            if packet == 0 {
                return true;
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            if unsafe { self.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }.is_err() {
                return false;
            }
            let count = frames as usize * CHANNELS;
            {
                let mut q = self.buffer.samples.lock().unwrap();
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    q.extend(std::iter::repeat_n(0.0, count));
                } else {
                    let samples = unsafe { std::slice::from_raw_parts(data as *const f32, count) };
                    q.extend(samples.iter().copied());
                }
                let max = RATE as usize * CHANNELS / 2;
                if q.len() > max {
                    let drop = q.len() - RATE as usize * CHANNELS / 10;
                    q.drain(..drop);
                }
            }
            if unsafe { self.capture.ReleaseBuffer(frames) }.is_err() {
                return false;
            }
        }
    }

    fn stop(&self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

fn default_id(en: &IMMDeviceEnumerator, flow: EDataFlow) -> Option<String> {
    unsafe { en.GetDefaultAudioEndpoint(flow, eConsole).ok().map(|d| device_id(&d)) }
}

fn open_device(
    en: &IMMDeviceEnumerator,
    origin: &Origin,
    name: &str,
) -> Result<(IAudioClient, IAudioCaptureClient, String)> {
    let device = unsafe {
        match origin {
            Origin::DefaultOutput => en.GetDefaultAudioEndpoint(eRender, eConsole)?,
            Origin::DefaultInput => en.GetDefaultAudioEndpoint(eCapture, eConsole)?,
            _ => {
                let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
                en.GetDevice(PCWSTR(wide.as_ptr())).with_context(|| format!("audio device {name} not found"))?
            }
        }
    };
    let loopback = is_render(&device);
    let client = unsafe { device.Activate::<IAudioClient>(CLSCTX_ALL, None)? };
    let capture = start_client(&client, loopback)?;
    Ok((client, capture, device_id(&device)))
}

pub fn spawn(
    tracks: Vec<Vec<Want>>,
    inputs: Vec<Arc<TrackInputs>>,
    stop: Arc<AtomicBool>,
) -> Result<std::thread::JoinHandle<()>> {
    let (tx, rx) = mpsc::channel::<Result<()>>();
    let handle = std::thread::Builder::new().name("audio-capture".into()).spawn(move || {
        com_init();
        let en = match enumerator() {
            Ok(e) => e,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let mut live: Vec<Live> = Vec::new();
        for (ti, wants) in tracks.iter().enumerate() {
            for w in wants {
                let Want::Device(name) = w else { continue };
                let origin = match name.as_str() {
                    "default_output" => Origin::DefaultOutput,
                    "default_input" => Origin::DefaultInput,
                    _ => Origin::Device,
                };
                match open_device(&en, &origin, name) {
                    Ok((client, capture, device)) => {
                        let buffer = SourceBuffer::new();
                        inputs[ti].sources.lock().unwrap().push(buffer.clone());
                        live.push(Live { client, capture, buffer, track: ti, origin, device });
                    }
                    Err(e) => crate::report::warning(&format!("{e:#}")),
                }
            }
        }
        let _ = tx.send(Ok(()));

        let wants_apps = tracks.iter().any(|w| w.iter().any(|x| matches!(x, Want::Apps { .. })));
        let mut last_scan = Instant::now() - Duration::from_secs(10);
        let mut warned_inverse = false;

        while !stop.load(Ordering::SeqCst) {
            let mut dead = Vec::new();
            for (i, l) in live.iter_mut().enumerate() {
                if !l.drain() {
                    dead.push(i);
                }
            }

            if last_scan.elapsed() >= Duration::from_secs(1) {
                last_scan = Instant::now();
                for (i, l) in live.iter_mut().enumerate() {
                    let wanted = match l.origin {
                        Origin::DefaultOutput => default_id(&en, eRender),
                        Origin::DefaultInput => default_id(&en, eCapture),
                        _ => continue,
                    };
                    if wanted.as_deref().is_some_and(|w| w != l.device) || dead.contains(&i) {
                        match open_device(&en, &l.origin, "") {
                            Ok((client, capture, device)) => {
                                l.stop();
                                l.client = client;
                                l.capture = capture;
                                l.device = device;
                                dead.retain(|d| *d != i);
                                crate::report::info("audio moved to the new default device");
                            }
                            Err(e) => crate::report::info(&format!("{e:#}")),
                        }
                    }
                }

                if wants_apps {
                    let sessions = audio_sessions();
                    let running: HashSet<u32> = sessions.iter().map(|(p, _)| *p).collect();
                    for (i, l) in live.iter().enumerate() {
                        if let Origin::App(pid) = l.origin {
                            if !running.contains(&pid) && !dead.contains(&i) {
                                dead.push(i);
                            }
                        }
                    }
                    for (ti, wants) in tracks.iter().enumerate() {
                        for w in wants {
                            let Want::Apps { names, inverse } = w else { continue };
                            let targets: Vec<u32> = sessions
                                .iter()
                                .filter(|(_, n)| names.iter().any(|want| n.to_lowercase() == *want))
                                .map(|(p, _)| *p)
                                .collect();
                            if *inverse {
                                if live.iter().any(|l| l.track == ti && matches!(l.origin, Origin::App(_))) {
                                    continue;
                                }
                                let Some(pid) = targets.first().copied() else { continue };
                                if targets.len() > 1 && !warned_inverse {
                                    warned_inverse = true;
                                    crate::report::warning(
                                        "Windows can leave out only one application, the first match is used",
                                    );
                                }
                                if let Ok(client) = process_client(pid, true) {
                                    if let Ok(capture) = start_client(&client, true) {
                                        let buffer = SourceBuffer::new();
                                        inputs[ti].sources.lock().unwrap().push(buffer.clone());
                                        live.push(Live {
                                            client,
                                            capture,
                                            buffer,
                                            track: ti,
                                            origin: Origin::App(pid),
                                            device: String::new(),
                                        });
                                    }
                                }
                                continue;
                            }
                            for pid in targets {
                                if live.iter().any(|l| l.track == ti && l.origin == Origin::App(pid)) {
                                    continue;
                                }
                                match process_client(pid, false)
                                    .and_then(|c| start_client(&c, true).map(|cap| (c, cap)))
                                {
                                    Ok((client, capture)) => {
                                        let buffer = SourceBuffer::new();
                                        inputs[ti].sources.lock().unwrap().push(buffer.clone());
                                        live.push(Live {
                                            client,
                                            capture,
                                            buffer,
                                            track: ti,
                                            origin: Origin::App(pid),
                                            device: String::new(),
                                        });
                                    }
                                    Err(e) => crate::report::info(&format!("application audio {pid}: {e:#}")),
                                }
                            }
                        }
                    }
                }
            }

            dead.sort_unstable();
            for i in dead.into_iter().rev() {
                if matches!(live[i].origin, Origin::App(_) | Origin::Device) {
                    let l = live.remove(i);
                    l.stop();
                    inputs[l.track].sources.lock().unwrap().retain(|b| !Arc::ptr_eq(b, &l.buffer));
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        for l in &live {
            l.stop();
        }
    })?;
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow!("audio setup timed out")),
    }
}
