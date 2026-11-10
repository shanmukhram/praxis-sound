//! Scoped edits through the offline rules and through a scripted provider: common phrasings,
//! locked layers, the repair loop (max 3 rounds) and the never-widen rule.

mod common;

use common::SR;
use praxis_planner::edit::{EditIntent, EditIssueCode, plan_edit};
use praxis_planner::provider::{Provider, ProviderError, ToolRequest};
use praxis_planner::{Catalogue, EditError, RuleProvider, plan_brief};
use praxis_score::apply::apply;
use praxis_score::patch::{Origin, PatchOp};
use praxis_score::project::Project;
use praxis_score::timeline::expand_notes;
use praxis_score::{BarRange, Score};
use serde_json::{Value, json};
use std::sync::Mutex;

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid with accents and overlapping \
sustain. Bars 9-16: the identical piano part plus a soft muted kick underneath. No new melody, \
no fills, no risers, sweeps, impacts or pads.";

fn acceptance1() -> Score {
    plan_brief(BRIEF, &RuleProvider::new(), &Catalogue::standard(), 1)
        .expect("plan")
        .score
}

fn locked(mut s: Score, id: &str) -> Score {
    s.layers
        .iter_mut()
        .find(|l| l.id == id)
        .expect("layer")
        .locked = true;
    s
}

fn edit(score: &Score, instruction: &str) -> praxis_planner::EditOutcome {
    plan_edit(
        score,
        instruction,
        &RuleProvider::new(),
        &Catalogue::standard(),
        5,
    )
    .unwrap_or_else(|e| panic!("{instruction:?}: {e}"))
}

#[test]
fn make_the_kick_softer() {
    let s = acceptance1();
    let out = edit(&s, "make the kick softer");
    println!("{}", out.summary);
    assert_eq!(out.plan.intent, EditIntent::ChangeDynamics);
    assert_eq!(out.patch.scope.bars, None);
    assert_eq!(out.patch.scope.layers, ["kick"]);
    let before = s.layer("kick").expect("kick").gain_db;
    let after = out.score.layer("kick").expect("kick").gain_db;
    assert!((before - 4.0 - after).abs() < 1e-6, "{before} -> {after}");
    assert_eq!(s.layer("piano"), out.score.layer("piano"));
    // Notes are unchanged; only the level moved.
    assert_eq!(expand_notes(&s, SR), expand_notes(&out.score, SR));
    assert_eq!(apply(&out.score, &out.inverse).expect("undo").score, s);
}

#[test]
fn softer_in_bars_only_scales_those_bars() {
    let s = acceptance1();
    let out = edit(&s, "make the kick a little softer in bars 13-16");
    assert_eq!(out.patch.scope.bars, Some(BarRange { start: 13, end: 16 }));
    let ev = |sc: &Score, a: u32, b: u32| -> Vec<(u32, f32)> {
        expand_notes(sc, SR)
            .into_iter()
            .filter(|e| e.layer == 1 && e.bar >= a && e.bar <= b)
            .map(|e| (e.bar, e.velocity))
            .collect()
    };
    assert_eq!(ev(&s, 1, 12), ev(&out.score, 1, 12));
    let (old, new) = (ev(&s, 13, 16), ev(&out.score, 13, 16));
    assert_eq!(old.len(), new.len());
    assert!(old.iter().zip(&new).all(|(o, n)| n.1 < o.1));
    assert_eq!(
        s.layer("kick").map(|l| l.gain_db),
        out.score.layer("kick").map(|l| l.gain_db)
    );
}

#[test]
fn remove_the_kick() {
    let s = acceptance1();
    let out = edit(&s, "remove the kick");
    println!("{}", out.summary);
    assert_eq!(out.plan.intent, EditIntent::RemoveLayer);
    assert!(out.score.layer("kick").is_none());
    assert!(out.score.pattern("kick_soft").is_none() || s.pattern("kick_soft").is_none());
    assert_eq!(s.layer("piano"), out.score.layer("piano"));
    assert_eq!(apply(&out.score, &out.inverse).expect("undo").score, s);

    // In some bars only: those bars go silent, the rest stay.
    let out = edit(&s, "remove the kick in the last four bars");
    let kick = out.score.layer("kick").expect("kick stays");
    assert!(kick.clips.iter().all(|c| c.bars.end <= 12));
    assert!(matches!(out.patch.ops[0], PatchOp::ClearBars { .. }));
}

#[test]
fn keep_the_first_half_scopes_the_second() {
    let s = acceptance1();
    let out = edit(&s, "keep the first half, add a shaker");
    assert_eq!(out.patch.scope.bars, Some(BarRange { start: 9, end: 16 }));
    let id = &out.new_layers[0];
    let l = out.score.layer(id).expect("shaker");
    assert_eq!(l.instrument.preset, "shaker");
    assert!(l.clips.iter().all(|c| c.bars.start >= 9));

    // A phrase that overlaps the kept bars is narrowed, never widened.
    let out = edit(&s, "keep the first half, thin out the piano in bars 5-16");
    assert_eq!(out.bars, BarRange { start: 9, end: 16 });
    assert!(out.summary.contains("Narrowed"), "{}", out.summary);
}

#[test]
fn locked_layers_are_refused_and_explained() {
    let s = locked(acceptance1(), "kick");
    for instruction in [
        "remove the kick",
        "make the kick softer",
        "bars 13-16: thin out the kick",
    ] {
        let out = edit(&s, instruction);
        println!("{instruction:?} -> {}", out.summary);
        assert!(!out.changed(), "{instruction:?} changed a locked layer");
        assert_eq!(out.score, s);
        assert_eq!(out.refused, ["kick"]);
        assert!(out.summary.contains("locked"), "{}", out.summary);
    }
    // An unnamed edit over a range changes the unlocked layers and says the kick was left alone.
    let out = edit(&s, "make the last four bars softer");
    assert!(out.changed());
    assert_eq!(out.patch.scope.layers, ["piano"]);
    assert_eq!(out.score.layer("kick"), s.layer("kick"));
    assert!(out.summary.contains("`kick` alone"), "{}", out.summary);
    // Adding a layer never touches the locked one.
    let out = edit(&s, "change the last four bars to add a subtle melody");
    assert_eq!(out.score.layer("kick"), s.layer("kick"));
    assert!(!out.patch.scope.layers.contains(&"kick".to_string()));
}

#[test]
fn swap_and_rewrite() {
    let s = acceptance1();
    let out = edit(&s, "swap the piano for bells in the second half");
    println!("{}", out.summary);
    assert_eq!(out.plan.intent, EditIntent::ChangeInstrument);
    assert_eq!(out.patch.scope.bars, Some(BarRange { start: 9, end: 16 }));
    let bells = out.score.layer(&out.new_layers[0]).expect("bell layer");
    assert_eq!(bells.instrument.preset, "bell");
    let piano = out.score.layer("piano").expect("piano");
    assert!(piano.clips.iter().all(|c| c.bars.end <= 8));

    let out = edit(&s, "use bells instead of the piano");
    assert_eq!(
        out.score
            .layer("piano")
            .map(|l| l.instrument.preset.as_str()),
        Some("bell")
    );
}

#[test]
fn project_edit_commits_and_undoes() {
    let s = acceptance1();
    let mut project = Project::new(s.clone());
    let out = praxis_planner::plan_project_edit(
        &mut project,
        "change the last four bars to add a subtle melody",
        &RuleProvider::new(),
        &Catalogue::standard(),
        5,
    )
    .expect("edit");
    assert_eq!(project.score, out.score);
    assert_eq!(project.undo.len(), 1);
    project.undo().expect("undo");
    assert_eq!(project.score, s);
}

/// A provider that answers with scripted tool inputs and records how often it was asked.
struct Scripted {
    answers: Vec<Value>,
    calls: Mutex<usize>,
}

impl Scripted {
    fn new(answers: Vec<Value>) -> Scripted {
        Scripted {
            answers,
            calls: Mutex::new(0),
        }
    }
    fn calls(&self) -> usize {
        *self.calls.lock().expect("lock")
    }
}

impl Provider for Scripted {
    fn id(&self) -> String {
        "scripted".into()
    }
    fn call_tool(&self, req: &ToolRequest) -> Result<Value, ProviderError> {
        assert_eq!(req.tool_name, "edit_score");
        let mut n = self.calls.lock().expect("lock");
        let i = (*n).min(self.answers.len() - 1);
        *n += 1;
        if i > 0 {
            // Every repair round carries the issues back.
            let last = req.messages.last().expect("message");
            assert!(last.content.contains("problems"), "{}", last.content);
        }
        Ok(self.answers[i].clone())
    }
}

#[test]
fn repair_rounds_fix_a_plan_and_never_widen() {
    let s = acceptance1();
    let c = Catalogue::standard();
    let p = Scripted::new(vec![
        // Unknown layer; the scope (bars 13-16) is bound from here on.
        json!({"intent": "rewrite_motif", "scope": {"phrase": "last 4 bars"}, "layers": ["violin"]}),
        // Widened to the second half: refused.
        json!({"intent": "rewrite_motif", "scope": {"phrase": "second half"}, "layers": ["piano"]}),
        // Inside the bound: accepted.
        json!({"intent": "rewrite_motif", "scope": {"bars": {"start": 13, "end": 16}}, "layers": ["piano"]}),
    ]);
    let out = plan_edit(&s, "rewrite the end", &p, &c, 2).expect("repaired");
    assert_eq!(out.attempts, 3);
    assert_eq!(out.rounds.len(), 2);
    assert_eq!(out.rounds[0][0].code, EditIssueCode::UnknownLayer);
    assert_eq!(out.rounds[1][0].code, EditIssueCode::Widened);
    assert_eq!(out.patch.scope.bars, Some(BarRange { start: 13, end: 16 }));
    assert_eq!(out.patch.origin, Origin::Model);
    assert_eq!(out.provider_id, "scripted");
}

#[test]
fn at_most_three_repair_rounds() {
    let s = acceptance1();
    let c = Catalogue::standard();
    let answers: Vec<Value> = (0..10)
        .map(|i| json!({"intent": "remove_layer", "layers": [format!("ghost{i}")]}))
        .collect();
    let p = Scripted::new(answers);
    let err = plan_edit(&s, "remove the ghost", &p, &c, 0).expect_err("never valid");
    assert_eq!(p.calls(), 4, "one call plus three repair rounds");
    match err {
        EditError::Invalid { issues, rounds } => {
            assert_eq!(rounds.len(), 4);
            assert_eq!(issues[0].code, EditIssueCode::UnknownLayer);
            assert!(issues[0].message.contains("piano"), "{}", issues[0].message);
        }
        e => panic!("{e}"),
    }
    // The same plan twice is not asked a third time.
    let p = Scripted::new(vec![json!({"intent": "remove_layer", "layers": ["ghost"]})]);
    assert!(plan_edit(&s, "remove the ghost", &p, &c, 0).is_err());
    assert_eq!(p.calls(), 2);
}

#[test]
fn a_locked_layer_named_by_a_model_is_never_in_scope() {
    let s = locked(acceptance1(), "piano");
    let p = Scripted::new(vec![json!({
        "intent": "rewrite_motif",
        "scope": {"phrase": "last four bars"},
        "layers": ["piano", "kick"]
    })]);
    let out = plan_edit(&s, "rewrite both", &p, &Catalogue::standard(), 1).expect("edit");
    assert_eq!(out.patch.scope.layers, ["kick"]);
    assert_eq!(out.refused, ["piano"]);
    assert_eq!(out.score.layer("piano"), s.layer("piano"));
    assert!(out.summary.contains("locked"));
}
