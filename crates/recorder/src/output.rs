use anyhow::{anyhow, Context, Result};
use ff::{Dictionary, Packet, Rational};
use ffmpeg_next as ff;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use crate::report;

pub struct Tagged {
    pub stream: usize,
    pub packet: Packet,
}

pub enum Msg {
    Packet(Tagged),
    Save(Option<u32>),
    Marker,
    Finish,
}

#[derive(Clone)]
pub struct StreamDesc {
    pub params: ff::codec::Parameters,
    pub time_base: Rational,
    pub video: bool,
    pub title: String,
    pub frame_rate: Option<Rational>,
}

unsafe impl Send for StreamDesc {}

pub fn format_name(container: &str) -> &str {
    match container {
        "mkv" | "matroska" => "matroska",
        "ts" | "mpegts" => "mpegts",
        "m3u8" | "hls" => "hls",
        other => other,
    }
}

pub fn wants_global_header(container: &str) -> bool {
    !matches!(format_name(container), "mpegts" | "hls" | "whip" | "rtsp")
}

fn is_url(target: &str) -> bool {
    target.contains("://")
}

#[derive(Clone, Default)]
pub struct MuxOptions {
    pub auth: Option<String>,
    pub exclude_metadata: bool,
    pub extra: Vec<(String, String)>,
}

pub struct Writer {
    ctx: ff::format::context::Output,
    in_tb: Vec<Rational>,
    pub path: String,
    base: Vec<i64>,
    markers: Vec<f64>,
    is_file: bool,
    wrote: bool,
    last_dts: Vec<i64>,
}

unsafe impl Send for Writer {}

impl Writer {
    pub fn create(target: &str, container: &str, streams: &[StreamDesc], mux: &MuxOptions) -> Result<Writer> {
        let format = format_name(container);
        let is_file = !is_url(target);
        if is_file {
            if let Some(dir) = Path::new(target).parent() {
                std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
            }
        }
        let mut ctx =
            ff::format::output_as(&target, format).map_err(|e| anyhow!("cannot open output {target}: {e}"))?;
        for s in streams {
            let mut ost = ctx.add_stream(ff::encoder::find(ff::codec::Id::None)).map_err(|e| anyhow!("{e}"))?;
            ost.set_parameters(s.params.clone());
            ost.set_time_base(s.time_base);
            if let Some(fr) = s.frame_rate {
                ost.set_avg_frame_rate(fr);
                ost.set_rate(fr);
            }
            unsafe {
                (*(*ost.as_mut_ptr()).codecpar).codec_tag = 0;
            }
            if !s.video && !s.title.is_empty() && !mux.exclude_metadata {
                let mut md = Dictionary::new();
                md.set("title", &s.title);
                ost.set_metadata(md);
            }
        }
        let mut md = Dictionary::new();
        if !mux.exclude_metadata {
            md.set("encoder", "TGOW Recorder");
        }
        ctx.set_metadata(md);

        let mut opts = Dictionary::new();
        match format {
            "mp4" | "mov" if is_file => opts.set("movflags", "+hybrid_fragmented"),
            "whip" => {
                opts.set("ts_buffer_size", "4194304");
                if let Some(t) = &mux.auth {
                    opts.set("authorization", t);
                }
            }
            "flv" if !is_file => opts.set("flvflags", "no_duration_filesize"),
            _ => {}
        }
        for (k, v) in &mux.extra {
            opts.set(k, v);
        }
        ctx.write_header_with(opts).map_err(|e| anyhow!("could not start writing {target}: {e}"))?;
        let n = streams.len();
        Ok(Writer {
            ctx,
            in_tb: streams.iter().map(|s| s.time_base).collect(),
            path: target.to_string(),
            base: vec![0; n],
            markers: Vec::new(),
            is_file,
            wrote: false,
            last_dts: vec![i64::MIN; n],
        })
    }

    pub fn set_base_seconds(&mut self, secs: f64) {
        for (i, tb) in self.in_tb.iter().enumerate() {
            self.base[i] = (secs * tb.denominator() as f64 / tb.numerator() as f64).round() as i64;
        }
    }

    pub fn write(&mut self, stream: usize, packet: &Packet) -> Result<()> {
        let mut pkt = packet.clone();
        let base = self.base[stream];
        let pts = pkt.pts().map(|p| p - base);
        let dts = pkt.dts().map(|d| d - base);
        if dts.map(|d| d < 0).unwrap_or(false) {
            return Ok(());
        }
        pkt.set_pts(pts);
        pkt.set_dts(dts);
        let out_tb = self.ctx.stream(stream).map(|s| s.time_base()).unwrap_or(self.in_tb[stream]);
        pkt.rescale_ts(self.in_tb[stream], out_tb);
        if let Some(d) = pkt.dts() {
            if d <= self.last_dts[stream] {
                let fixed = self.last_dts[stream] + 1;
                if pkt.pts().map(|p| p < fixed).unwrap_or(false) {
                    pkt.set_pts(Some(fixed));
                }
                pkt.set_dts(Some(fixed));
            }
            self.last_dts[stream] = pkt.dts().unwrap_or(d);
        }
        pkt.set_stream(stream);
        pkt.set_position(-1);
        pkt.write_interleaved(&mut self.ctx).map_err(|e| anyhow!("writing to {} failed: {e}", self.path))?;
        self.wrote = true;
        Ok(())
    }

    pub fn add_marker(&mut self, at_secs: f64) {
        self.markers.push(at_secs);
    }

    pub fn finish(mut self) -> Result<String> {
        let markers = std::mem::take(&mut self.markers);
        for (i, at) in markers.iter().enumerate() {
            let start = (at * 1000.0) as i64;
            let end = markers.get(i + 1).map(|n| (n * 1000.0) as i64).unwrap_or(start + 1000);
            let _ = self.ctx.add_chapter(i as i64, Rational::new(1, 1000), start, end, format!("Marker {}", i + 1));
        }
        self.ctx.write_trailer().map_err(|e| anyhow!("finishing {} failed: {e}", self.path))?;
        if self.is_file && !self.wrote {
            let _ = std::fs::remove_file(&self.path);
            return Err(anyhow!("nothing was recorded"));
        }
        Ok(self.path.clone())
    }
}

enum Body {
    Ram(Packet),
    Disk { chunk: u64, offset: u64, len: u32 },
}

struct Entry {
    stream: usize,
    secs: f64,
    key: bool,
    pts: Option<i64>,
    dts: Option<i64>,
    duration: i64,
    body: Body,
}

struct DiskStore {
    dir: PathBuf,
    current: Option<(u64, File, u64)>,
    next_id: u64,
    chunk_started: f64,
}

impl DiskStore {
    fn chunk_path(&self, id: u64) -> PathBuf {
        self.dir.join(format!("chunk-{id}.bin"))
    }

    fn append(&mut self, data: &[u8], now_secs: f64) -> Result<(u64, u64)> {
        let rotate = match &self.current {
            None => true,
            Some(_) => now_secs - self.chunk_started > 10.0,
        };
        if rotate {
            let id = self.next_id;
            self.next_id += 1;
            let file = File::options().create(true).truncate(true).read(true).write(true).open(self.chunk_path(id))?;
            self.current = Some((id, file, 0));
            self.chunk_started = now_secs;
        }
        let (id, file, pos) = self.current.as_mut().unwrap();
        let at = *pos;
        file.write_all(data)?;
        *pos += data.len() as u64;
        Ok((*id, at))
    }

    fn read(&self, chunk: u64, offset: u64, len: u32) -> Result<Vec<u8>> {
        let mut f = File::open(self.chunk_path(chunk))?;
        f.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len as usize];
        f.read_exact(&mut buf)?;
        Ok(buf)
    }
}

pub type Snapshot = (Vec<(usize, Packet)>, f64, f64);

pub struct ReplayBuffer {
    entries: VecDeque<Entry>,
    keep_secs: f64,
    disk: Option<DiskStore>,
    tbs: Vec<Rational>,
    video_stream: Option<usize>,
}

impl ReplayBuffer {
    pub fn new(keep_secs: u32, on_disk: bool, streams: &[StreamDesc]) -> Result<ReplayBuffer> {
        let disk = if on_disk {
            let dir = tgow_common::paths::replay_cache_dir().join(std::process::id().to_string());
            std::fs::create_dir_all(&dir)?;
            Some(DiskStore { dir, current: None, next_id: 0, chunk_started: 0.0 })
        } else {
            None
        };
        Ok(ReplayBuffer {
            entries: VecDeque::new(),
            keep_secs: keep_secs as f64,
            disk,
            tbs: streams.iter().map(|s| s.time_base).collect(),
            video_stream: streams.iter().position(|s| s.video),
        })
    }

    fn secs_of(&self, stream: usize, p: &Packet) -> f64 {
        let tb = self.tbs[stream];
        p.dts().or(p.pts()).unwrap_or(0) as f64 * tb.numerator() as f64 / tb.denominator() as f64
    }

    pub fn push(&mut self, t: Tagged) -> Result<()> {
        let secs = self.secs_of(t.stream, &t.packet);
        let key = Some(t.stream) == self.video_stream && t.packet.is_key();
        let body = match &mut self.disk {
            Some(d) => {
                let data = t.packet.data().unwrap_or(&[]);
                let (chunk, offset) = d.append(data, secs)?;
                Body::Disk { chunk, offset, len: data.len() as u32 }
            }
            None => Body::Ram(t.packet.clone()),
        };
        self.entries.push_back(Entry {
            stream: t.stream,
            secs,
            key,
            pts: t.packet.pts(),
            dts: t.packet.dts(),
            duration: t.packet.duration(),
            body,
        });
        self.prune(secs);
        Ok(())
    }

    fn prune(&mut self, newest: f64) {
        let cutoff = newest - self.keep_secs;
        if self.video_stream.is_none() {
            while self.entries.front().map(|e| e.secs < cutoff).unwrap_or(false) {
                self.entries.pop_front();
            }
        } else {
            let mut drop_until = 0;
            for (i, e) in self.entries.iter().enumerate() {
                if e.secs > cutoff {
                    break;
                }
                if e.key {
                    drop_until = i;
                }
            }
            for _ in 0..drop_until {
                self.entries.pop_front();
            }
        }
        if let Some(d) = &self.disk {
            let oldest = self.entries.iter().find_map(|e| match e.body {
                Body::Disk { chunk, .. } => Some(chunk),
                Body::Ram(_) => None,
            });
            if let Some(oldest) = oldest {
                for id in oldest.saturating_sub(4)..oldest {
                    let _ = std::fs::remove_file(d.chunk_path(id));
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn snapshot(&self, seconds: Option<u32>) -> Result<Snapshot> {
        let newest = self.entries.back().map(|e| e.secs).ok_or_else(|| anyhow!("the replay buffer is still empty"))?;
        let want = seconds.map(|s| s as f64).unwrap_or(self.keep_secs).min(self.keep_secs);
        let from = newest - want;
        let start_idx = if self.video_stream.is_some() {
            let mut idx = None;
            for (i, e) in self.entries.iter().enumerate() {
                if e.key {
                    if e.secs <= from || idx.is_none() {
                        idx = Some(i);
                    } else {
                        break;
                    }
                }
            }
            idx.ok_or_else(|| anyhow!("no keyframe in the replay buffer yet"))?
        } else {
            self.entries.iter().position(|e| e.secs >= from).unwrap_or(0)
        };
        let base = self.entries[start_idx].secs;
        let mut out = Vec::with_capacity(self.entries.len() - start_idx);
        for e in self.entries.iter().skip(start_idx) {
            if e.secs < base {
                continue;
            }
            let mut p = match &e.body {
                Body::Ram(p) => p.clone(),
                Body::Disk { chunk, offset, len } => {
                    let data = self.disk.as_ref().unwrap().read(*chunk, *offset, *len)?;
                    Packet::copy(&data)
                }
            };
            p.set_pts(e.pts);
            p.set_dts(e.dts);
            p.set_duration(e.duration);
            if e.key {
                p.set_flags(ff::codec::packet::Flags::KEY);
            }
            out.push((e.stream, p));
        }
        Ok((out, base, newest - base))
    }
}

impl Drop for ReplayBuffer {
    fn drop(&mut self) {
        if let Some(d) = &self.disk {
            let _ = std::fs::remove_dir_all(&d.dir);
        }
    }
}

pub enum Mode {
    Record { dir_hint: PathBuf, container: String, split_secs: Option<f64>, first_path: String },
    Stream { url: String, container: String },
    Replay { seconds: u32, on_disk: bool, dir: PathBuf, container: String, restart_on_save: bool, date_folders: bool },
}

pub struct OutputJob {
    pub mode: Mode,
    pub streams: Vec<StreamDesc>,
    pub mux: MuxOptions,
    pub script: Option<PathBuf>,
}

fn run_script(script: &Option<PathBuf>, path: &str, kind: &str) {
    if let Some(s) = script {
        let _ = std::process::Command::new(s).arg(path).arg(kind).spawn();
    }
}

fn next_record_path(dir: &Path, container: &str) -> String {
    let stamp = tgow_common::system::now_local().file_stamp();
    let mut p = dir.join(format!("Video_{stamp}.{container}"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("Video_{stamp}_{n}.{container}"));
        n += 1;
    }
    p.to_string_lossy().into_owned()
}

fn secs(tb: Rational, v: i64) -> f64 {
    v as f64 * tb.numerator() as f64 / tb.denominator() as f64
}

pub fn run(job: OutputJob, rx: Receiver<Msg>) -> Result<()> {
    match job.mode {
        Mode::Record { dir_hint, container, split_secs, first_path } => {
            let mut writer = Writer::create(&first_path, &container, &job.streams, &job.mux)?;
            let mut seg_start = 0.0;
            let mut last_video_secs = 0.0;
            let mut segment_open_secs = 0.0;
            let video = job.streams.iter().position(|s| s.video);
            while let Ok(msg) = rx.recv() {
                match msg {
                    Msg::Packet(t) => {
                        let tb = job.streams[t.stream].time_base;
                        let at = secs(tb, t.packet.dts().or(t.packet.pts()).unwrap_or(0));
                        if Some(t.stream) == video {
                            last_video_secs = at;
                            if let Some(split) = split_secs {
                                if t.packet.is_key() && at - seg_start >= split {
                                    let next = next_record_path(&dir_hint, &container);
                                    let fresh = Writer::create(&next, &container, &job.streams, &job.mux)?;
                                    let old = std::mem::replace(&mut writer, fresh);
                                    match old.finish() {
                                        Ok(p) => {
                                            report::saved("regular", Path::new(&p), at - seg_start);
                                            run_script(&job.script, &p, "regular");
                                        }
                                        Err(e) => report::warning(&format!("{e:#}")),
                                    }
                                    seg_start = at;
                                    segment_open_secs = at;
                                    writer.set_base_seconds(at);
                                }
                            }
                        }
                        if at + 0.5 < segment_open_secs {
                            continue;
                        }
                        writer.write(t.stream, &t.packet)?;
                    }
                    Msg::Marker => writer.add_marker((last_video_secs - seg_start).max(0.0)),
                    Msg::Save(_) => {}
                    Msg::Finish => break,
                }
            }
            let duration = last_video_secs - seg_start;
            let path = writer.finish()?;
            report::saved("regular", Path::new(&path), duration);
            run_script(&job.script, &path, "regular");
        }
        Mode::Stream { url, container } => {
            let mut writer = Writer::create(&url, &container, &job.streams, &job.mux)?;
            while let Ok(msg) = rx.recv() {
                match msg {
                    Msg::Packet(t) => writer.write(t.stream, &t.packet)?,
                    Msg::Finish => break,
                    _ => {}
                }
            }
            let _ = writer.finish();
        }
        Mode::Replay { seconds, on_disk, dir, container, restart_on_save, date_folders } => {
            let mut buffer = ReplayBuffer::new(seconds, on_disk, &job.streams)?;
            let mut saving: Vec<std::thread::JoinHandle<()>> = Vec::new();
            while let Ok(msg) = rx.recv() {
                match msg {
                    Msg::Packet(t) => buffer.push(t)?,
                    Msg::Save(want) => match buffer.snapshot(want) {
                        Ok((packets, base, duration)) => {
                            let mut target_dir = dir.clone();
                            if date_folders {
                                target_dir = target_dir.join(tgow_common::system::now_local().folder_stamp());
                            }
                            let stamp = tgow_common::system::now_local().file_stamp();
                            let path = target_dir.join(format!("Replay_{stamp}.{container}"));
                            let streams = job.streams.clone();
                            let mux = job.mux.clone();
                            let container = container.clone();
                            let script = job.script.clone();
                            let handle = std::thread::spawn(move || {
                                let result = (|| -> Result<String> {
                                    let mut w = Writer::create(&path.to_string_lossy(), &container, &streams, &mux)?;
                                    w.set_base_seconds(base);
                                    for (s, p) in &packets {
                                        w.write(*s, p)?;
                                    }
                                    w.finish()
                                })();
                                match result {
                                    Ok(p) => {
                                        report::saved("replay", Path::new(&p), duration);
                                        run_script(&script, &p, "replay");
                                    }
                                    Err(e) => report::error(&format!("saving the replay failed: {e:#}")),
                                }
                            });
                            saving.push(handle);
                            if restart_on_save {
                                buffer.clear();
                            }
                        }
                        Err(e) => report::warning(&format!("{e:#}")),
                    },
                    Msg::Marker => {}
                    Msg::Finish => break,
                }
            }
            for h in saving {
                let _ = h.join();
            }
        }
    }
    Ok(())
}
