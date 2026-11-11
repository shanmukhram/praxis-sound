//! The assistant's tools: one table ([`definitions`]) and one dispatcher ([`run`]).
//!
//! Every schema is strict (`strict: true`, `additionalProperties: false`, every property
//! required). The API allows at most 20 strict tools, 24 optional parameters and 16 union-typed
//! parameters per request, so the table has 17 tools, no optional and no union parameters:
//! "none" is said with an empty string or a 0. The three large documents (a ScorePlan, an
//! EditPlan, a ScorePatch) travel as JSON text and are checked by the same code the CLI and MCP
//! use (`compose_plan`, `compose_edit`, `apply`), whose errors point into the document; their
//! schemas come from `get_schema`. That keeps the tool grammar small enough to compile for every
//! request.
//!
//! State-changing tools work on the turn's draft ([`super::draft`]) or, for the history tools,
//! on the sound folder through the same ops as the CLI and MCP, so the take history stays shared.

use super::catalogue::catalogue_json;
use super::draft::{Draft, NewSound};
use super::measure::{compare, render_measure};
use super::{AGENT_NAME, AgentConfig};
use crate::error::OpError;
use crate::ops::changes::{changed_layers, identical_bars};
use crate::ops::check::decode;
use crate::ops::sound::{Sound, ViewName};
use crate::ops::{self, score_summary};
use praxis_engine::InstrumentFactory;
use praxis_instruments::Registry;
use praxis_planner::{Catalogue, compose_edit, compose_plan};
use praxis_score::patch::{LayerUpdate, Origin, PatchOp, Scope, ScorePatch};
use praxis_score::{BarRange, InstrumentRef, Layer, Score};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// An example ScorePlan (acceptance test 1).
const EXAMPLE_PLAN: &str =
    include_str!("../../../praxis-planner/tests/prompts/acceptance1.plan.json");
/// An example EditPlan.
const EXAMPLE_EDIT: &str =
    include_str!("../../../praxis-planner/tests/prompts/edit_last_four_melody.plan.json");

fn string(description: &str) -> Value {
    json!({"type": "string", "description": description})
}

fn integer(description: &str) -> Value {
    json!({"type": "integer", "description": description})
}

fn one_of(values: &[&str], description: &str) -> Value {
    json!({"type": "string", "enum": values, "description": description})
}

/// A strict object schema: every property required, nothing else allowed.
fn object(props: &[(&str, Value)]) -> Value {
    let mut p = Map::new();
    for (k, v) in props {
        p.insert((*k).to_string(), v.clone());
    }
    json!({
        "type": "object",
        "properties": p,
        "required": props.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        "additionalProperties": false,
    })
}

fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "input_schema": schema,
        "strict": true,
    })
}

/// The tool table, in a fixed order (it is part of the cached prefix).
pub fn definitions() -> Vec<Value> {
    vec![
        tool(
            "read_sound",
            "Read the sound being worked on. `part`: `summary` (every layer with its instrument \
             and parameters (defaults filled in), gain, pan, mute, lock, effects and the bars it \
             plays; sections; the takes; the current take; whether the draft has changes), \
             `layer` (one layer in full with its patterns and notes; name it in `layer`), \
             `score` (the whole Score JSON of the draft), `takes` (every take: who, the request, \
             the reply, what changed, the checks), or `changes` (what take `take` changed from \
             the take before it, and which bars stayed bit-identical; 0 = the current take). Use \
             \"\" for `layer` and 0 for `take` when the part does not need them.",
            object(&[
                (
                    "part",
                    one_of(
                        &["summary", "layer", "score", "takes", "changes"],
                        "What to read.",
                    ),
                ),
                ("layer", string("Layer id for part `layer`, else \"\".")),
                (
                    "take",
                    integer("Take number for part `changes`; 0 = the current take."),
                ),
            ]),
        ),
        tool(
            "catalogue",
            "The instrument presets with every parameter (range, default, what it does), whether \
             each renders on this machine, the effect kinds with their parameters, and the layer \
             parameters automation can reach. `preset`: one preset name, or \"\" for all.",
            object(&[("preset", string("A preset name, or \"\" for every preset."))]),
        ),
        tool(
            "get_schema",
            "The JSON Schema and an example of a document the other tools take as JSON text: \
             `score_plan` (compose_sound), `edit_plan` (compose_edit) or `score_patch` \
             (apply_patch).",
            object(&[(
                "which",
                one_of(
                    &["score_plan", "edit_plan", "score_patch"],
                    "Which document.",
                ),
            )]),
        ),
        tool(
            "compose_sound",
            "Compose a whole sound from a ScorePlan (JSON text; see get_schema `score_plan`): \
             validated, repaired where it can be (each repair listed) and arranged into the \
             draft. Use it for a new sound, or when the person asks to start over (it replaces \
             every layer that is not locked). Returns a summary, the repairs and the layers.",
            object(&[("plan_json", string("The ScorePlan as JSON text."))]),
        ),
        tool(
            "compose_edit",
            "Apply an EditPlan (JSON text; see get_schema `edit_plan`) to the draft: the \
             planner's musical edits (add_layer, add_melody, rewrite_motif, change_instrument, \
             change_dynamics, remove_layer, thin_out, and any newer intents the schema lists), \
             resolved to exact bars and layers. Locked layers are never changed (listed in \
             `refused`). Returns what changed, by layer and bars.",
            object(&[("edit_plan_json", string("The EditPlan as JSON text."))]),
        ),
        tool(
            "apply_patch",
            "Apply a ScorePatch (JSON text; see get_schema `score_patch`) to the draft: any op \
             (update_layer for gain, pan, mute, instrument, effects or automation; write_bars, \
             clear_bars and place_clip for notes in some bars; add_layer, remove_layer; global \
             ops such as set_tempo without a bar scope). Locks and the scope are enforced: an op \
             on a locked layer or outside the scope fails and nothing changes. Returns what \
             changed, by layer and bars.",
            object(&[("patch_json", string("The ScorePatch as JSON text."))]),
        ),
        tool(
            "set_params",
            "Set instrument parameters of one layer (they apply to the whole layer), e.g. \
             `tone` and `felt` of a felt piano to make it muffled or brighter. Names and ranges \
             are in the catalogue; a value outside the range fails. Parameters not named keep \
             their value.",
            object(&[
                ("layer", string("Layer id.")),
                (
                    "params",
                    json!({
                        "type": "array",
                        "description": "Parameters to set.",
                        "items": object(&[
                            ("name", string("Parameter name, e.g. \"tone\".")),
                            ("value", json!({"type": "number", "description": "New value."})),
                        ]),
                    }),
                ),
            ]),
        ),
        tool(
            "render_check",
            "Render the draft, run the checks (tempo, key, grid, loudness, true peak, clipping, \
             loop seam, silence) and measure it: loudness, true peak, onsets on the grid, and per \
             layer the spectral centroid (Hz; lower is darker or more muffled), the share of \
             energy below 150 Hz and above 2 kHz (dB; less high band is duller) and the RMS \
             level, each next to the current take's value \
             with the change, plus the bars that are bit-identical to the current take.",
            object(&[]),
        ),
        tool(
            "make_variations",
            "Make alternates of some bars of some layers of the current take (the draft must have \
             no changes), each rendered and checked; nothing is committed. Give layer ids (empty \
             = every unlocked layer playing there) and the bars (0 and 0 = the whole sound).",
            object(&[
                (
                    "layers",
                    json!({"type": "array", "items": {"type": "string"}, "description": "Layer ids."}),
                ),
                ("start_bar", integer("First bar, or 0 for the whole sound.")),
                ("end_bar", integer("Last bar, or 0 for the whole sound.")),
                ("count", integer("How many, 1 to 8.")),
            ]),
        ),
        tool(
            "keep_variation",
            "Make variation `index` (1-based, from make_variations) the new take. It is this \
             request's one take.",
            object(&[("index", integer("Variation number."))]),
        ),
        tool(
            "restore_take",
            "Go back to take `n` (an undoable step; no new take). The draft must have no changes.",
            object(&[("n", integer("Take number."))]),
        ),
        tool(
            "undo_redo",
            "Step the sound's history back (undo) or forward (redo) one step, as the app's undo \
             and redo buttons do. The draft must have no changes.",
            object(&[("action", one_of(&["undo", "redo"], "Which way."))]),
        ),
        tool(
            "open_view",
            "Show the person a panel view of the sound to point at what you changed. When the \
             draft has changes, the view opens once the take is made.",
            object(&[(
                "view",
                one_of(
                    &["overview", "rhythm", "notes", "mix", "export"],
                    "The view.",
                ),
            )]),
        ),
        tool(
            "analyze_clip",
            "Measure an audio or video file (a clip the person dropped): duration, tempo and \
             beats, key, loudness, onsets, loop seam.",
            object(&[("path", string("The file's path."))]),
        ),
        tool(
            "ask_user",
            "Ask the person one short clarifying question when the request could mean very \
             different things. Your turn ends at once and no take is made; their answer comes as \
             the next request.",
            object(&[("question", string("The question, in one sentence."))]),
        ),
        tool(
            "discard_changes",
            "Throw away every change in the draft and start again from the current take.",
            object(&[]),
        ),
        tool(
            FINISH_TURN,
            "End your turn. Write your reply to the person as text in the same message, then \
             call this as the only tool in that message. Your changes become the take; its \
             title is `title`.",
            object(&[
                (
                    "title",
                    string(
                        "The take's title: 3 to 6 words in sentence case saying what changed in \
                         sound terms, without \"I\" (\"Muffled felt piano\", \"Softer kick in \
                         the second half\"). \"\" when nothing changed.",
                    ),
                ),
                (
                    "no_take",
                    one_of(
                        &["", "refused", "nothing_to_change"],
                        "\"\" when you changed the sound. Otherwise why not: `refused` (you \
                         could not or would not do it, e.g. the layer is locked) or \
                         `nothing_to_change` (an answer, advice, an undo or a view).",
                    ),
                ),
            ]),
        ),
    ]
}

/// The tool that ends a turn with the take's title (handled by the turn loop, not [`run`]).
pub const FINISH_TURN: &str = "finish_turn";

/// What a tool call returned.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub ok: bool,
    /// The result (`null` on failure).
    pub value: Value,
    /// Why it failed (empty on success).
    pub error: String,
    /// The turn ends after this call (`ask_user`).
    pub ends_turn: bool,
}

impl ToolResult {
    fn ok(value: Value) -> ToolResult {
        ToolResult {
            ok: true,
            value,
            error: String::new(),
            ends_turn: false,
        }
    }

    fn fail(error: impl Into<String>) -> ToolResult {
        ToolResult {
            ok: false,
            value: Value::Null,
            error: error.into(),
            ends_turn: false,
        }
    }

    /// The `tool_result` content sent back to the model.
    pub fn content(&self) -> String {
        if self.ok {
            serde_json::to_string(&self.value).unwrap_or_default()
        } else {
            self.error.clone()
        }
    }

    /// A short form for the turn record.
    pub fn short(&self) -> String {
        let text = if self.ok {
            match &self.value {
                Value::Object(m) if m.contains_key("summary") => {
                    m["summary"].as_str().unwrap_or("").to_string()
                }
                Value::Object(m) if m.contains_key("check") => format!(
                    "passed {}, failed {}, can't tell {}",
                    m["check"]["passed"], m["check"]["failed"], m["check"]["inconclusive"]
                ),
                _ => "ok".to_string(),
            }
        } else {
            self.error.clone()
        };
        let mut s: String = text.chars().take(300).collect();
        if text.chars().count() > 300 {
            s.push_str("...");
        }
        s
    }
}

fn err_text(e: &OpError) -> String {
    if e.issues.is_empty() {
        e.to_string()
    } else {
        format!(
            "{e}\nissues: {}",
            serde_json::to_string(&e.issues).unwrap_or_default()
        )
    }
}

/// Everything a tool call can reach.
pub(crate) struct ToolCtx<'a> {
    pub draft: &'a mut Draft,
    /// The person's request, for the take record of a kept variation.
    pub request: &'a str,
    #[allow(dead_code)]
    pub config: &'a AgentConfig,
}

/// Runs one tool call. Never panics on model input; failures come back as `ok: false` with a
/// message that says what to fix.
pub(crate) fn run(name: &str, input: &Value, ctx: &mut ToolCtx) -> ToolResult {
    let s = |k: &str| input[k].as_str().unwrap_or("").to_string();
    let n = |k: &str| input[k].as_i64().unwrap_or(0);
    match name {
        "read_sound" => read_sound(ctx.draft, &s("part"), &s("layer"), n("take")),
        "catalogue" => ToolResult::ok(catalogue_json(&s("preset"))),
        "get_schema" => get_schema(&s("which")),
        "compose_sound" => compose_sound(ctx.draft, &s("plan_json")),
        "compose_edit" => edit(ctx.draft, &s("edit_plan_json")),
        "apply_patch" => apply_patch(ctx.draft, &s("patch_json")),
        "set_params" => set_params(ctx.draft, &s("layer"), &input["params"]),
        "render_check" => render_check(ctx.draft),
        "make_variations" => make_variations(
            ctx.draft,
            &input["layers"],
            n("start_bar"),
            n("end_bar"),
            n("count"),
        ),
        "keep_variation" => keep_variation(ctx, n("index")),
        "restore_take" => restore_take(ctx.draft, n("n")),
        "undo_redo" => undo_redo(ctx.draft, &s("action")),
        "open_view" => open_view(ctx.draft, &s("view")),
        "analyze_clip" => match ops::analyze(&ops::AnalyzeRequest {
            clip: s("path").into(),
            out: None,
            beats_per_bar: 4,
        }) {
            Ok(v) => ToolResult::ok(v),
            Err(e) => ToolResult::fail(err_text(&e)),
        },
        "ask_user" => {
            let q = s("question");
            if q.trim().is_empty() {
                return ToolResult::fail("the question is empty");
            }
            let dropped = ctx.draft.dirty();
            ctx.draft.question = Some(q.trim().to_string());
            let mut r = ToolResult::ok(json!({
                "asked": true,
                "note": if dropped {
                    "the draft's changes were dropped: ask before changing"
                } else {
                    "the person's answer comes as the next request"
                },
            }));
            r.ends_turn = true;
            r
        }
        "discard_changes" => match ctx.draft.reload() {
            Ok(()) => ToolResult::ok(json!({"discarded": true})),
            Err(e) => ToolResult::fail(err_text(&e)),
        },
        FINISH_TURN => {
            let mut r = ToolResult::ok(json!({"finished": true}));
            r.ends_turn = true;
            r
        }
        other => ToolResult::fail(format!(
            "there is no tool `{other}`; the tools are: {}",
            definitions()
                .iter()
                .filter_map(|t| t["name"].as_str().map(str::to_string))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

// ------------------------------------------------------------------------------- reading

/// A layer's instrument parameters with the preset's defaults filled in.
fn instrument_params(registry: &Registry, layer: &Layer) -> BTreeMap<String, f64> {
    let mut out = BTreeMap::new();
    if let Some(info) = registry.describe(layer.instrument.kind, &layer.instrument.preset) {
        for p in &info.params {
            out.insert(p.name.clone(), p.default);
        }
    }
    for (k, v) in &layer.instrument.params {
        out.insert(k.clone(), *v);
    }
    out
}

/// `score_summary` with every layer's sound settings.
pub fn detailed_summary(score: &Score) -> Value {
    let registry = Registry::with_default_packs();
    let mut v = score_summary(score);
    if let Some(layers) = v["layers"].as_array_mut() {
        for (entry, l) in layers.iter_mut().zip(&score.layers) {
            entry["params"] = json!(instrument_params(&registry, l));
            entry["gain_db"] = json!(l.gain_db);
            entry["pan"] = json!(l.pan);
            entry["solo"] = json!(l.solo);
            entry["locked"] = json!(l.locked);
            entry["effects"] = json!(
                l.effects
                    .iter()
                    .map(|e| json!({"id": e.id, "kind": e.kind, "params": e.params, "bypass": e.bypass}))
                    .collect::<Vec<_>>()
            );
            entry["automation"] = json!(
                l.automation
                    .iter()
                    .map(|a| a.target.clone())
                    .collect::<Vec<_>>()
            );
        }
    }
    v["chords"] = json!(score.chords);
    v
}

fn take_line(t: &ops::TakeRecord) -> Value {
    json!({
        "n": t.n,
        "who": t.who,
        "ask": t.ask,
        "title": t.title,
        "say": t.say,
        "from": t.from,
        "changed": t.changed,
        "identical_bars": t.identical_bars,
        "checks_passed": t.checks["passed"],
        "checks_failed": t.checks["failed"],
        "checks_cant_tell": t.checks["inconclusive"],
    })
}

fn read_sound(draft: &Draft, part: &str, layer: &str, take: i64) -> ToolResult {
    let sound = match Sound::open(&draft.dir) {
        Ok(s) => s,
        Err(e) => return ToolResult::fail(err_text(&e)),
    };
    let score = draft.score.as_ref();
    let no_score =
        || ToolResult::fail("the sound has no score yet: compose one with compose_sound first");
    match part {
        "summary" => {
            let project = sound.project.as_ref();
            ToolResult::ok(json!({
                "revision": sound.takes.revision,
                "current_take": sound.takes.current,
                "takes": sound.takes.takes.iter().map(|t| json!({
                    "n": t.n, "who": t.who, "title": t.title, "ask": t.ask,
                })).collect::<Vec<_>>(),
                "can_undo": project.is_some_and(|p| p.can_undo()),
                "can_redo": project.is_some_and(|p| p.can_redo()),
                "scope": draft.scope,
                "draft_has_changes": draft.dirty(),
                "draft_changed": match (&draft.base, score) {
                    (Some(b), Some(s)) => json!(changed_layers(b, s)),
                    _ => Value::Null,
                },
                "score": score.map(detailed_summary),
            }))
        }
        "layer" => {
            let Some(score) = score else {
                return no_score();
            };
            let Some(l) = score.layer(layer) else {
                return ToolResult::fail(format!(
                    "there is no layer `{layer}`; the layers are: {}",
                    score
                        .layers
                        .iter()
                        .map(|l| l.id.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            let patterns: Vec<&praxis_score::Pattern> = score
                .patterns
                .iter()
                .filter(|p| l.clips.iter().any(|c| c.pattern == p.id))
                .collect();
            ToolResult::ok(json!({
                "layer": l,
                "params_with_defaults": instrument_params(&Registry::with_default_packs(), l),
                "patterns": patterns,
            }))
        }
        "score" => match score {
            Some(s) => ToolResult::ok(serde_json::to_value(s).unwrap_or(Value::Null)),
            None => no_score(),
        },
        "takes" => ToolResult::ok(json!({
            "current_take": sound.takes.current,
            "takes": sound.takes.takes.iter().map(take_line).collect::<Vec<_>>(),
        })),
        "changes" => {
            let n = if take <= 0 {
                sound.takes.current
            } else {
                take as u32
            };
            match sound.takes.take(n) {
                Some(t) => ToolResult::ok(json!({
                    "n": t.n,
                    "from": t.from,
                    "who": t.who,
                    "ask": t.ask,
                    "say": t.say,
                    "changed": t.changed,
                    "untouched": t.untouched,
                    "identical_bars": t.identical_bars,
                    "patch": t.patch,
                })),
                None => ToolResult::fail(format!("there is no take {n}")),
            }
        }
        other => ToolResult::fail(format!(
            "unknown part `{other}`: use summary, layer, score, takes or changes"
        )),
    }
}

fn get_schema(which: &str) -> ToolResult {
    let example = |text: &str| serde_json::from_str::<Value>(text).unwrap_or(Value::Null);
    match which {
        "score_plan" => ToolResult::ok(json!({
            "schema": praxis_planner::plan_schema(),
            "example": example(EXAMPLE_PLAN),
        })),
        "edit_plan" => ToolResult::ok(json!({
            "schema": praxis_planner::edit_schema(),
            "example": example(EXAMPLE_EDIT),
        })),
        "score_patch" => ToolResult::ok(json!({
            "schema": praxis_score::patch_schema(),
            "example": {
                "description": "darker felt piano",
                "scope": {},
                "ops": [{"op": "update_layer", "id": "piano", "update": {"effects": [
                    {"id": "piano_eq", "kind": "eq", "params": {"lp_hz": 4000.0}, "bypass": false}
                ]}}],
            },
        })),
        other => ToolResult::fail(format!(
            "unknown document `{other}`: use score_plan, edit_plan or score_patch"
        )),
    }
}

// ------------------------------------------------------------------------------ changing

/// The result of a change to the draft: what changed against the turn's start.
fn changed_payload(draft: &Draft, summary: String) -> Value {
    let changed = match (&draft.base, &draft.score) {
        (Some(b), Some(s)) => json!(changed_layers(b, s)),
        _ => json!({}),
    };
    json!({
        "summary": summary,
        "changed_since_current_take": changed,
        "next": "render_check to hear it by numbers, or change more",
    })
}

fn parse_json(text: &str, what: &str) -> Result<Value, ToolResult> {
    serde_json::from_str(text)
        .map_err(|e| ToolResult::fail(format!("{what} is not valid JSON: {e}")))
}

fn compose_sound(draft: &mut Draft, text: &str) -> ToolResult {
    let plan = match parse_json(text, "plan_json") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let outcome = match compose_plan(&plan, &Catalogue::standard(), draft.seed) {
        Ok(o) => o,
        Err(e) => return ToolResult::fail(e.to_string()),
    };
    let summary = outcome.summary();
    let repairs = serde_json::to_value(&outcome.repairs).unwrap_or(Value::Null);
    let new = NewSound {
        summary: summary.clone(),
        repairs: repairs.clone(),
    };
    match draft.replace(outcome.score.clone(), new) {
        Ok(score) => ToolResult::ok(json!({
            "summary": summary,
            "repairs": repairs,
            "lint": outcome.lint,
            "score": detailed_summary(score),
        })),
        Err(e) => ToolResult::fail(e),
    }
}

fn edit(draft: &mut Draft, text: &str) -> ToolResult {
    let plan = match parse_json(text, "edit_plan_json") {
        Ok(v) => v,
        Err(r) => return r,
    };
    if let Some(why) = draft.frozen() {
        return ToolResult::fail(why);
    }
    let Some(score) = draft.score.clone() else {
        return ToolResult::fail(
            "the sound has no score yet: compose one with compose_sound first",
        );
    };
    let out = match compose_edit(&plan, &score, &Catalogue::standard(), draft.seed) {
        Ok(o) => o,
        Err(e) => {
            let issues = serde_json::to_string(e.issues()).unwrap_or_default();
            return ToolResult::fail(format!("{e}\nissues: {issues}"));
        }
    };
    if !out.changed() {
        return ToolResult::ok(json!({
            "summary": out.summary,
            "changed": false,
            "refused": out.refused,
            "note": "nothing changed",
        }));
    }
    let refused = out.refused.clone();
    let new_layers = out.new_layers.clone();
    let repairs = serde_json::to_value(&out.repairs).unwrap_or(Value::Null);
    if let Err(e) = draft.apply(out.patch) {
        return ToolResult::fail(e);
    }
    let mut v = changed_payload(draft, out.summary);
    v["refused"] = json!(refused);
    v["new_layers"] = json!(new_layers);
    v["repairs"] = repairs;
    ToolResult::ok(v)
}

fn apply_patch(draft: &mut Draft, text: &str) -> ToolResult {
    let value = match parse_json(text, "patch_json") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let patch: ScorePatch = match serde_json::from_value(value) {
        Ok(p) => p,
        Err(e) => {
            return ToolResult::fail(format!(
                "patch_json is not a ScorePatch ({e}); see get_schema `score_patch`"
            ));
        }
    };
    if patch.ops.is_empty() {
        return ToolResult::fail("the patch has no ops");
    }
    let description = if patch.description.trim().is_empty() {
        "patch applied".to_string()
    } else {
        patch.description.clone()
    };
    if let Err(e) = draft.apply(patch) {
        return ToolResult::fail(e);
    }
    ToolResult::ok(changed_payload(draft, description))
}

fn set_params(draft: &mut Draft, layer: &str, params: &Value) -> ToolResult {
    let Some(score) = draft.score.as_ref() else {
        return ToolResult::fail(
            "the sound has no score yet: compose one with compose_sound first",
        );
    };
    let Some(l) = score.layer(layer) else {
        return ToolResult::fail(format!(
            "there is no layer `{layer}`; the layers are: {}",
            score
                .layers
                .iter()
                .map(|l| l.id.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    let registry = Registry::with_default_packs();
    let Some(info) = registry.describe(l.instrument.kind, &l.instrument.preset) else {
        return ToolResult::fail(format!(
            "preset `{}` is not built here, so its parameters are unknown",
            l.instrument.preset
        ));
    };
    let list = params.as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        return ToolResult::fail("name at least one parameter");
    }
    let mut new_params = l.instrument.params.clone();
    let mut said = Vec::new();
    for p in &list {
        let name = p["name"].as_str().unwrap_or("").trim().to_string();
        let Some(value) = p["value"].as_f64().filter(|v| v.is_finite()) else {
            return ToolResult::fail(format!("parameter `{name}` needs a number"));
        };
        let Some(spec) = info.params.iter().find(|s| s.name == name) else {
            return ToolResult::fail(format!(
                "`{}` has no parameter `{name}`; its parameters are: {}",
                l.instrument.preset,
                info.params
                    .iter()
                    .map(|s| format!("{} ({}..{})", s.name, s.min, s.max))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        if !(spec.min..=spec.max).contains(&value) {
            return ToolResult::fail(format!(
                "`{name}` must be within {}..{}, not {value}",
                spec.min, spec.max
            ));
        }
        let old = new_params.get(&name).copied().unwrap_or(spec.default);
        said.push(format!("{name} {old} -> {value}"));
        new_params.insert(name, value);
    }
    let patch = ScorePatch {
        description: format!("{}: {}", layer, said.join(", ")),
        origin: Origin::Model,
        scope: Scope::default(),
        ops: vec![PatchOp::UpdateLayer {
            id: layer.to_string(),
            update: LayerUpdate {
                instrument: Some(InstrumentRef {
                    kind: l.instrument.kind,
                    preset: l.instrument.preset.clone(),
                    params: new_params,
                }),
                ..LayerUpdate::default()
            },
        }],
    };
    let summary = patch.description.clone();
    if let Err(e) = draft.apply(patch) {
        return ToolResult::fail(e);
    }
    ToolResult::ok(changed_payload(draft, summary))
}

// ------------------------------------------------------------------------------ checking

fn render_check(draft: &mut Draft) -> ToolResult {
    let Some(score) = draft.score.clone() else {
        return ToolResult::fail(
            "the sound has no score yet: compose one with compose_sound first",
        );
    };
    let work = draft.dir.join("agent");
    let measured = match render_measure(&score, &work.join("draft.wav"), draft.skip_missing) {
        Ok(m) => m,
        Err(e) => return ToolResult::fail(err_text(&e)),
    };
    // The current take, measured the same way (once per turn).
    if draft.base_measure.is_none()
        && let Some(base) = draft.base.clone()
    {
        match render_measure(&base, &work.join("current.wav"), draft.skip_missing) {
            Ok(m) => draft.base_measure = Some(m.layers),
            Err(e) => return ToolResult::fail(err_text(&e)),
        }
    }
    let identical = Sound::open(&draft.dir)
        .ok()
        .and_then(|s| {
            s.takes
                .take(s.takes.current)
                .map(|t| s.absolute(&t.render.path))
        })
        .and_then(|p| decode(&p).ok())
        .filter(|a| a.sample_rate == measured.sample_rate)
        .map(|a| identical_bars(&score, &a.audio, &measured.master, measured.sample_rate));
    let r = &measured.render;
    let check = &r["check"];
    ToolResult::ok(json!({
        "check": {
            "passed": check["passed"],
            "failed": check["failed"],
            "inconclusive": check["inconclusive"],
            "summary": check["summary"],
            "checks": check["checks"],
        },
        "loudness_lufs": r["loudness_lufs"],
        "true_peak_dbtp": r["true_peak_dbtp"],
        "onsets_on_grid": check["onsets_on_grid"],
        "tempo_bpm": check["tempo_bpm"],
        "key": check["key"],
        "duration_s": r["duration_s"],
        "layers": compare(&measured.layers, draft.base_measure.as_deref().unwrap_or(&[])),
        "identical_bars_to_current_take": identical,
        "skipped_layers": r["skipped_layers"],
        "warnings": r["warnings"],
    }))
}

// ------------------------------------------------------------------------- history tools

fn needs_clean(draft: &Draft) -> Option<ToolResult> {
    if let Some(why) = draft.frozen() {
        return Some(ToolResult::fail(why));
    }
    draft.dirty().then(|| {
        ToolResult::fail(
            "the draft has changes: finish this request with them (end your turn), or call \
             discard_changes first",
        )
    })
}

fn sound_arg(draft: &Draft) -> String {
    draft.dir.display().to_string()
}

fn make_variations(
    draft: &mut Draft,
    layers: &Value,
    start: i64,
    end: i64,
    count: i64,
) -> ToolResult {
    if let Some(r) = needs_clean(draft) {
        return r;
    }
    let bars = match (start, end) {
        (0, 0) => None,
        (s, e) if s >= 1 && e >= s => Some(BarRange {
            start: s as u32,
            end: e as u32,
        }),
        _ => {
            return ToolResult::fail("give bars as start_bar <= end_bar, both from 1 (or 0 and 0)");
        }
    };
    let layers: Vec<String> = layers
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| l.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let req = ops::VariationsRequest {
        project: None,
        score: None,
        sound: Some(sound_arg(draft)),
        scope: Scope { bars, layers },
        n: count.clamp(0, 64) as usize,
        seed: draft.seed,
        out_dir: None,
        render_settings: ops::RenderSettings {
            skip_missing: draft.skip_missing,
            ..ops::RenderSettings::default()
        },
    };
    match ops::variations(&req) {
        Ok(v) => ToolResult::ok(json!({
            "variations": v["variations"].as_array().map(|items| items.iter().map(|i| json!({
                "index": i["index"],
                "summary": i["summary"],
                "changed_bars": i["changed_bars"],
                "changed_layers": i["changed_layers"],
                "checks_passed": i["render"]["check"]["passed"],
                "checks_failed": i["render"]["check"]["failed"],
                "checks_cant_tell": i["render"]["check"]["inconclusive"],
            })).collect::<Vec<_>>()),
            "next": "keep_variation with the index of the one to keep, or leave them",
        })),
        Err(e) => ToolResult::fail(err_text(&e)),
    }
}

fn keep_variation(ctx: &mut ToolCtx, index: i64) -> ToolResult {
    if let Some(r) = needs_clean(ctx.draft) {
        return r;
    }
    if index < 1 {
        return ToolResult::fail("variations are numbered from 1");
    }
    let req = ops::KeepVariationRequest {
        sound: sound_arg(ctx.draft),
        index: index as usize,
        who: Some(AGENT_NAME.to_string()),
        ask: Some(ctx.request.to_string()),
        title: None,
        skip_missing: ctx.draft.skip_missing,
        include_score: false,
    };
    match ops::keep_variation(&req) {
        Ok(v) => {
            let n = v["take"]["n"].as_u64().unwrap_or(0) as u32;
            if let Err(e) = ctx.draft.reload() {
                return ToolResult::fail(err_text(&e));
            }
            ctx.draft.take_made = Some(n);
            ToolResult::ok(json!({
                "take": n,
                "summary": v["take"]["say"],
                "checks_passed": v["take"]["checks"]["passed"],
                "identical_bars": v["take"]["identical_bars"],
            }))
        }
        Err(e) => ToolResult::fail(err_text(&e)),
    }
}

fn restore_take(draft: &mut Draft, n: i64) -> ToolResult {
    if let Some(r) = needs_clean(draft) {
        return r;
    }
    if n < 1 {
        return ToolResult::fail("takes are numbered from 1");
    }
    match ops::restore(&ops::RestoreRequest {
        sound: sound_arg(draft),
        n: n as u32,
    }) {
        Ok(v) => match draft.reload() {
            Ok(()) => ToolResult::ok(json!({
                "current_take": v["current"],
                "restored": v["restored"],
                "summary": format!("take {n} is current again"),
            })),
            Err(e) => ToolResult::fail(err_text(&e)),
        },
        Err(e) => ToolResult::fail(err_text(&e)),
    }
}

fn undo_redo(draft: &mut Draft, action: &str) -> ToolResult {
    if let Some(r) = needs_clean(draft) {
        return r;
    }
    let action = match action {
        "undo" => ops::HistoryAction::Undo,
        "redo" => ops::HistoryAction::Redo,
        other => return ToolResult::fail(format!("unknown action `{other}`: undo or redo")),
    };
    match ops::undo_redo(&ops::UndoRedoRequest {
        sound: sound_arg(draft),
        action,
    }) {
        Ok(v) => match draft.reload() {
            Ok(()) => ToolResult::ok(json!({
                "moved": v["moved"],
                "step": v["description"],
                "current_take": v["current"],
                "can_undo": v["can_undo"],
                "can_redo": v["can_redo"],
            })),
            Err(e) => ToolResult::fail(err_text(&e)),
        },
        Err(e) => ToolResult::fail(err_text(&e)),
    }
}

fn open_view(draft: &mut Draft, view: &str) -> ToolResult {
    let view: ViewName = match serde_json::from_value(json!(view)) {
        Ok(v) => v,
        Err(_) => {
            return ToolResult::fail("unknown view: overview, rhythm, notes, mix or export");
        }
    };
    if draft.dirty() {
        draft.opens = Some(view);
        return ToolResult::ok(json!({"opens": view, "when": "the take is made"}));
    }
    match ops::open_view(&ops::OpenViewRequest {
        sound: sound_arg(draft),
        view,
        who: Some(AGENT_NAME.to_string()),
    }) {
        Ok(v) => ToolResult::ok(json!({"opened": v["view"], "take": v["take"]})),
        Err(e) => ToolResult::fail(err_text(&e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The API's strict-mode limits: at most 20 strict tools, 24 optional parameters and 16
    /// union-typed parameters per request; no numeric or length constraints.
    #[test]
    fn every_tool_is_strict_and_inside_the_limits() {
        let tools = definitions();
        assert!(tools.len() <= 20, "{} tools", tools.len());
        let mut names = std::collections::BTreeSet::new();
        let (mut optional, mut unions) = (0, 0);
        fn walk(v: &Value, optional: &mut usize, unions: &mut usize) {
            if let Some(m) = v.as_object() {
                if m.get("type").is_some_and(|t| t == "object") || m.contains_key("properties") {
                    assert_eq!(m["additionalProperties"], false, "{v}");
                    let props = m["properties"].as_object().unwrap();
                    let req: Vec<&str> = m["required"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|r| r.as_str().unwrap())
                        .collect();
                    for (k, p) in props {
                        if !req.contains(&k.as_str()) {
                            *optional += 1;
                        }
                        if p["type"].is_array() || p.get("anyOf").is_some() {
                            *unions += 1;
                        }
                        walk(p, optional, unions);
                    }
                }
                for k in ["minimum", "maximum", "minLength", "maxLength", "maxItems"] {
                    assert!(!m.contains_key(k), "{k} in {v}");
                }
                if let Some(items) = m.get("items") {
                    walk(items, optional, unions);
                }
            }
        }
        for t in &tools {
            assert_eq!(t["strict"], true, "{t}");
            assert!(names.insert(t["name"].as_str().unwrap().to_string()));
            assert!(t["description"].as_str().unwrap().len() > 40);
            walk(&t["input_schema"], &mut optional, &mut unions);
        }
        assert!(
            optional <= 24 && unions <= 16,
            "{optional} optional, {unions} unions"
        );
        assert_eq!((optional, unions), (0, 0));
    }

    #[test]
    fn examples_parse() {
        assert!(serde_json::from_str::<Value>(EXAMPLE_PLAN).is_ok());
        assert!(serde_json::from_str::<Value>(EXAMPLE_EDIT).is_ok());
        let patch = &get_schema("score_patch").value["example"];
        assert!(serde_json::from_value::<ScorePatch>(patch.clone()).is_ok());
    }
}
