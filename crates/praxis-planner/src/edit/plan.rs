//! The EditPlan: the tool input for a scoped edit (FR-7). A provider (or an agent through
//! [`crate::edit::compose_edit`]) fills it; the planner resolves the scope phrase and the layer
//! references against the Score and builds an exactly scoped `ScorePatch`.
//!
//! See the "Edits" section of docs/contracts/planner.md.

use crate::plan::{Dynamics, Register};
use praxis_score::{BarRange, GeneratorClass, Grid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Name of the tool the provider must call for an edit.
pub const EDIT_TOOL_NAME: &str = "edit_score";

/// One scoped edit: what to change (`intent`), where (`scope`), on which layers (`layers`), and
/// how (`params`).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EditPlan {
    pub intent: EditIntent,
    /// Bars the edit may change. Omit (or give the phrase "whole") for the whole score.
    #[serde(default)]
    pub scope: EditScope,
    /// Existing layers the edit changes, by id, name, preset or role ("piano", "kick",
    /// "melody"). Leave empty for `add_layer` and `add_melody` (they create a new layer). For
    /// `rewrite_motif`, `change_dynamics` and `thin_out`, empty means every unlocked layer that
    /// plays in the scope. Locked layers are never changed.
    #[serde(default)]
    pub layers: Vec<String>,
    #[serde(default)]
    pub params: EditParams,
    /// Remarks (assumptions, instruments the catalogue lacks).
    #[serde(default)]
    pub notes: Vec<String>,
}

/// Where an edit applies. `bars` wins over `phrase`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct EditScope {
    /// Explicit 1-based inclusive bar range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bars: Option<BarRange>,
    /// A bar phrase the planner resolves against the Score: "last four bars", "first 8 bars",
    /// "first half", "second half", "bars 9-12", "bar 5", "from bar 9", "until bar 8", "whole".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phrase: Option<String>,
    /// Bars the instruction says to keep, as a phrase ("first half"). The edit never touches
    /// them; with no `bars` or `phrase` the scope is the bars outside `keep`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<String>,
}

/// What the edit does.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EditIntent {
    /// A new layer (instrument in `params.preset` or `params.class`) playing in the scope.
    AddLayer,
    /// A new melody layer (default: a soft upright piano in the high register that follows the
    /// chords) playing in the scope.
    AddMelody,
    /// New motif material for the target layers in the scope (same instrument and role).
    RewriteMotif,
    /// The target layers play `params.preset` instead (in the scope only, when bars are given).
    ChangeInstrument,
    /// The target layers get softer or louder (`params.change`, `params.amount_db`).
    ChangeDynamics,
    /// The target layers stop playing in the scope (or are removed when the scope is whole).
    RemoveLayer,
    /// The target layers keep only their accented notes and the beats in the scope.
    ThinOut,
}

/// Softer or louder.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DynamicsChange {
    Softer,
    Louder,
}

/// How an edit is done. Every field is optional; the planner fills sensible defaults.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct EditParams {
    /// Catalogue preset for `add_layer`, `add_melody` and `change_instrument`, e.g. "bell".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Role of a new layer when no preset is named (the catalogue default for the class).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<GeneratorClass>,
    /// Display name of a new layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Register of new material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register: Option<Register>,
    /// Dynamics of new material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamics: Option<Dynamics>,
    /// Gain of a new layer in dB (-60..=24).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain_db: Option<f32>,
    /// `change_dynamics`: softer or louder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<DynamicsChange>,
    /// `change_dynamics`: size of the change in dB (positive; default 4 softer, 3 louder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_db: Option<f32>,
    /// Grid of new material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<Grid>,
    /// Spoken rhythm of new material (DON strong, din soft, don normal, TIN bright, - rest,
    /// ~ tie), one token per grid step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spoken: Option<String>,
    /// Scale degrees of new material, relative to each bar's chord root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub degrees: Vec<i32>,
    /// Motif length of new material in bars (1..=8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_bars: Option<u32>,
}

/// JSON Schema of [`EditPlan`] (the `edit_score` tool's input schema).
pub fn edit_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(EditPlan)).expect("schema serialises")
}

/// Why an edit plan could not be turned into a patch. Messages are written so they can go back
/// to a model verbatim.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EditIssueCode {
    /// The tool input does not match the schema.
    Schema,
    /// The scope phrase or bars cannot be resolved against the Score.
    Scope,
    /// The scope overlaps the bars the instruction says to keep.
    Keep,
    /// A layer reference names no layer of the Score.
    UnknownLayer,
    /// The edit names no layer it could change (or none plays in the scope).
    NoTarget,
    /// The preset is not in the catalogue, or does not fit the layer.
    UnknownPreset,
    /// The new material's class is on the Score's avoid-list.
    AvoidedClass,
    /// The edit's material could not be built.
    Material,
    /// `praxis_score::apply::apply` rejected the patch.
    Rejected,
    /// A repaired plan reached outside the scope of the first one.
    Widened,
}

/// One problem with an edit plan.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EditIssue {
    pub code: EditIssueCode,
    /// JSON pointer into the edit plan.
    pub path: String,
    pub message: String,
}

impl EditIssue {
    pub fn new(code: EditIssueCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        EditIssue {
            code,
            path: path.into(),
            message: message.into(),
        }
    }
}

/// Issues as a bullet list for a repair prompt or an error message.
pub fn edit_issues_text(issues: &[EditIssue]) -> String {
    issues
        .iter()
        .map(|i| {
            let code = serde_json::to_value(i.code)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            format!("- [{code}] {}: {}", i.path, i.message)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
