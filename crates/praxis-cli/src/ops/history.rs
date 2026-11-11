//! `undo_redo`: one step back or forward in a sound's project history (a take, a go-back or a
//! plain edit), the same step the desktop app's undo and redo buttons take. When the step lands
//! on a take's score, that take becomes current and `takes.json` gets a new revision.

use crate::error::{ErrorCode, OpError};
use crate::ops::score_summary;
use crate::ops::sound::{Sound, SoundLock, resolve_sound, save_project};
use praxis_score::project::Project;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAction {
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UndoRedoRequest {
    pub sound: String,
    pub action: HistoryAction,
}

/// The take whose score the live score equals: the current one when it still matches, else the
/// newest match.
fn matching_take(s: &Sound, project: &Project) -> Option<u32> {
    let is = |n: u32| {
        s.takes
            .take(n)
            .and_then(|t| project.version(&t.version))
            .is_some_and(|v| v.score == project.score)
    };
    if is(s.takes.current) {
        return Some(s.takes.current);
    }
    s.takes.takes.iter().rev().map(|t| t.n).find(|&n| is(n))
}

/// `{sound, moved, description, revision, current, score_summary, can_undo, can_redo}`.
/// `moved` is false when there was nothing to undo or redo.
pub fn undo_redo(req: &UndoRedoRequest) -> Result<Value, OpError> {
    let dir = resolve_sound(&req.sound)?;
    let _lock = SoundLock::acquire(&dir)?;
    let mut s = Sound::open(&dir)?;
    let Some(mut project) = s.project.take() else {
        return Err(OpError::new(
            ErrorCode::Error,
            format!("{} has no takes yet", dir.display()),
        ));
    };
    let reject =
        |e: praxis_score::patch::PatchError| OpError::new(ErrorCode::PatchRejected, e.to_string());
    let step = match req.action {
        HistoryAction::Undo => project.undo().map_err(reject)?.cloned(),
        HistoryAction::Redo => project.redo().map_err(reject)?.cloned(),
    };
    let description = step.as_ref().map(|e| e.patch.description.clone());
    if step.is_some() {
        let current = matching_take(&s, &project).unwrap_or(s.takes.current);
        if current == s.takes.current {
            save_project(&s.project_path(), &project)?;
            s.project = Some(project);
        } else {
            s.takes.current = current;
            s.project = Some(project);
            s.save()?;
        }
    } else {
        s.project = Some(project);
    }
    let project = s.project.as_ref().expect("put back above");
    Ok(json!({
        "sound": dir.display().to_string(),
        "moved": step.is_some(),
        "description": description,
        "revision": s.takes.revision,
        "current": s.takes.current,
        "score_summary": score_summary(&project.score),
        "can_undo": project.can_undo(),
        "can_redo": project.can_redo(),
    }))
}
