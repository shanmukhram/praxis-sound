//! praxis-engine: turns a Score into audio, offline (deterministic) and in real time, from the
//! same code. See docs/contracts/engine.md.

#![forbid(unsafe_code)]

// ---- frozen contract modules (orchestrator) ----
pub mod api;
pub mod instrument;

// ---- WP-F engine: implementation modules (owned by that package) ----
pub mod render;

// ---- WP-F engine-core modules ----
pub mod automation;
pub mod effects;
pub mod graph;
pub mod loopwrap;
pub mod mixer;
pub mod seeding;
pub mod sequencer;
pub mod testing;

// ---- WP-F2 engine mix bus modules ----
pub mod latency;
pub mod master;
pub mod sidechain;

// ---- WP-R real-time engine ----
pub mod realtime;

pub use render::{RenderConfig, render_offline_with};

pub use api::{RenderError, RenderOptions, RenderOutput, RenderReport, Stem};
pub use instrument::{
    Instrument, InstrumentContext, InstrumentError, InstrumentFactory, NoteOn, PresetInfo,
    PresetParam,
};
pub use realtime::{EngineControl, RealtimeEngine};
pub use render::render_offline;
