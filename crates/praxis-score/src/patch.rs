//! Scoped patches. FROZEN CONTRACT for the types; `apply` is implemented by work package
//! WP-A (score-ops). See docs/contracts/patch.md for the rules every implementation must keep.

use crate::types::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Who produced a patch. Only `User` may unlock a layer.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    #[default]
    User,
    Model,
    System,
}

/// What a patch may touch. Ops outside the scope are rejected, not clipped.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    /// Bars the patch may change. `None` means the whole score (global ops are allowed only then).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bars: Option<BarRange>,
    /// Layers the patch may change. Empty means any unlocked layer.
    #[serde(default)]
    pub layers: Vec<Id>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ScorePatch {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub origin: Origin,
    #[serde(default)]
    pub scope: Scope,
    pub ops: Vec<PatchOp>,
}

/// Partial update of a layer's properties. Absent fields are unchanged.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct LayerUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<GeneratorClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<InstrumentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solo: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<Vec<Effect>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation: Option<Vec<Automation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_offset: Option<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PatchOp {
    // ---- global ops: only valid when scope.bars is None ----
    SetTitle {
        title: String,
    },
    SetMeta {
        meta: Meta,
    },
    SetTempo {
        bpm: f64,
    },
    /// Changes the key. With `transpose_notes`, every pitched pattern moves by the
    /// smallest interval between the old and new tonic.
    SetKey {
        key: Key,
        transpose_notes: bool,
    },
    SetTimeSignature {
        time_signature: TimeSignature,
    },
    SetBars {
        bars: u32,
    },
    SetPlayback {
        playback: Playback,
    },
    SetConstraints {
        constraints: Constraints,
    },
    SetMaster {
        master: Master,
    },
    SetChords {
        chords: Vec<ChordSpan>,
    },
    AddSection {
        section: Section,
    },
    UpdateSection {
        section: Section,
    },
    RemoveSection {
        id: Id,
    },
    AddMarker {
        marker: Marker,
    },
    RemoveMarker {
        id: Id,
    },
    // ---- pattern ops ----
    /// Adds a new pattern. Fails if the id exists.
    AddPattern {
        pattern: Pattern,
    },
    /// Replaces a pattern used ONLY by in-scope clips; otherwise fails (use write_bars).
    ReplacePattern {
        pattern: Pattern,
    },
    /// Removes an unused pattern.
    RemovePattern {
        id: Id,
    },
    // ---- layer ops ----
    AddLayer {
        layer: Layer,
    },
    RemoveLayer {
        id: Id,
    },
    UpdateLayer {
        id: Id,
        update: LayerUpdate,
    },
    /// Lock or unlock a layer. Unlock requires origin = user.
    SetLock {
        id: Id,
        locked: bool,
    },
    /// Copy-on-write bar edit: the layer plays `pattern` (looped from bars.start) in `bars`,
    /// splitting existing clips at the range edges. The pattern is added under its own id.
    WriteBars {
        layer: Id,
        bars: BarRange,
        pattern: Pattern,
    },
    /// Silences `bars` on a layer, splitting clips at the edges.
    ClearBars {
        layer: Id,
        bars: BarRange,
    },
    /// Places an existing pattern in `bars`, splitting clips at the edges.
    PlaceClip {
        layer: Id,
        clip: Clip,
    },
    /// Replaces a layer's whole clip list. Allowed only when every bar outside the scope expands
    /// to the same note events before and after (apply checks this). Used for exact inverses.
    SetClips {
        layer: Id,
        clips: Vec<Clip>,
    },
}

/// Why a patch was rejected.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PatchError {
    #[error("layer `{0}` is locked")]
    LockedLayer(Id),
    #[error("op {index} ({op}) is outside the patch scope: {reason}")]
    OutOfScope {
        index: usize,
        op: String,
        reason: String,
    },
    #[error("op {index} ({op}) references unknown {what} `{id}`")]
    UnknownId {
        index: usize,
        op: String,
        what: String,
        id: Id,
    },
    #[error("op {index} ({op}) would duplicate {what} `{id}`")]
    DuplicateId {
        index: usize,
        op: String,
        what: String,
        id: Id,
    },
    #[error("only a user may unlock layer `{0}`")]
    UnlockNotAllowed(Id),
    #[error("the patched score is invalid: {0}")]
    Invalid(String),
}

/// Result of applying a patch: the new score and a patch that undoes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Applied {
    pub score: Score,
    pub inverse: ScorePatch,
}

impl PatchOp {
    pub fn name(&self) -> &'static str {
        match self {
            PatchOp::SetTitle { .. } => "set_title",
            PatchOp::SetMeta { .. } => "set_meta",
            PatchOp::SetTempo { .. } => "set_tempo",
            PatchOp::SetKey { .. } => "set_key",
            PatchOp::SetTimeSignature { .. } => "set_time_signature",
            PatchOp::SetBars { .. } => "set_bars",
            PatchOp::SetPlayback { .. } => "set_playback",
            PatchOp::SetConstraints { .. } => "set_constraints",
            PatchOp::SetMaster { .. } => "set_master",
            PatchOp::SetChords { .. } => "set_chords",
            PatchOp::AddSection { .. } => "add_section",
            PatchOp::UpdateSection { .. } => "update_section",
            PatchOp::RemoveSection { .. } => "remove_section",
            PatchOp::AddMarker { .. } => "add_marker",
            PatchOp::RemoveMarker { .. } => "remove_marker",
            PatchOp::AddPattern { .. } => "add_pattern",
            PatchOp::ReplacePattern { .. } => "replace_pattern",
            PatchOp::RemovePattern { .. } => "remove_pattern",
            PatchOp::AddLayer { .. } => "add_layer",
            PatchOp::RemoveLayer { .. } => "remove_layer",
            PatchOp::UpdateLayer { .. } => "update_layer",
            PatchOp::SetLock { .. } => "set_lock",
            PatchOp::WriteBars { .. } => "write_bars",
            PatchOp::ClearBars { .. } => "clear_bars",
            PatchOp::PlaceClip { .. } => "place_clip",
            PatchOp::SetClips { .. } => "set_clips",
        }
    }

    /// True for ops that affect the whole score and need an unscoped patch.
    pub fn is_global(&self) -> bool {
        matches!(
            self,
            PatchOp::SetTitle { .. }
                | PatchOp::SetMeta { .. }
                | PatchOp::SetTempo { .. }
                | PatchOp::SetKey { .. }
                | PatchOp::SetTimeSignature { .. }
                | PatchOp::SetBars { .. }
                | PatchOp::SetPlayback { .. }
                | PatchOp::SetConstraints { .. }
                | PatchOp::SetMaster { .. }
        )
    }
}
