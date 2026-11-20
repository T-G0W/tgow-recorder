use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::paths;

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccentPreset {
    #[default]
    Nvidia,
    Amd,
    Intel,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode {
    #[default]
    Enabled,
    VirtualOnly,
    NoGrab,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSpeed {
    #[default]
    Normal,
    Fast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum YesNoGames {
    #[default]
    No,
    Yes,
    GamesOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Hotkey {
    pub key: String,
    pub modifiers: Vec<String>,
}

impl Hotkey {
    pub fn new(key: &str, modifiers: &[&str]) -> Self {
        Self { key: key.to_string(), modifiers: modifiers.iter().map(|m| m.to_string()).collect() }
    }

    pub fn none() -> Self {
        Self::default()
    }

    pub fn is_set(&self) -> bool {
        !self.key.is_empty()
    }

    pub fn label(&self) -> String {
        if !self.is_set() {
            return String::new();
        }
        let mut parts: Vec<String> = self.modifiers.clone();
        parts.push(self.key.clone());
        parts.join(" + ")
    }

    pub fn portal_trigger(&self) -> String {
        let mut parts: Vec<String> = self
            .modifiers
            .iter()
            .map(|m| match m.as_str() {
                "Ctrl" => "CTRL".to_string(),
                "Shift" => "SHIFT".to_string(),
                "Alt" => "ALT".to_string(),
                "Super" => "LOGO".to_string(),
                other => other.to_uppercase(),
            })
            .collect();
        parts.push(self.key.clone());
        parts.join("+")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    ShowHide,
    ToggleReplay,
    SaveReplay,
    SaveReplayShort,
    SaveReplay1Min,
    SaveReplay5Min,
    SaveReplay10Min,
    ToggleRecord,
    TogglePause,
    ToggleRecordRegion,
    ToggleRecordWindow,
    ToggleRecordPortal,
    ToggleStream,
    Screenshot,
    ScreenshotRegion,
    ScreenshotWindow,
    ScreenshotPortal,
    ToggleMic,
    AddMarker,
}

impl Action {
    pub const ALL: [Action; 19] = [
        Action::ShowHide,
        Action::ToggleReplay,
        Action::SaveReplay,
        Action::SaveReplayShort,
        Action::SaveReplay1Min,
        Action::SaveReplay5Min,
        Action::SaveReplay10Min,
        Action::ToggleRecord,
        Action::TogglePause,
        Action::ToggleRecordRegion,
        Action::ToggleRecordWindow,
        Action::ToggleRecordPortal,
        Action::ToggleStream,
        Action::Screenshot,
        Action::ScreenshotRegion,
        Action::ScreenshotWindow,
        Action::ScreenshotPortal,
        Action::ToggleMic,
        Action::AddMarker,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Action::ShowHide => "show_hide",
            Action::ToggleReplay => "toggle_replay",
            Action::SaveReplay => "save_replay",
            Action::SaveReplayShort => "save_replay_short",
            Action::SaveReplay1Min => "save_replay_1_min",
            Action::SaveReplay5Min => "save_replay_5_min",
            Action::SaveReplay10Min => "save_replay_10_min",
            Action::ToggleRecord => "toggle_record",
            Action::TogglePause => "toggle_pause",
            Action::ToggleRecordRegion => "toggle_record_region",
            Action::ToggleRecordWindow => "toggle_record_window",
            Action::ToggleRecordPortal => "toggle_record_portal",
            Action::ToggleStream => "toggle_stream",
            Action::Screenshot => "screenshot",
            Action::ScreenshotRegion => "screenshot_region",
            Action::ScreenshotWindow => "screenshot_window",
            Action::ScreenshotPortal => "screenshot_portal",
            Action::ToggleMic => "toggle_mic",
            Action::AddMarker => "add_marker",
        }
    }

    pub fn from_id(id: &str) -> Option<Action> {
        let id = id.replace('-', "_");
        Action::ALL.iter().copied().find(|a| a.id() == id)
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Action::ShowHide => "Show/hide UI",
            Action::ToggleReplay => "Turn replay on/off",
            Action::SaveReplay => "Save replay",
            Action::SaveReplayShort => "Save 30 second replay",
            Action::SaveReplay1Min => "Save 1 minute replay",
            Action::SaveReplay5Min => "Save 5 minute replay",
            Action::SaveReplay10Min => "Save 10 minute replay",
            Action::ToggleRecord => "Start/stop recording",
            Action::TogglePause => "Pause/unpause recording",
            Action::ToggleRecordRegion => "Start/stop recording a region",
            Action::ToggleRecordWindow => "Start/stop recording a window",
            Action::ToggleRecordPortal => "Start/stop recording with desktop portal",
            Action::ToggleStream => "Start/stop streaming",
            Action::Screenshot => "Take a screenshot",
            Action::ScreenshotRegion => "Take a screenshot of a region",
            Action::ScreenshotWindow => "Take a screenshot of a window",
            Action::ScreenshotPortal => "Take a screenshot with desktop portal",
            Action::ToggleMic => "Mute/unmute microphone",
            Action::AddMarker => "Add chapter marker",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkeys {
    pub bindings: Vec<(Action, Hotkey)>,
}

impl Default for Hotkeys {
    fn default() -> Self {
        let b = |a, k: &str, m: &[&str]| (a, Hotkey::new(k, m));
        Self {
            bindings: vec![
                b(Action::ShowHide, "Z", &["Alt"]),
                b(Action::ToggleReplay, "F10", &["Alt", "Shift"]),
                b(Action::SaveReplay, "F10", &["Alt"]),
                b(Action::SaveReplayShort, "", &[]),
                b(Action::SaveReplay1Min, "", &[]),
                b(Action::SaveReplay5Min, "", &[]),
                b(Action::SaveReplay10Min, "", &[]),
                b(Action::ToggleRecord, "F9", &["Alt"]),
                b(Action::TogglePause, "F7", &["Alt"]),
                b(Action::ToggleRecordRegion, "", &[]),
                b(Action::ToggleRecordWindow, "", &[]),
                b(Action::ToggleRecordPortal, "", &[]),
                b(Action::ToggleStream, "F8", &["Alt"]),
                b(Action::Screenshot, "F1", &["Alt"]),
                b(Action::ScreenshotRegion, "S", &["Alt", "Shift"]),
                b(Action::ScreenshotWindow, "", &[]),
                b(Action::ScreenshotPortal, "", &[]),
                b(Action::ToggleMic, "", &[]),
                b(Action::AddMarker, "", &[]),
            ],
        }
    }
}

impl Hotkeys {
    pub fn get(&self, action: Action) -> Hotkey {
        self.bindings.iter().find(|(a, _)| *a == action).map(|(_, h)| h.clone()).unwrap_or_default()
    }

    pub fn set(&mut self, action: Action, hotkey: Hotkey) {
        if let Some(entry) = self.bindings.iter_mut().find(|(a, _)| *a == action) {
            entry.1 = hotkey;
        } else {
            self.bindings.push((action, hotkey));
        }
    }

    pub fn conflict(&self, action: Action, hotkey: &Hotkey) -> Option<Action> {
        if !hotkey.is_set() {
            return None;
        }
        self.bindings.iter().find(|(a, h)| *a != action && h == hotkey).map(|(a, _)| *a)
    }

    pub fn clear(&mut self) {
        for (_, h) in self.bindings.iter_mut() {
            *h = Hotkey::none();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MainConfig {
    pub accent: AccentPreset,
    pub accent_rgb: [u8; 3],
    pub rounded_corners: bool,
    pub start_on_login: bool,
    pub hotkey_mode: HotkeyMode,
    pub hotkeys: Hotkeys,
    pub controller_hotkeys: bool,
    pub notification_speed: NotificationSpeed,
    pub language: String,
    pub exclude_metadata: bool,
    pub show_tray_icon: bool,
    pub overlay_opacity: f32,
    pub countdown_seconds: u32,
    pub low_disk_warning_gb: u32,
    pub play_sounds: bool,
}

impl Default for MainConfig {
    fn default() -> Self {
        Self {
            accent: AccentPreset::Nvidia,
            accent_rgb: [118, 185, 0],
            rounded_corners: true,
            start_on_login: false,
            hotkey_mode: HotkeyMode::Enabled,
            hotkeys: Hotkeys::default(),
            controller_hotkeys: false,
            notification_speed: NotificationSpeed::Normal,
            language: String::new(),
            exclude_metadata: false,
            show_tray_icon: true,
            overlay_opacity: 0.45,
            countdown_seconds: 0,
            low_disk_warning_gb: 5,
            play_sounds: false,
        }
    }
}

impl MainConfig {
    pub fn accent_color(&self) -> [u8; 3] {
        match self.accent {
            AccentPreset::Nvidia => [118, 185, 0],
            AccentPreset::Amd => [221, 0, 49],
            AccentPreset::Intel => [8, 109, 183],
            AccentPreset::Custom => self.accent_rgb,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Medium,
    High,
    #[default]
    VeryHigh,
    Ultra,
    Custom,
}

impl Quality {
    pub fn arg(self) -> &'static str {
        match self {
            Quality::Medium => "medium",
            Quality::High => "high",
            Quality::VeryHigh => "very_high",
            Quality::Ultra => "ultra",
            Quality::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    #[default]
    Auto,
    H264,
    Hevc,
    Hevc10bit,
    HevcHdr,
    Av1,
    Av110bit,
    Av1Hdr,
    Vp8,
    Vp9,
    H264Software,
}

impl VideoCodec {
    pub const ALL: [VideoCodec; 11] = [
        VideoCodec::Auto,
        VideoCodec::H264,
        VideoCodec::Hevc,
        VideoCodec::Hevc10bit,
        VideoCodec::HevcHdr,
        VideoCodec::Av1,
        VideoCodec::Av110bit,
        VideoCodec::Av1Hdr,
        VideoCodec::Vp8,
        VideoCodec::Vp9,
        VideoCodec::H264Software,
    ];

    pub fn arg(self) -> &'static str {
        match self {
            VideoCodec::Auto => "auto",
            VideoCodec::H264 => "h264",
            VideoCodec::Hevc => "hevc",
            VideoCodec::Hevc10bit => "hevc_10bit",
            VideoCodec::HevcHdr => "hevc_hdr",
            VideoCodec::Av1 => "av1",
            VideoCodec::Av110bit => "av1_10bit",
            VideoCodec::Av1Hdr => "av1_hdr",
            VideoCodec::Vp8 => "vp8",
            VideoCodec::Vp9 => "vp9",
            VideoCodec::H264Software => "h264",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::Auto => "Auto (Recommended)",
            VideoCodec::H264 => "H264",
            VideoCodec::Hevc => "HEVC",
            VideoCodec::Hevc10bit => "HEVC (10 bit, reduces banding)",
            VideoCodec::HevcHdr => "HEVC (HDR)",
            VideoCodec::Av1 => "AV1",
            VideoCodec::Av110bit => "AV1 (10 bit, reduces banding)",
            VideoCodec::Av1Hdr => "AV1 (HDR)",
            VideoCodec::Vp8 => "VP8",
            VideoCodec::Vp9 => "VP9",
            VideoCodec::H264Software => "H264 Software Encoder (Slow, not recommended)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodec {
    #[default]
    Opus,
    Aac,
    Flac,
    Mp3,
}

impl AudioCodec {
    pub fn arg(self) -> &'static str {
        match self {
            AudioCodec::Opus => "opus",
            AudioCodec::Aac => "aac",
            AudioCodec::Flac => "flac",
            AudioCodec::Mp3 => "mp3",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FramerateMode {
    #[default]
    Auto,
    Constant,
    Variable,
    Content,
}

impl FramerateMode {
    pub fn arg(self) -> &'static str {
        match self {
            FramerateMode::Auto => "vfr",
            FramerateMode::Constant => "cfr",
            FramerateMode::Variable => "vfr",
            FramerateMode::Content => "content",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColorRange {
    #[default]
    Limited,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    #[default]
    Yuv420,
    Yuv444,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AudioTrack {
    pub name: String,
    pub inputs: Vec<String>,
    pub invert_application_audio: bool,
}

impl AudioTrack {
    pub fn default_output() -> Self {
        Self { name: String::new(), inputs: vec!["device:default_output".to_string()], invert_application_audio: false }
    }

    pub fn to_arg(&self) -> String {
        let mut parts = Vec::new();
        for input in &self.inputs {
            if let Some(app) = input.strip_prefix("app:") {
                if self.invert_application_audio {
                    parts.push(format!("app-inverse:{app}"));
                } else {
                    parts.push(format!("app:{app}"));
                }
            } else if let Some(dev) = input.strip_prefix("device:") {
                parts.push(dev.to_string());
            } else {
                parts.push(input.clone());
            }
        }
        let joined = parts.join("|");
        if self.name.is_empty() {
            joined
        } else {
            format!("{}/{}", self.name, joined)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebcamFormat {
    #[default]
    Auto,
    Yuyv,
    Mjpeg,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Webcam {
    pub source: String,
    pub format: WebcamFormat,
    pub camera_width: u32,
    pub camera_height: u32,
    pub camera_fps: u32,
    pub x: i32,
    pub y: i32,
    pub width_percent: u32,
    pub height_percent: u32,
    pub flip_horizontally: bool,
}

impl Default for Webcam {
    fn default() -> Self {
        Self {
            source: String::new(),
            format: WebcamFormat::Auto,
            camera_width: 0,
            camera_height: 0,
            camera_fps: 0,
            x: 0,
            y: 0,
            width_percent: 30,
            height_percent: 30,
            flip_horizontally: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureOptions {
    pub advanced_view: bool,
    pub source: String,
    pub area_width: u32,
    pub area_height: u32,
    pub change_resolution: bool,
    pub video_width: u32,
    pub video_height: u32,
    pub fps: u32,
    pub quality: Quality,
    pub bitrate_kbps: u32,
    pub codec: VideoCodec,
    pub audio_codec: AudioCodec,
    pub audio_bitrate_kbps: u32,
    pub audio_tracks: Vec<AudioTrack>,
    pub framerate_mode: FramerateMode,
    pub color_range: ColorRange,
    pub pixel_format: PixelFormat,
    pub record_cursor: bool,
    pub low_power: bool,
    pub vulkan: bool,
    pub keyframe_interval: f32,
    pub webcam: Webcam,
    pub show_notifications: bool,
    pub led_indicator: bool,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            advanced_view: false,
            source: "focused_monitor".to_string(),
            area_width: 1920,
            area_height: 1080,
            change_resolution: false,
            video_width: 1920,
            video_height: 1080,
            fps: 60,
            quality: Quality::VeryHigh,
            bitrate_kbps: 15000,
            codec: VideoCodec::Auto,
            audio_codec: AudioCodec::Opus,
            audio_bitrate_kbps: 0,
            audio_tracks: vec![AudioTrack::default_output()],
            framerate_mode: FramerateMode::Auto,
            color_range: ColorRange::Limited,
            pixel_format: PixelFormat::Yuv420,
            record_cursor: true,
            low_power: false,
            vulkan: false,
            keyframe_interval: 2.0,
            webcam: Webcam::default(),
            show_notifications: true,
            led_indicator: false,
        }
    }
}

impl CaptureOptions {
    pub fn estimated_mb_per_minute(&self) -> f64 {
        self.bitrate_kbps as f64 * 60.0 / 8.0 / 1000.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordConfig {
    pub capture: CaptureOptions,
    pub save_directory: String,
    pub container: String,
    pub organize_into_folders: YesNoGames,
    pub name_after_application: YesNoGames,
    pub split_minutes: u32,
}

impl Default for RecordConfig {
    fn default() -> Self {
        Self {
            capture: CaptureOptions::default(),
            save_directory: paths::videos_dir().join("Recordings").to_string_lossy().into_owned(),
            container: "mp4".to_string(),
            organize_into_folders: YesNoGames::No,
            name_after_application: YesNoGames::No,
            split_minutes: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReplayAutostart {
    #[default]
    Off,
    OnStartup,
    OnStartupWithPower,
    OnGame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReplayStorage {
    #[default]
    Ram,
    Disk,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReplayConfig {
    pub capture: CaptureOptions,
    pub save_directory: String,
    pub container: String,
    pub organize_into_folders: YesNoGames,
    pub name_after_application: YesNoGames,
    pub autostart: ReplayAutostart,
    pub duration_seconds: u32,
    pub storage: ReplayStorage,
    pub restart_on_save: bool,
    pub short_save_seconds: u32,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        let capture = CaptureOptions { quality: Quality::Custom, bitrate_kbps: 40000, ..CaptureOptions::default() };
        Self {
            capture,
            save_directory: paths::videos_dir().join("Replays").to_string_lossy().into_owned(),
            container: "mp4".to_string(),
            organize_into_folders: YesNoGames::Yes,
            name_after_application: YesNoGames::Yes,
            autostart: ReplayAutostart::Off,
            duration_seconds: 60,
            storage: ReplayStorage::Ram,
            restart_on_save: false,
            short_save_seconds: 30,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StreamService {
    #[default]
    Twitch,
    Youtube,
    Kick,
    Rumble,
    Whip,
    Custom,
}

impl StreamService {
    pub const ALL: [StreamService; 6] = [
        StreamService::Twitch,
        StreamService::Youtube,
        StreamService::Kick,
        StreamService::Rumble,
        StreamService::Whip,
        StreamService::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StreamService::Twitch => "Twitch",
            StreamService::Youtube => "YouTube",
            StreamService::Kick => "Kick",
            StreamService::Rumble => "Rumble",
            StreamService::Whip => "WHIP",
            StreamService::Custom => "Custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamConfig {
    pub capture: CaptureOptions,
    pub service: StreamService,
    pub twitch_key: String,
    pub youtube_key: String,
    pub kick_url: String,
    pub kick_key: String,
    pub rumble_key: String,
    pub whip_url: String,
    pub whip_token: String,
    pub custom_url: String,
    pub custom_key: String,
    pub custom_container: String,
    pub keep_local_copy: bool,
    pub local_copy_directory: String,
}

impl Default for StreamConfig {
    fn default() -> Self {
        let capture = CaptureOptions {
            quality: Quality::Custom,
            bitrate_kbps: 6000,
            audio_codec: AudioCodec::Aac,
            codec: VideoCodec::H264,
            ..CaptureOptions::default()
        };
        Self {
            capture,
            service: StreamService::Twitch,
            twitch_key: String::new(),
            youtube_key: String::new(),
            kick_url: String::new(),
            kick_key: String::new(),
            rumble_key: String::new(),
            whip_url: String::new(),
            whip_token: String::new(),
            custom_url: String::new(),
            custom_key: String::new(),
            custom_container: "flv".to_string(),
            keep_local_copy: false,
            local_copy_directory: paths::videos_dir().join("Streams").to_string_lossy().into_owned(),
        }
    }
}

impl StreamConfig {
    pub fn target_url(&self) -> String {
        let join = |base: &str, key: &str| {
            let base = base.trim().trim_end_matches('/');
            let key = key.trim();
            if key.is_empty() {
                base.to_string()
            } else {
                format!("{base}/{key}")
            }
        };
        match self.service {
            StreamService::Twitch => join("rtmp://live.twitch.tv/app", &self.twitch_key),
            StreamService::Youtube => join("rtmp://a.rtmp.youtube.com/live2", &self.youtube_key),
            StreamService::Kick => join(&self.kick_url, &self.kick_key),
            StreamService::Rumble => join("rtmp://rtmp.rumble.com/live", &self.rumble_key),
            StreamService::Whip => self.whip_url.trim().to_string(),
            StreamService::Custom => join(&self.custom_url, &self.custom_key),
        }
    }

    pub fn container(&self) -> String {
        match self.service {
            StreamService::Whip => "whip".to_string(),
            StreamService::Custom => {
                let url = self.custom_url.trim();
                if url.starts_with("srt://") {
                    "mpegts".to_string()
                } else if url.starts_with("rtsp://") {
                    "rtsp".to_string()
                } else {
                    self.custom_container.clone()
                }
            }
            _ => "flv".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    #[default]
    Png,
    Jpg,
}

impl ImageFormat {
    pub fn ext(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpg => "jpg",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenshotConfig {
    pub source: String,
    pub change_resolution: bool,
    pub image_width: u32,
    pub image_height: u32,
    pub quality: Quality,
    pub record_cursor: bool,
    pub save_directory: String,
    pub format: ImageFormat,
    pub organize_into_folders: YesNoGames,
    pub name_after_application: YesNoGames,
    pub save_to_clipboard: bool,
    pub save_to_disk: bool,
    pub show_notifications: bool,
    pub led_indicator: bool,
    pub open_command: String,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        Self {
            source: "focused_monitor".to_string(),
            change_resolution: false,
            image_width: 1920,
            image_height: 1080,
            quality: Quality::VeryHigh,
            record_cursor: true,
            save_directory: paths::pictures_dir().join("Screenshots").to_string_lossy().into_owned(),
            format: ImageFormat::Png,
            organize_into_folders: YesNoGames::No,
            name_after_application: YesNoGames::No,
            save_to_clipboard: false,
            save_to_disk: true,
            show_notifications: true,
            led_indicator: false,
            open_command: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub main: MainConfig,
    pub record: RecordConfig,
    pub replay: ReplayConfig,
    pub stream: StreamConfig,
    pub screenshot: ScreenshotConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            main: MainConfig::default(),
            record: RecordConfig::default(),
            replay: ReplayConfig::default(),
            stream: StreamConfig::default(),
            screenshot: ScreenshotConfig::default(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        paths::config_file()
    }

    pub fn load() -> Config {
        match std::fs::read_to_string(Self::path()) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|err| {
                eprintln!("tgow-recorder: config is broken ({err}), using defaults");
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}
