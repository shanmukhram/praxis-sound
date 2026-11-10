//! Score schema v1. FROZEN CONTRACT: only the orchestrator changes this file.
//! See docs/contracts/score.md for the meaning of every field.
//!
//! Conventions:
//! - Bars are 1-based and ranges are inclusive (`bars 9-16` is `{start: 9, end: 16}`).
//! - Pitches are MIDI note numbers (60 = C4). Drum layers use the General MIDI drum map.
//! - Note `step` and `length` are measured in steps of the pattern's grid.
//!   `length` above 1.0 overlaps the next step (FR-4).
//! - All maps are `BTreeMap` so serialisation order is deterministic.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Version written by this build. Readers reject other versions (no migration yet).
pub const SCHEMA_VERSION: u32 = 1;

/// Stable identifier: 1-64 chars of `[a-z0-9_-]`, starting with a letter or digit.
pub type Id = String;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Score {
    pub schema_version: u32,
    #[serde(default)]
    pub title: String,
    /// Root seed. Every random draw in rendering derives from this (see contracts/engine.md).
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub meta: Meta,
    pub tempo: Tempo,
    pub time_signature: TimeSignature,
    pub key: Key,
    /// Total bar count. Must be >= 1.
    pub bars: u32,
    #[serde(default)]
    pub playback: Playback,
    #[serde(default)]
    pub constraints: Constraints,
    #[serde(default)]
    pub sections: Vec<Section>,
    #[serde(default)]
    pub patterns: Vec<Pattern>,
    #[serde(default)]
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub master: Master,
    /// Hit points and other timed markers (FR-10).
    #[serde(default)]
    pub markers: Vec<Marker>,
    /// Harmonic map (FR-9). Informational for validators and generators; the engine ignores it.
    #[serde(default)]
    pub chords: Vec<ChordSpan>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    /// The brief this score was planned from, verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    /// One emotion word, e.g. "melancholy". Restraint rule: one emotion per score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotion: Option<String>,
    /// Overall intensity 0..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f32>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Names of Sound Profiles this score was derived from (FR-14, FR-19).
    #[serde(default)]
    pub reference_profiles: Vec<String>,
    /// Free-form provenance, e.g. planner provider and model id.
    #[serde(default)]
    pub provenance: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tempo {
    /// Constant tempo in quarter-note beats per minute, 20..=400. Tempo maps are reserved for v2.
    pub bpm: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TimeSignature {
    pub numerator: u8,
    /// 2, 4, 8 or 16.
    pub denominator: u8,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub tonic: PitchClass,
    pub mode: Mode,
}

#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
pub enum PitchClass {
    C,
    #[serde(rename = "C#", alias = "Db")]
    Cs,
    D,
    #[serde(rename = "Eb", alias = "D#")]
    Eb,
    E,
    F,
    #[serde(rename = "F#", alias = "Gb")]
    Fs,
    G,
    #[serde(rename = "Ab", alias = "G#")]
    Ab,
    A,
    #[serde(rename = "Bb", alias = "A#")]
    Bb,
    B,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Major,
    Minor,
    HarmonicMinor,
    MelodicMinor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    MajorPentatonic,
    MinorPentatonic,
    /// Atonal material (sound design). Disables the in-key check.
    Chromatic,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Playback {
    /// Seamless loop: tails wrap from the end into the start (FR-26).
    Loop,
    /// One-shot: rendered once, followed by `tail_seconds` of release/effect tail.
    OneShot {
        #[serde(default = "default_tail_seconds")]
        tail_seconds: f64,
    },
}

impl Default for Playback {
    fn default() -> Self {
        Playback::OneShot {
            tail_seconds: default_tail_seconds(),
        }
    }
}

fn default_tail_seconds() -> f64 {
    4.0
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Constraints {
    /// Generator classes that must not appear (FR-3). Enforced by the validator AND the engine.
    #[serde(default)]
    pub avoid: BTreeSet<GeneratorClass>,
}

/// What a layer is, musically. Every layer and every preset declares one; the avoid-list keys on it.
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum GeneratorClass {
    Melody,
    Motif,
    Harmony,
    Chords,
    Arp,
    Bass,
    Drums,
    Percussion,
    Pad,
    Drone,
    Texture,
    Ambience,
    Riser,
    Sweep,
    Impact,
    Boom,
    Whoosh,
    Glitch,
    Tick,
    Fill,
    Sfx,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct BarRange {
    /// First bar, 1-based, inclusive.
    pub start: u32,
    /// Last bar, inclusive.
    pub end: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub id: Id,
    #[serde(default)]
    pub name: String,
    pub bars: BarRange,
    pub role: SectionRole,
    /// 0..1 intensity for tension curves (FR-9).
    #[serde(default = "half")]
    pub intensity: f32,
}

fn half() -> f32 {
    0.5
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SectionRole {
    Intro,
    Main,
    Verse,
    Build,
    Peak,
    Breakdown,
    Release,
    Transition,
    Outro,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Grid {
    Quarter,
    Eighth,
    Sixteenth,
    EighthTriplet,
    SixteenthTriplet,
}

/// A rhythm-first motif on a step grid (FR-4). Patterns repeat when a clip is longer than them.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Pattern {
    pub id: Id,
    pub grid: Grid,
    /// Pattern length in steps. Must be a whole number of bars.
    pub length_steps: u32,
    pub notes: Vec<Note>,
    /// Spoken-rhythm source this pattern was built from, e.g. "DON din DON don TIN" (FR-5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Note {
    /// 0-based step index within the pattern.
    pub step: u32,
    /// MIDI note number 0..=127.
    pub pitch: u8,
    /// 0 < velocity <= 1.
    pub velocity: f32,
    #[serde(default)]
    pub accent: Accent,
    /// Length in steps, > 0. Values above 1.0 overlap the next step. Max 64.
    pub length: f32,
    /// Micro-timing offset as a fraction of one step, -0.5..=0.5 (swing, push, lay back).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// Accent vocabulary from spoken rhythm: DON = strong, din = soft, don = normal, TIN = bright.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Normal,
    Strong,
    Soft,
    Bright,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub id: Id,
    #[serde(default)]
    pub name: String,
    pub class: GeneratorClass,
    pub instrument: InstrumentRef,
    /// Clips on one layer must not overlap.
    #[serde(default)]
    pub clips: Vec<Clip>,
    /// A locked layer cannot be changed by any patch except an explicit user unlock (FR-6).
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default)]
    pub gain_db: f32,
    /// -1 (left) ..= 1 (right).
    #[serde(default)]
    pub pan: f32,
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub automation: Vec<Automation>,
    /// Added to the score seed for this layer's random draws; variations change it (FR-8).
    #[serde(default)]
    pub seed_offset: u64,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub bars: BarRange,
    pub pattern: Id,
    /// Pattern step at which this clip starts playing, so a clip split at bar N keeps the phase it
    /// had (rule 6 of contracts/patch.md). Default 0. Must be < the pattern's `length_steps`.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub pattern_offset_steps: u32,
    /// Semitones added to every note.
    #[serde(default)]
    pub transpose: i8,
    #[serde(default = "one")]
    pub velocity_scale: f32,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn one() -> f32 {
    1.0
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    /// Sample playback (SFZ / SoundFont packs).
    Sampler,
    /// Synthesis (subtractive, FM, wavetable, physical models, granular, noise).
    Synth,
    /// Drum and percussion voices (synthesised or sampled kits).
    Drum,
    /// Procedural sound-effect patch families with macro knobs (FR-16).
    Sfx,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InstrumentRef {
    pub kind: InstrumentKind,
    /// Preset name from the instrument registry, e.g. "felt_piano", "soft_kick", "bell".
    pub preset: String,
    /// Preset parameters by name; ranges come from the preset's declared parameter specs.
    #[serde(default)]
    pub params: BTreeMap<String, f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Eq,
    Filter,
    Compressor,
    Saturation,
    Delay,
    Reverb,
    ConvolutionReverb,
    Chorus,
    Width,
    Limiter,
    Gain,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    pub id: Id,
    pub kind: EffectKind,
    /// Parameters by name; names and ranges are listed in `crate::params::effect_params`.
    #[serde(default)]
    pub params: BTreeMap<String, f64>,
    #[serde(default)]
    pub bypass: bool,
    /// Layer id whose pre-effects signal keys this effect (compressor sidechain).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidechain: Option<Id>,
    /// Impulse response name for `convolution_reverb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impulse: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Automation {
    /// Parameter path: "gain_db", "pan", "instrument.<param>" or "effects.<effect id>.<param>".
    pub target: String,
    pub curve: Curve,
    /// Sorted by position, at least one point.
    pub points: Vec<AutoPoint>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AutoPoint {
    /// Position in bars, 1-based and fractional: 1.0 is the start of bar 1, 2.5 the middle of bar 2.
    pub bar: f64,
    pub value: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    Linear,
    Step,
    /// Cosine ease between points.
    Smooth,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
#[derive(Default)]
pub struct Master {
    #[serde(default)]
    pub effects: Vec<Effect>,
    #[serde(default)]
    pub loudness: Loudness,
    /// Frozen make-up gain from the last normalisation. When set, renders apply it instead of
    /// re-measuring, so a scoped edit cannot change the level of untouched bars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_gain_db: Option<f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Loudness {
    /// Integrated loudness target in LUFS (BS.1770). Default -16.
    #[serde(default = "default_lufs")]
    pub target_lufs: f64,
    /// True-peak ceiling in dBTP. Default -1.
    #[serde(default = "default_true_peak")]
    pub true_peak_dbtp: f64,
    /// Normalise to the target. Off means only the true-peak limiter applies.
    #[serde(default = "yes")]
    pub normalize: bool,
}

impl Default for Loudness {
    fn default() -> Self {
        Loudness {
            target_lufs: default_lufs(),
            true_peak_dbtp: default_true_peak(),
            normalize: true,
        }
    }
}

fn default_lufs() -> f64 {
    -16.0
}
fn default_true_peak() -> f64 {
    -1.0
}
fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub id: Id,
    /// Seconds from the start of bar 1.
    pub time_s: f64,
    #[serde(default)]
    pub label: String,
    pub kind: MarkerKind,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MarkerKind {
    /// The arrangement must land an accent or section change on this time.
    Hit,
    Cut,
    Reveal,
    Note,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChordSpan {
    pub bars: BarRange,
    /// Beat within the first bar where the chord starts, 0-based. Default 0.
    #[serde(default)]
    pub beat: f32,
    /// Chord symbol: root, optional quality and bass, e.g. "Am", "Fmaj7", "C/E", "G7", "Dsus4".
    pub symbol: String,
}
