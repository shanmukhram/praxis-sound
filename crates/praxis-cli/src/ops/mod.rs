//! One function per command. Each takes a typed request and returns the command's JSON payload
//! (without the `ok`/`command` envelope, which [`crate::envelope`] adds).
//!
//! Requests derive `Deserialize` with defaults for every optional field, so an MCP tool call's
//! JSON arguments map onto them directly. Paths in requests are used as given (relative paths
//! resolve against the process's current directory).

use crate::error::{ErrorCode, OpError, io_error};
use praxis_score::Score;
use praxis_score::project::Project;
use serde_json::{Value, json};
use std::path::Path;

pub mod analyze;
pub mod bench;
pub mod changes;
pub mod check;
pub mod crash;
pub mod edit;
pub mod generate;
pub mod history;
pub mod key;
pub mod library;
pub mod r#match;
pub mod midi;
pub mod packs;
pub mod presets;
pub mod render;
pub mod schema;
pub mod sound;
pub mod takes;
pub mod validate;
pub mod variations;

pub use analyze::{AnalyzeRequest, analyze};
pub use bench::{BenchRequest, bench};
pub use check::{CheckRequest, check};
pub use edit::{EditRequest, edit};
pub use generate::{GenerateRequest, ProviderChoice, generate};
pub use history::{HistoryAction, UndoRedoRequest, undo_redo};
pub use key::{KeyAction, KeyRequest, key};
pub use library::{LibraryAction, LibraryRequest, library};
pub use r#match::{MatchClipRequest, match_clip};
pub use midi::{ExportMidiRequest, export_midi};
pub use packs::{PacksInstallRequest, packs_install, packs_list};
pub use presets::presets;
pub use render::{RenderRequest, RenderSettings, render};
pub use schema::{SchemaRequest, schema};
pub use sound::{TakeRecord, TakeScope, ViewName, praxis_home, resolve_sound};
pub use takes::{
    AskOptions, AskRequest, KeepVariationRequest, OpenViewRequest, ReferenceArg, RestoreRequest,
    TakesRequest, ask, ask_with, keep_variation, open_view, restore, takes,
};
pub use validate::{ValidateRequest, validate};
pub use variations::{VariationsRequest, variations};

/// Reads a score from a `.score.json` file or a `.praxis` project (detected by the project's
/// `"format": "praxis-project"` field, not by extension) and validates it.
pub fn load_score(path: &Path) -> Result<Score, OpError> {
    let text = std::fs::read_to_string(path).map_err(|e| io_error(path, e))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| OpError::new(ErrorCode::BadJson, format!("{}: {e}", path.display())))?;
    let score_value = if value.get("format").and_then(Value::as_str) == Some("praxis-project") {
        let project: Project = serde_json::from_value(value)
            .map_err(|e| OpError::new(ErrorCode::BadScore, format!("{}: {e}", path.display())))?;
        serde_json::to_value(project.score).unwrap_or(Value::Null)
    } else {
        value
    };
    let score: Score = serde_json::from_value(score_value)
        .map_err(|e| OpError::new(ErrorCode::BadScore, format!("{}: {e}", path.display())))?;
    let report = praxis_score::validate::validate(&score);
    if !report.is_valid() {
        let issues = to_values(&report.errors);
        return Err(OpError::new(
            ErrorCode::InvalidScore,
            serde_json::to_string(&report.errors).unwrap_or_default(),
        )
        .with_issues(issues));
    }
    Ok(score)
}

/// Writes a new `.praxis` project holding `score` (no history yet). Creates parent directories.
pub fn write_project(path: &Path, score: &Score) -> Result<(), OpError> {
    let project = Project::new(score.clone());
    let text = project
        .to_json()
        .map_err(|e| OpError::new(ErrorCode::Error, e.to_string()))?;
    ensure_parent(path)?;
    std::fs::write(path, text + "\n").map_err(|e| io_error(path, e))
}

/// A short description of a Score: what an agent needs to see without reading the whole file.
pub fn score_summary(score: &Score) -> Value {
    json!({
        "title": score.title,
        "key": score.key.display(),
        "bpm": score.tempo.bpm,
        "bars": score.bars,
        "time_signature": format!("{}/{}", score.time_signature.numerator, score.time_signature.denominator),
        "playback": score.playback,
        "duration_s": praxis_score::timeline::TimeMap::new(score, 48_000).bar_s() * score.bars as f64,
        "layers": score.layers.iter().map(|l| json!({
            "id": l.id,
            "name": l.name,
            "class": l.class,
            "kind": l.instrument.kind,
            "preset": l.instrument.preset,
            "muted": l.muted,
            "bars": l.clips.iter().map(|c| [c.bars.start, c.bars.end]).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "sections": score.sections.iter().map(|s| json!({
            "id": s.id,
            "name": s.name,
            "role": s.role,
            "bars": [s.bars.start, s.bars.end],
        })).collect::<Vec<_>>(),
        "avoid": score.constraints.avoid,
    })
}

pub(crate) fn ensure_parent(path: &Path) -> Result<(), OpError> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
    }
    Ok(())
}

pub(crate) fn to_values<T: serde::Serialize>(items: &[T]) -> Vec<Value> {
    items
        .iter()
        .map(|i| serde_json::to_value(i).unwrap_or(Value::Null))
        .collect()
}

/// A finite number, or `null` (JSON has no infinity or NaN).
pub(crate) fn finite(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}
