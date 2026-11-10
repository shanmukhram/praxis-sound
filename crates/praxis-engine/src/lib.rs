//! praxis-engine: turns a Score into audio, offline (deterministic) and in real time, from the
//! same code. See docs/contracts/engine.md.

#![forbid(unsafe_code)]

// ---- frozen contract modules (orchestrator) ----
pub mod api;
pub mod instrument;

// ---- WP-F engine: implementation modules (owned by that package) ----
pub mod render;

pub use api::{RenderError, RenderOptions, RenderOutput, RenderReport, Stem};
pub use instrument::{
    Instrument, InstrumentContext, InstrumentError, InstrumentFactory, NoteOn, PresetInfo,
    PresetParam,
};
pub use render::render_offline;
