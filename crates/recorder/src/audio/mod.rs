pub mod sources;

#[cfg(unix)]
pub mod pulse;
#[cfg(unix)]
pub use pulse as backend;

#[cfg(windows)]
pub mod wasapi;
#[cfg(windows)]
pub use wasapi as backend;

use anyhow::{anyhow, Result};
use ff::ffi;
use ff::format::Sample;
use ff::{ChannelLayout, Dictionary, Packet, Rational};
use ffmpeg_next as ff;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::output::Tagged;
use sources::{SourceBuffer, TrackInputs, CHANNELS, RATE};

const LATENCY: Duration = Duration::from_millis(60);
const PRIME_SAMPLES: usize = (RATE as usize / 25) * CHANNELS;

pub struct AudioEncoder {
    enc: ff::encoder::Audio,
    format: Sample,
    frame_size: usize,
    pending: Vec<f32>,
    next_pts: i64,
    pub time_base: Rational,
}

unsafe impl Send for AudioEncoder {}

fn encoder_for(codec: &str) -> (&'static str, u32) {
    match codec {
        "aac" => ("aac", 160),
        "flac" => ("flac", 0),
        "mp3" => ("libmp3lame", 192),
        _ => ("libopus", 128),
    }
}

impl AudioEncoder {
    pub fn open(
        codec: &str,
        bitrate_kbps: u32,
        global_header: bool,
        extra: &[(String, String)],
    ) -> Result<AudioEncoder> {
        let (name, default_rate) = encoder_for(codec);
        let ff_codec = ff::encoder::find_by_name(name).ok_or_else(|| anyhow!("FFmpeg has no {name} encoder"))?;
        let supported: Vec<Sample> =
            ff_codec.audio().ok().and_then(|a| a.formats()).map(|f| f.collect()).unwrap_or_default();
        let preferred = [
            Sample::F32(ff::format::sample::Type::Packed),
            Sample::F32(ff::format::sample::Type::Planar),
            Sample::I16(ff::format::sample::Type::Packed),
            Sample::I16(ff::format::sample::Type::Planar),
            Sample::I32(ff::format::sample::Type::Packed),
            Sample::I32(ff::format::sample::Type::Planar),
        ];
        let format = preferred
            .into_iter()
            .find(|p| supported.is_empty() || supported.contains(p))
            .ok_or_else(|| anyhow!("{name} wants a sample format we can't produce"))?;

        let ctx = ff::codec::context::Context::new_with_codec(ff_codec);
        let mut enc = ctx.encoder().audio()?;
        enc.set_rate(RATE as i32);
        enc.set_format(format);
        enc.set_channel_layout(ChannelLayout::STEREO);
        enc.set_time_base(Rational::new(1, RATE as i32));
        let rate = if bitrate_kbps > 0 { bitrate_kbps } else { default_rate };
        if rate > 0 {
            enc.set_bit_rate(rate as usize * 1000);
        }
        if global_header {
            enc.set_flags(ff::codec::Flags::GLOBAL_HEADER);
        }
        let mut dict = Dictionary::new();
        if name == "libopus" {
            dict.set("application", "audio");
        }
        for (k, v) in extra {
            dict.set(k, v);
        }
        let enc = enc.open_as_with(ff_codec, dict).map_err(|e| anyhow!("could not open {name}: {e}"))?;
        let frame_size = match enc.frame_size() as usize {
            0 => 1024,
            n => n,
        };
        Ok(AudioEncoder {
            enc,
            format,
            frame_size,
            pending: Vec::with_capacity(frame_size * CHANNELS * 4),
            next_pts: 0,
            time_base: Rational::new(1, RATE as i32),
        })
    }

    pub fn parameters(&self) -> ff::codec::Parameters {
        let mut params = ff::codec::Parameters::new();
        unsafe {
            ffi::avcodec_parameters_from_context(params.as_mut_ptr(), self.enc.as_ptr());
        }
        params
    }

    fn fill(&self, frame: &mut ff::frame::Audio, chunk: &[f32]) {
        let n = self.frame_size;
        match self.format {
            Sample::F32(ff::format::sample::Type::Packed) => {
                let dst = frame.data_mut(0);
                for (i, s) in chunk.iter().enumerate() {
                    dst[i * 4..i * 4 + 4].copy_from_slice(&s.to_ne_bytes());
                }
            }
            Sample::F32(ff::format::sample::Type::Planar) => {
                for ch in 0..CHANNELS {
                    let dst = frame.data_mut(ch);
                    for i in 0..n {
                        dst[i * 4..i * 4 + 4].copy_from_slice(&chunk[i * CHANNELS + ch].to_ne_bytes());
                    }
                }
            }
            Sample::I16(t) => {
                let conv = |s: f32| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_ne_bytes();
                if t == ff::format::sample::Type::Packed {
                    let dst = frame.data_mut(0);
                    for (i, s) in chunk.iter().enumerate() {
                        dst[i * 2..i * 2 + 2].copy_from_slice(&conv(*s));
                    }
                } else {
                    for ch in 0..CHANNELS {
                        let dst = frame.data_mut(ch);
                        for i in 0..n {
                            dst[i * 2..i * 2 + 2].copy_from_slice(&conv(chunk[i * CHANNELS + ch]));
                        }
                    }
                }
            }
            Sample::I32(t) => {
                let conv = |s: f32| ((s.clamp(-1.0, 1.0) as f64 * 2147483647.0) as i32).to_ne_bytes();
                if t == ff::format::sample::Type::Packed {
                    let dst = frame.data_mut(0);
                    for (i, s) in chunk.iter().enumerate() {
                        dst[i * 4..i * 4 + 4].copy_from_slice(&conv(*s));
                    }
                } else {
                    for ch in 0..CHANNELS {
                        let dst = frame.data_mut(ch);
                        for i in 0..n {
                            dst[i * 4..i * 4 + 4].copy_from_slice(&conv(chunk[i * CHANNELS + ch]));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub fn push(&mut self, samples: &[f32], out: &mut Vec<Packet>) -> Result<()> {
        self.pending.extend_from_slice(samples);
        let per = self.frame_size * CHANNELS;
        while self.pending.len() >= per {
            let chunk: Vec<f32> = self.pending.drain(..per).collect();
            let mut frame = ff::frame::Audio::new(self.format, self.frame_size, ChannelLayout::STEREO);
            frame.set_rate(RATE);
            self.fill(&mut frame, &chunk);
            frame.set_pts(Some(self.next_pts));
            self.next_pts += self.frame_size as i64;
            self.enc.send_frame(&frame).map_err(|e| anyhow!("audio encoder: {e}"))?;
            self.drain(out)?;
        }
        Ok(())
    }

    fn drain(&mut self, out: &mut Vec<Packet>) -> Result<()> {
        loop {
            let mut pkt = Packet::empty();
            match self.enc.receive_packet(&mut pkt) {
                Ok(()) => {
                    if pkt.dts().is_none() {
                        pkt.set_dts(pkt.pts());
                    }
                    out.push(pkt)
                }
                Err(ff::Error::Other { errno }) if errno == ff::util::error::EAGAIN => return Ok(()),
                Err(ff::Error::Eof) => return Ok(()),
                Err(e) => return Err(anyhow!("audio encoding failed: {e}")),
            }
        }
    }

    pub fn finish(&mut self, out: &mut Vec<Packet>) -> Result<()> {
        let per = self.frame_size * CHANNELS;
        if !self.pending.is_empty() {
            let pad = per - self.pending.len() % per;
            if pad != per {
                self.pending.extend(std::iter::repeat_n(0.0, pad));
            }
            self.push(&[], out)?;
        }
        let _ = self.enc.send_eof();
        self.drain(out)
    }
}

pub struct Mixer {
    pub inputs: Vec<Arc<TrackInputs>>,
    pub encoders: Vec<AudioEncoder>,
    pub stream_index: Vec<usize>,
    pub muted: Arc<Vec<AtomicBool>>,
}

fn take_source(buf: &SourceBuffer, n: usize, acc: &mut [f32]) {
    let mut q = buf.samples.lock().unwrap();
    if !buf.primed.load(Ordering::Relaxed) {
        if q.len() >= PRIME_SAMPLES {
            buf.primed.store(true, Ordering::Relaxed);
        } else {
            return;
        }
    }
    let have = q.len().min(n);
    for (i, s) in q.drain(..have).enumerate() {
        acc[i] += s;
    }
    if have < n {
        buf.primed.store(false, Ordering::Relaxed);
    }
}

impl Mixer {
    pub fn run(
        mut self,
        tx: Sender<Tagged>,
        paused: Arc<AtomicBool>,
        stop: Arc<AtomicBool>,
        start: Instant,
    ) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name("audio-mix".into())
            .spawn(move || {
                let mut paused_total = Duration::ZERO;
                let mut paused_since: Option<Instant> = None;
                let mut produced: u64 = 0;
                let mut packets = Vec::new();
                let mut acc: Vec<f32> = Vec::new();
                loop {
                    let stopping = stop.load(Ordering::SeqCst);
                    let now = Instant::now();
                    if paused.load(Ordering::SeqCst) {
                        if paused_since.is_none() {
                            paused_since = Some(now);
                        }
                        for t in &self.inputs {
                            for s in t.sources.lock().unwrap().iter() {
                                s.samples.lock().unwrap().clear();
                                s.primed.store(false, Ordering::Relaxed);
                            }
                        }
                    } else {
                        if let Some(since) = paused_since.take() {
                            paused_total += now - since;
                        }
                        let active = now.duration_since(start).saturating_sub(paused_total);
                        let target = active.saturating_sub(LATENCY).as_secs_f64() * RATE as f64;
                        let target =
                            if stopping { target + LATENCY.as_secs_f64() * RATE as f64 } else { target } as u64;
                        if target > produced {
                            let frames = (target - produced) as usize;
                            let n = frames * CHANNELS;
                            for (ti, input) in self.inputs.iter().enumerate() {
                                acc.clear();
                                acc.resize(n, 0.0);
                                for src in input.sources.lock().unwrap().iter() {
                                    take_source(src, n, &mut acc);
                                }
                                if self.muted.get(ti).map(|m| m.load(Ordering::Relaxed)).unwrap_or(false) {
                                    acc.iter_mut().for_each(|s| *s = 0.0);
                                }
                                for s in acc.iter_mut() {
                                    *s = s.clamp(-1.0, 1.0);
                                }
                                packets.clear();
                                if let Err(e) = self.encoders[ti].push(&acc, &mut packets) {
                                    crate::report::warning(&format!("{e:#}"));
                                }
                                for p in packets.drain(..) {
                                    let _ = tx.send(Tagged { stream: self.stream_index[ti], packet: p });
                                }
                            }
                            produced = target;
                        }
                    }
                    if stopping {
                        for (ti, enc) in self.encoders.iter_mut().enumerate() {
                            packets.clear();
                            let _ = enc.finish(&mut packets);
                            for p in packets.drain(..) {
                                let _ = tx.send(Tagged { stream: self.stream_index[ti], packet: p });
                            }
                        }
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
            .expect("spawn audio mixer")
    }
}
