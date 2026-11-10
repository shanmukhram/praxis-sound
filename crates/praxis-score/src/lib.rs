//! praxis-score: the Score document every part of Praxis Sound reads and writes.
//!
//! The engine renders only a [`Score`]. Prompts, reference clips and manual edits all change it
//! through [`patch::ScorePatch`]es. This crate has no I/O and compiles to WebAssembly.

#![forbid(unsafe_code)]

pub mod params;
pub mod patch;
pub mod theory;
pub mod timeline;
pub mod types;
pub mod validate;

// Implemented by work package WP-A (score-ops); module files are owned by that package.
pub mod apply;
pub mod lint;
pub mod project;
pub mod rhythm;

pub use types::*;

/// JSON Schema (draft 2020-12) for [`Score`], as published in `schema/score.schema.json`.
pub fn score_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Score)).expect("schema serialises")
}

/// JSON Schema for [`patch::ScorePatch`], as published in `schema/patch.schema.json`.
pub fn patch_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(patch::ScorePatch)).expect("schema serialises")
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("not valid score JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("score failed validation: {0:?}")]
    Invalid(Vec<validate::Issue>),
}

/// Parses and validates a score from JSON.
pub fn from_json(text: &str) -> Result<Score, LoadError> {
    let score: Score = serde_json::from_str(text)?;
    let report = validate::validate(&score);
    if !report.is_valid() {
        return Err(LoadError::Invalid(report.errors));
    }
    Ok(score)
}

/// Canonical pretty JSON (stable field order; maps are sorted).
pub fn to_json(score: &Score) -> String {
    serde_json::to_string_pretty(score).expect("score serialises")
}

impl Score {
    /// A valid score with no layers: `bars` bars of silence.
    pub fn empty(bars: u32, bpm: f64) -> Score {
        Score {
            schema_version: SCHEMA_VERSION,
            title: String::new(),
            seed: 0,
            meta: Meta::default(),
            tempo: Tempo { bpm },
            time_signature: TimeSignature {
                numerator: 4,
                denominator: 4,
            },
            key: Key {
                tonic: PitchClass::C,
                mode: Mode::Major,
            },
            bars,
            playback: Playback::default(),
            constraints: Constraints::default(),
            sections: vec![],
            patterns: vec![],
            layers: vec![],
            master: Master::default(),
            markers: vec![],
            chords: vec![],
        }
    }

    pub fn layer(&self, id: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn pattern(&self, id: &str) -> Option<&Pattern> {
        self.patterns.iter().find(|p| p.id == id)
    }
}
