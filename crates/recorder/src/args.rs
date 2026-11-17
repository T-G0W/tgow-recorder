use anyhow::{anyhow, bail, Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Auto,
    H264,
    Hevc,
    Av1,
    Vp8,
    Vp9,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Eight,
    Ten,
    Hdr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateMode {
    Auto,
    Qp,
    Vbr,
    Cbr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpsMode {
    Cfr,
    Vfr,
    Content,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Quality {
    Preset(u8),
    Bitrate(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn parse(s: &str) -> Result<Region> {
        let (size, pos) = s.split_once('+').ok_or_else(|| anyhow!("region must look like WxH+X+Y"))?;
        let (w, h) = size.split_once('x').ok_or_else(|| anyhow!("region must look like WxH+X+Y"))?;
        let (x, y) = pos.split_once('+').ok_or_else(|| anyhow!("region must look like WxH+X+Y"))?;
        Ok(Region { width: w.trim().parse()?, height: h.trim().parse()?, x: x.trim().parse()?, y: y.trim().parse()? })
    }
}

#[derive(Debug, Clone)]
pub struct AudioTrackArg {
    pub title: String,
    pub sources: Vec<String>,
}

impl AudioTrackArg {
    pub fn parse(s: &str) -> AudioTrackArg {
        let (title, rest) = match s.split_once('/') {
            Some((t, r)) if !t.contains('|') && !t.contains(':') => (t.to_string(), r),
            _ => (String::new(), s),
        };
        AudioTrackArg {
            title,
            sources: rest.split('|').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub source: Option<String>,
    pub region: Option<Region>,
    pub size_limit: (u32, u32),
    pub fps: u32,
    pub container: Option<String>,
    pub codec: Codec,
    pub depth: BitDepth,
    pub cpu_encoder: bool,
    pub fallback_cpu: bool,
    pub vulkan: bool,
    pub gpu_api: Option<String>,
    pub quality: Quality,
    pub rate_mode: RateMode,
    pub fps_mode: FpsMode,
    pub full_range: bool,
    pub yuv444: bool,
    pub tune_quality: bool,
    pub audio: Vec<AudioTrackArg>,
    pub audio_codec: Option<String>,
    pub audio_bitrate: u32,
    pub replay_seconds: Option<u32>,
    pub replay_on_disk: bool,
    pub restart_replay_on_save: bool,
    pub date_folders: bool,
    pub output: Option<String>,
    pub cursor: bool,
    pub keyint: f32,
    pub low_power: bool,
    pub restore_portal: bool,
    pub token_file: Option<PathBuf>,
    pub auth: Option<String>,
    pub exclude_metadata: bool,
    pub split_minutes: u32,
    pub screenshot: Option<PathBuf>,
    pub script: Option<PathBuf>,
    pub muxer_opts: Vec<(String, String)>,
    pub video_opts: Vec<(String, String)>,
    pub audio_opts: Vec<(String, String)>,
    pub verbose: bool,
    pub webcam: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            source: None,
            region: None,
            size_limit: (0, 0),
            fps: 60,
            container: None,
            codec: Codec::Auto,
            depth: BitDepth::Eight,
            cpu_encoder: false,
            fallback_cpu: false,
            vulkan: false,
            gpu_api: None,
            quality: Quality::Preset(2),
            rate_mode: RateMode::Auto,
            fps_mode: FpsMode::Vfr,
            full_range: false,
            yuv444: false,
            tune_quality: false,
            audio: Vec::new(),
            audio_codec: None,
            audio_bitrate: 0,
            replay_seconds: None,
            replay_on_disk: false,
            restart_replay_on_save: false,
            date_folders: false,
            output: None,
            cursor: true,
            keyint: 2.0,
            low_power: false,
            restore_portal: false,
            token_file: None,
            auth: std::env::var("TGOW_AUTH").ok().filter(|v| !v.is_empty()),
            exclude_metadata: false,
            split_minutes: 0,
            screenshot: None,
            script: None,
            muxer_opts: Vec::new(),
            video_opts: Vec::new(),
            audio_opts: Vec::new(),
            verbose: false,
            webcam: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Invocation {
    Run(Box<Options>),
    ListMonitors,
    ListAudioDevices,
    ListApplicationAudio,
    ListCameras,
    Info,
    Version,
    Help,
}

fn yes_no(flag: &str, value: &str) -> Result<bool> {
    match value {
        "yes" | "true" | "1" | "on" => Ok(true),
        "no" | "false" | "0" | "off" => Ok(false),
        _ => bail!("{flag} expects yes or no, got \"{value}\""),
    }
}

fn key_values(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

pub fn parse(raw: Vec<String>) -> Result<Invocation> {
    let mut opts = Options::default();
    let mut it = raw.into_iter().skip(1).peekable();
    let mut quality_text: Option<String> = None;

    while let Some(flag) = it.next() {
        match flag.as_str() {
            "-h" | "--help" => return Ok(Invocation::Help),
            "--version" => return Ok(Invocation::Version),
            "--info" => return Ok(Invocation::Info),
            "--list-monitors" | "--list-capture-options" => return Ok(Invocation::ListMonitors),
            "--list-audio-devices" => return Ok(Invocation::ListAudioDevices),
            "--list-application-audio" => return Ok(Invocation::ListApplicationAudio),
            "--list-v4l2-devices" | "--list-cameras" => return Ok(Invocation::ListCameras),
            _ => {}
        }
        let mut value = || it.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "-w" => opts.source = Some(value()?),
            "-region" => opts.region = Some(Region::parse(&value()?)?),
            "-s" => {
                let v = value()?;
                let (w, h) = v.split_once('x').ok_or_else(|| anyhow!("-s expects WxH"))?;
                opts.size_limit = (w.parse()?, h.parse()?);
            }
            "-f" => opts.fps = value()?.parse::<u32>()?.clamp(1, 500),
            "-c" => opts.container = Some(value()?),
            "-k" => {
                let v = value()?;
                let base = v.trim_end_matches("_vulkan");
                if base != v {
                    opts.vulkan = true;
                }
                let (codec, depth) = match base {
                    "auto" => (Codec::Auto, BitDepth::Eight),
                    "h264" => (Codec::H264, BitDepth::Eight),
                    "hevc" => (Codec::Hevc, BitDepth::Eight),
                    "hevc_10bit" => (Codec::Hevc, BitDepth::Ten),
                    "hevc_hdr" => (Codec::Hevc, BitDepth::Hdr),
                    "av1" => (Codec::Av1, BitDepth::Eight),
                    "av1_10bit" => (Codec::Av1, BitDepth::Ten),
                    "av1_hdr" => (Codec::Av1, BitDepth::Hdr),
                    "vp8" => (Codec::Vp8, BitDepth::Eight),
                    "vp9" => (Codec::Vp9, BitDepth::Eight),
                    other => bail!("unknown video codec \"{other}\""),
                };
                opts.codec = codec;
                opts.depth = depth;
            }
            "-encoder" => opts.cpu_encoder = value()? == "cpu",
            "-fallback-cpu-encoding" => opts.fallback_cpu = yes_no(&flag, &value()?)?,
            "-vulkan" => opts.vulkan = yes_no(&flag, &value()?)?,
            "-gpu-api" => {
                let v = value()?;
                match v.as_str() {
                    "auto" => opts.gpu_api = None,
                    "nvenc" | "vaapi" | "amf" | "qsv" | "vulkan" => opts.gpu_api = Some(v),
                    other => bail!("unknown gpu api \"{other}\", use auto, nvenc, vaapi, amf, qsv or vulkan"),
                }
            }
            "-q" => quality_text = Some(value()?),
            "-bm" => {
                opts.rate_mode = match value()?.as_str() {
                    "auto" => RateMode::Auto,
                    "qp" => RateMode::Qp,
                    "vbr" => RateMode::Vbr,
                    "cbr" => RateMode::Cbr,
                    other => bail!("unknown bitrate mode \"{other}\""),
                }
            }
            "-fm" => {
                opts.fps_mode = match value()?.as_str() {
                    "cfr" => FpsMode::Cfr,
                    "vfr" => FpsMode::Vfr,
                    "content" => FpsMode::Content,
                    other => bail!("unknown frame rate mode \"{other}\""),
                }
            }
            "-cr" => opts.full_range = value()? == "full",
            "-pixfmt" => opts.yuv444 = value()? == "yuv444",
            "-tune" => opts.tune_quality = value()? == "quality",
            "-a" => opts.audio.push(AudioTrackArg::parse(&value()?)),
            "-ac" => opts.audio_codec = Some(value()?),
            "-ab" => opts.audio_bitrate = value()?.parse()?,
            "-r" => opts.replay_seconds = Some(value()?.parse::<u32>()?.clamp(2, 86400)),
            "-replay-storage" => opts.replay_on_disk = value()? == "disk",
            "-restart-replay-on-save" => opts.restart_replay_on_save = yes_no(&flag, &value()?)?,
            "-df" => opts.date_folders = yes_no(&flag, &value()?)?,
            "-o" | "-ro" => opts.output = Some(value()?),
            "-cursor" => opts.cursor = yes_no(&flag, &value()?)?,
            "-keyint" => opts.keyint = value()?.parse::<f32>()?.clamp(0.1, 60.0),
            "-low-power" => opts.low_power = yes_no(&flag, &value()?)?,
            "-restore-portal-session" => opts.restore_portal = yes_no(&flag, &value()?)?,
            "-portal-session-token-filepath" => opts.token_file = Some(PathBuf::from(value()?)),
            "-auth" => opts.auth = Some(value()?),
            "-exclude-metadata" => opts.exclude_metadata = yes_no(&flag, &value()?)?,
            "-split" => opts.split_minutes = value()?.parse()?,
            "-screenshot" => opts.screenshot = Some(PathBuf::from(value()?)),
            "-sc" => opts.script = Some(PathBuf::from(value()?)),
            "-ffmpeg-opts" => opts.muxer_opts = key_values(&value()?),
            "-ffmpeg-video-opts" => opts.video_opts = key_values(&value()?),
            "-ffmpeg-audio-opts" => opts.audio_opts = key_values(&value()?),
            "-v" => opts.verbose = yes_no(&flag, &value()?)?,
            "-webcam" => opts.webcam = Some(value()?),
            other => bail!("unknown option \"{other}\", see --help"),
        }
    }

    if let Some(q) = quality_text {
        opts.quality = match q.as_str() {
            "medium" => Quality::Preset(0),
            "high" => Quality::Preset(1),
            "very_high" => Quality::Preset(2),
            "ultra" => Quality::Preset(3),
            n => Quality::Bitrate(n.parse().with_context(|| format!("bad quality \"{n}\""))?),
        };
    }
    if matches!(opts.quality, Quality::Bitrate(_)) && opts.rate_mode == RateMode::Auto {
        opts.rate_mode = RateMode::Cbr;
    }
    if opts.source.is_none() && opts.audio.is_empty() {
        bail!("nothing to record, use -w and/or -a");
    }
    if opts.output.is_none() && opts.screenshot.is_none() {
        bail!("missing -o");
    }
    Ok(Invocation::Run(Box::new(opts)))
}

pub const HELP: &str = "usage: tgow-rec -w <screen|focused_monitor|monitor name|portal|region|window|focused> [options] -o <file|url|directory>

capture
  -w <source>            what to capture
  -region WxH+X+Y        area for -w region
  -s WxH                 output resolution limit, 0x0 keeps the native size
  -f <fps>               frame rate, default 60
  -cursor yes|no         draw the cursor, default yes
  -restore-portal-session yes|no
  -webcam <device>[;x=..;y=..;width=..;height=..;flip=yes]

video
  -k <codec>             auto h264 hevc hevc_10bit hevc_hdr av1 av1_10bit av1_hdr vp8 vp9
  -encoder gpu|cpu       default gpu
  -fallback-cpu-encoding yes|no
  -vulkan yes|no         use vulkan video encoding
  -gpu-api <api>         auto nvenc vaapi amf qsv vulkan
  -q <quality|kbps>      medium high very_high ultra, or a bitrate with -bm cbr
  -bm auto|qp|vbr|cbr
  -fm cfr|vfr|content
  -cr limited|full
  -pixfmt yuv420|yuv444
  -tune performance|quality
  -keyint <seconds>
  -low-power yes|no

audio
  -a [title/]source[|source...]   default_output default_input device:<name> app:<name> app-inverse:<name>
  -ac opus|aac|flac|mp3
  -ab <kbps>

output
  -c <container>         mp4 mkv mov webm flv mpegts whip
  -o <path|url>          file, stream url, or the save directory in replay mode
  -r <seconds>           replay buffer length
  -replay-storage ram|disk
  -restart-replay-on-save yes|no
  -df yes|no             date based folders for replays
  -split <minutes>       start a new file every n minutes
  -screenshot <file>     grab a single image and exit
  -auth <token>          bearer token for whip
  -exclude-metadata yes|no
  -sc <script>           run after a file was saved
  -ffmpeg-opts k=v;k=v   -ffmpeg-video-opts k=v   -ffmpeg-audio-opts k=v

info
  --list-monitors --list-audio-devices --list-application-audio --list-cameras --info --version

control
  while running, these lines are read from stdin: save, save <seconds>, pause, resume, toggle-pause,
  mute <track> <0|1>, marker, stop.  SIGINT/SIGTERM stop, SIGUSR1 saves the replay, SIGUSR2 toggles pause.
";
