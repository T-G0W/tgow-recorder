mod args;
mod audio;
mod capture;
mod control;
mod convert;
mod frame;
mod output;
mod report;
mod session;
mod video;

use args::Invocation;

#[cfg(windows)]
fn list_monitors() {
    match capture::dxgi::monitors() {
        Ok(list) => {
            for m in list {
                println!("{}|{}x{}|{}|{}", m.name, m.width, m.height, m.x, m.y);
            }
        }
        Err(e) => eprintln!("tgow-rec: {e:#}"),
    }
}

#[cfg(unix)]
fn list_monitors() {
    if tgow_common::system::is_wayland() {
        for c in capture::drm::connected() {
            println!("{}|{}x{}|0|0", c.name, c.width, c.height);
        }
    } else {
        match capture::x11::monitors() {
            Ok(list) => {
                for m in list {
                    println!("{}|{}x{}|{}|{}", m.name, m.width, m.height, m.x, m.y);
                }
            }
            Err(e) => eprintln!("tgow-rec: {e:#}"),
        }
    }
}

fn list_audio() {
    println!("default_output|Default output|output");
    println!("default_input|Default input|input");
    match audio::backend::list_devices() {
        Ok(list) => {
            for d in list {
                println!("{}|{}|{}", d.id, d.description, if d.output { "output" } else { "input" });
            }
        }
        Err(e) => eprintln!("tgow-rec: {e:#}"),
    }
}

fn list_apps() {
    match audio::backend::list_applications() {
        Ok(list) => {
            for a in list {
                println!("{a}");
            }
        }
        Err(e) => eprintln!("tgow-rec: {e:#}"),
    }
}

fn info() {
    let _ = ffmpeg_next::init();
    let vendor = tgow_common::system::gpu_vendor();
    println!("gpu_vendor|{vendor}");
    println!("wayland|{}", if tgow_common::system::is_wayland() { "yes" } else { "no" });
    let candidates: [(&str, &[&str]); 5] = if cfg!(windows) {
        [
            ("h264", &["h264_nvenc", "h264_amf", "h264_qsv", "h264_vulkan", "h264_mf"]),
            ("hevc", &["hevc_nvenc", "hevc_amf", "hevc_qsv", "hevc_vulkan", "hevc_mf"]),
            ("av1", &["av1_nvenc", "av1_amf", "av1_qsv", "av1_vulkan"]),
            ("vp8", &["libvpx"]),
            ("vp9", &["vp9_qsv", "libvpx-vp9"]),
        ]
    } else {
        [
            ("h264", &["h264_nvenc", "h264_vaapi", "h264_vulkan", "libx264"]),
            ("hevc", &["hevc_nvenc", "hevc_vaapi", "hevc_vulkan"]),
            ("av1", &["av1_nvenc", "av1_vaapi", "av1_vulkan"]),
            ("vp8", &["vp8_vaapi", "libvpx"]),
            ("vp9", &["vp9_vaapi", "libvpx-vp9"]),
        ]
    };
    for (codec, names) in candidates {
        let found: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| !n.is_empty() && ffmpeg_next::encoder::find_by_name(n).is_some())
            .collect();
        println!("video_codec|{codec}|{}", found.join(","));
    }
    for (name, enc) in [("opus", "libopus"), ("aac", "aac"), ("flac", "flac"), ("mp3", "libmp3lame")] {
        if ffmpeg_next::encoder::find_by_name(enc).is_some() {
            println!("audio_codec|{name}");
        }
    }
}

fn main() {
    let invocation = match args::parse(std::env::args().collect()) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("tgow-rec: {e:#}");
            std::process::exit(2);
        }
    };
    let result = match invocation {
        Invocation::Help => {
            print!("{}", args::HELP);
            Ok(())
        }
        Invocation::Version => {
            println!("tgow-rec {}", tgow_common::VERSION);
            Ok(())
        }
        Invocation::ListMonitors => {
            list_monitors();
            Ok(())
        }
        Invocation::ListAudioDevices => {
            list_audio();
            Ok(())
        }
        Invocation::ListApplicationAudio => {
            list_apps();
            Ok(())
        }
        Invocation::ListCameras => {
            for (path, name) in capture::camera::list() {
                println!("{path}|{name}");
            }
            Ok(())
        }
        Invocation::Info => {
            info();
            Ok(())
        }
        Invocation::Run(opts) => match opts.screenshot.clone() {
            Some(path) => {
                let _ = ffmpeg_next::init();
                session::screenshot(&opts, &path)
            }
            None => session::run(*opts),
        },
    };
    if let Err(e) = result {
        report::error(&format!("{e:#}"));
        std::process::exit(1);
    }
}
