//! Real-time output through cpal (docs/contracts/audio-io.md `device`, Phase 3).
//!
//! [`OutputDevice::open`] picks the default output (or one by name or id), resolves a stream
//! config (the device's preferred format and rate, or a requested rate) and requests a buffer of
//! [`DEFAULT_BUFFER_FRAMES`] frames. [`OutputDevice::start`] runs a callback that pulls planar
//! stereo from the caller (the real-time engine) and writes the device's interleaved format.
//!
//! The callback never allocates: scratch buffers are preallocated and big device buffers are
//! rendered in pieces. It measures itself with [`StreamStats`]:
//!
//! - **xruns** = underruns the backend reports (`cpal::ErrorKind::Xrun`) + callbacks that found
//!   the device buffer drained (WASAPI shared mode, see below) + deadline misses (a callback that
//!   took longer than the audio it produced, which starves the device if it repeats).
//! - **late wakeups**: the gap between two callbacks exceeded twice the audio the previous one
//!   produced (the OS woke the thread late; usually absorbed by the device buffer). Reported, not
//!   counted as xruns.
//!
//! **Windows (WASAPI shared mode).** The callback period is the engine's device period (usually
//! 10 ms) whatever buffer is requested; the request only sizes the ring buffer, and each callback
//! is asked for "buffer size minus what is still queued". The first callback is asked for the
//! whole (empty) buffer; a later callback asking for the whole buffer again found it drained, so
//! it is counted as an xrun ("starved"), once callbacks smaller than the buffer have been seen.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The buffer the engine is built for (5.33 ms at 48 kHz).
pub const DEFAULT_BUFFER_FRAMES: u32 = 256;
/// Frames rendered per piece inside one callback (preallocated scratch).
pub const SCRATCH_FRAMES: usize = 8192;
/// Render-time histogram resolution and range.
const HIST_BUCKET_NS: u64 = 10_000;
const HIST_BUCKETS: usize = 2048;

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("no default output device")]
    NoDevice,
    #[error("no output device matches `{0}`")]
    NotFound(String),
    #[error("unsupported sample format {0}")]
    UnsupportedFormat(String),
    #[error("audio backend: {0}")]
    Backend(String),
}

fn backend(e: impl std::fmt::Display) -> DeviceError {
    DeviceError::Backend(e.to_string())
}

/// One output device.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub name: String,
    /// Backend id (`host:id`), stable across runs; also accepted by [`OutputRequest::device`].
    pub id: String,
    pub is_default: bool,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub sample_format: Option<String>,
    /// Buffer sizes the backend says it accepts, when it says.
    pub buffer_range: Option<(u32, u32)>,
}

fn device_name(d: &cpal::Device) -> String {
    d.description()
        .map(|desc| desc.name().to_string())
        .unwrap_or_else(|_| d.to_string())
}

fn device_id(d: &cpal::Device) -> String {
    d.id().map(|i| i.to_string()).unwrap_or_default()
}

/// Every output device of the default host.
pub fn list_output_devices() -> Result<Vec<DeviceInfo>, DeviceError> {
    let host = cpal::default_host();
    let default_id = host.default_output_device().map(|d| device_id(&d));
    let mut out = Vec::new();
    for d in host.output_devices().map_err(backend)? {
        let cfg = d.default_output_config().ok();
        let id = device_id(&d);
        out.push(DeviceInfo {
            name: device_name(&d),
            is_default: default_id.as_deref() == Some(id.as_str()),
            id,
            sample_rate: cfg.as_ref().map(|c| c.sample_rate()),
            channels: cfg.as_ref().map(|c| c.channels()),
            sample_format: cfg.as_ref().map(|c| c.sample_format().to_string()),
            buffer_range: cfg.as_ref().and_then(|c| match c.buffer_size() {
                cpal::SupportedBufferSize::Range { min, max } => Some((*min, *max)),
                _ => None,
            }),
        });
    }
    Ok(out)
}

/// What to open.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputRequest {
    /// Device name (case-insensitive, exact or substring) or id; `None` for the default output.
    pub device: Option<String>,
    /// Requested buffer in frames.
    pub buffer_frames: u32,
    /// Sample rate; `None` for the device's own.
    pub sample_rate: Option<u32>,
}

impl Default for OutputRequest {
    fn default() -> Self {
        OutputRequest {
            device: None,
            buffer_frames: DEFAULT_BUFFER_FRAMES,
            sample_rate: None,
        }
    }
}

/// A resolved output device and stream config, not yet running.
pub struct OutputDevice {
    device: cpal::Device,
    name: String,
    config: cpal::StreamConfig,
    format: SampleFormat,
}

impl std::fmt::Debug for OutputDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputDevice")
            .field("name", &self.name)
            .field("config", &self.config)
            .field("format", &self.format)
            .finish()
    }
}

fn find_device(host: &cpal::Host, want: &str) -> Result<cpal::Device, DeviceError> {
    let w = want.to_lowercase();
    let devices: Vec<cpal::Device> = host.output_devices().map_err(backend)?.collect();
    let exact = devices
        .iter()
        .position(|d| device_name(d).to_lowercase() == w || device_id(d).to_lowercase() == w);
    let any = exact.or_else(|| {
        devices
            .iter()
            .position(|d| device_name(d).to_lowercase().contains(&w))
    });
    match any {
        Some(i) => Ok(devices[i].clone()),
        None => Err(DeviceError::NotFound(want.to_string())),
    }
}

impl OutputDevice {
    pub fn open(req: &OutputRequest) -> Result<OutputDevice, DeviceError> {
        let host = cpal::default_host();
        let device = match &req.device {
            Some(name) => find_device(&host, name)?,
            None => host.default_output_device().ok_or(DeviceError::NoDevice)?,
        };
        let default = device.default_output_config().map_err(backend)?;
        let mut supported = default;
        if let Some(sr) = req.sample_rate.filter(|sr| *sr != default.sample_rate()) {
            let ranges: Vec<_> = device
                .supported_output_configs()
                .map_err(backend)?
                .filter(|r| r.channels() >= 1)
                .collect();
            // Prefer the default's format and channel count, then f32, then anything.
            let pick = ranges
                .iter()
                .filter(|r| r.min_sample_rate() <= sr && sr <= r.max_sample_rate())
                .max_by_key(|r| {
                    (
                        r.sample_format() == default.sample_format(),
                        r.channels() == default.channels(),
                        r.sample_format() == SampleFormat::F32,
                    )
                })
                .ok_or_else(|| {
                    DeviceError::Backend(format!("the device does not support {sr} Hz"))
                })?;
            supported = (*pick).with_sample_rate(sr);
        }
        let format = supported.sample_format();
        // A real range is honoured. A single value is what WASAPI shared mode reports (its device
        // period); there any request is accepted and only sizes the ring buffer, so it is passed
        // through as asked.
        let frames = match supported.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } if min < max => {
                req.buffer_frames.clamp(*min, *max)
            }
            _ => req.buffer_frames,
        };
        let mut config = supported.config();
        config.buffer_size = cpal::BufferSize::Fixed(frames.max(1));
        Ok(OutputDevice {
            name: device_name(&device),
            device,
            config,
            format,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.config.channels
    }

    pub fn sample_format(&self) -> String {
        self.format.to_string()
    }

    /// The buffer size requested from the backend (it may run other callback sizes; WASAPI in
    /// shared mode calls back once per device period).
    pub fn requested_buffer_frames(&self) -> Option<u32> {
        match self.config.buffer_size {
            cpal::BufferSize::Fixed(n) => Some(n),
            cpal::BufferSize::Default => None,
        }
    }

    /// Starts the stream. `render(left, right, ctx)` fills planar stereo for `left.len()` frames
    /// (at most [`SCRATCH_FRAMES`] per call) on the audio thread; it must not allocate, lock or
    /// block.
    pub fn start<F>(self, render: F) -> Result<OutputStream, DeviceError>
    where
        F: FnMut(&mut [f32], &mut [f32], &CallbackContext) + Send + 'static,
    {
        let stats = Arc::new(StreamStats::new());
        let (dev, cfg, st) = (&self.device, &self.config, stats.clone());
        let stream = match self.format {
            SampleFormat::F32 => build_stream::<f32, F>(dev, cfg, render, st),
            SampleFormat::F64 => build_stream::<f64, F>(dev, cfg, render, st),
            SampleFormat::I16 => build_stream::<i16, F>(dev, cfg, render, st),
            SampleFormat::I32 => build_stream::<i32, F>(dev, cfg, render, st),
            SampleFormat::U16 => build_stream::<u16, F>(dev, cfg, render, st),
            SampleFormat::I8 => build_stream::<i8, F>(dev, cfg, render, st),
            SampleFormat::U8 => build_stream::<u8, F>(dev, cfg, render, st),
            other => Err(DeviceError::UnsupportedFormat(other.to_string())),
        }?;
        stream.play().map_err(backend)?;
        Ok(OutputStream {
            stream,
            stats,
            name: self.name,
            sample_rate: self.config.sample_rate,
            channels: self.config.channels,
            requested_buffer: match self.config.buffer_size {
                cpal::BufferSize::Fixed(n) => Some(n),
                cpal::BufferSize::Default => None,
            },
        })
    }
}

/// Counters passed to the render callback each time (so the engine can report them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallbackContext {
    /// [`StatsSnapshot::xruns`] so far.
    pub xruns: u64,
    /// Deadline misses so far.
    pub late_callbacks: u64,
    /// Frames the device asked for in this callback (the render call may get a piece of it).
    pub device_frames: usize,
}

/// Live counters of a running stream (lock-free; the audio thread writes, anyone reads).
#[derive(Debug)]
pub struct StreamStats {
    callbacks: AtomicU64,
    frames: AtomicU64,
    min_frames: AtomicU64,
    max_frames: AtomicU64,
    first_frames: AtomicU64,
    reported_xruns: AtomicU64,
    starved: AtomicU64,
    deadline_misses: AtomicU64,
    late_wakeups: AtomicU64,
    errors: AtomicU64,
    max_render_ns: AtomicU64,
    total_render_ns: AtomicU64,
    max_interval_ns: AtomicU64,
    hist: Vec<AtomicU64>,
}

impl StreamStats {
    fn new() -> StreamStats {
        StreamStats {
            callbacks: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            min_frames: AtomicU64::new(u64::MAX),
            max_frames: AtomicU64::new(0),
            first_frames: AtomicU64::new(0),
            reported_xruns: AtomicU64::new(0),
            starved: AtomicU64::new(0),
            deadline_misses: AtomicU64::new(0),
            late_wakeups: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            max_render_ns: AtomicU64::new(0),
            total_render_ns: AtomicU64::new(0),
            max_interval_ns: AtomicU64::new(0),
            hist: (0..HIST_BUCKETS).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    fn xruns(&self) -> u64 {
        self.reported_xruns.load(Ordering::Relaxed)
            + self.starved.load(Ordering::Relaxed)
            + self.deadline_misses.load(Ordering::Relaxed)
    }

    fn record_render(&self, ns: u64) {
        self.max_render_ns.fetch_max(ns, Ordering::Relaxed);
        self.total_render_ns.fetch_add(ns, Ordering::Relaxed);
        let b = ((ns / HIST_BUCKET_NS) as usize).min(HIST_BUCKETS - 1);
        self.hist[b].fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        let callbacks = self.callbacks.load(Ordering::Relaxed);
        let counts: Vec<u64> = self
            .hist
            .iter()
            .map(|h| h.load(Ordering::Relaxed))
            .collect();
        let total: u64 = counts.iter().sum();
        let p99_render_ms = percentile_ms(&counts, total, 0.99);
        let ms = |ns: u64| ns as f64 / 1e6;
        StatsSnapshot {
            callbacks,
            frames: self.frames.load(Ordering::Relaxed),
            min_callback_frames: match self.min_frames.load(Ordering::Relaxed) {
                u64::MAX => 0,
                n => n,
            },
            max_callback_frames: self.max_frames.load(Ordering::Relaxed),
            first_callback_frames: self.first_frames.load(Ordering::Relaxed),
            reported_xruns: self.reported_xruns.load(Ordering::Relaxed),
            starved: self.starved.load(Ordering::Relaxed),
            deadline_misses: self.deadline_misses.load(Ordering::Relaxed),
            late_wakeups: self.late_wakeups.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            max_render_ms: ms(self.max_render_ns.load(Ordering::Relaxed)),
            mean_render_ms: if callbacks > 0 {
                ms(self.total_render_ns.load(Ordering::Relaxed)) / callbacks as f64
            } else {
                0.0
            },
            p99_render_ms,
            max_interval_ms: ms(self.max_interval_ns.load(Ordering::Relaxed)),
        }
    }
}

/// Upper edge of the bucket holding quantile `q` (ms).
fn percentile_ms(counts: &[u64], total: u64, q: f64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let want = ((total as f64) * q).ceil() as u64;
    let mut acc = 0u64;
    for (i, c) in counts.iter().enumerate() {
        acc += c;
        if acc >= want {
            return ((i as u64 + 1) * HIST_BUCKET_NS) as f64 / 1e6;
        }
    }
    (counts.len() as u64 * HIST_BUCKET_NS) as f64 / 1e6
}

/// A copy of the counters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StatsSnapshot {
    pub callbacks: u64,
    pub frames: u64,
    pub min_callback_frames: u64,
    pub max_callback_frames: u64,
    pub first_callback_frames: u64,
    /// Underruns the backend reported.
    pub reported_xruns: u64,
    /// Callbacks that found the device buffer drained (WASAPI heuristic, see module docs).
    pub starved: u64,
    /// Callbacks that took longer to render than the audio they produced.
    pub deadline_misses: u64,
    /// Callbacks that started more than twice the previous callback's audio after it.
    pub late_wakeups: u64,
    /// Other stream errors.
    pub errors: u64,
    pub max_render_ms: f64,
    pub mean_render_ms: f64,
    /// 99th percentile render time (10 µs resolution).
    pub p99_render_ms: f64,
    pub max_interval_ms: f64,
}

impl StatsSnapshot {
    /// Reported underruns + drained buffers + deadline misses.
    pub fn xruns(&self) -> u64 {
        self.reported_xruns + self.starved + self.deadline_misses
    }
}

/// A running output stream. Dropping it stops the stream.
pub struct OutputStream {
    stream: cpal::Stream,
    stats: Arc<StreamStats>,
    name: String,
    sample_rate: u32,
    channels: u16,
    requested_buffer: Option<u32>,
}

impl std::fmt::Debug for OutputStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputStream")
            .field("name", &self.name)
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("requested_buffer", &self.requested_buffer)
            .finish_non_exhaustive()
    }
}

impl OutputStream {
    pub fn stats(&self) -> StatsSnapshot {
        self.stats.snapshot()
    }

    /// The shared counters (to read from another thread).
    pub fn stats_handle(&self) -> Arc<StreamStats> {
        self.stats.clone()
    }

    pub fn device_name(&self) -> &str {
        &self.name
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// The buffer size the stream was opened with (`None`: the backend's default).
    pub fn requested_buffer_frames(&self) -> Option<u32> {
        self.requested_buffer
    }

    pub fn pause(&self) -> Result<(), DeviceError> {
        self.stream.pause().map_err(backend)
    }

    pub fn resume(&self) -> Result<(), DeviceError> {
        self.stream.play().map_err(backend)
    }
}

/// Writes planar stereo into an interleaved buffer of `channels` channels: mono gets the
/// average, channels past the second get silence.
fn interleave<T: Sample + FromSample<f32>>(out: &mut [T], channels: usize, l: &[f32], r: &[f32]) {
    let ch = channels.max(1);
    for (f, frame) in out.chunks_exact_mut(ch).enumerate().take(l.len()) {
        if ch == 1 {
            frame[0] = T::from_sample(0.5 * (l[f] + r[f]));
        } else {
            frame[0] = T::from_sample(l[f]);
            frame[1] = T::from_sample(r[f]);
            for s in &mut frame[2..] {
                *s = T::EQUILIBRIUM;
            }
        }
    }
}

/// Per-stream state owned by the audio thread.
struct CallbackState<F> {
    render: F,
    left: Vec<f32>,
    right: Vec<f32>,
    channels: usize,
    sample_rate: u32,
    stats: Arc<StreamStats>,
    last_start: Option<Instant>,
    last_frames: usize,
    first_frames: usize,
    seen_smaller: bool,
}

impl<F: FnMut(&mut [f32], &mut [f32], &CallbackContext)> CallbackState<F> {
    fn run<T: Sample + FromSample<f32>>(&mut self, data: &mut [T]) {
        let start = Instant::now();
        let ch = self.channels.max(1);
        let frames = data.len() / ch;
        let st = &self.stats;
        let n_cb = st.callbacks.fetch_add(1, Ordering::Relaxed);
        st.frames.fetch_add(frames as u64, Ordering::Relaxed);
        st.min_frames.fetch_min(frames as u64, Ordering::Relaxed);
        st.max_frames.fetch_max(frames as u64, Ordering::Relaxed);
        let secs = |n: usize| Duration::from_secs_f64(n as f64 / self.sample_rate.max(1) as f64);
        if n_cb == 0 {
            self.first_frames = frames;
            st.first_frames.store(frames as u64, Ordering::Relaxed);
        } else {
            if frames < self.first_frames {
                self.seen_smaller = true;
            } else if cfg!(windows) && self.seen_smaller {
                st.starved.fetch_add(1, Ordering::Relaxed);
            }
            if let Some(prev) = self.last_start {
                let gap = start.saturating_duration_since(prev);
                st.max_interval_ns
                    .fetch_max(gap.as_nanos() as u64, Ordering::Relaxed);
                if gap > 2 * secs(self.last_frames) + Duration::from_millis(1) {
                    st.late_wakeups.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        self.last_start = Some(start);
        self.last_frames = frames;
        let ctx = CallbackContext {
            xruns: st.xruns(),
            late_callbacks: st.deadline_misses.load(Ordering::Relaxed),
            device_frames: frames,
        };
        let mut done = 0;
        while done < frames {
            let n = (frames - done).min(self.left.len());
            let (l, r) = (&mut self.left[..n], &mut self.right[..n]);
            (self.render)(l, r, &ctx);
            interleave(&mut data[done * ch..(done + n) * ch], ch, l, r);
            done += n;
        }
        for s in &mut data[frames * ch..] {
            *s = T::EQUILIBRIUM;
        }
        let took = start.elapsed();
        st.record_render(took.as_nanos() as u64);
        if took > secs(frames) {
            st.deadline_misses.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn build_stream<T, F>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    render: F,
    stats: Arc<StreamStats>,
) -> Result<cpal::Stream, DeviceError>
where
    T: SizedSample + FromSample<f32>,
    F: FnMut(&mut [f32], &mut [f32], &CallbackContext) + Send + 'static,
{
    let err_stats = stats.clone();
    let mut state = CallbackState {
        render,
        left: vec![0.0; SCRATCH_FRAMES],
        right: vec![0.0; SCRATCH_FRAMES],
        channels: config.channels as usize,
        sample_rate: config.sample_rate,
        stats,
        last_start: None,
        last_frames: 0,
        first_frames: 0,
        seen_smaller: false,
    };
    device
        .build_output_stream::<T, _, _>(
            *config,
            move |data: &mut [T], _info: &cpal::OutputCallbackInfo| state.run(data),
            move |err: cpal::Error| {
                if err.kind() == cpal::ErrorKind::Xrun {
                    err_stats.reported_xruns.fetch_add(1, Ordering::Relaxed);
                } else {
                    err_stats.errors.fetch_add(1, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaves_stereo_mono_and_surround() {
        let l = [0.5f32, -0.25];
        let r = [0.1f32, 0.2];
        let mut st = [0.0f32; 4];
        interleave(&mut st, 2, &l, &r);
        assert_eq!(st, [0.5, 0.1, -0.25, 0.2]);
        let mut mono = [0.0f32; 2];
        interleave(&mut mono, 1, &l, &r);
        assert_eq!(mono, [0.5 * (0.5 + 0.1), 0.5 * (-0.25 + 0.2)]);
        let mut six = [9i16; 12];
        interleave(&mut six, 6, &l, &r);
        assert_eq!(six[0], (0.5f32 * 32768.0) as i16);
        assert_eq!(&six[2..6], &[0, 0, 0, 0]);
    }

    #[test]
    fn stats_count_and_percentile() {
        let s = StreamStats::new();
        for ns in [50_000u64, 60_000, 70_000, 2_000_000] {
            s.callbacks.fetch_add(1, Ordering::Relaxed);
            s.record_render(ns);
        }
        let snap = s.snapshot();
        assert_eq!(snap.callbacks, 4);
        assert!((snap.max_render_ms - 2.0).abs() < 1e-9);
        assert!((snap.p99_render_ms - 2.01).abs() < 1e-9);
        assert_eq!(snap.xruns(), 0);
    }
}
