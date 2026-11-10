//! WP-C1 sampler: SFZ and SoundFont 2 parsing, WAV/FLAC sample decoding, the sampler voice
//! engine and the sampled presets (`felt_piano`, `upright_piano`). See
//! docs/contracts/instruments.md and docs/adr/0005-sample-packs.md.
//!
//! - [`sfz`] and [`sf2`] parse instrument files into the shared [`region::Region`] model.
//! - [`program::Program`] loads a parsed file's samples through [`crate::Packs`] (decoded to f32
//!   once, shared between voices and instruments through `Arc`).
//! - [`engine::SamplerInstrument`] plays a program (implements `praxis_engine::Instrument`).
//! - [`presets`] voices Upright Piano KW into `felt_piano` and `upright_piano`.
//! - [`index`] reads `packs/index.json`; [`install`] (feature `install`) downloads, verifies and
//!   unpacks packs.

use crate::registry::PresetEntry;

pub mod engine;
pub mod index;
#[cfg(feature = "install")]
pub mod install;
pub mod presets;
pub mod program;
pub mod region;
pub mod sample;
pub mod sf2;
pub mod sfz;

pub use engine::{SamplerInstrument, Tone, Voicing};
pub use program::Program;
pub use region::Region;

/// Presets this family provides.
pub fn presets() -> Vec<PresetEntry> {
    presets::entries()
}
