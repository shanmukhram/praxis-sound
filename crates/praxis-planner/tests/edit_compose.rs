//! `compose_edit`: an agent writes the EditPlan itself (no provider). The example plan for
//! agents, schema errors with pointers, actionable issues, and the same isolation as plan_edit.

mod common;

use common::SR;
use praxis_planner::edit::{EditIssueCode, edit_schema};
use praxis_planner::{Catalogue, EditError, RuleProvider, compose_edit, plan_brief, plan_edit};
use praxis_score::timeline::expand_notes;
use praxis_score::{BarRange, Score};
use serde_json::{Value, json};
use std::path::PathBuf;

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid with accents and overlapping \
sustain. Bars 9-16: the identical piano part plus a soft muted kick underneath. No new melody, \
no fills, no risers, sweeps, impacts or pads.";

fn acceptance1() -> Score {
    plan_brief(BRIEF, &RuleProvider::new(), &Catalogue::standard(), 1)
        .expect("plan")
        .score
}

fn example_plan() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/prompts/edit_last_four_melody.plan.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("example")).expect("json")
}

fn issues(r: Result<praxis_planner::EditOutcome, EditError>) -> Vec<(EditIssueCode, String)> {
    match r {
        Err(EditError::Invalid { issues, .. }) => {
            issues.into_iter().map(|i| (i.code, i.path)).collect()
        }
        Err(e) => panic!("{e}"),
        Ok(o) => panic!("unexpectedly accepted: {}", o.summary),
    }
}

#[test]
fn the_agent_example_plan_composes() {
    let s = acceptance1();
    let out = compose_edit(&example_plan(), &s, &Catalogue::standard(), 4).expect("compose");
    println!("{}", out.summary);
    assert_eq!(out.bars, BarRange { start: 13, end: 16 });
    assert_eq!(out.attempts, 0);
    assert_eq!(out.provider_id, "direct-plan");
    let head = |sc: &Score| -> Vec<_> {
        expand_notes(sc, SR)
            .into_iter()
            .filter(|e| e.bar <= 12)
            .collect()
    };
    assert_eq!(head(&s), head(&out.score));
    let id = &out.new_layers[0];
    assert_eq!(
        out.score.layer(id).map(|l| l.instrument.preset.as_str()),
        Some("upright_piano")
    );
}

#[test]
fn compose_matches_plan_edit_for_the_same_plan() {
    // The offline rules' plan for an instruction, composed directly, gives the same Score.
    let s = acceptance1();
    let c = Catalogue::standard();
    let via_rules = plan_edit(
        &s,
        "change the last four bars to add a subtle melody",
        &RuleProvider::new(),
        &c,
        9,
    )
    .expect("plan_edit");
    let plan = serde_json::to_value(&via_rules.plan).expect("plan json");
    let direct = compose_edit(&plan, &s, &c, 9).expect("compose");
    assert_eq!(direct.score, via_rules.score);
    assert_eq!(direct.patch, via_rules.patch);
}

#[test]
fn bad_plans_get_pointed_issues() {
    let s = acceptance1();
    let c = Catalogue::standard();
    let cases: Vec<(Value, EditIssueCode, &str)> = vec![
        (json!({"intent": "explode"}), EditIssueCode::Schema, ""),
        (
            json!({"intent": "add_layer", "unknown_field": 1}),
            EditIssueCode::Schema,
            "",
        ),
        (
            json!({"intent": "add_melody", "scope": {"phrase": "somewhere nice"}}),
            EditIssueCode::Scope,
            "/scope/phrase",
        ),
        (
            json!({"intent": "add_melody", "scope": {"bars": {"start": 13, "end": 20}}}),
            EditIssueCode::Scope,
            "/scope/bars",
        ),
        (
            json!({"intent": "add_melody", "scope": {"phrase": "bars 5-12", "keep": "bars 7-9"}}),
            EditIssueCode::Keep,
            "/scope",
        ),
        (
            json!({"intent": "remove_layer", "layers": ["violin"]}),
            EditIssueCode::UnknownLayer,
            "/layers/0",
        ),
        (
            json!({"intent": "remove_layer"}),
            EditIssueCode::NoTarget,
            "/layers",
        ),
        (
            json!({"intent": "add_layer", "params": {"preset": "theremin"}}),
            EditIssueCode::UnknownPreset,
            "/params/preset",
        ),
        (
            json!({"intent": "add_layer", "params": {"preset": "riser"}}),
            EditIssueCode::AvoidedClass,
            "/params/preset",
        ),
        (
            json!({"intent": "change_instrument", "layers": ["kick"], "params": {"preset": "bell"}}),
            EditIssueCode::UnknownPreset,
            "/params/preset",
        ),
        (
            json!({"intent": "rewrite_motif", "scope": {"phrase": "bars 1-4"}, "layers": ["kick"]}),
            EditIssueCode::NoTarget,
            "/layers",
        ),
    ];
    for (plan, code, path) in cases {
        let got = issues(compose_edit(&plan, &s, &c, 0));
        assert!(
            got.iter().any(|(c, p)| *c == code && p == path),
            "{plan}: expected {code:?} at {path:?}, got {got:?}"
        );
    }
    // Messages name the allowed values.
    match compose_edit(
        &json!({"intent": "remove_layer", "layers": ["violin"]}),
        &s,
        &c,
        0,
    ) {
        Err(e) => {
            let text = e.to_string();
            assert!(
                text.contains("`piano`") && text.contains("`kick`"),
                "{text}"
            );
        }
        Ok(_) => panic!("accepted"),
    }
}

#[test]
fn the_schema_is_published() {
    let schema = edit_schema();
    let text = schema.to_string();
    for word in [
        "add_melody",
        "rewrite_motif",
        "change_instrument",
        "change_dynamics",
        "remove_layer",
        "thin_out",
        "add_layer",
        "keep",
        "phrase",
    ] {
        assert!(text.contains(word), "schema lacks {word}");
    }
}
