//! Project file, undo/redo and named versions (FR-31). OWNED BY WP-A. Types are a FROZEN CONTRACT.

use crate::patch::ScorePatch;
use crate::types::Score;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const PROJECT_FORMAT: &str = "praxis-project";
pub const PROJECT_VERSION: u32 = 1;

/// One applied edit: the patch and its exact inverse.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub seq: u64,
    pub patch: ScorePatch,
    pub inverse: ScorePatch,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Version {
    pub name: String,
    /// History sequence number the version was taken at.
    pub seq: u64,
    pub score: Score,
}

/// Reference clip used by this project. Only measurements are stored, never the audio.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ReferenceRef {
    pub name: String,
    /// Path of the profile JSON relative to the project, or a library style name.
    pub profile: String,
}

/// The `.praxis` project file (JSON).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct Project {
    pub format: String,
    pub version: u32,
    pub score: Score,
    #[serde(default)]
    pub undo: Vec<HistoryEntry>,
    #[serde(default)]
    pub redo: Vec<HistoryEntry>,
    #[serde(default)]
    pub next_seq: u64,
    #[serde(default)]
    pub versions: Vec<Version>,
    #[serde(default)]
    pub references: Vec<ReferenceRef>,
}

impl Project {
    pub fn new(score: Score) -> Project {
        Project {
            format: PROJECT_FORMAT.into(),
            version: PROJECT_VERSION,
            score,
            undo: vec![],
            redo: vec![],
            next_seq: 1,
            versions: vec![],
            references: vec![],
        }
    }
}
