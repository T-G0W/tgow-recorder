use eframe::egui::{self, DragValue, RichText, TextEdit, Ui};
use tgow_common::config::{
    AudioCodec, AudioTrack, CaptureOptions, ColorRange, Config, FramerateMode, ImageFormat, PixelFormat, Quality,
    RecordConfig, ReplayAutostart, ReplayConfig, ReplayStorage, ScreenshotConfig, StreamConfig, StreamService,
    VideoCodec, WebcamFormat,
};
use tgow_common::{system, tr, trf};

use crate::sources::Sources;
use crate::theme::{self, Theme};
use crate::widgets::{self, field_label, info_line, section};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Replay,
    Record,
    Stream,
    Screenshot,
}

impl Mode {
    pub fn title(self) -> String {
        match self {
            Mode::Replay => tr("Instant Replay"),
            Mode::Record => tr("Record"),
            Mode::Stream => tr("Livestream"),
            Mode::Screenshot => tr("Screenshot"),
        }
    }
}

pub fn advanced_flag(config: &mut Config, mode: Mode) -> Option<&mut bool> {
    match mode {
        Mode::Replay => Some(&mut config.replay.capture.advanced_view),
        Mode::Record => Some(&mut config.record.capture.advanced_view),
        Mode::Stream => Some(&mut config.stream.capture.advanced_view),
        Mode::Screenshot => None,
    }
}

pub fn page(ui: &mut Ui, theme: &Theme, config: &mut Config, mode: Mode, sources: &Sources) {
    match mode {
        Mode::Replay => replay(ui, theme, &mut config.replay, sources),
        Mode::Record => record(ui, theme, &mut config.record, sources),
        Mode::Stream => stream(ui, theme, &mut config.stream, sources),
        Mode::Screenshot => screenshot(ui, theme, &mut config.screenshot, sources),
    }
}

fn source_options(sources: &Sources, current: &str, allow_window: bool) -> Vec<(String, String)> {
    let main = if cfg!(windows) { tr("Main monitor") } else { tr("Focused monitor") };
    let mut options = vec![("focused_monitor".to_string(), main)];
    for m in &sources.monitors {
        options
            .push((m.name.clone(), trf("Monitor %s (%dx%d)", &[&m.name, &m.width.to_string(), &m.height.to_string()])));
    }
    if allow_window && !system::is_wayland() {
        options.push(("window".to_string(), tr("Window")));
        options.push(("focused".to_string(), tr("Follow focused window")));
    }
    options.push(("region".to_string(), tr("Region")));
    if !cfg!(windows) {
        options.push(("portal".to_string(), tr("Desktop portal")));
    }
    if !options.iter().any(|(id, _)| id == current) {
        options.push((current.to_string(), current.to_string()));
    }
    options
}

fn quality_options(constant_label: &str) -> Vec<(Quality, String)> {
    vec![
        (Quality::Medium, tr("Medium")),
        (Quality::High, tr("High")),
        (Quality::VeryHigh, tr("Very high (Recommended)")),
        (Quality::Ultra, tr("Ultra")),
        (Quality::Custom, tr(constant_label)),
    ]
}

fn capture(ui: &mut Ui, theme: &Theme, opts: &mut CaptureOptions, sources: &Sources, mode: Mode) {
    let advanced = opts.advanced_view;
    section(ui, theme, &tr("Capture"), |ui| {
        let options = source_options(sources, &opts.source, true);
        widgets::labeled_combo(ui, &tr("Capture source:"), &mut opts.source, &options);
        if opts.source == "focused" {
            field_label(ui, &tr("Area size:"));
            widgets::size_pair(ui, &mut opts.area_width, &mut opts.area_height);
        }
        ui.checkbox(&mut opts.change_resolution, tr("Change video resolution"));
        if opts.change_resolution {
            field_label(ui, &tr("Video resolution limit:"));
            widgets::size_pair(ui, &mut opts.video_width, &mut opts.video_height);
        }
        ui.checkbox(&mut opts.record_cursor, tr("Record cursor"));
    });

    if advanced {
        webcam(ui, theme, opts, sources);
    }

    audio(ui, theme, opts, sources, advanced);
    video(ui, theme, opts, mode, advanced);
}

fn webcam(ui: &mut Ui, theme: &Theme, opts: &mut CaptureOptions, sources: &Sources) {
    let cam = &mut opts.webcam;
    section(ui, theme, &tr("Webcam"), |ui| {
        let mut options = vec![(String::new(), tr("None"))];
        for c in &sources.cameras {
            options.push((c.path.clone(), format!("{} ({})", c.name, c.path)));
        }
        if !cam.source.is_empty() && !options.iter().any(|(p, _)| *p == cam.source) {
            options.push((cam.source.clone(), cam.source.clone()));
        }
        widgets::labeled_combo(ui, &tr("Webcam source:"), &mut cam.source, &options);
        if cam.source.is_empty() {
            return;
        }
        widgets::labeled_combo(
            ui,
            &tr("Video format:"),
            &mut cam.format,
            &[
                (WebcamFormat::Auto, tr("Auto (recommended)")),
                (WebcamFormat::Yuyv, "YUYV".to_string()),
                (WebcamFormat::Mjpeg, "Motion-JPEG".to_string()),
            ],
        );
        field_label(ui, &tr("Camera resolution and fps (0 = best):"));
        ui.horizontal(|ui| {
            widgets::number(ui, &mut cam.camera_width, 0, 7680);
            ui.label("x");
            widgets::number(ui, &mut cam.camera_height, 0, 4320);
            ui.label("@");
            widgets::number(ui, &mut cam.camera_fps, 0, 240);
        });
        field_label(ui, &tr("Position in the video (percent, 0 = left/top, 100 = right/bottom):"));
        ui.horizontal(|ui| {
            ui.label("X");
            ui.add(DragValue::new(&mut cam.x).range(0..=100).suffix(" %"));
            ui.label("Y");
            ui.add(DragValue::new(&mut cam.y).range(0..=100).suffix(" %"));
        });
        field_label(ui, &tr("Size in percent of the video:"));
        ui.horizontal(|ui| {
            widgets::number(ui, &mut cam.width_percent, 1, 100);
            ui.label("x");
            widgets::number(ui, &mut cam.height_percent, 1, 100);
        });
        ui.checkbox(&mut cam.flip_horizontally, tr("Flip camera horizontally"));
    });
}

#[derive(Clone, Copy, PartialEq)]
enum InputKind {
    Output,
    Input,
    App,
}

fn input_kind(sources: &Sources, value: &str) -> InputKind {
    if value.starts_with("app:") {
        return InputKind::App;
    }
    let id = value.strip_prefix("device:").unwrap_or(value);
    if id == "default_input" || sources.inputs().any(|d| d.id == id) {
        InputKind::Input
    } else {
        InputKind::Output
    }
}

fn audio(ui: &mut Ui, theme: &Theme, opts: &mut CaptureOptions, sources: &Sources, advanced: bool) {
    section(ui, theme, &tr("Audio"), |ui| {
        let mut codecs = vec![(AudioCodec::Opus, tr("Opus (Recommended)")), (AudioCodec::Aac, tr("AAC"))];
        if advanced {
            codecs.push((AudioCodec::Flac, "FLAC".to_string()));
            codecs.push((AudioCodec::Mp3, "MP3".to_string()));
        }
        widgets::labeled_combo(ui, &tr("Audio codec:"), &mut opts.audio_codec, &codecs);
        if advanced {
            widgets::labeled_number(ui, &tr("Audio bitrate (Kbps, 0 = auto):"), &mut opts.audio_bitrate_kbps, 0, 512);
        }
        ui.add_space(6.0);

        let mut remove_track = None;
        let track_count = opts.audio_tracks.len();
        for (index, track) in opts.audio_tracks.iter_mut().enumerate() {
            egui::Frame::new()
                .fill(theme::FIELD_FILL.gamma_multiply(0.55))
                .corner_radius(theme.radius(8))
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(trf("Audio track #%d", &[&(index + 1).to_string()])).strong());
                        if advanced && track_count > 1 {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button(tr("Delete")).clicked() {
                                    remove_track = Some(index);
                                }
                            });
                        }
                    });
                    if advanced {
                        widgets::text_field(ui, &tr("Track title:"), &mut track.name, false);
                    }
                    track_inputs(ui, theme, track, sources, index);
                });
            ui.add_space(8.0);
        }
        if let Some(i) = remove_track {
            opts.audio_tracks.remove(i);
        }
        if advanced && ui.button(tr("Add audio track")).clicked() {
            opts.audio_tracks.push(AudioTrack::default());
        }
    });
}

fn track_inputs(ui: &mut Ui, theme: &Theme, track: &mut AudioTrack, sources: &Sources, track_index: usize) {
    let mut remove = None;
    for (i, value) in track.inputs.iter_mut().enumerate() {
        let kind = input_kind(sources, value);
        ui.horizontal(|ui| {
            let label = match kind {
                InputKind::Output => tr("Output device:"),
                InputKind::Input => tr("Input device:"),
                InputKind::App => tr("Application:"),
            };
            ui.add_sized([150.0, 30.0], egui::Label::new(RichText::new(label).color(theme::TEXT_DIM)));
            let salt = ("audio-input", track_index, i);
            match kind {
                InputKind::Output | InputKind::Input => {
                    let mut options: Vec<(String, String)> = Vec::new();
                    if kind == InputKind::Output {
                        options.push(("device:default_output".into(), tr("Default output")));
                        for d in sources.outputs() {
                            options.push((format!("device:{}", d.id), d.description.clone()));
                        }
                    } else {
                        options.push(("device:default_input".into(), tr("Default input")));
                        for d in sources.inputs() {
                            options.push((format!("device:{}", d.id), d.description.clone()));
                        }
                    }
                    if !value.starts_with("device:") {
                        *value = format!("device:{value}");
                    }
                    if !options.iter().any(|(id, _)| id == value) {
                        let id = value.trim_start_matches("device:").to_string();
                        options.push((value.clone(), sources.device_label(&id)));
                    }
                    widgets::combo(ui, salt, value, &options);
                }
                InputKind::App => {
                    let mut name = value.trim_start_matches("app:").to_string();
                    let mut options: Vec<(String, String)> =
                        sources.apps.iter().map(|a| (a.clone(), a.clone())).collect();
                    if !name.is_empty() && !options.iter().any(|(a, _)| *a == name) {
                        options.push((name.clone(), name.clone()));
                    }
                    let mut changed = widgets::combo(ui, salt, &mut name, &options);
                    if ui.add(TextEdit::singleline(&mut name).hint_text(tr("Custom...")).desired_width(170.0)).changed()
                    {
                        changed = true;
                    }
                    if changed {
                        *value = format!("app:{name}");
                    }
                }
            }
            if widgets::small_icon_button(ui, theme, 30.0, |p, r, c| crate::icons::close(p, r.center(), 12.0, c))
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        track.inputs.remove(i);
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("Add output device")).clicked() {
            track.inputs.push("device:default_output".into());
        }
        if ui.button(tr("Add input device")).clicked() {
            track.inputs.push("device:default_input".into());
        }
        if ui.button(tr("Add application audio")).clicked() {
            let first = sources.apps.first().cloned().unwrap_or_default();
            track.inputs.push(format!("app:{first}"));
        }
    });
    let has_app = track.inputs.iter().any(|v| v.starts_with("app:"));
    let has_output = track.inputs.iter().any(|v| input_kind(sources, v) == InputKind::Output);
    if has_app {
        ui.checkbox(
            &mut track.invert_application_audio,
            tr("Record audio from all applications except the selected ones"),
        );
        if has_output {
            ui.label(
                RichText::new(tr("Recording output devices and application audio may record all output audio, which is likely not what you want to do. Remove the output devices."))
                    .color(egui::Color32::from_rgb(230, 180, 60))
                    .size(14.0),
            );
        }
    }
}

fn video(ui: &mut Ui, theme: &Theme, opts: &mut CaptureOptions, mode: Mode, advanced: bool) {
    section(ui, theme, &tr("Video"), |ui| {
        let constant = if mode == Mode::Record { "Constant bitrate" } else { "Constant bitrate (Recommended)" };
        widgets::labeled_combo(ui, &tr("Video quality:"), &mut opts.quality, &quality_options(constant));
        if opts.quality == Quality::Custom {
            widgets::labeled_number(ui, &tr("Video bitrate (Kbps):"), &mut opts.bitrate_kbps, 500, 500_000);
            info_line(
                ui,
                &trf(
                    "Estimated video file size per minute (excluding audio): %sMB",
                    &[&format!("{:.2}", opts.estimated_mb_per_minute())],
                ),
            );
        }
        let codecs: Vec<(VideoCodec, String)> = VideoCodec::ALL
            .iter()
            .copied()
            .filter(|c| {
                advanced || matches!(c, VideoCodec::Auto | VideoCodec::H264 | VideoCodec::Hevc | VideoCodec::Av1)
            })
            .map(|c| (c, tr(c.label())))
            .collect();
        let mut codecs = codecs;
        if !codecs.iter().any(|(c, _)| *c == opts.codec) {
            codecs.push((opts.codec, tr(opts.codec.label())));
        }
        widgets::labeled_combo(ui, &tr("Video codec:"), &mut opts.codec, &codecs);
        widgets::labeled_number(ui, &tr("Frame rate:"), &mut opts.fps, 1, 500);
        if advanced {
            widgets::labeled_combo(
                ui,
                &tr("Frame rate mode:"),
                &mut opts.framerate_mode,
                &[
                    (FramerateMode::Auto, tr("Auto (Recommended)")),
                    (FramerateMode::Constant, tr("Constant")),
                    (FramerateMode::Variable, tr("Variable")),
                    (FramerateMode::Content, tr("Sync to content")),
                ],
            );
            widgets::labeled_combo(
                ui,
                &tr("Color range:"),
                &mut opts.color_range,
                &[(ColorRange::Limited, tr("Limited")), (ColorRange::Full, tr("Full"))],
            );
            widgets::labeled_combo(
                ui,
                &tr("Pixel format:"),
                &mut opts.pixel_format,
                &[(PixelFormat::Yuv420, "YUV 4:2:0".to_string()), (PixelFormat::Yuv444, "YUV 4:4:4".to_string())],
            );
            field_label(ui, &tr("Keyframe interval (seconds):"));
            ui.add(DragValue::new(&mut opts.keyframe_interval).range(0.5..=30.0).speed(0.1).fixed_decimals(1));
            ui.checkbox(&mut opts.low_power, tr("Record in low-power mode"));
            ui.checkbox(&mut opts.vulkan, tr("Enable vulkan video encoding (experimental)"));
        }
    });

    let (title, notify, led) = match mode {
        Mode::Replay => ("Replay indicator", "Show replay notifications", "Show replay status with scroll lock LED"),
        Mode::Stream => {
            ("Streaming indicator", "Show streaming notifications", "Show streaming status with scroll lock LED")
        }
        _ => ("Recording indicator", "Show recording notifications", "Show recording status with scroll lock LED"),
    };
    section(ui, theme, &tr(title), |ui| {
        ui.checkbox(&mut opts.show_notifications, tr(notify));
        ui.checkbox(&mut opts.led_indicator, tr(led));
    });
}

fn containers(include_ts: bool) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> =
        ["mp4", "mkv", "mov", "webm", "flv"].iter().map(|c| (c.to_string(), c.to_string())).collect();
    if include_ts {
        list.push(("ts".to_string(), "ts".to_string()));
    }
    list
}

fn record(ui: &mut Ui, theme: &Theme, cfg: &mut RecordConfig, sources: &Sources) {
    capture(ui, theme, &mut cfg.capture, sources, Mode::Record);
    let advanced = cfg.capture.advanced_view;
    section(ui, theme, &tr("File info"), |ui| {
        widgets::path_field(ui, &tr("Directory to save videos:"), &mut cfg.save_directory);
        widgets::labeled_combo(ui, &tr("Container:"), &mut cfg.container, &containers(true));
        widgets::yes_no_games(
            ui,
            &tr("Organize videos into folders:"),
            &mut cfg.organize_into_folders,
            "Yes, but only for videos of games",
        );
        widgets::yes_no_games(
            ui,
            &tr("Name the video file after the captured application:"),
            &mut cfg.name_after_application,
            "Yes, but only for videos of games",
        );
        if advanced {
            widgets::labeled_number(
                ui,
                &tr("Split recording every (minutes, 0 = off):"),
                &mut cfg.split_minutes,
                0,
                1440,
            );
        }
    });
}

fn replay(ui: &mut Ui, theme: &Theme, cfg: &mut ReplayConfig, sources: &Sources) {
    capture(ui, theme, &mut cfg.capture, sources, Mode::Replay);
    section(ui, theme, &tr("General"), |ui| {
        widgets::labeled_number(ui, &tr("Replay duration in seconds:"), &mut cfg.duration_seconds, 2, 86_400);
        widgets::labeled_number(ui, &tr("Short save duration in seconds:"), &mut cfg.short_save_seconds, 2, 86_400);
        widgets::labeled_combo(
            ui,
            &tr("Where should temporary replay data be stored?"),
            &mut cfg.storage,
            &[(ReplayStorage::Ram, tr("RAM")), (ReplayStorage::Disk, tr("Disk (Not recommended on SSDs)"))],
        );
        ui.checkbox(&mut cfg.restart_on_save, tr("Restart replay on save"));
        if cfg.capture.quality == Quality::Custom {
            let mb = cfg.capture.bitrate_kbps as f64 * cfg.duration_seconds as f64 / 8.0 / 1000.0;
            info_line(ui, &trf("Estimated max replay size: %sMB", &[&format!("{mb:.2}")]));
        }
    });
    section(ui, theme, &tr("Autostart"), |ui| {
        widgets::combo(
            ui,
            "replay-autostart",
            &mut cfg.autostart,
            &[
                (ReplayAutostart::Off, tr("Don't turn on replay automatically")),
                (ReplayAutostart::OnStartup, tr("Turn on replay when this program starts")),
                (ReplayAutostart::OnStartupWithPower, tr("Only turn on replay if a power supply is connected")),
                (ReplayAutostart::OnGame, tr("Turn on replay when starting a game")),
            ],
        );
    });
    section(ui, theme, &tr("File info"), |ui| {
        widgets::path_field(ui, &tr("Directory to save replays:"), &mut cfg.save_directory);
        widgets::labeled_combo(ui, &tr("Container:"), &mut cfg.container, &containers(false));
        widgets::yes_no_games(
            ui,
            &tr("Organize videos into folders:"),
            &mut cfg.organize_into_folders,
            "Yes, but only for videos of games",
        );
        widgets::yes_no_games(
            ui,
            &tr("Name the video file after the captured application:"),
            &mut cfg.name_after_application,
            "Yes, but only for videos of games",
        );
    });
}

fn stream(ui: &mut Ui, theme: &Theme, cfg: &mut StreamConfig, sources: &Sources) {
    section(ui, theme, &tr("Streaming info"), |ui| {
        let services: Vec<(StreamService, String)> = StreamService::ALL.iter().map(|s| (*s, tr(s.label()))).collect();
        widgets::labeled_combo(ui, &tr("Stream service:"), &mut cfg.service, &services);
        match cfg.service {
            StreamService::Twitch => widgets::text_field(ui, &tr("Stream key:"), &mut cfg.twitch_key, true),
            StreamService::Youtube => widgets::text_field(ui, &tr("Stream key:"), &mut cfg.youtube_key, true),
            StreamService::Rumble => widgets::text_field(ui, &tr("Stream key:"), &mut cfg.rumble_key, true),
            StreamService::Kick => {
                widgets::text_field(ui, &tr("Stream URL:"), &mut cfg.kick_url, false);
                widgets::text_field(ui, &tr("Stream key:"), &mut cfg.kick_key, true);
            }
            StreamService::Whip => {
                widgets::text_field(ui, &tr("Stream URL:"), &mut cfg.whip_url, false);
                widgets::text_field(ui, &tr("Bearer token (optional):"), &mut cfg.whip_token, true);
            }
            StreamService::Custom => {
                widgets::text_field(ui, &tr("Stream URL:"), &mut cfg.custom_url, false);
                widgets::text_field(ui, &tr("Stream key:"), &mut cfg.custom_key, true);
                widgets::labeled_combo(
                    ui,
                    &tr("Container:"),
                    &mut cfg.custom_container,
                    &[
                        ("flv".to_string(), "flv".to_string()),
                        ("mpegts".to_string(), "mpegts".to_string()),
                        ("mp4".to_string(), "mp4".to_string()),
                        ("mkv".to_string(), "mkv".to_string()),
                        ("webm".to_string(), "webm".to_string()),
                    ],
                );
            }
        }
        ui.checkbox(&mut cfg.keep_local_copy, tr("Keep a local copy of the stream"));
        if cfg.keep_local_copy {
            widgets::path_field(ui, &tr("Directory to save videos:"), &mut cfg.local_copy_directory);
        }
    });
    capture(ui, theme, &mut cfg.capture, sources, Mode::Stream);
}

fn screenshot(ui: &mut Ui, theme: &Theme, cfg: &mut ScreenshotConfig, sources: &Sources) {
    section(ui, theme, &tr("Capture"), |ui| {
        let options = source_options(sources, &cfg.source, true);
        widgets::labeled_combo(ui, &tr("Capture source:"), &mut cfg.source, &options);
        ui.checkbox(&mut cfg.change_resolution, tr("Change image resolution"));
        if cfg.change_resolution {
            field_label(ui, &tr("Image resolution limit:"));
            widgets::size_pair(ui, &mut cfg.image_width, &mut cfg.image_height);
        }
        ui.checkbox(&mut cfg.record_cursor, tr("Record cursor"));
    });
    section(ui, theme, &tr("Image"), |ui| {
        widgets::labeled_combo(
            ui,
            &tr("Image quality:"),
            &mut cfg.quality,
            &[
                (Quality::Medium, tr("Medium")),
                (Quality::High, tr("High")),
                (Quality::VeryHigh, tr("Very high (Recommended)")),
                (Quality::Ultra, tr("Ultra")),
            ],
        );
        widgets::labeled_combo(
            ui,
            &tr("Image format:"),
            &mut cfg.format,
            &[(ImageFormat::Png, "PNG".to_string()), (ImageFormat::Jpg, "JPEG".to_string())],
        );
    });
    section(ui, theme, &tr("File info"), |ui| {
        widgets::path_field(ui, &tr("Directory to save screenshots:"), &mut cfg.save_directory);
        widgets::yes_no_games(
            ui,
            &tr("Organize screenshots into folders:"),
            &mut cfg.organize_into_folders,
            "Yes, but only for screenshots of games",
        );
        widgets::yes_no_games(
            ui,
            &tr("Name the screenshot file after the captured application:"),
            &mut cfg.name_after_application,
            "Yes, but only for screenshots of games",
        );
    });
    section(ui, theme, &tr("General"), |ui| {
        ui.checkbox(&mut cfg.save_to_clipboard, tr("Save screenshot to clipboard"));
        ui.checkbox(&mut cfg.save_to_disk, tr("Save screenshot to disk"));
    });
    section(ui, theme, &tr("Screenshot indicator"), |ui| {
        ui.checkbox(&mut cfg.show_notifications, tr("Show screenshot notifications"));
        ui.checkbox(&mut cfg.led_indicator, tr("Blink scroll lock led when taking a screenshot"));
    });
    section(ui, theme, &tr("Script"), |ui| {
        widgets::text_field(ui, &tr("Command to open the screenshot with:"), &mut cfg.open_command, false);
    });
}
