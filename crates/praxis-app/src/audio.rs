//! The audio service: plays the open sound's live score through the real-time engine
//! (`praxis_engine::realtime`, FR-27) on the default output device (`praxis_audio_io::device`).
//!
//! Three threads:
//!
//! - the **audio thread** (cpal's, or the fake output's in tests) owns the [`RealtimeEngine`];
//! - the **service thread** owns the output stream (not `Send` on every platform, so it is opened
//!   and dropped there) and the [`EngineControl`]. It takes [`AudioService`] requests from the
//!   Tauri commands, polls the engine about 30 times a second and emits a [`TransportEvent`]
//!   (position, meters, device counters);
//! - the **preparer thread** builds graphs off the audio thread ([`prepare`]) with the same
//!   instrument registry the renders use, at the device's rate. Only the newest score asked for
//!   is built; a graph for an older one is dropped.
//!
//! A new score that differs from the playing graph's score only in layer mute, solo or gain is
//! applied live (`SetLayerMute` / `SetLayerSolo` / `SetLayerGain`, ramped over 10 ms): no
//! rebuild. Any other change is prepared and swapped in, cross-faded over 30 ms while playing.
//! Live layer overrides ([`LayerLive`]) sit on top of the score and survive swaps.

use praxis_audio_io::device::{OutputDevice, OutputRequest, OutputStream};
use praxis_engine::InstrumentFactory;
use praxis_engine::realtime::{
    Command, EngineControl, LoopRange, PreparedGraph, RealtimeEngine, RealtimeOptions, Status,
    prepare,
};
use praxis_score::timeline::{TimeMap, score_length_samples, tail_samples};
use praxis_score::{BarRange, Playback, Score};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Event carrying a [`TransportEvent`], about 30 times a second while playing.
pub const TRANSPORT_EVENT: &str = "praxis://transport";
/// How often the service polls the engine and emits position and meters.
pub const EVENT_INTERVAL: Duration = Duration::from_millis(33);
/// While stopped and unchanged, an event is still sent this often (device counters).
const IDLE_EVENT_INTERVAL: Duration = Duration::from_millis(1000);
/// Playing with no new callback for this long means the device stopped calling back.
const STALL_AFTER: Duration = Duration::from_millis(750);
/// Meter floor (dBFS) for silence: JSON has no -Infinity.
pub const METER_FLOOR_DB: f32 = -120.0;

// ------------------------------------------------------------------ outputs

/// Counters of a running output (the device's, or the fake's).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DeviceStats {
    pub callbacks: u64,
    pub frames: u64,
    pub min_callback_frames: u64,
    pub max_callback_frames: u64,
    /// Underruns reported by the backend + drained buffers + deadline misses.
    pub xruns: u64,
    pub late_wakeups: u64,
    /// Stream errors the backend reported (a device that went away, say).
    pub errors: u64,
    pub max_render_ms: f64,
    pub p99_render_ms: f64,
}

/// An output that can run the engine: the default device ([`cpal_opener`]) or, in tests, a
/// [`FakeOutput`].
pub trait Output {
    fn name(&self) -> String;
    fn sample_rate(&self) -> u32;
    /// The buffer asked for (`None`: the backend's).
    fn requested_buffer_frames(&self) -> Option<u32>;
    /// Starts calling `engine.process` on an audio thread. The returned handle keeps it running
    /// until dropped.
    fn start(self: Box<Self>, engine: RealtimeEngine) -> Result<Box<dyn Running>, String>;
}

/// A running output; dropping it stops the audio thread.
pub trait Running {
    fn stats(&self) -> DeviceStats;
}

/// Opens an output; called on the service thread (again on `Play` while none is open).
pub type Opener = Box<dyn FnMut() -> Result<Box<dyn Output>, String> + Send>;

/// The default output device through cpal, at its own rate, asking for a 256-frame buffer.
pub fn cpal_opener() -> Opener {
    Box::new(|| {
        let device = OutputDevice::open(&OutputRequest::default()).map_err(|e| e.to_string())?;
        Ok(Box::new(CpalOutput(device)) as Box<dyn Output>)
    })
}

struct CpalOutput(OutputDevice);

impl Output for CpalOutput {
    fn name(&self) -> String {
        self.0.name().to_string()
    }

    fn sample_rate(&self) -> u32 {
        self.0.sample_rate()
    }

    fn requested_buffer_frames(&self) -> Option<u32> {
        self.0.requested_buffer_frames()
    }

    fn start(self: Box<Self>, mut engine: RealtimeEngine) -> Result<Box<dyn Running>, String> {
        let stream = self
            .0
            .start(move |l, r, ctx| {
                engine.set_device_counters(ctx.xruns, ctx.late_callbacks);
                engine.process(l, r);
            })
            .map_err(|e| e.to_string())?;
        Ok(Box::new(CpalRunning(stream)))
    }
}

struct CpalRunning(OutputStream);

impl Running for CpalRunning {
    fn stats(&self) -> DeviceStats {
        let s = self.0.stats();
        DeviceStats {
            callbacks: s.callbacks,
            frames: s.frames,
            min_callback_frames: s.min_callback_frames,
            max_callback_frames: s.max_callback_frames,
            xruns: s.xruns(),
            late_wakeups: s.late_wakeups,
            errors: s.errors,
            max_render_ms: s.max_render_ms,
            p99_render_ms: s.p99_render_ms,
        }
    }
}

/// Recorded output of a [`FakeOutput`] (left, right), for tests.
pub type Tap = Arc<Mutex<(Vec<f32>, Vec<f32>)>>;

/// An output without a device: a thread that calls the engine in fixed callbacks, paced by the
/// wall clock (`speed` times real time), optionally recording what it plays. The service drives
/// it through exactly the same command path as the device.
#[derive(Clone, Debug)]
pub struct FakeOutput {
    pub sample_rate: u32,
    pub callback_frames: usize,
    pub speed: f64,
    pub tap: Option<Tap>,
    /// Xruns the fake reports (tests raise it to stand for a starved device).
    pub xruns: Option<Arc<AtomicU64>>,
}

impl FakeOutput {
    /// An opener for the service that always opens this fake.
    pub fn opener(self) -> Opener {
        Box::new(move || Ok(Box::new(self.clone()) as Box<dyn Output>))
    }
}

struct FakeRunning {
    xruns: Option<Arc<AtomicU64>>,
    stop: Arc<AtomicBool>,
    callbacks: Arc<AtomicU64>,
    frames: u64,
    thread: Option<JoinHandle<()>>,
}

impl Output for FakeOutput {
    fn name(&self) -> String {
        "fake output".into()
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn requested_buffer_frames(&self) -> Option<u32> {
        Some(self.callback_frames as u32)
    }

    fn start(self: Box<Self>, mut engine: RealtimeEngine) -> Result<Box<dyn Running>, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let callbacks = Arc::new(AtomicU64::new(0));
        let (st, cb) = (stop.clone(), callbacks.clone());
        let n = self.callback_frames.max(1);
        let per_second = self.sample_rate as f64 * self.speed.max(0.01);
        let tap = self.tap.clone();
        let thread = std::thread::spawn(move || {
            let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
            let t0 = Instant::now();
            let mut done = 0u64;
            while !st.load(Ordering::Relaxed) {
                let due = (t0.elapsed().as_secs_f64() * per_second) as u64;
                while done < due && !st.load(Ordering::Relaxed) {
                    engine.process(&mut l, &mut r);
                    if let Some(tap) = &tap
                        && let Ok(mut t) = tap.lock()
                    {
                        t.0.extend_from_slice(&l);
                        t.1.extend_from_slice(&r);
                    }
                    done += n as u64;
                    cb.fetch_add(1, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        Ok(Box::new(FakeRunning {
            xruns: self.xruns.clone(),
            stop,
            callbacks,
            frames: n as u64,
            thread: Some(thread),
        }))
    }
}

impl Running for FakeRunning {
    fn stats(&self) -> DeviceStats {
        let callbacks = self.callbacks.load(Ordering::Relaxed);
        DeviceStats {
            callbacks,
            frames: callbacks * self.frames,
            min_callback_frames: self.frames,
            max_callback_frames: self.frames,
            xruns: self.xruns.as_ref().map_or(0, |x| x.load(Ordering::Relaxed)),
            ..DeviceStats::default()
        }
    }
}

impl Drop for FakeRunning {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ------------------------------------------------------------------ what the front end sees

/// Position, meters and device counters, emitted as [`TRANSPORT_EVENT`].
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TransportEvent {
    /// An output device is open and running the engine.
    pub available: bool,
    pub device: Option<String>,
    pub sample_rate: u32,
    /// Why there is no device, or why the graph could not be built.
    pub error: Option<String>,
    pub playing: bool,
    /// A one-shot played to its end.
    pub ended: bool,
    /// Musical position in samples and seconds (within the period for loops).
    pub position: u64,
    pub seconds: f64,
    /// Peak and RMS (dBFS, floored at [`METER_FLOOR_DB`]) since the last event, left and right.
    pub peak_db: [f32; 2],
    pub rms_db: [f32; 2],
    /// Xruns while the engine was playing (the ones that can be heard).
    pub xruns: u64,
    /// Every xrun since the device opened, stopped time included (the stream keeps running
    /// silent between plays; a busy machine can starve it then, inaudibly).
    pub xruns_total: u64,
    pub late_wakeups: u64,
    pub device_errors: u64,
    /// Callback sizes the device actually used, in frames.
    pub min_callback_frames: u64,
    pub max_callback_frames: u64,
    pub callbacks: u64,
    pub swaps: u64,
    /// A graph is being built for the newest score.
    pub preparing: bool,
    /// Playing, but the device has not called back for a while.
    pub stalled: bool,
}

/// [`AudioService::status`]: the event's state plus what the last build said.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TransportStatus {
    #[serde(flatten)]
    pub event: TransportEvent,
    pub requested_buffer_frames: Option<u32>,
    /// A graph is loaded for the newest score asked for.
    pub loaded: bool,
    /// How long the last graph took to build, in milliseconds.
    pub prepare_ms: Option<f64>,
    pub warnings: Vec<String>,
    pub max_render_ms: f64,
    pub p99_render_ms: f64,
    /// Live overrides by layer id.
    pub layers: BTreeMap<String, LayerLive>,
    pub loop_range: Option<BarRange>,
}

/// A live layer override on top of the score (not saved): a trim in dB, and mute / solo when
/// set. Fields left out keep their value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayerLive {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solo: Option<bool>,
}

// ------------------------------------------------------------------ the service

enum Req {
    Play,
    Stop,
    Seek(f64),
    SetLoop(Option<BarRange>),
    Layer(String, LayerLive),
    Score {
        generation: u64,
        score: Box<Score>,
        master_gain_db: Option<f64>,
    },
    Prepared {
        generation: u64,
        graph: Box<PreparedGraph>,
        score: Box<Score>,
        ms: f64,
    },
    PrepareFailed {
        generation: u64,
        error: String,
    },
    Clear,
    Status(Sender<TransportStatus>),
    Shutdown,
}

struct PrepareJob {
    generation: u64,
    score: Box<Score>,
    master_gain_db: Option<f64>,
    sample_rate: u32,
}

/// The handle the Tauri commands use. Every call is a message to the service thread and returns
/// at once (except [`Self::status`], which waits for the answer).
pub struct AudioService {
    tx: Sender<Req>,
    next: AtomicU64,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for AudioService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioService").finish_non_exhaustive()
    }
}

/// Where events go (the Tauri emitter, or a test's collector).
pub type Emit = Box<dyn Fn(&TransportEvent) + Send>;

impl AudioService {
    /// Starts the service and tries to open an output with `opener` right away. Never fails: no
    /// device is reported in the events (`available: false`, `error`) and retried on `Play`.
    pub fn start(opener: Opener, factory: Arc<dyn InstrumentFactory>, emit: Emit) -> AudioService {
        let (tx, rx) = mpsc::channel();
        let (job_tx, job_rx) = mpsc::channel::<PrepareJob>();
        let back = tx.clone();
        std::thread::Builder::new()
            .name("praxis-prepare".into())
            .spawn(move || preparer(job_rx, back, factory))
            .ok();
        let thread = std::thread::Builder::new()
            .name("praxis-audio".into())
            .spawn(move || Service::new(opener, job_tx, emit).run(rx))
            .ok();
        AudioService {
            tx,
            next: AtomicU64::new(0),
            thread,
        }
    }

    fn send(&self, r: Req) {
        let _ = self.tx.send(r);
    }

    pub fn play(&self) {
        self.send(Req::Play);
    }

    pub fn stop(&self) {
        self.send(Req::Stop);
    }

    pub fn seek(&self, seconds: f64) {
        self.send(Req::Seek(seconds));
    }

    /// Loops bars `start..=end` (1-based), or stops looping.
    pub fn set_loop(&self, range: Option<BarRange>) {
        self.send(Req::SetLoop(range));
    }

    pub fn set_layer(&self, layer: &str, live: LayerLive) {
        self.send(Req::Layer(layer.into(), live));
    }

    /// The live score changed (a new take, a restore, a hand edit, undo, redo, a mute): plays it
    /// from now on. `master_gain_db` is the gain the take's offline render applied, used when the
    /// score has no frozen gain.
    pub fn load(&self, score: Score, master_gain_db: Option<f64>) -> u64 {
        let generation = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.send(Req::Score {
            generation,
            score: Box::new(score),
            master_gain_db,
        });
        generation
    }

    /// No sound is open: stop and forget the score and the overrides.
    pub fn clear(&self) {
        self.send(Req::Clear);
    }

    pub fn status(&self) -> TransportStatus {
        let (tx, rx) = mpsc::channel();
        self.send(Req::Status(tx));
        rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default()
    }
}

impl Drop for AudioService {
    fn drop(&mut self) {
        self.send(Req::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn preparer(rx: Receiver<PrepareJob>, back: Sender<Req>, factory: Arc<dyn InstrumentFactory>) {
    while let Ok(mut job) = rx.recv() {
        // Only the newest score matters.
        while let Ok(newer) = rx.try_recv() {
            job = newer;
        }
        let t0 = Instant::now();
        let r = prepare(
            &job.score,
            factory.as_ref(),
            &RealtimeOptions {
                sample_rate: job.sample_rate,
                master_gain_db: job.master_gain_db,
            },
        );
        let msg = match r {
            Ok(graph) => Req::Prepared {
                generation: job.generation,
                graph: Box::new(graph),
                score: job.score,
                ms: t0.elapsed().as_secs_f64() * 1000.0,
            },
            Err(e) => Req::PrepareFailed {
                generation: job.generation,
                error: e.to_string(),
            },
        };
        if back.send(msg).is_err() {
            return;
        }
    }
}

/// What the engine was last told for one layer (mute, solo, trim dB).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sent {
    muted: bool,
    solo: bool,
    trim_db: f32,
}

struct Device {
    running: Box<dyn Running>,
    control: EngineControl,
    name: String,
    sample_rate: u32,
    requested: Option<u32>,
}

struct Service {
    opener: Opener,
    jobs: Sender<PrepareJob>,
    emit: Emit,
    device: Option<Device>,
    device_error: Option<String>,
    /// The newest score asked for, its generation and gain.
    want: Option<(u64, Score, Option<f64>)>,
    /// The score the engine's graph was built from, and its generation.
    graph: Option<(u64, Score)>,
    preparing: bool,
    prepare_error: Option<String>,
    prepare_ms: Option<f64>,
    warnings: Vec<String>,
    sent: Vec<Sent>,
    overrides: BTreeMap<String, LayerLive>,
    loop_bars: Option<BarRange>,
    statuses: Vec<Status>,
    peak: [f32; 2],
    sum_sq: [f64; 2],
    meter_n: usize,
    last_event: TransportEvent,
    last_emit: Instant,
    last_callbacks: (u64, Instant),
    /// The device's xrun counter at the last tick, and the xruns counted while playing.
    xruns_seen: u64,
    xruns_playing: u64,
    was_playing: bool,
}

fn to_db(x: f32) -> f32 {
    if x > 0.0 && x.is_finite() {
        (20.0 * x.log10()).max(METER_FLOOR_DB)
    } else {
        METER_FLOOR_DB
    }
}

/// The score with every layer's mute, solo, gain and lock set alike: two scores that are equal
/// after this differ only in what the engine can change live.
fn without_mix(score: &Score) -> Score {
    let mut s = score.clone();
    for l in &mut s.layers {
        l.muted = false;
        l.solo = false;
        l.gain_db = 0.0;
        l.locked = false;
    }
    s
}

impl Service {
    fn new(opener: Opener, jobs: Sender<PrepareJob>, emit: Emit) -> Service {
        let mut s = Service {
            opener,
            jobs,
            emit,
            device: None,
            device_error: None,
            want: None,
            graph: None,
            preparing: false,
            prepare_error: None,
            prepare_ms: None,
            warnings: Vec::new(),
            sent: Vec::new(),
            overrides: BTreeMap::new(),
            loop_bars: None,
            statuses: Vec::with_capacity(praxis_engine::realtime::STATUS_CAPACITY),
            peak: [0.0; 2],
            sum_sq: [0.0; 2],
            meter_n: 0,
            last_event: TransportEvent::default(),
            last_emit: Instant::now(),
            last_callbacks: (0, Instant::now()),
            xruns_seen: 0,
            xruns_playing: 0,
            was_playing: false,
        };
        s.open();
        s
    }

    fn open(&mut self) {
        if self.device.is_some() {
            return;
        }
        let opened = (self.opener)().and_then(|out| {
            let name = out.name();
            let sample_rate = out.sample_rate();
            let requested = out.requested_buffer_frames();
            let (engine, control) = RealtimeEngine::new(sample_rate);
            let running = out.start(engine)?;
            Ok(Device {
                running,
                control,
                name,
                sample_rate,
                requested,
            })
        });
        match opened {
            Ok(d) => {
                self.device = Some(d);
                self.device_error = None;
                self.graph = None;
                self.xruns_seen = 0;
                self.sent.clear();
                self.send_loop();
                // A score asked for before the device opened is built now, at its rate.
                if let Some((generation, score, gain)) = self.want.clone() {
                    self.request_prepare(generation, score, gain);
                }
            }
            Err(e) => self.device_error = Some(format!("No audio output: {e}")),
        }
    }

    fn run(mut self, rx: Receiver<Req>) {
        let mut next = Instant::now() + EVENT_INTERVAL;
        loop {
            let wait = next.saturating_duration_since(Instant::now());
            match rx.recv_timeout(wait) {
                Ok(Req::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(r) => self.handle(r),
                Err(RecvTimeoutError::Timeout) => {}
            }
            if Instant::now() >= next {
                self.tick();
                next = Instant::now() + EVENT_INTERVAL;
            }
        }
        // Stop the audio thread before the control side goes.
        if let Some(d) = self.device.take() {
            drop(d.running);
            drop(d.control);
        }
    }

    fn control(&mut self) -> Option<&mut EngineControl> {
        self.device.as_mut().map(|d| &mut d.control)
    }

    fn send(&mut self, cmd: Command) {
        if let Some(c) = self.control() {
            // A full queue (256 commands) only happens if the audio thread stopped; drop it.
            let _ = c.send(cmd);
        }
    }

    fn handle(&mut self, r: Req) {
        match r {
            Req::Play => {
                // A device that reported errors and stopped calling back (unplugged, say) is
                // opened again; the score is rebuilt at its rate.
                let dead = self.device.as_ref().is_some_and(|d| {
                    let s = d.running.stats();
                    s.errors > 0
                        && s.callbacks == self.last_callbacks.0
                        && self.last_callbacks.1.elapsed() > STALL_AFTER
                });
                if dead {
                    self.device = None;
                }
                self.open();
                self.send(Command::Play);
            }
            Req::Stop => self.send(Command::Stop),
            Req::Seek(seconds) => {
                if let Some(p) = self.seek_samples(seconds) {
                    self.send(Command::Seek(p));
                }
            }
            Req::SetLoop(range) => {
                self.loop_bars = range.filter(|r| r.start >= 1 && r.start <= r.end);
                self.send_loop();
            }
            Req::Layer(id, live) => {
                let o = self.overrides.entry(id).or_default();
                if live.gain_db.is_some() {
                    o.gain_db = live.gain_db;
                }
                if live.muted.is_some() {
                    o.muted = live.muted;
                }
                if live.solo.is_some() {
                    o.solo = live.solo;
                }
                self.send_mix();
            }
            Req::Score {
                generation,
                score,
                master_gain_db,
            } => self.score(generation, *score, master_gain_db),
            Req::Prepared {
                generation,
                graph,
                score,
                ms,
            } => self.prepared(generation, graph, *score, ms),
            Req::PrepareFailed { generation, error } => {
                if self.want.as_ref().is_some_and(|w| w.0 == generation) {
                    self.preparing = false;
                    self.prepare_error =
                        Some(format!("Could not build the sound to play: {error}"));
                }
            }
            Req::Clear => {
                self.send(Command::Stop);
                self.send(Command::Seek(0));
                self.want = None;
                self.overrides.clear();
                self.loop_bars = None;
                self.prepare_error = None;
                self.send(Command::SetLoop(None));
            }
            Req::Status(tx) => {
                let _ = tx.send(self.status());
            }
            Req::Shutdown => {}
        }
    }

    fn score(&mut self, generation: u64, score: Score, gain: Option<f64>) {
        if self.want.as_ref().is_some_and(|w| w.0 > generation) {
            return;
        }
        self.want = Some((generation, score.clone(), gain));
        self.prepare_error = None;
        let mix_only = self.graph.as_ref().is_some_and(|(_, g)| {
            g.layers.len() == score.layers.len()
                && g.master.frozen_gain_db == score.master.frozen_gain_db
                && without_mix(g) == without_mix(&score)
        });
        if mix_only && !self.preparing {
            // Same graph: the new mute / solo / gain go in live.
            if let Some(g) = &mut self.graph {
                g.0 = generation;
            }
            self.send_mix();
            self.send_loop();
        } else {
            self.request_prepare(generation, score, gain);
        }
    }

    fn request_prepare(&mut self, generation: u64, score: Score, gain: Option<f64>) {
        let Some(d) = &self.device else {
            return;
        };
        self.preparing = true;
        let _ = self.jobs.send(PrepareJob {
            generation,
            score: Box::new(score),
            master_gain_db: gain,
            sample_rate: d.sample_rate,
        });
    }

    fn prepared(&mut self, generation: u64, graph: Box<PreparedGraph>, score: Score, ms: f64) {
        let newest = self.want.as_ref().map(|w| w.0);
        if newest != Some(generation) {
            // An older score: dropped here, off the audio thread.
            return;
        }
        let Some(d) = &mut self.device else {
            return;
        };
        if graph.sample_rate() != d.sample_rate {
            return;
        }
        self.preparing = false;
        self.prepare_ms = Some(ms);
        self.warnings = graph.warnings().to_vec();
        if d.control.swap(*graph).is_err() {
            self.prepare_error = Some("the engine's command queue is full".into());
            return;
        }
        // The new graph starts from its score's mute and solo, and 0 dB trims.
        self.sent = score
            .layers
            .iter()
            .map(|l| Sent {
                muted: l.muted,
                solo: l.solo,
                trim_db: 0.0,
            })
            .collect();
        self.graph = Some((generation, score));
        self.send_mix();
        self.send_loop();
    }

    /// Sends every layer whose wanted mute, solo or trim differs from what the engine has.
    fn send_mix(&mut self) {
        let (Some((_, graph)), Some((_, want, _))) = (&self.graph, &self.want) else {
            return;
        };
        if graph.layers.len() != want.layers.len() {
            return; // a rebuild is on its way
        }
        let mut cmds = Vec::new();
        for (i, (g, w)) in graph.layers.iter().zip(&want.layers).enumerate() {
            let o = self.overrides.get(&w.id).copied().unwrap_or_default();
            let next = Sent {
                muted: o.muted.unwrap_or(w.muted),
                solo: o.solo.unwrap_or(w.solo),
                trim_db: (w.gain_db - g.gain_db) + o.gain_db.unwrap_or(0.0),
            };
            let Some(prev) = self.sent.get_mut(i) else {
                continue;
            };
            if next.muted != prev.muted {
                cmds.push(Command::SetLayerMute {
                    layer: i,
                    muted: next.muted,
                });
            }
            if next.solo != prev.solo {
                cmds.push(Command::SetLayerSolo {
                    layer: i,
                    solo: next.solo,
                });
            }
            if next.trim_db != prev.trim_db {
                cmds.push(Command::SetLayerGain {
                    layer: i,
                    gain_db: next.trim_db,
                });
            }
            *prev = next;
        }
        for c in cmds {
            self.send(c);
        }
    }

    fn time_map(&self) -> Option<(TimeMap, &Score, u32)> {
        let sr = self.device.as_ref()?.sample_rate;
        let score = self
            .want
            .as_ref()
            .map(|w| &w.1)
            .or(self.graph.as_ref().map(|g| &g.1))?;
        Some((TimeMap::new(score, sr), score, sr))
    }

    fn send_loop(&mut self) {
        let range = match (self.loop_bars, self.time_map()) {
            (Some(b), Some((tm, score, sr))) => {
                let len = score_length_samples(score, sr);
                let start = tm.bar_start_sample(b.start).min(len);
                let end = tm.bar_start_sample(b.end.saturating_add(1)).min(len);
                (start < end).then_some(LoopRange { start, end })
            }
            _ => None,
        };
        self.send(Command::SetLoop(range));
    }

    /// Seconds -> a musical sample position the engine accepts: within the period for loops,
    /// before the end (bars plus tail) for one-shots.
    fn seek_samples(&self, seconds: f64) -> Option<u64> {
        let (tm, score, sr) = self.time_map()?;
        let p = tm.s_to_samples(seconds.max(0.0));
        let len = score_length_samples(score, sr).max(1);
        Some(match score.playback {
            Playback::Loop => p % len,
            _ => p.min(len + tail_samples(score, sr)),
        })
    }

    fn tick(&mut self) {
        let Some(d) = &mut self.device else {
            self.maybe_emit(TransportEvent {
                error: self.device_error.clone(),
                ..TransportEvent::default()
            });
            return;
        };
        self.statuses.clear();
        d.control.drain_status(&mut self.statuses);
        d.control.collect_garbage();
        for s in &self.statuses {
            for c in 0..2 {
                self.peak[c] = self.peak[c].max(s.peak[c]);
                self.sum_sq[c] += (s.rms[c] as f64).powi(2);
            }
            self.meter_n += 1;
        }
        let st = d.control.last_status();
        let stats = d.running.stats();
        let now = Instant::now();
        if stats.callbacks != self.last_callbacks.0 {
            self.last_callbacks = (stats.callbacks, now);
        }
        let stalled = st.playing && now.duration_since(self.last_callbacks.1) > STALL_AFTER;
        // Xruns between two ticks count as heard when the engine played at either end.
        let new_xruns = stats.xruns.saturating_sub(self.xruns_seen);
        if st.playing || self.was_playing {
            self.xruns_playing += new_xruns;
        }
        self.xruns_seen = stats.xruns;
        self.was_playing = st.playing;
        let n = self.meter_n.max(1) as f64;
        let rms = [
            (self.sum_sq[0] / n).sqrt() as f32,
            (self.sum_sq[1] / n).sqrt() as f32,
        ];
        let ev = TransportEvent {
            available: true,
            device: Some(d.name.clone()),
            sample_rate: d.sample_rate,
            error: self.prepare_error.clone(),
            playing: st.playing,
            ended: st.ended,
            position: st.position,
            seconds: st.position as f64 / d.sample_rate.max(1) as f64,
            peak_db: [to_db(self.peak[0]), to_db(self.peak[1])],
            rms_db: [to_db(rms[0]), to_db(rms[1])],
            xruns: self.xruns_playing,
            xruns_total: stats.xruns,
            late_wakeups: stats.late_wakeups,
            device_errors: stats.errors,
            min_callback_frames: stats.min_callback_frames,
            max_callback_frames: stats.max_callback_frames,
            callbacks: stats.callbacks,
            swaps: st.swaps,
            preparing: self.preparing,
            stalled,
        };
        self.peak = [0.0; 2];
        self.sum_sq = [0.0; 2];
        self.meter_n = 0;
        self.maybe_emit(ev);
    }

    /// Emits every tick while playing (or right after a change), else once a second.
    fn maybe_emit(&mut self, ev: TransportEvent) {
        let changed = ev.playing != self.last_event.playing
            || ev.position != self.last_event.position
            || ev.error != self.last_event.error
            || ev.preparing != self.last_event.preparing
            || ev.available != self.last_event.available
            || ev.swaps != self.last_event.swaps
            || ev.xruns != self.last_event.xruns
            || ev.device_errors != self.last_event.device_errors;
        if ev.playing || changed || self.last_emit.elapsed() >= IDLE_EVENT_INTERVAL {
            (self.emit)(&ev);
            self.last_emit = Instant::now();
        }
        self.last_event = ev;
    }

    fn status(&self) -> TransportStatus {
        let stats = self
            .device
            .as_ref()
            .map(|d| d.running.stats())
            .unwrap_or_default();
        let mut event = self.last_event.clone();
        if self.device.is_none() {
            event.error = self.device_error.clone();
        }
        if let Some(d) = &self.device {
            event.available = true;
            event.device = Some(d.name.clone());
            event.sample_rate = d.sample_rate;
        }
        event.preparing = self.preparing;
        if self.prepare_error.is_some() {
            event.error = self.prepare_error.clone();
        }
        TransportStatus {
            event,
            requested_buffer_frames: self.device.as_ref().and_then(|d| d.requested),
            loaded: match (&self.want, &self.graph) {
                (Some(w), Some(g)) => w.0 == g.0,
                _ => false,
            },
            prepare_ms: self.prepare_ms,
            warnings: self.warnings.clone(),
            max_render_ms: stats.max_render_ms,
            p99_render_ms: stats.p99_render_ms,
            layers: self.overrides.clone(),
            loop_range: self.loop_bars,
        }
    }
}
