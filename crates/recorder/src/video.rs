use anyhow::{anyhow, bail, Result};
use ff::ffi;
use ff::format::Pixel;
use ff::{Dictionary, Packet, Rational};
use ffmpeg_next as ff;
use std::ptr;

use crate::args::{BitDepth, Codec, FpsMode, Options, Quality, RateMode};
use crate::convert::{self, Layout};
use crate::frame::{Frame, PixFmt};
use crate::report;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Nvenc,
    Vaapi,
    Amf,
    Qsv,
    Vulkan,
    Cpu,
}

impl Backend {
    fn label(self) -> &'static str {
        match self {
            Backend::Nvenc => "nvenc",
            Backend::Vaapi => "vaapi",
            Backend::Amf => "amf",
            Backend::Qsv => "quicksync",
            Backend::Vulkan => "vulkan",
            Backend::Cpu => "cpu",
        }
    }
}

struct Hw {
    device: *mut ffi::AVBufferRef,
    frames: *mut ffi::AVBufferRef,
}

impl Drop for Hw {
    fn drop(&mut self) {
        unsafe {
            ffi::av_buffer_unref(&mut self.frames);
            ffi::av_buffer_unref(&mut self.device);
        }
    }
}

unsafe impl Send for Hw {}

struct Scaler {
    ctx: *mut ffi::SwsContext,
}

impl Drop for Scaler {
    fn drop(&mut self) {
        unsafe { ffi::sws_free_context(&mut self.ctx) };
    }
}

unsafe impl Send for Scaler {}

pub struct VideoEncoder {
    enc: ff::encoder::Video,
    hw: Option<Hw>,
    scaler: Scaler,
    staging: ff::frame::Video,
    pub time_base: Rational,
    pub width: u32,
    pub height: u32,
    pub backend: Backend,
    pub codec_name: String,
    full_range: bool,
    layout: Option<Layout>,
    frames_in: u64,
    pub convert_time: std::time::Duration,
    pub encode_time: std::time::Duration,
}

unsafe impl Send for VideoEncoder {}

pub fn fit_size(src_w: u32, src_h: u32, limit: (u32, u32)) -> (u32, u32) {
    let (mut w, mut h) = (src_w.max(2), src_h.max(2));
    if limit.0 > 0 && limit.1 > 0 && (w > limit.0 || h > limit.1) {
        let scale = (limit.0 as f64 / w as f64).min(limit.1 as f64 / h as f64);
        w = (w as f64 * scale).round() as u32;
        h = (h as f64 * scale).round() as u32;
    }
    (w.max(2) & !1, h.max(2) & !1)
}

fn find_render_node(vendor: &str) -> String {
    let want = match vendor {
        "nvidia" => "0x10de",
        "amd" => "0x1002",
        "intel" => "0x8086",
        _ => "",
    };
    let mut first = None;
    for n in 128..136 {
        let node = format!("/dev/dri/renderD{n}");
        if !std::path::Path::new(&node).exists() {
            continue;
        }
        let v = std::fs::read_to_string(format!("/sys/class/drm/renderD{n}/device/vendor")).unwrap_or_default();
        if first.is_none() {
            first = Some(node.clone());
        }
        if !want.is_empty() && v.trim() == want {
            return node;
        }
    }
    first.unwrap_or_else(|| "/dev/dri/renderD128".into())
}

fn vaapi_node() -> String {
    for vendor in ["amd", "intel"] {
        let node = find_render_node(vendor);
        let v =
            std::fs::read_to_string(format!("/sys/class/drm/{}/device/vendor", node.trim_start_matches("/dev/dri/")))
                .unwrap_or_default();
        if (vendor == "amd" && v.trim() == "0x1002") || (vendor == "intel" && v.trim() == "0x8086") {
            return node;
        }
    }
    find_render_node("")
}

pub fn pick_backend(opts: &Options) -> Backend {
    if opts.cpu_encoder {
        return Backend::Cpu;
    }
    if opts.vulkan {
        return Backend::Vulkan;
    }
    match opts.gpu_api.as_deref() {
        Some("nvenc") => Backend::Nvenc,
        Some("vaapi") => Backend::Vaapi,
        Some("amf") => Backend::Amf,
        Some("qsv") => Backend::Qsv,
        Some("vulkan") => Backend::Vulkan,
        _ => native_backend(),
    }
}

#[cfg(unix)]
fn native_backend() -> Backend {
    if tgow_common::system::gpu_vendor() == "nvidia" {
        Backend::Nvenc
    } else {
        Backend::Vaapi
    }
}

#[cfg(unix)]
fn other_backends(first: Backend) -> Vec<Backend> {
    let native = native_backend();
    if native != first {
        vec![native]
    } else {
        Vec::new()
    }
}

#[cfg(windows)]
fn other_backends(first: Backend) -> Vec<Backend> {
    let vendors = tgow_common::platform::gpu_vendors();
    let mut out = Vec::new();
    for (vendor, backend) in [("nvidia", Backend::Nvenc), ("intel", Backend::Qsv), ("amd", Backend::Amf)] {
        if backend != first && vendors.contains(&vendor) && !out.contains(&backend) {
            out.push(backend);
        }
    }
    out
}

#[cfg(windows)]
fn native_backend() -> Backend {
    match tgow_common::system::gpu_vendor().as_str() {
        "nvidia" => Backend::Nvenc,
        "amd" => Backend::Amf,
        "intel" => Backend::Qsv,
        _ => Backend::Cpu,
    }
}

const CPU_H264: [&str; 3] = ["libx264", "libopenh264", "h264_mf"];

fn encoder_names(codec: Codec, backend: Backend) -> Vec<&'static str> {
    if backend == Backend::Cpu && matches!(codec, Codec::H264 | Codec::Auto) {
        return CPU_H264.into_iter().filter(|n| ff::encoder::find_by_name(n).is_some()).collect();
    }
    encoder_name(codec, backend).into_iter().collect()
}

fn encoder_name(codec: Codec, backend: Backend) -> Option<&'static str> {
    let codec = if codec == Codec::Auto { Codec::H264 } else { codec };
    Some(match (backend, codec) {
        (Backend::Nvenc, Codec::H264) => "h264_nvenc",
        (Backend::Nvenc, Codec::Hevc) => "hevc_nvenc",
        (Backend::Nvenc, Codec::Av1) => "av1_nvenc",
        (Backend::Vaapi, Codec::H264) => "h264_vaapi",
        (Backend::Vaapi, Codec::Hevc) => "hevc_vaapi",
        (Backend::Vaapi, Codec::Av1) => "av1_vaapi",
        (Backend::Vaapi, Codec::Vp8) => "vp8_vaapi",
        (Backend::Vaapi, Codec::Vp9) => "vp9_vaapi",
        (Backend::Amf, Codec::H264) => "h264_amf",
        (Backend::Amf, Codec::Hevc) => "hevc_amf",
        (Backend::Amf, Codec::Av1) => "av1_amf",
        (Backend::Qsv, Codec::H264) => "h264_qsv",
        (Backend::Qsv, Codec::Hevc) => "hevc_qsv",
        (Backend::Qsv, Codec::Av1) => "av1_qsv",
        (Backend::Qsv, Codec::Vp9) => "vp9_qsv",
        (Backend::Vulkan, Codec::H264) => "h264_vulkan",
        (Backend::Vulkan, Codec::Hevc) => "hevc_vulkan",
        (Backend::Vulkan, Codec::Av1) => "av1_vulkan",
        (Backend::Cpu, Codec::H264) => "libx264",
        (Backend::Cpu, Codec::Vp8) => "libvpx",
        (Backend::Cpu, Codec::Vp9) => "libvpx-vp9",
        _ => return None,
    })
}

fn qp_for(codec: Codec, backend: Backend, preset: u8) -> i64 {
    let i = preset.min(3) as usize;
    match (backend, codec) {
        (Backend::Cpu, Codec::Vp8 | Codec::Vp9) => [36, 32, 28, 24][i],
        (Backend::Cpu, _) => [27, 24, 21, 18][i],
        (Backend::Vaapi, Codec::Vp8 | Codec::Vp9) => [110, 90, 70, 50][i],
        (_, Codec::Av1) => [160, 130, 110, 90][i],
        (_, Codec::Hevc) => [31, 27, 24, 21][i],
        _ => [30, 26, 23, 20][i],
    }
}

fn sw_format(backend: Backend, depth: BitDepth, yuv444: bool) -> Pixel {
    let ten = depth != BitDepth::Eight;
    match backend {
        Backend::Cpu => {
            if yuv444 {
                Pixel::YUV444P
            } else {
                Pixel::YUV420P
            }
        }
        Backend::Nvenc => {
            if ten {
                Pixel::P010LE
            } else if yuv444 {
                Pixel::YUV444P
            } else {
                Pixel::NV12
            }
        }
        Backend::Vaapi | Backend::Vulkan | Backend::Amf | Backend::Qsv => {
            if ten {
                Pixel::P010LE
            } else {
                Pixel::NV12
            }
        }
    }
}

unsafe fn make_hw(backend: Backend, sw: Pixel, w: u32, h: u32) -> Result<(Hw, Pixel)> {
    let (kind, hw_pix, node) = match backend {
        Backend::Vaapi => (ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI, Pixel::VAAPI, Some(vaapi_node())),
        Backend::Vulkan => (ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VULKAN, Pixel::VULKAN, None),
        _ => bail!("no hardware frames needed"),
    };
    let mut device: *mut ffi::AVBufferRef = ptr::null_mut();
    let node_c = node.as_deref().map(|n| std::ffi::CString::new(n).unwrap());
    let rc = ffi::av_hwdevice_ctx_create(
        &mut device,
        kind,
        node_c.as_ref().map(|c| c.as_ptr()).unwrap_or(ptr::null()),
        ptr::null_mut(),
        0,
    );
    if rc < 0 || device.is_null() {
        bail!("could not open the {} device", backend.label());
    }
    let mut frames = ffi::av_hwframe_ctx_alloc(device);
    if frames.is_null() {
        ffi::av_buffer_unref(&mut device);
        bail!("could not allocate hardware frames");
    }
    let fc = (*frames).data as *mut ffi::AVHWFramesContext;
    (*fc).format = hw_pix.into();
    (*fc).sw_format = sw.into();
    (*fc).width = w as i32;
    (*fc).height = h as i32;
    (*fc).initial_pool_size = 8;
    if ffi::av_hwframe_ctx_init(frames) < 0 {
        ffi::av_buffer_unref(&mut frames);
        ffi::av_buffer_unref(&mut device);
        bail!("could not set up {} frames", backend.label());
    }
    Ok((Hw { device, frames }, hw_pix))
}

pub struct Setup<'a> {
    pub opts: &'a Options,
    pub src_w: u32,
    pub src_h: u32,
    pub global_header: bool,
}

impl VideoEncoder {
    pub fn open(setup: &Setup) -> Result<VideoEncoder> {
        let opts = setup.opts;
        let first = pick_backend(opts);
        match Self::open_backend(setup, first, opts.codec) {
            Ok(e) => Ok(e),
            Err(err) => {
                let codec = if opts.codec == Codec::Auto { Codec::H264 } else { opts.codec };
                if first != Backend::Cpu && matches!(codec, Codec::Vp8 | Codec::Vp9) {
                    report::warning(&format!("{err:#}, falling back to the cpu encoder"));
                    return Self::open_backend(setup, Backend::Cpu, codec);
                }
                let explicit = opts.gpu_api.is_some() || opts.vulkan || opts.cpu_encoder;
                if first == Backend::Vulkan || !explicit {
                    for alt in other_backends(first) {
                        report::warning(&format!("{err:#}, trying {}", alt.label()));
                        if let Ok(e) = Self::open_backend(setup, alt, opts.codec) {
                            return Ok(e);
                        }
                    }
                }
                if (opts.fallback_cpu || cfg!(windows) && !explicit) && first != Backend::Cpu {
                    report::warning(&format!("{err:#}, falling back to cpu encoding"));
                    return Self::open_backend(setup, Backend::Cpu, Codec::H264);
                }
                Err(err)
            }
        }
    }

    fn open_backend(setup: &Setup, backend: Backend, codec: Codec) -> Result<VideoEncoder> {
        let codec = if codec == Codec::Auto { Codec::H264 } else { codec };
        let names = encoder_names(codec, backend);
        let mut last = anyhow!("{:?} can't be encoded with {}", codec, backend.label());
        for name in names {
            match Self::open_with(setup, backend, codec, name) {
                Ok(e) => return Ok(e),
                Err(e) => {
                    report::info(&format!("{e:#}"));
                    last = e;
                }
            }
        }
        Err(last)
    }

    fn open_with(setup: &Setup, backend: Backend, codec: Codec, name: &'static str) -> Result<VideoEncoder> {
        let opts = setup.opts;
        let ff_codec = ff::encoder::find_by_name(name).ok_or_else(|| anyhow!("FFmpeg has no {name} encoder"))?;

        let (width, height) = fit_size(setup.src_w, setup.src_h, opts.size_limit);
        let depth = if opts.depth == BitDepth::Hdr {
            report::warning("the captured image is SDR, recording 10 bit instead of HDR");
            BitDepth::Ten
        } else {
            opts.depth
        };
        let yuv444 = opts.yuv444 && (backend == Backend::Nvenc || name == "libx264") && codec == Codec::H264
            || opts.yuv444 && backend == Backend::Nvenc && codec == Codec::Hevc;
        if opts.yuv444 && !yuv444 {
            report::warning("yuv444 isn't available for this codec, using yuv420");
        }
        let sw = sw_format(backend, depth, yuv444);

        let time_base =
            if opts.fps_mode == FpsMode::Cfr { Rational::new(1, opts.fps as i32) } else { Rational::new(1, 90_000) };

        let ctx = ff::codec::context::Context::new_with_codec(ff_codec);
        let mut enc = ctx.encoder().video()?;
        enc.set_width(width);
        enc.set_height(height);
        enc.set_time_base(time_base);
        enc.set_frame_rate(Some(Rational::new(opts.fps as i32, 1)));
        enc.set_gop(((opts.fps as f32 * opts.keyint).round() as u32).max(1));
        enc.set_max_b_frames(0);
        enc.set_colorspace(ff::color::Space::BT709);
        enc.set_color_primaries(ff::color::Primaries::BT709);
        enc.set_color_transfer_characteristic(ff::color::TransferCharacteristic::BT709);
        enc.set_color_range(if opts.full_range { ff::color::Range::JPEG } else { ff::color::Range::MPEG });
        if setup.global_header {
            enc.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        }

        let mut hw = None;
        if matches!(backend, Backend::Vaapi | Backend::Vulkan) {
            let (h, hw_pix) = unsafe { make_hw(backend, sw, width, height)? };
            enc.set_format(hw_pix);
            unsafe {
                (*enc.as_mut_ptr()).hw_frames_ctx = ffi::av_buffer_ref(h.frames);
            }
            hw = Some(h);
        } else {
            enc.set_format(sw);
        }

        let mut dict = Dictionary::new();
        let rate_mode = match (opts.rate_mode, &opts.quality) {
            (RateMode::Auto, Quality::Bitrate(_)) => RateMode::Cbr,
            (RateMode::Auto, _) => RateMode::Qp,
            (m, _) => m,
        };
        let bitrate_kbps = match opts.quality {
            Quality::Bitrate(k) => k.max(500),
            Quality::Preset(p) => [8000, 15000, 25000, 40000][p.min(3) as usize],
        };
        let preset = match opts.quality {
            Quality::Preset(p) => p,
            Quality::Bitrate(_) => 2,
        };

        match rate_mode {
            RateMode::Cbr | RateMode::Vbr => {
                let bps = bitrate_kbps as usize * 1000;
                enc.set_bit_rate(bps);
                enc.set_max_bit_rate(bps);
                unsafe {
                    (*enc.as_mut_ptr()).rc_buffer_size = (bps * 2) as i32;
                }
                let cbr = rate_mode == RateMode::Cbr;
                match backend {
                    Backend::Nvenc => dict.set("rc", if cbr { "cbr" } else { "vbr" }),
                    Backend::Vaapi => dict.set("rc_mode", if cbr { "CBR" } else { "VBR" }),
                    Backend::Amf => dict.set("rc", if cbr { "cbr" } else { "vbr_peak" }),
                    Backend::Qsv => {
                        if !cbr {
                            enc.set_max_bit_rate(bps * 3 / 2);
                        }
                    }
                    Backend::Vulkan => dict.set("rc_mode", if cbr { "cbr" } else { "vbr" }),
                    Backend::Cpu if name == "libopenh264" => dict.set("rc_mode", "bitrate"),
                    Backend::Cpu => {
                        if cbr && name == "libx264" {
                            dict.set("nal-hrd", "cbr");
                        }
                    }
                }
            }
            RateMode::Qp | RateMode::Auto => {
                let qp = qp_for(codec, backend, preset);
                match backend {
                    Backend::Nvenc => {
                        dict.set("rc", "constqp");
                        dict.set("qp", &qp.to_string());
                    }
                    Backend::Vaapi => {
                        dict.set("rc_mode", "CQP");
                        enc.set_global_quality(qp as i32);
                    }
                    Backend::Vulkan => {
                        dict.set("rc_mode", "cqp");
                        dict.set("qp", &qp.to_string());
                    }
                    Backend::Amf => {
                        dict.set("rc", "cqp");
                        for key in ["qp_i", "qp_p", "qp_b"] {
                            dict.set(key, &qp.to_string());
                        }
                    }
                    Backend::Qsv => {
                        unsafe {
                            (*enc.as_mut_ptr()).flags |= ffi::AV_CODEC_FLAG_QSCALE as i32;
                        }
                        enc.set_global_quality((qp * ffi::FF_QP2LAMBDA as i64) as i32);
                    }
                    Backend::Cpu if name == "libopenh264" => {
                        enc.set_bit_rate(bitrate_kbps as usize * 1000);
                        dict.set("rc_mode", "quality");
                    }
                    Backend::Cpu if name == "h264_mf" => {
                        dict.set("quality", &(100 - qp * 2).clamp(10, 100).to_string());
                    }
                    Backend::Cpu => {
                        dict.set("crf", &qp.to_string());
                        if codec != Codec::H264 {
                            enc.set_bit_rate(0);
                        }
                    }
                }
            }
        }

        match backend {
            Backend::Nvenc => {
                dict.set("preset", if opts.tune_quality { "p6" } else { "p4" });
                dict.set("tune", if opts.tune_quality { "hq" } else { "ll" });
                dict.set("zerolatency", "1");
                dict.set("delay", "0");
                if codec == Codec::H264 && yuv444 {
                    dict.set("profile", "high444p");
                } else if codec == Codec::Hevc && depth == BitDepth::Ten {
                    dict.set("profile", "main10");
                }
            }
            Backend::Vaapi => {
                if opts.low_power {
                    dict.set("low_power", "1");
                }
                dict.set("async_depth", "3");
            }
            Backend::Vulkan => {}
            Backend::Amf => {
                dict.set("usage", "lowlatency");
                dict.set("quality", if opts.tune_quality { "quality" } else { "balanced" });
            }
            Backend::Qsv => {
                dict.set("preset", if opts.tune_quality { "medium" } else { "veryfast" });
                dict.set("async_depth", "4");
            }
            Backend::Cpu => match codec {
                Codec::H264 if name == "libopenh264" => {
                    dict.set("allow_skip_frames", "0");
                }
                Codec::H264 if name == "h264_mf" => {
                    dict.set("rate_control", if rate_mode == RateMode::Cbr { "cbr" } else { "quality" });
                    dict.set("scenario", "display_remoting");
                }
                Codec::H264 => {
                    dict.set("preset", if opts.tune_quality { "fast" } else { "veryfast" });
                    dict.set("tune", "zerolatency");
                }
                _ => {
                    dict.set("deadline", "realtime");
                    dict.set("cpu-used", "8");
                    dict.set("row-mt", "1");
                }
            },
        }
        for (k, v) in &opts.video_opts {
            dict.set(k, v);
        }

        let enc = enc.open_as_with(ff_codec, dict).map_err(|e| anyhow!("could not open {name}: {e}"))?;

        let scaler = unsafe {
            let ctx = ffi::sws_alloc_context();
            if ctx.is_null() {
                bail!("could not allocate the color converter");
            }
            let scaled = setup.src_w != width || setup.src_h != height;
            (*ctx).flags = if scaled { ffi::SwsFlags::SWS_BICUBIC as u32 } else { ffi::SwsFlags::SWS_POINT as u32 };
            (*ctx).threads = 0;
            Scaler { ctx }
        };
        let mut staging = ff::frame::Video::new(sw, width, height);
        unsafe {
            let f = staging.as_mut_ptr();
            (*f).colorspace = ffi::AVColorSpace::AVCOL_SPC_BT709;
            (*f).color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
            (*f).color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
            (*f).color_range =
                if opts.full_range { ffi::AVColorRange::AVCOL_RANGE_JPEG } else { ffi::AVColorRange::AVCOL_RANGE_MPEG };
        }

        report::info(&format!("video: {name} via {} at {width}x{height}", backend.label()));
        Ok(VideoEncoder {
            enc,
            hw,
            scaler,
            staging,
            time_base,
            width,
            height,
            backend,
            codec_name: name.to_string(),
            full_range: opts.full_range,
            layout: match sw {
                Pixel::NV12 => Some(Layout::Nv12),
                Pixel::P010LE => Some(Layout::P010),
                Pixel::YUV420P => Some(Layout::Yuv420p),
                Pixel::YUV444P => Some(Layout::Yuv444p),
                _ => None,
            },
            frames_in: 0,
            convert_time: std::time::Duration::ZERO,
            encode_time: std::time::Duration::ZERO,
        })
    }

    pub fn parameters(&self) -> ff::codec::Parameters {
        let mut params = ff::codec::Parameters::new();
        unsafe {
            ffi::avcodec_parameters_from_context(params.as_mut_ptr(), self.enc.as_ptr());
        }
        params
    }

    fn convert(&mut self, frame: &Frame) -> Result<()> {
        if frame.width & !1 == self.width && frame.height & !1 == self.height {
            if let Some(layout) = self.layout {
                let planes = match layout {
                    Layout::Nv12 | Layout::P010 => 2,
                    Layout::Yuv420p | Layout::Yuv444p => 3,
                };
                let strides: Vec<usize> = (0..planes).map(|i| self.staging.stride(i)).collect();
                let ptr = unsafe { self.staging.as_mut_ptr() };
                let mut slices: Vec<&mut [u8]> = Vec::with_capacity(planes);
                for (i, stride) in strides.iter().enumerate() {
                    let rows = match (layout, i) {
                        (_, 0) | (Layout::Yuv444p, _) => self.height as usize,
                        _ => self.height as usize / 2,
                    };
                    let data = unsafe { (*ptr).data[i] };
                    slices.push(unsafe { std::slice::from_raw_parts_mut(data, stride * rows) });
                }
                convert::convert(frame, layout, self.full_range, convert::Target { planes: slices, strides });
                return Ok(());
            }
        }
        unsafe {
            let src = ffi::av_frame_alloc();
            if src.is_null() {
                bail!("out of memory");
            }
            (*src).width = frame.width as i32;
            (*src).height = frame.height as i32;
            (*src).format = match frame.format {
                PixFmt::Bgrx => ffi::AVPixelFormat::AV_PIX_FMT_BGR0 as i32,
                PixFmt::Rgbx => ffi::AVPixelFormat::AV_PIX_FMT_RGB0 as i32,
            };
            (*src).data[0] = frame.data.as_ptr() as *mut u8;
            (*src).linesize[0] = frame.stride as i32;
            (*src).color_range = ffi::AVColorRange::AVCOL_RANGE_JPEG;
            (*src).colorspace = ffi::AVColorSpace::AVCOL_SPC_RGB;
            (*src).color_primaries = ffi::AVColorPrimaries::AVCOL_PRI_BT709;
            (*src).color_trc = ffi::AVColorTransferCharacteristic::AVCOL_TRC_IEC61966_2_1;
            let dst = self.staging.as_mut_ptr();
            let rc = ffi::sws_scale_frame(self.scaler.ctx, dst, src);
            (*src).data[0] = ptr::null_mut();
            let mut src = src;
            ffi::av_frame_free(&mut src);
            if rc < 0 {
                bail!("color conversion failed ({rc})");
            }
        }
        Ok(())
    }

    pub fn encode(&mut self, frame: &Frame, pts: i64, out: &mut Vec<Packet>) -> Result<()> {
        let t0 = std::time::Instant::now();
        self.convert(frame)?;
        let t1 = std::time::Instant::now();
        self.convert_time += t1 - t0;
        self.frames_in += 1;
        let r = self.submit(pts, out);
        self.encode_time += t1.elapsed();
        r
    }

    fn submit(&mut self, pts: i64, out: &mut Vec<Packet>) -> Result<()> {
        if let Some(hw) = &self.hw {
            unsafe {
                let mut hwf = ffi::av_frame_alloc();
                if ffi::av_hwframe_get_buffer(hw.frames, hwf, 0) < 0 {
                    ffi::av_frame_free(&mut hwf);
                    bail!("no free hardware surface");
                }
                if ffi::av_hwframe_transfer_data(hwf, self.staging.as_ptr(), 0) < 0 {
                    ffi::av_frame_free(&mut hwf);
                    bail!("uploading the frame to the gpu failed");
                }
                ffi::av_frame_copy_props(hwf, self.staging.as_ptr());
                (*hwf).pts = pts;
                let wrapped = ff::frame::Video::wrap(hwf);
                self.enc.send_frame(&wrapped).map_err(|e| anyhow!("encoder rejected frame: {e}"))?;
            }
        } else {
            self.staging.set_pts(Some(pts));
            self.enc.send_frame(&self.staging).map_err(|e| anyhow!("encoder rejected frame: {e}"))?;
        }
        self.drain(out)
    }

    fn drain(&mut self, out: &mut Vec<Packet>) -> Result<()> {
        loop {
            let mut pkt = Packet::empty();
            match self.enc.receive_packet(&mut pkt) {
                Ok(()) => {
                    if pkt.dts().is_none() {
                        pkt.set_dts(pkt.pts());
                    }
                    out.push(pkt);
                }
                Err(ff::Error::Other { errno }) if errno == ff::util::error::EAGAIN => return Ok(()),
                Err(ff::Error::Eof) => return Ok(()),
                Err(e) => return Err(anyhow!("encoding failed: {e}")),
            }
        }
    }

    pub fn finish(&mut self, out: &mut Vec<Packet>) -> Result<()> {
        if self.frames_in == 0 {
            return Ok(());
        }
        let _ = self.enc.send_eof();
        self.drain(out)
    }
}
