//! Sound Profile v1 (FR-12). FROZEN CONTRACT. See docs/contracts/sound-profile.md.
//! A profile holds measurements only, never audio, so it can be kept for copyrighted clips.

use praxis_score::{Grid, Key};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const PROFILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SoundProfile {
    pub version: u32,
    pub source: SourceInfo,
    pub class: Classification,
    pub tempo: Option<TempoEstimate>,
    pub key: Option<KeyEstimate>,
    pub loudness: LoudnessStats,
    pub envelope: Envelope,
    pub spectral: Spectral,
    pub groove: Option<Groove>,
    /// Onset times in seconds.
    pub onsets_s: Vec<f64>,
    /// Pitch track at a 10 ms hop (YIN); `None` frames are unvoiced.
    pub pitch: PitchTrack,
    /// 0 = pure noise, 1 = pure tone.
    pub tonality: f32,
    pub stereo: StereoStats,
    pub reverb: Option<ReverbEstimate>,
    pub sections: Vec<SectionBoundary>,
    /// Fixed-length feature vector for similarity; see [`FEATURE_NAMES`].
    pub features: Vec<f32>,
    /// User-facing tags and the library style name (FR-19).
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub style_name: Option<String>,
}

/// Names of the entries of [`SoundProfile::features`], in order. WP-D fills this list; once
/// published, entries are only appended.
pub const FEATURE_NAMES: &[&str] = &[];

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub file_name: String,
    pub duration_s: f64,
    pub sample_rate: u32,
    pub channels: u16,
    /// blake3 of the decoded PCM, to recognise the same clip again (not the audio itself).
    pub pcm_hash: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ClipKind {
    Music,
    Sfx,
    Ambience,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Classification {
    pub kind: ClipKind,
    pub confidence: f32,
    /// Probabilities for music, sfx, ambience (sum 1).
    pub scores: [f32; 3],
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct TempoEstimate {
    pub bpm: f64,
    pub confidence: f32,
    /// Beat times in seconds.
    pub beats_s: Vec<f64>,
    /// Time of the first downbeat, seconds.
    pub downbeat_s: f64,
    pub beats_per_bar: u8,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct KeyEstimate {
    pub key: Key,
    pub confidence: f32,
    /// Mean chroma, C first, normalised to max 1.
    pub chroma: [f32; 12],
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct LoudnessStats {
    pub integrated_lufs: f64,
    pub range_lu: f64,
    pub true_peak_dbtp: f64,
    pub sample_peak_dbfs: f64,
    pub rms_dbfs: f64,
    /// Peak-to-RMS ratio, dB.
    pub crest_db: f64,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Envelope {
    pub attack_s: f32,
    pub decay_s: f32,
    /// Sustain level relative to peak, 0..1.
    pub sustain: f32,
    pub release_s: f32,
    /// RMS envelope in dBFS at `hop_s` spacing, at most 2000 frames (decimated for long clips).
    pub frames_db: Vec<f32>,
    pub hop_s: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Spectral {
    pub centroid_hz: f32,
    pub rolloff_hz: f32,
    pub flatness: f32,
    /// Energy in octave bands centred 31.5 Hz .. 16 kHz (10 bands), dB relative to total.
    pub bands_db: Vec<f32>,
    /// Spectral slope, dB per octave.
    pub tilt_db_per_oct: f32,
    /// Centroid over time (same hop as the envelope), Hz. Used for sweeps and risers.
    pub centroid_track_hz: Vec<f32>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Groove {
    pub grid: Grid,
    pub steps_per_bar: u32,
    /// Onset strength per grid step, averaged over bars, max 1.
    pub step_weights: Vec<f32>,
    /// 0 = straight, 1 = full triplet swing.
    pub swing: f32,
    pub onsets_per_bar: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct PitchTrack {
    pub hop_s: f32,
    /// MIDI pitch (fractional) per frame or null when unvoiced.
    pub midi: Vec<Option<f32>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct StereoStats {
    /// Side energy / mid energy, 0 = mono.
    pub width: f32,
    /// L/R correlation -1..1.
    pub correlation: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ReverbEstimate {
    pub rt60_s: f32,
    pub confidence: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SectionBoundary {
    pub start_s: f64,
    pub end_s: f64,
    pub label: String,
}
