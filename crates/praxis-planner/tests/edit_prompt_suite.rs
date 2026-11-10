//! Edit prompt suite: every tests/prompts/edit_*.json plans its base brief with the offline
//! provider, applies the edit instruction with `plan_edit`, and automatic validators check the
//! result: valid and lint-clean, in key and on grid, nothing banned, every layer outside the
//! patch scope structurally identical, every bar outside the edited bars note-identical, an
//! exact inverse, determinism, and the case's expectations.
//!
//! The base brief and its `expect` also run in `prompt_suite.rs` (which reads every prompt file).

mod common;

use common::SR;
use praxis_planner::brief::lint_context;
use praxis_planner::catalogue::class_name;
use praxis_planner::edit::plan_edit;
use praxis_planner::plan::PlanContext;
use praxis_planner::{Catalogue, EditOutcome, RuleProvider, plan_brief};
use praxis_score::apply::apply;
use praxis_score::lint::lint;
use praxis_score::timeline::{NoteEvent, expand_notes};
use praxis_score::validate::{unpitched_class, validate};
use praxis_score::{BarRange, Layer, Mode, Score};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn edit_files() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("prompts");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("prompts dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("edit_"))
                && !p.to_string_lossy().ends_with(".plan.json")
        })
        .collect();
    files.sort();
    files
}

fn events_of(score: &Score, id: &str) -> Vec<NoteEvent> {
    let Some(li) = score.layers.iter().position(|l| l.id == id) else {
        return vec![];
    };
    expand_notes(score, SR)
        .into_iter()
        .filter(|e| e.layer == li)
        .map(|mut e| {
            e.layer = 0;
            e
        })
        .collect()
}

fn inside(bar: u32, r: BarRange) -> bool {
    bar >= r.start && bar <= r.end
}

/// Layers of `score` a case refers to by id or preset word ("kick", "hats", "bell").
fn layers_named<'a>(score: &'a Score, name: &str) -> Vec<&'a Layer> {
    let n = name.trim_end_matches('s');
    score
        .layers
        .iter()
        .filter(|l| l.id == name || l.id == n || l.instrument.preset.contains(n))
        .collect()
}

fn names(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn generic_checks(before: &Score, out: &EditOutcome, catalogue: &Catalogue) -> Result<(), String> {
    let after = &out.score;
    let report = validate(after);
    if !report.is_valid() {
        return Err(format!("edited score invalid: {:?}", report.errors));
    }
    let requested = BTreeSet::new();
    let ctx = PlanContext {
        catalogue,
        requested: &requested,
    };
    let lints = lint(after, &lint_context(after, &ctx));
    if !lints.is_empty() {
        return Err(format!("lint warnings after the edit: {lints:?}"));
    }
    for p in &after.patterns {
        let spb = after
            .time_signature
            .steps_per_bar(p.grid)
            .ok_or("grid does not divide the bar")?;
        if p.length_steps % spb != 0 {
            return Err(format!("pattern {} is not whole bars", p.id));
        }
        if p.notes
            .iter()
            .any(|n| n.offset != 0.0 || n.step >= p.length_steps)
        {
            return Err(format!("pattern {} has off-grid notes", p.id));
        }
    }
    for l in &after.layers {
        if after.constraints.avoid.contains(&l.class) {
            return Err(format!(
                "layer {} has avoided class {}",
                l.id,
                class_name(l.class)
            ));
        }
        if unpitched_class(l.class) || after.key.mode == Mode::Chromatic {
            continue;
        }
        for e in events_of(after, &l.id) {
            if !after.key.contains(e.pitch) {
                return Err(format!("layer {} note {} is off the key", l.id, e.pitch));
            }
        }
    }
    if out.patch.origin != praxis_score::patch::Origin::Model {
        return Err("patch origin is not model".into());
    }
    // Isolation: layers outside the patch's layer scope are structurally identical; layers in it
    // are note-identical outside the edited bars.
    for l in &before.layers {
        let in_scope = out.patch.scope.layers.contains(&l.id);
        if !in_scope {
            if after.layer(&l.id) != Some(l) {
                return Err(format!("layer {} outside the scope changed", l.id));
            }
            continue;
        }
        if l.locked {
            return Err(format!("locked layer {} is in scope", l.id));
        }
        if out.scope.bars.is_some() {
            let a: Vec<NoteEvent> = events_of(before, &l.id)
                .into_iter()
                .filter(|e| !inside(e.bar, out.bars))
                .collect();
            let b: Vec<NoteEvent> = events_of(after, &l.id)
                .into_iter()
                .filter(|e| !inside(e.bar, out.bars))
                .collect();
            if a != b {
                return Err(format!(
                    "layer {} changed outside bars {}-{}",
                    l.id, out.bars.start, out.bars.end
                ));
            }
        }
    }
    for id in &out.new_layers {
        if events_of(after, id)
            .iter()
            .any(|e| !inside(e.bar, out.bars))
        {
            return Err(format!("new layer {id} plays outside the edited bars"));
        }
    }
    let undone = apply(after, &out.inverse).map_err(|e| format!("inverse failed: {e}"))?;
    if undone.score != *before {
        return Err("inverse does not restore the score".into());
    }
    Ok(())
}

fn check_case(case: &Value) -> Result<String, String> {
    let catalogue = Catalogue::standard();
    let provider = RuleProvider::new();
    let brief = case["brief"].as_str().ok_or("brief")?;
    let seed = case["seed"].as_u64().unwrap_or(0);
    let mut before = plan_brief(brief, &provider, &catalogue, seed)
        .map_err(|e| e.to_string())?
        .score;
    let edit = &case["edit"];
    for name in names(&edit["lock"]) {
        for l in before.layers.iter_mut() {
            if l.id == name {
                l.locked = true;
            }
        }
    }
    let instruction = edit["instruction"].as_str().ok_or("instruction")?;
    let eseed = edit["seed"].as_u64().unwrap_or(0);
    let out =
        plan_edit(&before, instruction, &provider, &catalogue, eseed).map_err(|e| e.to_string())?;
    generic_checks(&before, &out, &catalogue)?;
    let again =
        plan_edit(&before, instruction, &provider, &catalogue, eseed).map_err(|e| e.to_string())?;
    if again != out {
        return Err("not deterministic".into());
    }
    if out.attempts != 1 {
        return Err(format!("not first try: {} attempts", out.attempts));
    }
    let after = &out.score;
    let ex = &edit["expect"];
    if let Some(i) = ex.get("intent").and_then(Value::as_str) {
        let got = serde_json::to_value(out.plan.intent).map_err(|e| e.to_string())?;
        if got != i {
            return Err(format!("intent {got} != {i}"));
        }
    }
    if let Some(b) = ex.get("bars").and_then(Value::as_array) {
        let want = BarRange {
            start: b[0].as_u64().unwrap_or(0) as u32,
            end: b[1].as_u64().unwrap_or(0) as u32,
        };
        if out.bars != want || out.scope.bars != Some(want) {
            return Err(format!(
                "edited bars {:?} (scope {:?}) != {want:?}",
                out.bars, out.scope.bars
            ));
        }
    }
    if ex.get("whole").and_then(Value::as_bool) == Some(true) && out.scope.bars.is_some() {
        return Err(format!(
            "expected a whole-score edit, got {:?}",
            out.scope.bars
        ));
    }
    if let Some(layers) = ex.get("scope_layers") {
        let want = names(layers);
        if out.scope.layers != want {
            return Err(format!("scope layers {:?} != {want:?}", out.scope.layers));
        }
    }
    if let Some(n) = ex.get("new_layers").and_then(Value::as_u64)
        && out.new_layers.len() as u64 != n
    {
        return Err(format!("new layers {:?}, expected {n}", out.new_layers));
    }
    let new: Vec<&Layer> = out
        .new_layers
        .iter()
        .filter_map(|id| after.layer(id))
        .collect();
    if let Some(p) = ex.get("new_preset").and_then(Value::as_str)
        && !new.iter().any(|l| l.instrument.preset == p)
    {
        return Err(format!("no new `{p}` layer"));
    }
    if let Some(c) = ex.get("new_class").and_then(Value::as_str)
        && !new.iter().any(|l| class_name(l.class) == c)
    {
        return Err(format!("no new {c} layer"));
    }
    for l in &new {
        if events_of(after, &l.id).is_empty() {
            return Err(format!("new layer {} is silent", l.id));
        }
    }
    if ex.get("new_dynamics_soft").and_then(Value::as_bool) == Some(true) {
        // Subtle: never louder than the loudest note already there, and turned down.
        let ceiling = expand_notes(&before, SR)
            .iter()
            .map(|e| e.velocity)
            .fold(0.0f32, f32::max);
        for l in &new {
            let loudest = events_of(after, &l.id)
                .iter()
                .map(|e| e.velocity)
                .fold(0.0f32, f32::max);
            if loudest > ceiling + 1e-6 || l.gain_db >= 0.0 {
                return Err(format!(
                    "new layer {} is not subtle (velocity {loudest} > {ceiling}, gain {} dB)",
                    l.id, l.gain_db
                ));
            }
        }
    }
    for name in names(&ex["removed_layers"]) {
        let ids: Vec<String> = layers_named(&before, &name)
            .iter()
            .map(|l| l.id.clone())
            .collect();
        if ids.is_empty() || ids.iter().any(|id| after.layer(id).is_some()) {
            return Err(format!("`{name}` was not removed"));
        }
    }
    for name in names(&ex["silent_in_scope"]) {
        for l in layers_named(&before, &name) {
            if events_of(after, &l.id)
                .iter()
                .any(|e| inside(e.bar, out.bars))
            {
                return Err(format!("`{}` still plays in the edited bars", l.id));
            }
        }
    }
    for name in names(&ex["changed_in_scope"]) {
        for l in layers_named(&before, &name) {
            let pick = |s: &Score| -> Vec<NoteEvent> {
                events_of(s, &l.id)
                    .into_iter()
                    .filter(|e| inside(e.bar, out.bars))
                    .collect()
            };
            if pick(&before) == pick(after) {
                return Err(format!("`{}` did not change in the edited bars", l.id));
            }
        }
    }
    if let Some(m) = ex.get("gain_change").and_then(Value::as_object) {
        for (id, d) in m {
            let (a, b) = (
                before.layer(id).map(|l| l.gain_db).unwrap_or(0.0),
                after.layer(id).map(|l| l.gain_db).unwrap_or(0.0),
            );
            if ((b - a) as f64 - d.as_f64().unwrap_or(0.0)).abs() > 1e-4 {
                return Err(format!("`{id}` gain {a} -> {b}, expected change {d}"));
            }
        }
    }
    if ex.get("unchanged").and_then(Value::as_bool) == Some(true)
        && (out.changed() || *after != before)
    {
        return Err("expected no change".into());
    }
    if let Some(r) = ex.get("refused") {
        let want = names(r);
        if out.refused != want {
            return Err(format!("refused {:?} != {want:?}", out.refused));
        }
    }
    if let Some(s) = ex.get("summary_contains").and_then(Value::as_str)
        && !out.summary.contains(s)
    {
        return Err(format!("summary lacks `{s}`: {}", out.summary));
    }
    Ok(out.summary)
}

#[test]
fn edit_prompt_suite_passes() {
    let files = edit_files();
    assert!(
        files.len() >= 12,
        "edit suite needs at least 12 cases, has {}",
        files.len()
    );
    let mut failures = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).expect("read case");
        let case: Value = serde_json::from_str(&text).expect("case json");
        let name = f.file_stem().map(|s| s.to_string_lossy().to_string());
        match check_case(&case) {
            Ok(summary) => println!("PASS {name:?}: {summary}"),
            Err(e) => {
                println!("FAIL {name:?}: {e}");
                failures.push(format!("{name:?}: {e}"));
            }
        }
    }
    println!(
        "edit prompt suite: {}/{} passed",
        files.len() - failures.len(),
        files.len()
    );
    assert!(failures.is_empty(), "failures:\n{}", failures.join("\n"));
}
