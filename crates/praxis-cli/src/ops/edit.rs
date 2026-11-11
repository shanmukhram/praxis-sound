//! `edit`: a scoped edit of a `.praxis` project (FR-7), from an instruction (through the offline
//! rules or Claude) or an EditPlan the caller wrote (`compose_edit`). The patch is committed to
//! the project's undo history; the bars and layers outside its scope are untouched.

use crate::error::{ErrorCode, OpError};
use crate::ops::changes::changed_layers;
use crate::ops::generate::{ProviderChoice, choose_provider};
use crate::ops::render::{RenderSettings, render_score};
use crate::ops::sound::{load_project, save_project};
use crate::ops::to_values;
use praxis_planner::edit::EditIssueCode;
use praxis_planner::{Catalogue, EditError, EditOutcome, ProviderError, compose_edit, plan_edit};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditRequest {
    /// The `.praxis` project to edit (written back when the edit changes anything).
    pub project: PathBuf,
    /// The instruction, e.g. "change the last four bars to add a subtle melody". Exactly one of
    /// `prompt` and `plan`.
    #[serde(default)]
    pub prompt: Option<String>,
    /// An EditPlan written by the caller (`praxis_planner::edit_schema()`).
    #[serde(default)]
    pub plan: Option<Value>,
    #[serde(default)]
    pub provider: Option<ProviderChoice>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "default_seed")]
    pub seed: u64,
    /// Also render the edited Score to this file (and check it).
    #[serde(default)]
    pub render: Option<PathBuf>,
    #[serde(default)]
    pub render_settings: RenderSettings,
}

fn default_seed() -> u64 {
    crate::ops::generate::DEFAULT_SEED
}

/// Maps an edit failure onto a contract code: no key, a provider failure, or `patch_rejected`
/// with each issue (`code, path, message`; `path` points into the EditPlan).
pub(crate) fn edit_error(e: EditError) -> OpError {
    match e {
        EditError::Provider(ProviderError::NoApiKey) => {
            OpError::new(ErrorCode::NoApiKey, ProviderError::NoApiKey.to_string())
        }
        EditError::Provider(p) => OpError::new(ErrorCode::ProviderFailed, p.to_string()),
        EditError::Invalid { ref issues, .. } => {
            let code = if issues.iter().all(|i| i.code == EditIssueCode::Schema) {
                ErrorCode::BadJson
            } else {
                ErrorCode::PatchRejected
            };
            OpError::new(code, e.to_string()).with_issues(to_values(issues))
        }
    }
}

/// Runs the instruction (or the written plan) on `score`.
pub(crate) fn run_edit(
    score: &praxis_score::Score,
    prompt: Option<&str>,
    plan: Option<&Value>,
    provider: Option<ProviderChoice>,
    model: Option<&str>,
    seed: u64,
) -> Result<(EditOutcome, Option<String>), OpError> {
    let catalogue = Catalogue::standard();
    match (prompt, plan) {
        (Some(_), Some(_)) => Err(OpError::new(
            ErrorCode::Error,
            "give a prompt or a plan, not both",
        )),
        (None, None) => Err(OpError::new(
            ErrorCode::Error,
            "give a prompt (--prompt) or an edit plan (--plan)",
        )),
        (None, Some(plan)) => Ok((
            compose_edit(plan, score, &catalogue, seed).map_err(edit_error)?,
            None,
        )),
        (Some(prompt), None) => {
            if prompt.trim().is_empty() {
                return Err(OpError::new(ErrorCode::Error, "the instruction is empty"));
            }
            let (p, note) = choose_provider(provider, model)?;
            let out = plan_edit(score, prompt, p.as_ref(), &catalogue, seed).map_err(edit_error)?;
            Ok((out, note))
        }
    }
}

/// The planner matches a layer reference loosely (id, name, preset or a word of one), so a
/// request for layer `piano` would also reach a melody played by `upright_piano`. When a caller
/// names exact layer ids, the planner gets this view of the Score: every other layer locked
/// (locked layers are never targets). Returns the view and the layers locked for it.
pub(crate) fn only_layers(
    score: &praxis_score::Score,
    ids: &[String],
) -> (praxis_score::Score, Vec<String>) {
    let mut view = score.clone();
    let mut locked = Vec::new();
    for l in &mut view.layers {
        if !ids.contains(&l.id) && !l.locked {
            l.locked = true;
            locked.push(l.id.clone());
        }
    }
    (view, locked)
}

/// Removes the planner's "left ... alone (locked)" remark when it only names layers that
/// [`only_layers`] locked (they were never asked for, so the remark would mislead).
pub(crate) fn drop_view_lock_note(
    summary: &str,
    refused: &[String],
    view_locks: &[String],
) -> String {
    if refused.is_empty() || !refused.iter().all(|r| view_locks.contains(r)) {
        return summary.to_string();
    }
    // The planner's summaries are sentences joined by ". ", ending in ".".
    let kept: Vec<&str> = summary
        .trim_end_matches('.')
        .split(". ")
        .filter(|s| !s.starts_with("Left `"))
        .collect();
    format!("{}.", kept.join(". "))
}

/// The edit payload fields shared by `edit` and the takes (docs/contracts/cli.md).
pub(crate) fn outcome_json(before: &praxis_score::Score, out: &EditOutcome) -> Value {
    let changed = changed_layers(before, &out.score);
    json!({
        "changed": out.changed(),
        "patch": out.patch,
        "scope": out.scope,
        "changed_bars": if out.changed() { json!([out.bars.start, out.bars.end]) } else { Value::Null },
        "changed_layers": changed.keys().collect::<Vec<_>>(),
        "changed_by_layer": changed,
        "summary": out.summary,
        "refused": out.refused,
        "new_layers": out.new_layers,
        "plan": out.plan,
        "provider": out.provider_id,
        "attempts": out.attempts,
        "rounds": out.rounds,
        "repairs": out.repairs,
    })
}

/// `{project, changed, patch, scope, changed_bars, changed_layers, changed_by_layer, summary,
/// refused, new_layers, plan, provider, provider_note, attempts, rounds, repairs, can_undo,
/// render?}`. A request that changes nothing (every named layer locked) is not an error:
/// `changed` is false and `summary` says why.
pub fn edit(req: &EditRequest) -> Result<Value, OpError> {
    let mut project = load_project(&req.project)?;
    let before = project.score.clone();
    let (out, note) = run_edit(
        &before,
        req.prompt.as_deref(),
        req.plan.as_ref(),
        req.provider,
        req.model.as_deref(),
        req.seed,
    )?;
    if out.changed() {
        project.commit(out.patch.clone()).map_err(|e| {
            OpError::new(
                ErrorCode::PatchRejected,
                format!("the project rejected the patch: {e}"),
            )
        })?;
        save_project(&req.project, &project)?;
    }
    let mut v = outcome_json(&before, &out);
    v["project"] = json!(req.project.display().to_string());
    v["provider_note"] = json!(note);
    v["seed"] = json!(req.seed);
    v["can_undo"] = json!(project.can_undo());
    if let Some(path) = &req.render {
        v["render"] = render_score(&project.score, path, &req.render_settings)?;
    }
    Ok(v)
}
