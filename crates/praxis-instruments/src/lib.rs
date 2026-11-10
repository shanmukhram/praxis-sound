//! praxis-instruments: every sound source the engine can play.
//!
//! Each family module exposes `pub fn presets() -> Vec<PresetEntry>`; the [`Registry`] collects
//! them. Families are owned by different work packages, so they never edit each other's files.

#![forbid(unsafe_code)]

// ---- frozen contract modules (orchestrator) ----
pub mod packs;
pub mod registry;

// ---- family modules (work packages) ----
/// WP-C2 drums: synthesised kick, snare, hats, toms; sampled kits via the sampler.
pub mod drums;
/// WP-C1 sampler: SFZ / SoundFont parsing, sample decoding, sampler voices, sampled presets.
pub mod sampler;
/// WP-H sfx: procedural whoosh, impact, riser, drone, glitch, tick families with macros.
pub mod sfx;
/// WP-C2 synth: bell, subtractive, FM, wavetable, pluck, struck, granular, noise presets.
pub mod synth;

pub use packs::Packs;
pub use registry::{PresetEntry, Registry};
