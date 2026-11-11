//! Real-time playback (Phase 3, FR-27; docs/contracts/engine.md "Real time").
//!
//! The same graph the offline renderer drives (`graph.rs`, built by the same code in
//! `render.rs`) plays inside a [`RealtimeEngine`] owned by the audio callback. The control side
//! talks to it only through `rtrb` single-producer/single-consumer queues, held by an
//! [`EngineControl`]:
//!
//! - **commands in** ([`Command`]): play, stop, seek, loop range, layer gain/mute/solo, master
//!   gain, and swapping in a graph prepared off the audio thread ([`prepare`]). The graph moves
//!   through the queue as a `Box`; nothing is built or freed on the audio thread.
//! - **status out** ([`Status`], one per callback): position, peak/RMS per channel, the device's
//!   xrun and late-callback counters, swap counters.
//! - **retired graphs back**: a replaced graph returns through a third queue and is dropped by
//!   [`EngineControl::poll`] (or [`EngineControl::collect_garbage`]) on the control thread.
//!
//! **Blocks.** Callbacks may be any size; the engine keeps the offline renderer's fixed
//! [`BLOCK`] (128) internally and buffers one block, so events land on their exact sample and
//! automation is evaluated at the same grid points as offline.
//!
//! **Equivalence.** Played from 0 with the master gain fixed (a frozen gain, or the applied gain
//! of the last offline render), a one-shot's output is the offline render sample for sample, in
//! any callback size: same graph, same blocks, same gain multiply, same limiter fed the same
//! 128-sample chunks, latency compensated by dropping the first samples (see `player.rs`). The
//! only place they can differ is the last limiter-lookahead (~3 ms) of the render, where offline
//! feeds the limiter silence and real time feeds it the graph's continuing tail; the limiter
//! output is identical unless that tail is loud enough to make it reduce gain.
//!
//! **Master.** Real time uses the frozen gain, else the gain the caller passes from the last
//! offline render, and the same true-peak limiter. It never measures loudness.
//!
//! **Transport.** Stop and seek fade out over [`DECLICK_MS`], then stop or restart cleanly at the
//! new position (fresh state, pre-rolled, latency compensated). A one-shot stops at its end
//! (bars plus tail). `playback: loop` scores repeat seamlessly forever: the sequencers and
//! automation are periodic, so tails of the last bar ring into bar 1 exactly as in the offline
//! loop wrap. A loop range ([`LoopRange`]) jumps from its end back to its start with tails
//! continuing: notes are released (their release rings), effect state is kept (reverb tails
//! continue). A layer whose own chain has latency (a lookahead limiter or compressor on the
//! layer) re-enters the range up to that latency (a few ms) late.
//!
//! **Swap.** A swapped-in graph starts at the current position and is cross-faded in over
//! [`SWAP_CROSSFADE_MS`] while the old graph keeps playing, so an edit while playing does not
//! click. Notes that were already sounding in the old graph fade out with it. Live layer
//! settings belong to a graph: the new one starts from its score's mute/solo and 0 dB trims.
//!
//! The module is WebAssembly-safe: no threads, no clocks, no I/O. Timing (xruns, late callbacks)
//! is measured by the host (`praxis_audio_io::device`) and passed in with
//! [`RealtimeEngine::set_device_counters`].

mod engine;
mod player;
mod prepare;

pub use engine::RealtimeEngine;
pub use player::LoopRange;
pub use prepare::{PreparedGraph, RealtimeOptions, prepare};

use rtrb::{Consumer, Producer, RingBuffer};

/// The fixed internal block, equal to the offline default (`RenderOptions::block_size`).
pub const BLOCK: usize = 128;
/// Longest stretch rendered at once inside a callback (the crossfade scratch size).
pub const CHUNK: usize = 1024;
/// Declick fade before a stop or a seek.
pub const DECLICK_MS: f64 = 5.0;
/// Crossfade when a new graph is swapped in while playing.
pub const SWAP_CROSSFADE_MS: f64 = 30.0;
/// Ramp for live gain, mute, solo and master gain changes.
pub const RAMP_MS: f64 = 10.0;
/// Command queue capacity.
pub const COMMAND_CAPACITY: usize = 256;
/// Status queue capacity (one status per callback; the oldest are kept if nobody reads).
pub const STATUS_CAPACITY: usize = 1024;
/// Return-queue capacity for retired graphs.
pub const RETIRED_CAPACITY: usize = 8;
/// Commands applied per callback at most (the rest wait for the next one).
pub const MAX_COMMANDS_PER_CALLBACK: usize = 64;

fn ms_to_samples(ms: f64, sample_rate: u32) -> usize {
    ((ms * 0.001 * sample_rate as f64).round() as usize).max(1)
}

fn declick_samples(sample_rate: u32) -> usize {
    ms_to_samples(DECLICK_MS, sample_rate)
}

fn swap_crossfade_samples(sample_rate: u32) -> usize {
    ms_to_samples(SWAP_CROSSFADE_MS, sample_rate)
}

fn ramp_samples(sample_rate: u32) -> f32 {
    ms_to_samples(RAMP_MS, sample_rate) as f32
}

/// A command for the engine. Layer indices are indices into `score.layers`
/// ([`PreparedGraph::layer_index`]); unknown or avoid-listed layers are ignored.
#[derive(Debug)]
pub enum Command {
    /// Start playing from the stopped position (0, the last stop, or a seek).
    Play,
    /// Fade out and stop; the next `Play` continues from here.
    Stop,
    /// Jump to a musical position in samples (within the loop period for loops).
    Seek(u64),
    /// Loop between two musical positions, or stop looping.
    SetLoop(Option<LoopRange>),
    /// Live trim on top of the score's layer gain, in dB (-60..=24).
    SetLayerGain {
        layer: usize,
        gain_db: f32,
    },
    SetLayerMute {
        layer: usize,
        muted: bool,
    },
    SetLayerSolo {
        layer: usize,
        solo: bool,
    },
    /// Replace the master gain (dB), e.g. after normalising again offline.
    SetMasterGain {
        gain_db: f32,
    },
    /// Swap in a prepared graph (cross-faded when playing). Rejected (returned) when its sample
    /// rate differs from the engine's.
    Swap(Box<PreparedGraph>),
}

/// What the engine reports after every callback.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Status {
    /// Musical position of the next output sample (within the period for loops).
    pub position: u64,
    pub playing: bool,
    /// A one-shot played to its end and stopped.
    pub ended: bool,
    /// Peak and RMS of this callback's output, per channel (linear).
    pub peak: [f32; 2],
    pub rms: [f32; 2],
    pub callbacks: u64,
    pub frames: u64,
    /// Device underruns and late callbacks, as reported by the host.
    pub xruns: u64,
    pub late_callbacks: u64,
    pub swaps: u64,
    pub rejected_swaps: u64,
    pub crossfading: bool,
    /// Statuses dropped because the status queue was full.
    pub dropped_reports: u64,
}

impl RealtimeEngine {
    /// A stopped engine with no graph, and its control handle. Swap a graph in to play.
    /// Allocates the queues: call off the audio thread, then move the engine into the callback.
    pub fn new(sample_rate: u32) -> (RealtimeEngine, EngineControl) {
        let (cmd_tx, cmd_rx) = RingBuffer::new(COMMAND_CAPACITY);
        let (st_tx, st_rx) = RingBuffer::new(STATUS_CAPACITY);
        let (ret_tx, ret_rx) = RingBuffer::new(RETIRED_CAPACITY);
        let engine = RealtimeEngine::from_parts(sample_rate, cmd_rx, st_tx, ret_tx);
        let control = EngineControl {
            commands: cmd_tx,
            status: st_rx,
            retired: ret_rx,
            last: Status::default(),
        };
        (engine, control)
    }

    /// [`Self::new`] with `graph` already loaded (stopped at 0).
    pub fn with_graph(graph: PreparedGraph) -> (RealtimeEngine, EngineControl) {
        let (mut engine, control) = RealtimeEngine::new(graph.sample_rate());
        engine.apply_now(Command::Swap(Box::new(graph)));
        (engine, control)
    }
}

/// The control side (UI or main thread) of a [`RealtimeEngine`].
pub struct EngineControl {
    commands: Producer<Command>,
    status: Consumer<Status>,
    retired: Consumer<Box<PreparedGraph>>,
    last: Status,
}

impl std::fmt::Debug for EngineControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineControl")
            .field("last", &self.last)
            .finish_non_exhaustive()
    }
}

impl EngineControl {
    /// Queues a command. Returns it back when the queue is full.
    pub fn send(&mut self, cmd: Command) -> Result<(), Command> {
        self.commands
            .push(cmd)
            .map_err(|rtrb::PushError::Full(c)| c)
    }

    pub fn play(&mut self) -> bool {
        self.send(Command::Play).is_ok()
    }

    pub fn stop(&mut self) -> bool {
        self.send(Command::Stop).is_ok()
    }

    pub fn seek(&mut self, position: u64) -> bool {
        self.send(Command::Seek(position)).is_ok()
    }

    pub fn set_loop(&mut self, range: Option<LoopRange>) -> bool {
        self.send(Command::SetLoop(range)).is_ok()
    }

    /// Queues a graph swap; returns the graph when the queue is full.
    pub fn swap(&mut self, graph: PreparedGraph) -> Result<(), Box<PreparedGraph>> {
        match self.send(Command::Swap(Box::new(graph))) {
            Ok(()) => Ok(()),
            Err(Command::Swap(g)) => Err(g),
            Err(_) => Ok(()),
        }
    }

    /// Drains the status queue, drops retired graphs, and returns the latest status.
    pub fn poll(&mut self) -> Status {
        while let Ok(s) = self.status.pop() {
            self.last = s;
        }
        self.collect_garbage();
        self.last
    }

    /// Drains every queued status (oldest first) into `out`.
    pub fn drain_status(&mut self, out: &mut Vec<Status>) {
        while let Ok(s) = self.status.pop() {
            self.last = s;
            out.push(s);
        }
    }

    /// Drops graphs the engine has retired. Returns how many.
    pub fn collect_garbage(&mut self) -> usize {
        let mut n = 0;
        while let Ok(g) = self.retired.pop() {
            drop(g);
            n += 1;
        }
        n
    }

    /// The last status seen by [`Self::poll`] or [`Self::drain_status`].
    pub fn last_status(&self) -> Status {
        self.last
    }
}
