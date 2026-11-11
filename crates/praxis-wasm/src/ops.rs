//! Everything the browser asks of the engine, as plain Rust over JSON values, so the same code
//! runs in the wasm exports (`lib.rs`) and in native tests. Every result is a JSON object with
//! `ok: true` and its data, or `ok: false` and a `message` in plain words (and, for a refused
//! patch, `error` in the shape of the app's `PatchError`).

use crate::library::Library;
use praxis_analyze::features::{self, loudness, onset, seam, tempo};
use praxis_analyze::verify::{Tolerances, VerifyReport, verify_render};
use praxis_dsp::StereoBuffer;
use praxis_engine::{InstrumentFactory, RenderOptions, RenderReport, render_offline};
use praxis_planner::edit::{
    DynamicsChange, EditIntent, EditPlan, EditRules, EditScope, edit_request,
};
use praxis_planner::{Catalogue, EditOutcome, PlanOutcome, Provider, RuleProvider};
use praxis_score::patch::{PatchError, ScorePatch};
use praxis_score::{BarRange, Score};
use serde_json::{Value, json};

/// Seed of the first take (the CLI's `praxis generate` default and the take number).
pub const DEFAULT_SEED: u64 = 1;

/// `{ok: false, message}`.
pub fn fail(message: impl Into<String>) -> Value {
    json!({ "ok": false, "message": message.into() })
}

/// A finite number, else `null` (JSON has no infinity).
pub fn finite(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}

/// Parses a score, or says why it is not one.
pub fn parse_score(json: &str) -> Result<Score, Value> {
    serde_json::from_str::<Score>(json).map_err(|e| fail(format!("not a valid score: {e}")))
}

/// `{ok, errors, warnings}` from the score validator.
pub fn validate(json: &str) -> Value {
    match parse_score(json) {
        Ok(score) => {
            let report = praxis_score::validate::validate(&score);
            json!({ "ok": true, "errors": report.errors, "warnings": report.warnings })
        }
        Err(e) => e,
    }
}

fn plan_json(out: &PlanOutcome) -> Value {
    let title = if out.score.title.trim().is_empty() {
        "New sound".to_string()
    } else {
        out.score.title.clone()
    };
    json!({
        "ok": true,
        "score": out.score,
        "title": title,
        "summary": out.summary(),
        "provider": out.provider_id,
        "attempts": out.attempts,
        "repairs": out.repairs,
        "lint": out.lint,
        "plan": out.plan,
    })
}

/// A brief in words -> a Score, through the offline rule planner (no API key in the browser).
pub fn plan_brief(brief: &str, seed: u64) -> Value {
    let brief = brief.trim();
    if brief.is_empty() {
        return fail("Describe a sound to start from.");
    }
    match praxis_planner::plan_brief(brief, &RuleProvider::new(), &Catalogue::standard(), seed) {
        Ok(out) => plan_json(&out),
        Err(e) => fail(e.to_string()),
    }
}

/// An agent-written ScorePlan -> a Score (validation, repairs, arranger; no provider).
pub fn compose_plan(plan: &str, seed: u64) -> Value {
    let value: Value = match serde_json::from_str(plan) {
        Ok(v) => v,
        Err(e) => return fail(format!("not valid JSON: {e}")),
    };
    match praxis_planner::compose_plan(&value, &Catalogue::standard(), seed) {
        Ok(out) => plan_json(&out),
        Err(e) => fail(e.to_string()),
    }
}

/// The app's `PatchError` (app/src/engine/EngineAdapter.ts).
pub fn patch_error_json(e: &PatchError) -> Value {
    match e {
        PatchError::LockedLayer(layer) => json!({ "kind": "locked_layer", "layer": layer }),
        PatchError::OutOfScope { index, op, reason } => {
            json!({ "kind": "out_of_scope", "index": index, "op": op, "reason": reason })
        }
        PatchError::UnknownId {
            index,
            op,
            what,
            id,
        } => json!({ "kind": "unknown_id", "index": index, "op": op, "what": what, "id": id }),
        PatchError::DuplicateId {
            index,
            op,
            what,
            id,
        } => json!({ "kind": "duplicate_id", "index": index, "op": op, "what": what, "id": id }),
        PatchError::UnlockNotAllowed(layer) => {
            json!({ "kind": "unlock_not_allowed", "layer": layer })
        }
        PatchError::Invalid(message) => json!({ "kind": "invalid", "message": message }),
    }
}

/// Applies a patch atomically: `{ok, score, inverse}` or `{ok: false, message, error}`.
pub fn apply_patch(score: &str, patch: &str) -> Value {
    let score = match parse_score(score) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let patch: ScorePatch = match serde_json::from_str(patch) {
        Ok(p) => p,
        Err(e) => {
            let message = format!("not a valid patch: {e}");
            return json!({
                "ok": false,
                "message": message,
                "error": { "kind": "invalid", "message": message },
            });
        }
    };
    match praxis_score::apply::apply(&score, &patch) {
        Ok(a) => json!({ "ok": true, "score": a.score, "inverse": a.inverse }),
        Err(e) => json!({ "ok": false, "message": e.to_string(), "error": patch_error_json(&e) }),
    }
}

/// `"bar 3"` or `"bars 13–16"` (the CLI's wording).
pub fn bars_label(b: BarRange) -> String {
    if b.start == b.end {
        format!("bar {}", b.start)
    } else {
        format!("bars {}\u{2013}{}", b.start, b.end)
    }
}

/// A take's title from its edit plan (the CLI's `title_of`).
fn edit_title(plan: &EditPlan, bars: Option<BarRange>) -> String {
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
        Some(b) => format!("{word}, {}", bars_label(b)),
        None => word.to_string(),
    }
}

fn edit_json(out: &EditOutcome) -> Value {
    json!({
        "ok": true,
        "changed": out.changed(),
        "understood": out.understood(),
        "patch": out.patch,
        "inverse": out.inverse,
        "score": out.score,
        "scope": out.scope,
        "bars": [out.bars.start, out.bars.end],
        "summary": out.summary,
        "title": edit_title(&out.plan, out.patch.scope.bars),
        "refused": out.refused,
        "new_layers": out.new_layers,
        "plan": out.plan,
        "provider": out.provider_id,
        "repairs": out.repairs,
    })
}

/// A scope from the app: bars of one layer.
#[derive(Clone, Debug, PartialEq)]
pub struct EditTarget {
    pub layer: String,
    pub bars: BarRange,
}

/// Parses `{"layer": "...", "bars": {"start": n, "end": m}}` (the app's `TakeScope`).
pub fn parse_target(json: &str) -> Result<Option<EditTarget>, Value> {
    let v: Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => return Err(fail(format!("not a valid scope: {e}"))),
    };
    if v.is_null() {
        return Ok(None);
    }
    let layer = v["layer"].as_str().unwrap_or_default().to_string();
    let start = v["bars"]["start"].as_u64();
    let end = v["bars"]["end"].as_u64();
    match (layer.is_empty(), start, end) {
        (false, Some(start), Some(end)) => Ok(Some(EditTarget {
            layer,
            bars: BarRange {
                start: start as u32,
                end: end as u32,
            },
        })),
        _ => Err(fail("a scope names a layer and its bars")),
    }
}

/// An edit request in words on `score` -> the patch, through the offline edit rules. With a
/// `target`, the edit is held to exactly those bars of that layer (as `praxis ask` does): the
/// planner's intent is kept, its bars and layers are replaced by the scope, and a result that
/// changes another existing layer or other bars is refused.
pub fn plan_edit(score: &str, instruction: &str, target: &str, seed: u64) -> Value {
    let score = match parse_score(score) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let target = match parse_target(target) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let text = instruction.trim();
    if text.is_empty() {
        return fail("Say what to change.");
    }
    let catalogue = Catalogue::standard();
    let out = match target {
        None => praxis_planner::plan_edit(&score, text, &RuleProvider::new(), &catalogue, seed)
            .map_err(|e| fail(e.to_string())),
        Some(t) => scoped_edit(&score, text, &t, &catalogue, seed),
    };
    match out {
        Ok(out) => edit_json(&out),
        Err(e) => e,
    }
}

fn layer_name(score: &Score, id: &str) -> String {
    score
        .layer(id)
        .map(|l| {
            if l.name.is_empty() {
                l.id.clone()
            } else {
                l.name.clone()
            }
        })
        .unwrap_or_else(|| id.to_string())
}

fn scoped_edit(
    score: &Score,
    text: &str,
    t: &EditTarget,
    catalogue: &Catalogue,
    seed: u64,
) -> Result<EditOutcome, Value> {
    if score.layer(&t.layer).is_none() {
        return Err(fail(format!(
            "The scope names layer `{}`, which is not in the sound.",
            t.layer
        )));
    }
    let b = t.bars;
    if b.start < 1 || b.end < b.start || b.end > score.bars {
        return Err(fail(format!(
            "The scope's bars {}-{} are outside the sound's bars 1-{}.",
            b.start, b.end, score.bars
        )));
    }
    // "bars 9-12 of `piano`: ..." reads as the scope for the offline rules.
    let instruction = format!("bars {}-{} of `{}`: {text}", b.start, b.end, t.layer);
    // The planner sees only the scope's layer as editable.
    let mut view = score.clone();
    let mut view_locks = Vec::new();
    for l in &mut view.layers {
        if l.id != t.layer && !l.locked {
            l.locked = true;
            view_locks.push(l.id.clone());
        }
    }
    let req = edit_request(&instruction, &view, catalogue, seed);
    let value = EditRules::new()
        .call_tool(&req)
        .map_err(|e| fail(e.to_string()))?;
    let mut plan: EditPlan = serde_json::from_value(value)
        .map_err(|e| fail(format!("the edit plan does not match the schema: {e}")))?;
    plan.scope = EditScope {
        bars: Some(b),
        phrase: None,
        keep: None,
    };
    plan.layers = match plan.intent {
        EditIntent::AddLayer | EditIntent::AddMelody => Vec::new(),
        _ => vec![t.layer.clone()],
    };
    let plan_value = serde_json::to_value(&plan).unwrap_or(Value::Null);
    let mut out = praxis_planner::compose_edit(&plan_value, &view, catalogue, seed)
        .map_err(|e| fail(e.to_string()))?;
    out.provider_id = RuleProvider::new().id();
    // The same patch on the real score, where the other layers are not locked.
    let applied =
        praxis_score::apply::apply(score, &out.patch).map_err(|e| fail(e.to_string()))?;
    out.score = applied.score;
    out.inverse = applied.inverse;
    if !out.refused.is_empty() && out.refused.iter().all(|r| view_locks.contains(r)) {
        let kept: Vec<&str> = out
            .summary
            .trim_end_matches('.')
            .split(". ")
            .filter(|s| !s.starts_with("Left `"))
            .collect();
        out.summary = format!("{}.", kept.join(". "));
    }
    out.refused.retain(|r| !view_locks.contains(r));
    // Nothing wider than the scope: no other existing layer, no other bars.
    let scope_words = format!("{}, {}", bars_label(b), layer_name(score, &t.layer));
    for (layer, r) in changed_bars(score, &out.score) {
        let existing = score.layer(&layer).is_some();
        if (existing && layer != t.layer) || r.start < b.start || r.end > b.end {
            return Err(fail(format!(
                "The edit would change {} of layer `{layer}`, outside the scope ({scope_words}); \
                 nothing was changed.",
                bars_label(r)
            )));
        }
    }
    Ok(out)
}

/// Bars each layer's notes changed in between two scores (by expanded note events), plus the
/// whole extent of layers whose sound settings changed or that were added or removed. Only used
/// to hold a scoped edit to its scope.
fn changed_bars(before: &Score, after: &Score) -> Vec<(String, BarRange)> {
    use praxis_score::timeline::expand_notes;
    let sr = 48_000;
    let a = expand_notes(before, sr);
    let b = expand_notes(after, sr);
    let mut out: Vec<(String, BarRange)> = Vec::new();
    let mut add = |id: &str, bar: u32| {
        if let Some((_, r)) = out.iter_mut().find(|(l, _)| l == id) {
            r.start = r.start.min(bar);
            r.end = r.end.max(bar);
        } else {
            out.push((id.to_string(), BarRange { start: bar, end: bar }));
        }
    };
    let ids: std::collections::BTreeSet<&str> = before
        .layers
        .iter()
        .chain(after.layers.iter())
        .map(|l| l.id.as_str())
        .collect();
    for id in ids {
        let lb = before.layer(id);
        let la = after.layer(id);
        let notes = |s: &Score, evs: &[praxis_score::timeline::NoteEvent]| {
            let idx = s.layers.iter().position(|l| l.id == id);
            evs.iter()
                .filter(|e| Some(e.layer) == idx)
                .map(|e| {
                    (
                        e.bar,
                        e.start_sample,
                        e.pitch,
                        e.velocity.to_bits(),
                        e.length_samples,
                    )
                })
                .collect::<Vec<_>>()
        };
        match (lb, la) {
            (Some(x), Some(y)) => {
                let mut sx = x.clone();
                let mut sy = y.clone();
                sx.clips.clear();
                sy.clips.clear();
                if sx != sy {
                    for e in notes(before, &a).iter().chain(notes(after, &b).iter()) {
                        add(id, e.0);
                    }
                    continue;
                }
                let na = notes(before, &a);
                let nb = notes(after, &b);
                for e in na.iter().filter(|e| !nb.contains(e)) {
                    add(id, e.0);
                }
                for e in nb.iter().filter(|e| !na.contains(e)) {
                    add(id, e.0);
                }
            }
            (Some(_), None) => notes(before, &a).iter().for_each(|e| add(id, e.0)),
            (None, Some(_)) => notes(after, &b).iter().for_each(|e| add(id, e.0)),
            (None, None) => {}
        }
    }
    out
}

/// Render settings from the app.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderSettings {
    pub sample_rate: u32,
    /// Only these layers (stems); empty = every audible layer.
    pub layers: Vec<String>,
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings {
            sample_rate: 48_000,
            layers: Vec::new(),
        }
    }
}

/// Parses `{"sample_rate"?: n, "layers"?: [..]}` (unknown fields ignored, `null` = defaults).
pub fn parse_settings(json: &str) -> RenderSettings {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    let mut s = RenderSettings::default();
    if let Some(sr) = v["sample_rate"].as_u64().filter(|&r| r > 0) {
        s.sample_rate = sr as u32;
    }
    if let Some(list) = v["layers"].as_array() {
        s.layers = list
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect();
    }
    s
}

/// The app's `RenderReport` (app/src/engine/EngineAdapter.ts).
pub fn report_json(r: &RenderReport, frames: usize, render_seconds: f64, notes: &[String]) -> Value {
    let mut warnings = r.warnings.clone();
    warnings.extend(notes.iter().cloned());
    json!({
        "mock": false,
        "sample_rate": r.sample_rate,
        "length_samples": r.length_samples,
        "tail_samples": r.tail_samples,
        "is_loop": r.is_loop,
        "loudness_before_lufs": finite(r.loudness_before_lufs),
        "loudness_lufs": finite(r.loudness_lufs),
        "true_peak_dbtp": finite(r.true_peak_dbtp),
        "applied_gain_db": r.applied_gain_db,
        "warmup_cycles": r.warmup_cycles,
        "notes_rendered": r.notes_rendered,
        "blocked_layers": r.blocked_layers,
        "render_seconds": render_seconds,
        "warnings": warnings,
        "frames": frames,
        "duration_s": frames as f64 / r.sample_rate.max(1) as f64,
        "fallbacks": notes,
    })
}

/// The app's `VerifyReport`: the analyzer's report plus `passed` and the summary line.
pub fn verify_json(report: &VerifyReport) -> Value {
    let mut v = serde_json::to_value(report).unwrap_or(Value::Null);
    let names = |pred: fn(&praxis_analyze::verify::Check) -> bool| {
        report
            .checks
            .iter()
            .filter(|c| pred(c))
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
    };
    v["passed"] = json!(report.passed());
    v["failed"] = json!(names(|c| c.failed()));
    v["inconclusive"] = json!(names(|c| c.inconclusive));
    v["summary"] = json!(report.summary());
    v
}

/// A finished offline render: the master, its report and the checks.
#[derive(Debug)]
pub struct Render {
    pub audio: StereoBuffer,
    pub report: Value,
    pub checks: Value,
    /// The score as played (after synth stand-ins for missing packs).
    pub played: Score,
}

/// Renders `score` offline (the same `render_offline` as the desktop) and checks the master
/// against the score as played. `elapsed` measures the render in seconds (the browser passes
/// `performance.now`, native tests a clock); `None` reports 0.
pub fn render(
    lib: &mut Library,
    score: &str,
    settings: &RenderSettings,
    elapsed: Option<&dyn Fn() -> f64>,
) -> Result<Render, Value> {
    let score = parse_score(score)?;
    let (played, notes) = lib.playable(&score);
    let opts = RenderOptions {
        sample_rate: settings.sample_rate,
        only_layers: settings.layers.clone(),
        ..RenderOptions::default()
    };
    let t0 = elapsed.map(|f| f());
    let out = render_offline(&played, lib.registry(), &opts).map_err(|e| fail(e.to_string()))?;
    let seconds = match (t0, elapsed) {
        (Some(a), Some(f)) => (f() - a).max(0.0),
        _ => 0.0,
    };
    // Check what was rendered: a stem render checks only its layers.
    let mut checked = played.clone();
    if !settings.layers.is_empty() {
        checked.layers.retain(|l| settings.layers.contains(&l.id));
    }
    let report = verify_render(
        &checked,
        &out.master,
        settings.sample_rate,
        &Tolerances::default(),
    );
    Ok(Render {
        report: report_json(&out.report, out.master.len(), seconds, &notes),
        checks: verify_json(&report),
        audio: out.master,
        played,
    })
}

/// Checks a master render (`left`, `right`) against `score`.
pub fn verify(score: &str, left: &[f32], right: &[f32], sample_rate: u32) -> Value {
    let score = match parse_score(score) {
        Ok(s) => s,
        Err(e) => return e,
    };
    if left.len() != right.len() || sample_rate == 0 {
        return fail("the two channels must have the same length and the rate must be above 0");
    }
    let audio = StereoBuffer {
        left: left.to_vec(),
        right: right.to_vec(),
    };
    let report = verify_render(&score, &audio, sample_rate, &Tolerances::default());
    let mut v = verify_json(&report);
    v["ok"] = json!(true);
    v
}

/// Measures a clip's PCM: tempo, key, loudness, onsets and loop seam (`praxis analyze`'s
/// measurements, under the Sound Profile's field names).
pub fn analyse(left: &[f32], right: &[f32], sample_rate: u32, beats_per_bar: u8) -> Value {
    if left.len() != right.len() || sample_rate == 0 || left.is_empty() {
        return fail("give two channels of the same, non-zero length and a rate above 0");
    }
    let mono = features::mono(left, right);
    let loud = match loudness::measure(left, right, sample_rate) {
        Ok(l) => l,
        Err(e) => return fail(e.to_string()),
    };
    let tempo_est = tempo::analyze(&mono, sample_rate, beats_per_bar.max(1));
    let key_est = features::key::analyze(&mono, sample_rate);
    let (_, onsets) = onset::detect_onsets(&mono, sample_rate);
    let s = seam::analyze_seam(left, right, sample_rate, &seam::SeamConfig::default());
    json!({
        "ok": true,
        "source": {
            "duration_s": left.len() as f64 / sample_rate as f64,
            "sample_rate": sample_rate,
            "channels": 2,
        },
        "tempo": tempo_est,
        "key": key_est.as_ref().map(|k| json!({
            "key": k.key,
            "name": k.key.display(),
            "confidence": k.confidence,
            "chroma": k.chroma,
        })),
        "loudness": loud,
        "onsets_s": onsets.iter().map(|o| o.time_s).collect::<Vec<_>>(),
        "loop_seam": {
            "passed": s.passed,
            "jump_ratio": finite(s.jump_ratio),
            "excess_db": finite(s.excess_db),
        },
    })
}

/// The planner's preset catalogue and whether each can play here (`praxis presets`): a sampled
/// preset is available once its pack's files are loaded.
pub fn presets(lib: &mut Library) -> Value {
    let catalogue = Catalogue::standard();
    let mut list = Vec::new();
    for p in &catalogue.presets {
        let info = lib.registry().describe(p.kind, &p.name);
        let reason = match info {
            None => Some(format!("preset `{}` is not built yet", p.name)),
            Some(i) => match crate::library::pack_of(&i.source) {
                Some(pack) if !lib.complete(pack, &p.name) => Some(format!(
                    "pack `{pack}` is not on this server (the browser loads it from packs/)"
                )),
                _ => None,
            },
        };
        list.push(json!({
            "name": p.name,
            "kind": p.kind,
            "class": p.class,
            "pitched": p.pitched,
            "sustaining": p.sustaining,
            "range": [p.range.0, p.range.1],
            "description": p.description,
            "available": reason.is_none(),
            "reason": reason,
        }));
    }
    let engine_only: Vec<Value> = lib
        .registry()
        .presets()
        .iter()
        .filter(|i| catalogue.get_kind(i.kind, &i.name).is_none())
        .map(|i| json!({ "name": i.name, "kind": i.kind, "class": i.class, "source": i.source }))
        .collect();
    json!({ "ok": true, "presets": list, "engine_only": engine_only })
}

/// `[{pack, preset}]`: the packs the sampled layers of `score` load from.
pub fn pack_needs(lib: &mut Library, score: &str) -> Value {
    match parse_score(score) {
        Ok(s) => {
            let needs: Vec<Value> = lib
                .needs(&s)
                .into_iter()
                .map(|n| {
                    json!({
                        "pack": n.pack,
                        "preset": n.preset,
                        "complete": lib.complete(&n.pack, &n.preset),
                    })
                })
                .collect();
            json!({ "ok": true, "needs": needs })
        }
        Err(e) => e,
    }
}

/// `{ok, missing: [path]}`: the files of `preset` in `pack` still to fetch.
pub fn missing_pack_files(lib: &Library, pack: &str, preset: &str) -> Value {
    match lib.missing(pack, preset) {
        Ok(m) => json!({ "ok": true, "missing": m }),
        Err(e) => fail(e),
    }
}
