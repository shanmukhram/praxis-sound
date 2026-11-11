//! The real-time engine for an AudioWorklet: `praxis_engine::RealtimeEngine` and its control
//! handle in one object, because a worklet is a single thread (commands and audio run on it in
//! turn).
//!
//! **Audio path.** [`Realtime::render`] renders the next frames into buffers allocated once in
//! the constructor; the worklet reads them through `Float32Array` views on the wasm memory
//! ([`Realtime::left_ptr`]), so a block allocates nothing on either side. [`Realtime::process`]
//! (copy in, copy out) does the same for callers that pass their own arrays.
//!
//! **Building.** [`Realtime::load`] prepares the score's graph (instruments, effects; this
//! allocates and, the first time, decodes the pack's samples) and swaps it in, cross-faded when
//! playing. In a worklet that work runs on the audio thread between two blocks: the first load
//! happens while stopped; a swap while playing builds from samples already decoded (they are
//! shared with the playing graph), so it costs one block of a few tens of milliseconds at most.
//! A graph the engine retires is dropped when the status is read after the next block.

use crate::library::Library;
use crate::ops::fail;
use praxis_engine::realtime::{
    Command, EngineControl, LoopRange, RealtimeEngine, RealtimeOptions, Status, prepare,
};
use serde_json::{Value, json};

/// Most frames one [`Realtime::render`] call produces (a worklet asks for 128).
pub const MAX_FRAMES: usize = 4096;

/// Status slots, in order (see [`Realtime::status_ptr`]).
pub const STATUS_FIELDS: &[&str] = &[
    "position",
    "playing",
    "ended",
    "peak_l",
    "peak_r",
    "rms_l",
    "rms_r",
    "callbacks",
    "frames",
    "swaps",
    "rejected_swaps",
    "crossfading",
];

/// One engine instance and everything it plays from.
#[derive(Debug)]
pub struct Realtime {
    engine: RealtimeEngine,
    control: EngineControl,
    sample_rate: u32,
    lib: Library,
    layers: Vec<String>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    status: Vec<f64>,
    length: u64,
    is_loop: bool,
}

impl Realtime {
    /// A stopped engine with nothing loaded, at the device's rate.
    pub fn new(sample_rate: u32) -> Realtime {
        let sample_rate = sample_rate.max(1);
        let (engine, control) = RealtimeEngine::new(sample_rate);
        Realtime {
            engine,
            control,
            sample_rate,
            lib: Library::new(),
            layers: Vec::new(),
            out_l: vec![0.0; MAX_FRAMES],
            out_r: vec![0.0; MAX_FRAMES],
            status: vec![0.0; STATUS_FIELDS.len()],
            length: 0,
            is_loop: false,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn library(&mut self) -> &mut Library {
        &mut self.lib
    }

    /// Prepares `score` and swaps it in. `master_gain_db`: the gain the last offline render
    /// applied (ignored when the score has a frozen gain); `None` plays at 0 dB.
    pub fn load(&mut self, score: &str, master_gain_db: Option<f64>) -> Value {
        let score = match crate::ops::parse_score(score) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let (played, notes) = self.lib.playable(&score);
        let opts = RealtimeOptions {
            sample_rate: self.sample_rate,
            master_gain_db,
        };
        let graph = match prepare(&played, self.lib.registry(), &opts) {
            Ok(g) => g,
            Err(e) => return fail(e.to_string()),
        };
        let info = json!({
            "ok": true,
            "sample_rate": graph.sample_rate(),
            "length_samples": graph.length_samples(),
            "end_samples": graph.end_samples(),
            "is_loop": graph.is_loop(),
            "latency_samples": graph.latency_samples(),
            "master_gain_db": graph.master_gain_db(),
            "blocked_layers": graph.blocked_layers(),
            "warnings": graph.warnings(),
            "fallbacks": notes,
        });
        self.layers = played.layers.iter().map(|l| l.id.clone()).collect();
        self.length = graph.length_samples();
        self.is_loop = graph.is_loop();
        match self.control.swap(graph) {
            Ok(()) => info,
            Err(_) => fail("the engine is busy; try again"),
        }
    }

    fn send(&mut self, cmd: Command) -> bool {
        self.control.send(cmd).is_ok()
    }

    pub fn play(&mut self) -> bool {
        self.control.play()
    }

    pub fn stop(&mut self) -> bool {
        self.control.stop()
    }

    /// Jumps to a musical position in samples (wrapped into the loop for loops).
    pub fn seek(&mut self, position: u64) -> bool {
        let p = if self.is_loop && self.length > 0 {
            position % self.length
        } else {
            position
        };
        self.control.seek(p)
    }

    /// Loops between two musical positions in samples; `end <= start` stops looping.
    pub fn set_loop(&mut self, start: u64, end: u64) -> bool {
        let range = (end > start).then_some(LoopRange { start, end });
        self.control.set_loop(range)
    }

    fn layer(&self, id: &str) -> Option<usize> {
        self.layers.iter().position(|l| l == id)
    }

    pub fn set_layer_mute(&mut self, id: &str, muted: bool) -> bool {
        match self.layer(id) {
            Some(layer) => self.send(Command::SetLayerMute { layer, muted }),
            None => false,
        }
    }

    pub fn set_layer_solo(&mut self, id: &str, solo: bool) -> bool {
        match self.layer(id) {
            Some(layer) => self.send(Command::SetLayerSolo { layer, solo }),
            None => false,
        }
    }

    pub fn set_layer_gain(&mut self, id: &str, gain_db: f32) -> bool {
        match self.layer(id) {
            Some(layer) => self.send(Command::SetLayerGain { layer, gain_db }),
            None => false,
        }
    }

    pub fn set_master_gain(&mut self, gain_db: f32) -> bool {
        self.send(Command::SetMasterGain { gain_db })
    }

    /// Renders the next `frames` frames (at most [`MAX_FRAMES`]) into the engine's own buffers
    /// and updates the status. Returns the frames rendered. Allocates nothing.
    pub fn render(&mut self, frames: usize) -> usize {
        let n = frames.min(MAX_FRAMES);
        self.engine
            .process(&mut self.out_l[..n], &mut self.out_r[..n]);
        let s = self.control.poll();
        self.write_status(&s);
        n
    }

    /// Renders into the caller's arrays (equal length; any size).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        self.engine.process(&mut left[..n], &mut right[..n]);
        let s = self.control.poll();
        self.write_status(&s);
    }

    fn write_status(&mut self, s: &Status) {
        let v = [
            s.position as f64,
            f64::from(u8::from(s.playing)),
            f64::from(u8::from(s.ended)),
            f64::from(s.peak[0]),
            f64::from(s.peak[1]),
            f64::from(s.rms[0]),
            f64::from(s.rms[1]),
            s.callbacks as f64,
            s.frames as f64,
            s.swaps as f64,
            s.rejected_swaps as f64,
            f64::from(u8::from(s.crossfading)),
        ];
        self.status.copy_from_slice(&v);
    }

    /// The last rendered left channel (`MAX_FRAMES` long; the first `render` frames are new).
    pub fn left(&self) -> &[f32] {
        &self.out_l
    }

    pub fn right(&self) -> &[f32] {
        &self.out_r
    }

    /// The status after the last block, in [`STATUS_FIELDS`] order.
    pub fn status(&self) -> &[f64] {
        &self.status
    }

    /// Address of the left buffer in the wasm memory (for a `Float32Array` view).
    pub fn left_ptr(&self) -> usize {
        self.out_l.as_ptr() as usize
    }

    pub fn right_ptr(&self) -> usize {
        self.out_r.as_ptr() as usize
    }

    pub fn status_ptr(&self) -> usize {
        self.status.as_ptr() as usize
    }
}
