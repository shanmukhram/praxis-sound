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

// ---- logic (WP-A) ----

mod diff;
mod error;

pub use diff::patch_between;
pub use error::ProjectError;

use crate::apply::apply;
use crate::patch::{Applied, PatchError};

impl Project {
    /// Applies `patch` to the score, records `{seq, patch, inverse}` on the undo stack and clears
    /// the redo stack. On error nothing changes.
    pub fn commit(&mut self, patch: ScorePatch) -> Result<&HistoryEntry, PatchError> {
        let applied = apply(&self.score, &patch)?;
        Ok(self.record(patch, applied))
    }

    fn record(&mut self, patch: ScorePatch, applied: Applied) -> &HistoryEntry {
        let seq = self.next_seq.max(1);
        self.next_seq = seq.saturating_add(1);
        self.score = applied.score;
        self.redo.clear();
        self.undo.push(HistoryEntry {
            seq,
            patch,
            inverse: applied.inverse,
        });
        &self.undo[self.undo.len() - 1]
    }

    /// Undoes the newest edit: applies its inverse and moves the entry to the redo stack.
    /// `Ok(None)` when there is nothing to undo. On error the stacks and score are unchanged.
    pub fn undo(&mut self) -> Result<Option<&HistoryEntry>, PatchError> {
        let Some(entry) = self.undo.pop() else {
            return Ok(None);
        };
        match apply(&self.score, &entry.inverse) {
            Ok(a) => {
                self.score = a.score;
                self.redo.push(entry);
                Ok(self.redo.last())
            }
            Err(e) => {
                self.undo.push(entry);
                Err(e)
            }
        }
    }

    /// Re-applies the newest undone edit and moves it back to the undo stack.
    /// `Ok(None)` when there is nothing to redo. On error the stacks and score are unchanged.
    pub fn redo(&mut self) -> Result<Option<&HistoryEntry>, PatchError> {
        let Some(entry) = self.redo.pop() else {
            return Ok(None);
        };
        match apply(&self.score, &entry.patch) {
            Ok(a) => {
                self.score = a.score;
                self.undo.push(entry);
                Ok(self.undo.last())
            }
            Err(e) => {
                self.redo.push(entry);
                Err(e)
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Sequence number of the newest applied edit, 0 before the first edit.
    pub fn current_seq(&self) -> u64 {
        self.undo.last().map_or(0, |e| e.seq)
    }

    /// Snapshots the current score under `name` (trimmed, non-empty, unique).
    pub fn save_version(&mut self, name: &str) -> Result<&Version, ProjectError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProjectError::EmptyVersionName);
        }
        if self.version(name).is_some() {
            return Err(ProjectError::DuplicateVersion(name.to_string()));
        }
        self.versions.push(Version {
            name: name.to_string(),
            seq: self.current_seq(),
            score: self.score.clone(),
        });
        Ok(&self.versions[self.versions.len() - 1])
    }

    pub fn version(&self, name: &str) -> Option<&Version> {
        let name = name.trim();
        self.versions.iter().find(|v| v.name == name)
    }

    /// Deletes a named version and returns it. History is not affected.
    pub fn delete_version(&mut self, name: &str) -> Result<Version, ProjectError> {
        let name = name.trim();
        let i = self
            .versions
            .iter()
            .position(|v| v.name == name)
            .ok_or_else(|| ProjectError::UnknownVersion(name.to_string()))?;
        Ok(self.versions.remove(i))
    }

    /// Makes the score equal to version `name` through an ordinary user patch
    /// ([`patch_between`]), so the restore itself can be undone.
    pub fn restore_version(&mut self, name: &str) -> Result<&HistoryEntry, ProjectError> {
        let version = self
            .version(name)
            .ok_or_else(|| ProjectError::UnknownVersion(name.trim().to_string()))?;
        let target = version.score.clone();
        let vname = version.name.clone();
        let patch = patch_between(&self.score, &target, format!("restore version `{vname}`"))
            .map_err(|reason| ProjectError::Restore {
                name: vname.clone(),
                reason,
            })?;
        let applied = apply(&self.score, &patch)?;
        if applied.score != target {
            return Err(ProjectError::Restore {
                name: vname,
                reason: "the restore patch did not reproduce the version".into(),
            });
        }
        Ok(self.record(patch, applied))
    }

    /// Pretty JSON of the whole project (stable field order).
    pub fn to_json(&self) -> Result<String, ProjectError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parses a project. Checks `format` and `version` before anything else, then validates the
    /// score. A `next_seq` that is missing or behind the history is moved past it.
    pub fn from_json(text: &str) -> Result<Project, ProjectError> {
        let value: serde_json::Value = serde_json::from_str(text)?;
        let format = value.get("format").and_then(|f| f.as_str());
        if format != Some(PROJECT_FORMAT) {
            return Err(ProjectError::Format(
                format.unwrap_or("<missing>").to_string(),
            ));
        }
        let version = value.get("version");
        if version.and_then(|v| v.as_u64()) != Some(PROJECT_VERSION as u64) {
            return Err(ProjectError::Version(
                version.map_or_else(|| "<missing>".to_string(), |v| v.to_string()),
            ));
        }
        let mut project: Project = serde_json::from_value(value)?;
        let report = crate::validate::validate(&project.score);
        if !report.is_valid() {
            return Err(ProjectError::InvalidScore(report.errors));
        }
        let max_seq = project
            .undo
            .iter()
            .chain(&project.redo)
            .map(|e| e.seq)
            .max()
            .unwrap_or(0);
        project.next_seq = project.next_seq.max(max_seq.saturating_add(1)).max(1);
        Ok(project)
    }
}
