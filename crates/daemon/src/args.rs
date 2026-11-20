use std::path::Path;

use tgow_common::config::{
    CaptureOptions, ColorRange, Config, MainConfig, PixelFormat, Quality, ReplayStorage, VideoCodec, Webcam,
    WebcamFormat, YesNoGames,
};

fn yes_no(v: bool) -> String {
    if v { "yes" } else { "no" }.to_string()
}

pub fn target_name(source: &str) -> &'static str {
    match source {
        "window" | "focused" => "window",
        "region" => "region",
        "portal" if !cfg!(windows) => "portal",
        _ if cfg!(windows) => "the main monitor",
        _ => "this monitor",
    }
}

pub struct Source {
    pub name: String,
    pub region: Option<String>,
}

impl Source {
    pub fn plain(name: &str) -> Self {
        Self { name: name.to_string(), region: None }
    }
}

fn push(args: &mut Vec<String>, flag: &str, value: impl Into<String>) {
    args.push(flag.to_string());
    args.push(value.into());
}

fn push_source(args: &mut Vec<String>, source: &Source, capture_area: Option<(u32, u32)>) {
    push(args, "-w", source.name.clone());
    if let Some(region) = &source.region {
        push(args, "-region", region.clone());
    }
    if source.name == "focused" {
        if let Some((w, h)) = capture_area {
            push(args, "-s", format!("{w}x{h}"));
        }
    }
}

fn capture_args(args: &mut Vec<String>, c: &CaptureOptions, source: &Source, main: &MainConfig) {
    push_source(args, source, Some((c.area_width.max(2), c.area_height.max(2))));
    if c.change_resolution && source.name != "focused" {
        push(args, "-s", format!("{}x{}", c.video_width, c.video_height));
    }
    push(args, "-f", c.fps.max(1).to_string());
    push(args, "-k", c.codec.arg());
    push(args, "-encoder", if c.codec == VideoCodec::H264Software { "cpu" } else { "gpu" });
    if c.quality == Quality::Custom {
        push(args, "-bm", "cbr");
        push(args, "-q", c.bitrate_kbps.max(100).to_string());
    } else {
        push(args, "-bm", "auto");
        push(args, "-q", c.quality.arg());
    }
    push(args, "-fm", c.framerate_mode.arg());
    push(
        args,
        "-cr",
        match c.color_range {
            ColorRange::Limited => "limited",
            ColorRange::Full => "full",
        },
    );
    push(
        args,
        "-pixfmt",
        match c.pixel_format {
            PixelFormat::Yuv420 => "yuv420",
            PixelFormat::Yuv444 => "yuv444",
        },
    );
    for track in &c.audio_tracks {
        if track.inputs.is_empty() {
            continue;
        }
        push(args, "-a", track.to_arg());
    }
    push(args, "-ac", c.audio_codec.arg());
    if c.audio_bitrate_kbps > 0 {
        push(args, "-ab", c.audio_bitrate_kbps.to_string());
    }
    push(args, "-cursor", yes_no(c.record_cursor));
    push(args, "-keyint", format!("{}", c.keyframe_interval.max(0.1)));
    push(args, "-low-power", yes_no(c.low_power));
    push(args, "-vulkan", yes_no(c.vulkan));
    push(args, "-exclude-metadata", yes_no(main.exclude_metadata));
    if !c.webcam.source.trim().is_empty() {
        push(args, "-webcam", webcam_spec(&c.webcam));
    }
    if source.name == "portal" {
        push(args, "-restore-portal-session", "yes");
    }
}

fn webcam_spec(w: &Webcam) -> String {
    let pixfmt = match w.format {
        WebcamFormat::Auto => "auto",
        WebcamFormat::Yuyv => "yuyv",
        WebcamFormat::Mjpeg => "mjpeg",
    };
    format!(
        "{};x={};y={};width={}%;height={}%;flip={};pixfmt={};camera_width={};camera_height={};camera_fps={}",
        w.source.trim(),
        w.x.clamp(0, 100),
        w.y.clamp(0, 100),
        w.width_percent.clamp(1, 100),
        w.height_percent.clamp(1, 100),
        yes_no(w.flip_horizontally),
        pixfmt,
        w.camera_width,
        w.camera_height,
        w.camera_fps
    )
}

pub fn record(cfg: &Config, source: &Source, output: &Path) -> Vec<String> {
    let mut args = Vec::new();
    capture_args(&mut args, &cfg.record.capture, source, &cfg.main);
    push(&mut args, "-c", cfg.record.container.clone());
    if cfg.record.split_minutes > 0 {
        push(&mut args, "-split", cfg.record.split_minutes.to_string());
    }
    push(&mut args, "-o", output.to_string_lossy());
    args
}

pub fn replay(cfg: &Config, source: &Source, output_dir: &Path) -> Vec<String> {
    let r = &cfg.replay;
    let mut args = Vec::new();
    capture_args(&mut args, &r.capture, source, &cfg.main);
    push(&mut args, "-c", r.container.clone());
    push(&mut args, "-r", r.duration_seconds.clamp(2, 86400).to_string());
    push(
        &mut args,
        "-replay-storage",
        match r.storage {
            ReplayStorage::Ram => "ram",
            ReplayStorage::Disk => "disk",
        },
    );
    push(&mut args, "-restart-replay-on-save", yes_no(r.restart_on_save));
    if r.organize_into_folders != YesNoGames::No {
        push(&mut args, "-df", "yes");
    }
    push(&mut args, "-o", output_dir.to_string_lossy());
    args
}

pub fn stream(cfg: &Config, source: &Source) -> Vec<String> {
    let s = &cfg.stream;
    let mut args = Vec::new();
    capture_args(&mut args, &s.capture, source, &cfg.main);
    push(&mut args, "-c", s.container());
    if s.service == tgow_common::config::StreamService::Whip && !s.whip_token.trim().is_empty() {
        push(&mut args, "-auth", s.whip_token.trim());
    }
    push(&mut args, "-o", s.target_url());
    args
}

pub fn screenshot(cfg: &Config, source: &Source, output: &Path) -> Vec<String> {
    let s = &cfg.screenshot;
    let mut args = Vec::new();
    push_source(&mut args, source, Some((s.image_width.max(2), s.image_height.max(2))));
    if s.change_resolution && source.name != "focused" {
        push(&mut args, "-s", format!("{}x{}", s.image_width, s.image_height));
    }
    push(&mut args, "-cursor", yes_no(s.record_cursor));
    let quality = if s.quality == Quality::Custom { Quality::VeryHigh } else { s.quality };
    push(&mut args, "-q", quality.arg());
    if source.name == "portal" {
        push(&mut args, "-restore-portal-session", "yes");
    }
    push(&mut args, "-screenshot", output.to_string_lossy());
    args
}
