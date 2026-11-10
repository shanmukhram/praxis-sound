//! Instrument traits. FROZEN CONTRACT: praxis-instruments implements these; the engine drives them.

use praxis_score::{GeneratorClass, InstrumentKind, InstrumentRef, Key};

/// A note start delivered to an instrument at the first sample of a `process` call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteOn {
    /// Unique per event within a render; the matching [`Instrument::note_off`] uses it.
    pub id: u64,
    pub pitch: u8,
    /// 0..=1 after accents and clip scaling.
    pub velocity: f32,
    /// 0..=1. 1 for `bright` accents (TIN): the instrument opens its tone; 0 otherwise.
    pub brightness: f32,
    /// Seed for this note's random draws, derived from (score seed, layer seed, note key).
    pub seed: u64,
    /// Scheduled length in samples (note-off follows after this many samples).
    pub length_samples: u64,
}

/// One running instrument instance (one per layer).
///
/// Real-time rules: `note_on`, `note_off`, `set_param` and `process` never allocate, lock, do I/O
/// or panic. Voices are preallocated in the factory. When voices run out, steal the quietest.
pub trait Instrument: Send {
    fn note_on(&mut self, note: &NoteOn);
    fn note_off(&mut self, id: u64);
    /// All notes enter release (transport stop, loop jump).
    fn all_notes_off(&mut self);
    /// Sets a preset parameter by index into [`PresetInfo::params`]; value already clamped.
    fn set_param(&mut self, index: usize, value: f32);
    /// Writes (overwrites) `left`/`right` with the next block of output.
    fn process(&mut self, left: &mut [f32], right: &mut [f32]);
    /// Longest release tail in samples after a note-off.
    fn tail_samples(&self) -> usize;
    /// Silences all voices and clears state.
    fn reset(&mut self);
}

/// A preset parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetParam {
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub unit: String,
}

/// What a preset is. The validator and the planner use this; the engine enforces `class`.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetInfo {
    pub kind: InstrumentKind,
    pub name: String,
    /// The preset's intrinsic class. A layer using a "pad" preset is a pad, whatever it claims.
    pub class: GeneratorClass,
    /// False for unpitched drums/noise (in-key checks skip them).
    pub pitched: bool,
    /// True when notes ring past their length (piano, mallets): patterns should overlap.
    pub sustaining: bool,
    /// Playable MIDI range, inclusive.
    pub range: (u8, u8),
    pub params: Vec<PresetParam>,
    /// Pack the preset comes from and its licence, for THIRD_PARTY reporting.
    pub source: String,
}

/// Context a factory needs to build an instrument.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstrumentContext {
    pub sample_rate: f32,
    pub max_block: usize,
    /// Score key, for SFX patches that tune to the music (audio-quality rule: always in key).
    pub key: Key,
    pub bpm: f64,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum InstrumentError {
    #[error("unknown {kind:?} preset `{preset}`")]
    UnknownPreset {
        kind: InstrumentKind,
        preset: String,
    },
    #[error("preset `{preset}` has no parameter `{param}`")]
    UnknownParam { preset: String, param: String },
    #[error("instrument pack `{0}` is not installed")]
    PackMissing(String),
    #[error("failed to load instrument: {0}")]
    Load(String),
}

/// Builds instruments from score references. Implemented by `praxis_instruments::Registry`.
pub trait InstrumentFactory: Send + Sync {
    fn create(
        &self,
        spec: &InstrumentRef,
        ctx: &InstrumentContext,
    ) -> Result<Box<dyn Instrument>, InstrumentError>;

    fn describe(&self, kind: InstrumentKind, preset: &str) -> Option<PresetInfo>;

    fn presets(&self) -> Vec<PresetInfo>;
}
