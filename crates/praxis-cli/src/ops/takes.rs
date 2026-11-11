//! The shared take history (ADR-0011): `ask`, `takes`, `restore`, `keep_variation` and
//! `open_view` over a sound folder (see [`crate::ops::sound`] for the folder layout).
//!
//! - `ask` on an empty sound plans the brief and makes take 1. After its first render the
//!   applied master gain is stored in `master.frozen_gain_db` through an undoable `system`
//!   patch, so every later take renders with the same gain and a scoped edit leaves the
//!   untouched bars of the master bit-identical (ADR-0007, contracts/patch.md).
//! - `ask` on a sound with takes is a scoped edit of the current take. With a `scope`
//!   (`{layer, bars}`) the edit is held to exactly those bars, on that layer: the planner's
//!   intent is kept, its bars and target layers are replaced by the scope, and a result that
//!   changes another existing layer or other bars is refused. Adding a new layer inside the bars
//!   is allowed (it changes no existing layer). With a `patch` instead of planning, the patch is
//!   committed as given (a hand edit from a panel view).
//! - Every take is rendered to `takes/<n>/mix.wav`, checked, saved as project version
//!   `take-<n>`, and recorded with what changed and which bars stayed bit-identical (measured
//!   on the two renders).

use crate::error::{ErrorCode, OpError};
use crate::ops::changes::{bars_label, changed_layers, identical_bars, untouched_text};
use crate::ops::check::decode;
use crate::ops::edit::{drop_view_lock_note, edit_error, only_layers, outcome_json};
use crate::ops::generate::{ProviderChoice, choose_provider, plan_error_of};
use crate::ops::render::{RenderSettings, render_score_master};
use crate::ops::score_summary;
use crate::ops::sound::{
    Sound, SoundLock, TakeRecord, TakeRender, TakeScope, ViewName, ViewRequest, agent_name,
    normalize_who, resolve_sound,
};
use crate::ops::variations::{load_variations, variations_dir};
use praxis_planner::edit::{DynamicsChange, EditIntent, EditPlan, EditScope, edit_request};
use praxis_planner::{Catalogue, EditOutcome, compose_edit, plan_brief, plan_edit};
use praxis_score::patch::{Origin, PatchOp, Scope, ScorePatch};
use praxis_score::{BarRange, Score};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Who a take is from when the request does not say (the CLI's default).
pub const DEFAULT_WHO: &str = "You";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AskRequest {
    /// The sound folder, or a bare name under PRAXIS_HOME/sounds.
    pub sound: String,
    /// The request in words: a brief for a new sound, an edit instruction otherwise.
    pub text: String,
    /// `"You"` or an agent's name (stored as `"Agent · <name>"`). Default `"You"`.
    #[serde(default)]
    pub who: Option<String>,
    /// Bars of one layer: the edit changes nothing outside them.
    #[serde(default)]
    pub scope: Option<TakeScope>,
    /// A hand edit: commit this patch instead of planning one from `text`.
    #[serde(default)]
    pub patch: Option<ScorePatch>,
    /// Overrides the take's title.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub provider: Option<ProviderChoice>,
    #[serde(default)]
    pub model: Option<String>,
    /// Planner and arranger seed. Default: the new take's number.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Render only layers whose instrument is available here (the rest are listed).
    #[serde(default)]
    pub skip_missing: bool,
    /// Also return the take's score under `take.score`.
    #[serde(default)]
    pub include_score: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TakesRequest {
    pub sound: String,
    /// Add each take's score (`take.score`, from its project version).
    #[serde(default)]
    pub include_scores: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RestoreRequest {
    pub sound: String,
    /// The take to go back to.
    pub n: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeepVariationRequest {
    pub sound: String,
    /// 1-based, as `variations` numbered it (`variations/<index>.wav`).
    pub index: usize,
    #[serde(default)]
    pub who: Option<String>,
    /// The request as it reads in the thread. Default "Keep variation <index>".
    #[serde(default)]
    pub ask: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub skip_missing: bool,
    #[serde(default)]
    pub include_score: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenViewRequest {
    pub sound: String,
    pub view: ViewName,
    /// Who asks for it: `"You"` (shown as opened by Praxis) or an agent's name.
    #[serde(default)]
    pub who: Option<String>,
}

/// The words of a take.
struct Words {
    who: String,
    ask: String,
    title: String,
    say: String,
    scope: Option<TakeScope>,
    provider: String,
    seed: u64,
}

fn render_settings(skip_missing: bool) -> RenderSettings {
    RenderSettings {
        skip_missing,
        ..RenderSettings::default()
    }
}

/// The TypeScript `RenderReport` from a render payload.
fn render_report(r: &Value) -> Value {
    json!({
        "mock": false,
        "sample_rate": r["sample_rate"],
        "length_samples": r["length_samples"],
        "tail_samples": r["tail_samples"],
        "is_loop": r["is_loop"],
        "loudness_before_lufs": r["loudness_before_lufs"],
        "loudness_lufs": r["loudness_lufs"],
        "true_peak_dbtp": r["true_peak_dbtp"],
        "applied_gain_db": r["applied_gain_db"],
        "warmup_cycles": r["warmup_cycles"],
        "notes_rendered": r["notes_rendered"],
        "blocked_layers": r["blocked_layers"],
        "render_seconds": r["render_seconds"],
        "warnings": r["warnings"],
        "skipped_layers": r["skipped_layers"],
        "format": r["format"],
        "frames": r["frames"],
        "duration_s": r["duration_s"],
    })
}

fn project_error(e: impl std::fmt::Display) -> OpError {
    OpError::new(ErrorCode::PatchRejected, e.to_string())
}

/// Renders the sound's current score as take `n`, freezes the master gain after a new sound's
/// first render, saves version `take-<n>`, records the take and makes it current. The caller
/// has already committed `patch` (when there is one) to the project.
fn make_take(
    sound: &mut Sound,
    before: Option<&Score>,
    patch: Option<ScorePatch>,
    words: Words,
    settings: &RenderSettings,
) -> Result<TakeRecord, OpError> {
    let n = sound.takes.next_n();
    let prev = sound.takes.current;
    let path = sound.mix_path(n);
    let score = sound
        .project
        .as_ref()
        .map(|p| p.score.clone())
        .ok_or_else(|| OpError::new(ErrorCode::Error, "the sound has no project"))?;
    let (render, _) = render_score_master(&score, &path, settings)?;
    let project = sound.project.as_mut().expect("checked above");
    if before.is_none() && score.master.frozen_gain_db.is_none() {
        let gain = render["applied_gain_db"].as_f64().unwrap_or(0.0);
        let mut master = score.master.clone();
        master.frozen_gain_db = Some(gain);
        project
            .commit(ScorePatch {
                description: format!(
                    "freeze the master gain at {gain:.2} dB after the first render, so later \
                     edits leave untouched bars sample-identical"
                ),
                origin: Origin::System,
                scope: Scope::default(),
                ops: vec![PatchOp::SetMaster { master }],
            })
            .map_err(project_error)?;
    }
    let after = project.score.clone();
    let version = format!("take-{n}");
    if project.version(&version).is_some() {
        project.delete_version(&version).map_err(project_error)?;
    }
    project.save_version(&version).map_err(project_error)?;

    let (changed, identical, untouched) = match (before, sound.takes.take(prev)) {
        (Some(before), Some(prev_take)) => {
            let changed = changed_layers(before, &after);
            let a = decode(&sound.absolute(&prev_take.render.path))?;
            let b = decode(&path)?;
            let identical = if a.sample_rate == b.sample_rate {
                identical_bars(&after, &a.audio, &b.audio, b.sample_rate)
            } else {
                Vec::new()
            };
            let text = untouched_text(before, &after, &changed, &identical, prev);
            (changed, identical, text)
        }
        _ => (Default::default(), Vec::new(), String::new()),
    };
    let record = TakeRecord {
        n,
        who: words.who,
        ask: words.ask,
        title: words.title,
        say: words.say,
        patch,
        changed,
        untouched,
        render: TakeRender {
            path: Sound::mix_rel(n),
            report: render_report(&render),
        },
        checks: render["check"].clone(),
        opens: None,
        scope: words.scope,
        version,
        from: if before.is_some() { prev } else { 0 },
        identical_bars: identical,
        provider: words.provider,
        seed: words.seed,
    };
    sound.takes.takes.push(record.clone());
    sound.takes.current = n;
    sound.save()?;
    Ok(record)
}

/// The payload of a command that made or selected a take.
fn take_payload(sound: &Sound, take: &TakeRecord, include_score: bool) -> Value {
    let project = sound.project.as_ref();
    json!({
        "sound": sound.dir.display().to_string(),
        "project": sound.project_path().display().to_string(),
        "revision": sound.takes.revision,
        "current": sound.takes.current,
        "take": sound.take_json(take, include_score),
        "score_summary": project.map(|p| score_summary(&p.score)),
        "can_undo": project.is_some_and(|p| p.can_undo()),
        "can_redo": project.is_some_and(|p| p.can_redo()),
    })
}

fn title_of(plan: &EditPlan, bars: Option<BarRange>) -> String {
    let word = match plan.intent {
        EditIntent::AddLayer => "New layer",
        EditIntent::AddMelody => "New melody",
        EditIntent::RewriteMotif => "New motif",
        EditIntent::ChangeInstrument => "New instrument",
        EditIntent::ChangeDynamics => match plan.params.change {
            Some(DynamicsChange::Softer) => "Softer",
            Some(DynamicsChange::Louder) => "Louder",
            None => "Dynamics",
        },
        EditIntent::RemoveLayer => "Layer removed",
        EditIntent::ThinOut => "Thinned out",
        EditIntent::ChangeTone => "New tone",
        EditIntent::ChangeSpace => "New space",
        EditIntent::NotUnderstood => "No change",
    };
    match bars {
        Some(b) => format!("{word}, {}", bars_label([b.start, b.end])),
        None => word.to_string(),
    }
}

fn scope_label(score: &Score, scope: &TakeScope) -> String {
    let name = score
        .layer(&scope.layer)
        .map(|l| {
            if l.name.is_empty() {
                l.id.clone()
            } else {
                l.name.clone()
            }
        })
        .unwrap_or_else(|| scope.layer.clone());
    format!(
        "{}, {}",
        bars_label([scope.bars.start, scope.bars.end]),
        name
    )
}

/// A scoped edit held exactly to `scope`: the planner reads the instruction (with the bars and
/// layer named), then its plan's bars and target layers are replaced by the scope.
fn scoped_edit(
    score: &Score,
    text: &str,
    scope: &TakeScope,
    provider: Option<ProviderChoice>,
    model: Option<&str>,
    seed: u64,
) -> Result<(EditOutcome, Option<String>), OpError> {
    let catalogue = Catalogue::standard();
    if score.layer(&scope.layer).is_none() {
        return Err(OpError::new(
            ErrorCode::PatchRejected,
            format!(
                "the scope names layer `{}`, which is not in the sound",
                scope.layer
            ),
        ));
    }
    let b = scope.bars;
    if b.start < 1 || b.end < b.start || b.end > score.bars {
        return Err(OpError::new(
            ErrorCode::PatchRejected,
            format!(
                "the scope's bars {}-{} are outside the sound's bars 1-{}",
                b.start, b.end, score.bars
            ),
        ));
    }
    let (p, note) = choose_provider(provider, model)?;
    // "bars 9-12 of `piano`: ..." reads as the scope for the offline rules and for a model; the
    // word "layer" is avoided because the rules read it as a request to add one.
    let instruction = format!("bars {}-{} of `{}`: {text}", b.start, b.end, scope.layer);
    // The planner sees only the scope's layer as editable (see `only_layers`).
    let (view, view_locks) = only_layers(score, std::slice::from_ref(&scope.layer));
    let req = edit_request(&instruction, &view, &catalogue, seed);
    let value = p
        .call_tool(&req)
        .map_err(|e| edit_error(praxis_planner::EditError::Provider(e)))?;
    let mut plan: EditPlan = serde_json::from_value(value).map_err(|e| {
        OpError::new(
            ErrorCode::ProviderFailed,
            format!("the planner's edit plan does not match the schema: {e}"),
        )
    })?;
    plan.scope = EditScope {
        bars: Some(b),
        phrase: None,
        keep: None,
    };
    plan.layers = match plan.intent {
        EditIntent::AddLayer | EditIntent::AddMelody => Vec::new(),
        _ => vec![scope.layer.clone()],
    };
    let plan_json = serde_json::to_value(&plan).unwrap_or(Value::Null);
    let mut out = compose_edit(&plan_json, &view, &catalogue, seed).map_err(edit_error)?;
    out.provider_id = p.id();
    // The same patch on the real Score (where the other layers are not locked).
    let applied = praxis_score::apply::apply(score, &out.patch).map_err(project_error)?;
    out.score = applied.score;
    out.inverse = applied.inverse;
    out.summary = drop_view_lock_note(&out.summary, &out.refused, &view_locks);
    out.refused.retain(|r| !view_locks.contains(r));
    // Nothing wider than the scope: no other existing layer, no other bars.
    for (layer, r) in changed_layers(score, &out.score) {
        let existing = score.layer(&layer).is_some();
        if (existing && layer != scope.layer) || r[0] < b.start || r[1] > b.end {
            return Err(OpError::new(
                ErrorCode::PatchRejected,
                format!(
                    "the edit would change {} of layer `{layer}`, outside the scope ({}); \
                     nothing was changed",
                    bars_label(r),
                    scope_label(score, scope)
                ),
            ));
        }
    }
    Ok((out, note))
}

/// `{sound, project, revision, current, take, score_summary, can_undo, can_redo, provider_note,
/// edit?}`: a new sound (take 1) or a scoped edit of the current take, rendered, checked and
/// recorded. An edit that changes nothing fails with `patch_rejected` and makes no take.
pub fn ask(req: &AskRequest) -> Result<Value, OpError> {
    let dir = resolve_sound(&req.sound)?;
    let _lock = SoundLock::acquire(&dir)?;
    let mut sound = Sound::open(&dir)?;
    let text = req.text.trim().to_string();
    let who = normalize_who(req.who.as_deref(), DEFAULT_WHO);
    let settings = render_settings(req.skip_missing);
    let n = sound.takes.next_n();
    let seed = req.seed.unwrap_or(n as u64);

    if sound.takes.takes.is_empty() {
        if sound.project.is_some() {
            return Err(OpError::new(
                ErrorCode::Error,
                format!(
                    "{} holds a project.praxis without takes.json; use a new folder for a new sound",
                    dir.display()
                ),
            ));
        }
        if req.patch.is_some() || req.scope.is_some() {
            return Err(OpError::new(
                ErrorCode::Error,
                "this sound has no takes yet: describe the sound first (no patch or scope)",
            ));
        }
        if text.is_empty() {
            return Err(OpError::new(
                ErrorCode::Error,
                "describe a sound to start from",
            ));
        }
        let (provider, note) = choose_provider(req.provider, req.model.as_deref())?;
        let outcome = plan_brief(&text, provider.as_ref(), &Catalogue::standard(), seed)
            .map_err(plan_error_of)?;
        let title = req.title.clone().unwrap_or_else(|| {
            if outcome.score.title.trim().is_empty() {
                "New sound".to_string()
            } else {
                outcome.score.title.clone()
            }
        });
        sound.project = Some(praxis_score::project::Project::new(outcome.score.clone()));
        let words = Words {
            who,
            ask: text,
            title,
            say: outcome.summary(),
            scope: None,
            provider: outcome.provider_id.clone(),
            seed,
        };
        let take = make_take(&mut sound, None, None, words, &settings)?;
        let mut v = take_payload(&sound, &take, req.include_score);
        v["provider_note"] = json!(note);
        v["repairs"] = json!(outcome.repairs);
        return Ok(v);
    }

    let project = sound
        .project
        .as_mut()
        .expect("Sound::open checks takes need a project");
    let before = project.score.clone();
    let ask_text = match &req.scope {
        Some(s) => format!("{text} ({})", scope_label(&before, s)),
        None => text.clone(),
    };
    let (patch, words, note, edit) = if let Some(patch) = &req.patch {
        project.commit(patch.clone()).map_err(project_error)?;
        let bars = patch.scope.bars.or(req.scope.as_ref().map(|s| s.bars));
        let title = req.title.clone().unwrap_or_else(|| match bars {
            Some(b) => format!("Hand edit, {}", bars_label([b.start, b.end])),
            None => "Hand edit".into(),
        });
        let say = if patch.description.trim().is_empty() {
            "Changed by hand.".to_string()
        } else {
            format!(
                "Changed by hand: {}.",
                patch.description.trim_end_matches('.')
            )
        };
        let words = Words {
            who,
            ask: ask_text,
            title,
            say,
            scope: req.scope.clone(),
            provider: "hand".into(),
            seed,
        };
        (patch.clone(), words, None, Value::Null)
    } else {
        if text.is_empty() {
            return Err(OpError::new(ErrorCode::Error, "say what to change"));
        }
        let (out, note) = match &req.scope {
            Some(scope) => scoped_edit(
                &before,
                &text,
                scope,
                req.provider,
                req.model.as_deref(),
                seed,
            )?,
            None => {
                let (p, note) = choose_provider(req.provider, req.model.as_deref())?;
                let out = plan_edit(&before, &text, p.as_ref(), &Catalogue::standard(), seed)
                    .map_err(edit_error)?;
                (out, note)
            }
        };
        if !out.changed() {
            return Err(OpError::new(
                ErrorCode::PatchRejected,
                format!("no take was made: {}", out.summary),
            ));
        }
        project.commit(out.patch.clone()).map_err(project_error)?;
        let title = req
            .title
            .clone()
            .unwrap_or_else(|| title_of(&out.plan, out.patch.scope.bars));
        let words = Words {
            who,
            ask: ask_text,
            title,
            say: out.summary.clone(),
            scope: req.scope.clone(),
            provider: out.provider_id.clone(),
            seed,
        };
        let edit = outcome_json(&before, &out);
        (out.patch.clone(), words, note, edit)
    };
    let take = make_take(&mut sound, Some(&before), Some(patch), words, &settings)?;
    let mut v = take_payload(&sound, &take, req.include_score);
    v["provider_note"] = json!(note);
    if !edit.is_null() {
        v["edit"] = json!({
            "refused": edit["refused"],
            "new_layers": edit["new_layers"],
            "plan": edit["plan"],
            "changed_bars": edit["changed_bars"],
            "attempts": edit["attempts"],
            "repairs": edit["repairs"],
        });
    }
    Ok(v)
}

/// `{sound, project, revision, current, takes[], score, can_undo, can_redo, views[]}`. A folder
/// with no takes yet gives an empty list, `current` 0 and `score` null.
pub fn takes(req: &TakesRequest) -> Result<Value, OpError> {
    let dir = resolve_sound(&req.sound)?;
    let sound = Sound::open(&dir)?;
    let project = sound.project.as_ref();
    Ok(json!({
        "sound": dir.display().to_string(),
        "project": sound.project_path().display().to_string(),
        "revision": sound.takes.revision,
        "current": sound.takes.current,
        "takes": sound.takes.takes.iter().map(|t| sound.take_json(t, req.include_scores)).collect::<Vec<_>>(),
        "score": project.map(|p| serde_json::to_value(&p.score).unwrap_or(Value::Null)),
        "can_undo": project.is_some_and(|p| p.can_undo()),
        "can_redo": project.is_some_and(|p| p.can_redo()),
        "views": sound.takes.views,
    }))
}

/// Goes back to take `n` through an undoable patch (`restore_version("take-<n>")`) and makes it
/// current. `{sound, project, revision, current, take, restored, patch, ...}`; `restored` is
/// false when the score already was take `n`.
pub fn restore(req: &RestoreRequest) -> Result<Value, OpError> {
    let dir = resolve_sound(&req.sound)?;
    let _lock = SoundLock::acquire(&dir)?;
    let mut sound = Sound::open(&dir)?;
    let take = sound.takes.take(req.n).cloned().ok_or_else(|| {
        OpError::new(
            ErrorCode::Error,
            format!(
                "there is no take {} (takes: {})",
                req.n,
                sound
                    .takes
                    .takes
                    .iter()
                    .map(|t| t.n.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
    })?;
    let project = sound.project.as_mut().expect("takes need a project");
    let target = project
        .version(&take.version)
        .map(|v| v.score.clone())
        .ok_or_else(|| {
            OpError::new(
                ErrorCode::BadScore,
                format!("the project has no version `{}`", take.version),
            )
        })?;
    let (restored, patch) = if project.score == target {
        (false, Value::Null)
    } else {
        let entry = project
            .restore_version(&take.version)
            .map_err(project_error)?;
        (
            true,
            serde_json::to_value(&entry.patch).unwrap_or(Value::Null),
        )
    };
    sound.takes.current = req.n;
    sound.save()?;
    let mut v = take_payload(&sound, &take, false);
    v["restored"] = json!(restored);
    v["patch"] = patch;
    Ok(v)
}

/// Commits variation `index` of the sound's last `variations` run as a new take. Fails when the
/// sound changed since the variations were made.
pub fn keep_variation(req: &KeepVariationRequest) -> Result<Value, OpError> {
    let dir = resolve_sound(&req.sound)?;
    let _lock = SoundLock::acquire(&dir)?;
    let mut sound = Sound::open(&dir)?;
    let file = load_variations(&variations_dir(&sound))?;
    let rec = file
        .variations
        .iter()
        .find(|v| v.index == req.index)
        .cloned()
        .ok_or_else(|| {
            OpError::new(
                ErrorCode::Error,
                format!(
                    "there is no variation {} (1 to {})",
                    req.index,
                    file.variations.len()
                ),
            )
        })?;
    let project = sound
        .project
        .as_mut()
        .ok_or_else(|| OpError::new(ErrorCode::Error, "the sound has no takes yet"))?;
    if file.base_take != sound.takes.current || file.base_seq != project.current_seq() {
        return Err(OpError::new(
            ErrorCode::PatchRejected,
            format!(
                "the sound changed since these variations were made (from take {}, now take {}); \
                 make the variations again",
                file.base_take, sound.takes.current
            ),
        ));
    }
    let before = project.score.clone();
    project.commit(rec.patch.clone()).map_err(project_error)?;
    let bars = rec.patch.scope.bars;
    let scope = match (bars, file.scope.layers.as_slice()) {
        (Some(b), [layer]) => Some(TakeScope {
            layer: layer.clone(),
            bars: b,
        }),
        _ => None,
    };
    let words = Words {
        who: normalize_who(req.who.as_deref(), DEFAULT_WHO),
        ask: req
            .ask
            .clone()
            .unwrap_or_else(|| format!("Keep variation {}", req.index)),
        title: req.title.clone().unwrap_or_else(|| match bars {
            Some(b) => format!("Variation {}, {}", req.index, bars_label([b.start, b.end])),
            None => format!("Variation {}", req.index),
        }),
        say: rec.summary.clone(),
        scope,
        provider: "variation".into(),
        seed: rec.seed,
    };
    let take = make_take(
        &mut sound,
        Some(&before),
        Some(rec.patch.clone()),
        words,
        &render_settings(req.skip_missing),
    )?;
    let mut v = take_payload(&sound, &take, req.include_score);
    v["variation"] = json!(req.index);
    Ok(v)
}

/// Records a request to show a panel view (`{revision, view, by, take}` in `takes.json`'s
/// `views`); the app shows it ("Notes opened by cut-assistant"). When the current take was made
/// by the same `who`, it also becomes that take's `opens`.
pub fn open_view(req: &OpenViewRequest) -> Result<Value, OpError> {
    let dir = resolve_sound(&req.sound)?;
    let _lock = SoundLock::acquire(&dir)?;
    let mut sound = Sound::open(&dir)?;
    let who = normalize_who(req.who.as_deref(), DEFAULT_WHO);
    let by = if who == "You" {
        "Praxis".to_string()
    } else {
        agent_name(&who).to_string()
    };
    let current = sound.takes.current;
    if let Some(t) = sound.takes.takes.iter_mut().find(|t| t.n == current)
        && t.who == who
    {
        t.opens = Some(req.view);
    }
    let request = ViewRequest {
        revision: sound.takes.revision + 1,
        view: req.view,
        by: by.clone(),
        take: current,
    };
    sound.takes.views.push(request.clone());
    sound.save()?;
    Ok(json!({
        "sound": dir.display().to_string(),
        "revision": sound.takes.revision,
        "view": req.view,
        "by": by,
        "take": current,
    }))
}
