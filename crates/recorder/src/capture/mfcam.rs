use anyhow::{anyhow, Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::{GUID, PWSTR};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFAttributes, IMFMediaSource, IMFMediaType, IMFSourceReader, MFCreateAttributes, MFCreateMediaType,
    MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources, MFMediaType_Video, MFStartup, MFVideoFormat_RGB32,
    MFSTARTUP_FULL, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};

use super::camera::{Feed, Image, Spec};

fn init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    }
}

fn string_attr(a: &IMFActivate, key: &GUID) -> String {
    unsafe {
        let mut p = PWSTR::null();
        let mut len = 0u32;
        if a.GetAllocatedString(key, &mut p, &mut len).is_err() || p.is_null() {
            return String::new();
        }
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

fn devices() -> Result<Vec<IMFActivate>> {
    init();
    unsafe {
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.ok_or_else(|| anyhow!("no attributes"))?;
        attrs.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        MFEnumDeviceSources(&attrs, &mut list, &mut count)?;
        let mut out = Vec::new();
        for i in 0..count as usize {
            if let Some(a) = (*list.add(i)).take() {
                out.push(a);
            }
        }
        CoTaskMemFree(Some(list as *const _));
        Ok(out)
    }
}

pub fn list() -> Vec<(String, String)> {
    devices()
        .unwrap_or_default()
        .iter()
        .map(|a| {
            (
                string_attr(a, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK),
                string_attr(a, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME),
            )
        })
        .collect()
}

fn split(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

fn pick_type(reader: &IMFSourceReader, spec: &Spec) -> Option<(u32, u32, u64)> {
    let mut best: Option<(u32, u32, u64, u64)> = None;
    let mut i = 0;
    unsafe {
        while let Ok(t) = reader.GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, i) {
            i += 1;
            let Ok(size) = t.GetUINT64(&MF_MT_FRAME_SIZE) else { continue };
            let rate = t.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or((30u64 << 32) | 1);
            let (w, h) = split(size);
            let (num, den) = split(rate);
            let fps = num as u64 / den.max(1) as u64;
            if spec.width > 0 && w != spec.width {
                continue;
            }
            if spec.height > 0 && h != spec.height {
                continue;
            }
            if spec.fps > 0 && fps != spec.fps as u64 {
                continue;
            }
            let score = (w as u64 * h as u64).min(1920 * 1080) * 1000 + fps;
            if best.map(|b| score > b.3).unwrap_or(true) {
                best = Some((w, h, rate, score));
            }
        }
    }
    best.map(|(w, h, r, _)| (w, h, r))
}

pub fn start(spec: &Spec, feed: Arc<Feed>, stop: Arc<AtomicBool>) -> Result<std::thread::JoinHandle<()>> {
    let wanted = spec.device.clone();
    let spec_owned = Spec {
        device: spec.device.clone(),
        placement: spec.placement.clone(),
        format: spec.format,
        width: spec.width,
        height: spec.height,
        fps: spec.fps,
    };
    let (tx, rx) = std::sync::mpsc::channel::<Result<()>>();
    let handle = std::thread::Builder::new().name("webcam".into()).spawn(move || {
        let opened = (|| -> Result<(IMFSourceReader, u32, u32, i32)> {
            let list = devices()?;
            let activate = list
                .iter()
                .find(|a| {
                    let link = string_attr(a, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK);
                    let name = string_attr(a, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME);
                    link.eq_ignore_ascii_case(&wanted) || name == wanted
                })
                .or_else(|| wanted.parse::<usize>().ok().and_then(|i| list.get(i)))
                .ok_or_else(|| anyhow!("camera \"{wanted}\" was not found"))?;
            unsafe {
                let source: IMFMediaSource = activate.ActivateObject().context("the camera is busy or unavailable")?;
                let mut attrs: Option<IMFAttributes> = None;
                MFCreateAttributes(&mut attrs, 1)?;
                let attrs = attrs.ok_or_else(|| anyhow!("no attributes"))?;
                attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
                let reader = MFCreateSourceReaderFromMediaSource(&source, &attrs)?;
                let (w, h, rate) =
                    pick_type(&reader, &spec_owned).ok_or_else(|| anyhow!("the camera offers no matching format"))?;
                let t: IMFMediaType = MFCreateMediaType()?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                t.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
                t.SetUINT64(&MF_MT_FRAME_RATE, rate)?;
                reader.SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32, None, &t)?;
                let current = reader.GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32)?;
                let stride = current.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or(w as i32 * 4);
                Ok((reader, w, h, stride))
            }
        })();
        let (reader, w, h, stride) = match opened {
            Ok(v) => {
                let _ = tx.send(Ok(()));
                v
            }
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        while !stop.load(Ordering::SeqCst) {
            let mut flags = 0u32;
            let mut sample = None;
            let read = unsafe {
                reader.ReadSample(
                    MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                    0,
                    None,
                    Some(&mut flags),
                    None,
                    Some(&mut sample),
                )
            };
            if read.is_err() {
                break;
            }
            let Some(sample) = sample else { continue };
            let Ok(buffer) = (unsafe { sample.ConvertToContiguousBuffer() }) else { continue };
            unsafe {
                let mut ptr = std::ptr::null_mut();
                let mut len = 0u32;
                if buffer.Lock(&mut ptr, None, Some(&mut len)).is_err() {
                    continue;
                }
                let data = std::slice::from_raw_parts(ptr, len as usize);
                let pitch = stride.unsigned_abs() as usize;
                let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h as usize {
                    let row = if stride < 0 { h as usize - 1 - y } else { y };
                    let start = row * pitch;
                    if start + w as usize * 4 > data.len() {
                        break;
                    }
                    for px in data[start..start + w as usize * 4].as_chunks::<4>().0 {
                        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                    }
                }
                let _ = buffer.Unlock();
                if rgba.len() == (w * h * 4) as usize {
                    feed.set(Image { rgba, width: w, height: h });
                }
            }
        }
    })?;
    match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow!("the camera did not start")),
    }
}
