//! Plans written directly by an agent (no brief, no provider): `compose_plan`.

mod common;

use common::check_acceptance1;
use praxis_planner::plan::{PlanContext, PlanIssueCode, validate_plan};
use praxis_planner::{Catalogue, PlanError, ScorePlan, compose_plan, plan_schema};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn example() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("prompts")
        .join("acceptance1.plan.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("plan example")).expect("json")
}

#[test]
fn acceptance1_plan_example_composes() {
    let out = compose_plan(&example(), &Catalogue::standard(), 1).expect("compose");
    check_acceptance1(&out.score).expect("acceptance 1");
    assert!(out.repairs.is_empty(), "{:?}", out.repairs);
    assert_eq!(out.attempts, 0);
    assert_eq!(out.provider_id, "direct-plan");
    assert!(out.lint.is_empty());
    // Deterministic.
    let again = compose_plan(&example(), &Catalogue::standard(), 1).expect("compose");
    assert_eq!(again.score, out.score);
}

#[test]
fn schema_and_catalogue_are_published() {
    let s = plan_schema();
    assert_eq!(s["type"], "object");
    for f in ["emotion", "key", "layers", "sections", "requested"] {
        assert!(s["properties"].get(f).is_some(), "{f}");
    }
    let c = Catalogue::standard().to_json();
    let names: Vec<&str> = c["presets"]
        .as_array()
        .expect("presets")
        .iter()
        .filter_map(|p| p["name"].as_str())
        .collect();
    assert!(names.contains(&"felt_piano") && names.contains(&"soft_kick"));
    assert_eq!(
        c["presets"][0]["range"].as_array().map(|a| a.len()),
        Some(2)
    );
    assert!(
        c["trailer_classes"]
            .as_array()
            .is_some_and(|a| a.contains(&json!("riser")))
    );
    // The example parses into the typed plan.
    let _: ScorePlan = serde_json::from_value(example()).expect("typed plan");
}

#[test]
fn deterministic_repairs_are_recorded() {
    let mut plan = example();
    // Off-scale explicit note, off-grid step, a pad nobody asked for, a sustaining layer without
    // overlap, and a missing section for the second half.
    plan["layers"][0]["motif"]["notes"] = json!([
        {"step": 0, "pitch": 57, "accent": "strong"},
        {"step": 2.4, "pitch": 61},
        {"step": 4, "pitch": 64, "accent": "bright"}
    ]);
    plan["layers"][0]["motif"]["sustain_steps"] = json!(1.0);
    plan["layers"].as_array_mut().expect("layers").push(json!({
        "id": "wash",
        "class": "melody",
        "preset": {"kind": "synth", "name": "granular_texture"},
        "motif": {"grid": "eighth", "spoken": "DON ~ ~ ~ ~ ~ ~ ~", "degrees": [0]}
    }));
    plan["sections"][0]["layers"] = json!(["piano", "wash"]);
    plan["sections"].as_array_mut().expect("sections").pop();
    let out = compose_plan(&plan, &Catalogue::standard(), 1).expect("compose");
    let codes: BTreeSet<PlanIssueCode> = out.repairs.iter().map(|r| r.code).collect();
    for c in [
        PlanIssueCode::OffScale,
        PlanIssueCode::OffGrid,
        PlanIssueCode::ClassMismatch,
        PlanIssueCode::AvoidedClass,
        PlanIssueCode::Sustain,
        PlanIssueCode::Sections,
    ] {
        assert!(
            codes.contains(&c),
            "missing repair {c:?}: {:?}",
            out.repairs
        );
    }
    assert!(out.score.layer("wash").is_none(), "the pad was dropped");
    assert!(praxis_score::validate::validate(&out.score).is_valid());
    let piano = out.score.pattern("piano_motif").expect("pattern");
    assert!(piano.notes.iter().all(|n| out.score.key.contains(n.pitch)));
    assert!(piano.notes.iter().any(|n| n.step == 2));
    assert!(piano.notes.iter().all(|n| n.length > 1.0));
    assert_eq!(out.rounds.len(), 1);
    assert!(
        out.score
            .meta
            .provenance
            .get("repair_codes")
            .is_some_and(|s| s.contains("off_scale"))
    );
}

#[test]
fn requested_pads_are_allowed_when_listed() {
    let mut plan = example();
    plan["requested"] = json!(["pad"]);
    plan["layers"].as_array_mut().expect("layers").push(json!({
        "id": "wash",
        "class": "pad",
        "preset": {"kind": "synth", "name": "granular_texture"},
        "motif": {"grid": "eighth", "spoken": "DON ~ ~ ~ ~ ~ ~ ~", "degrees": [0], "follow_chords": true}
    }));
    plan["sections"][1]["layers"] = json!(["piano", "kick", "wash"]);
    let out = compose_plan(&plan, &Catalogue::standard(), 1).expect("compose");
    assert!(out.score.layer("wash").is_some());
    assert!(
        !out.score
            .constraints
            .avoid
            .contains(&praxis_score::GeneratorClass::Pad)
    );
}

#[test]
fn schema_errors_point_into_the_plan() {
    let mut plan = example();
    plan["layers"][1]["preset"]["kind"] = json!("drums");
    plan["tempo"] = json!(128);
    let err = compose_plan(&plan, &Catalogue::standard(), 1).unwrap_err();
    let PlanError::Invalid(issues) = err else {
        panic!("expected Invalid, got {err:?}");
    };
    let paths: Vec<&str> = issues.iter().map(|i| i.path.as_str()).collect();
    assert!(paths.contains(&"/layers/1/preset"), "{paths:?}");
    assert!(paths.contains(&"/tempo"), "{paths:?}");
    let kind = issues
        .iter()
        .find(|i| i.path == "/layers/1/preset")
        .expect("preset issue");
    assert!(kind.message.contains("sampler"), "{}", kind.message);
    let tempo = issues
        .iter()
        .find(|i| i.path == "/tempo")
        .expect("tempo issue");
    assert!(tempo.message.contains("tempo_bpm"), "{}", tempo.message);
}

#[test]
fn validation_messages_name_allowed_values() {
    let mut plan: ScorePlan = serde_json::from_value(example()).expect("plan");
    plan.layers[0].preset.name = "grand_piano_xl".into();
    plan.layers[1].motif.grid = praxis_score::Grid::EighthTriplet;
    plan.time_signature = praxis_score::TimeSignature {
        numerator: 5,
        denominator: 8,
    };
    plan.layers[0].motif.notes = vec![praxis_planner::plan::PlanNote {
        step: 0.0,
        pitch: 61,
        accent: praxis_score::Accent::Normal,
        velocity: None,
        length: None,
    }];
    let requested = BTreeSet::new();
    let catalogue = Catalogue::standard();
    let issues = validate_plan(
        &plan,
        &PlanContext {
            catalogue: &catalogue,
            requested: &requested,
        },
    );
    let msg = |code| {
        issues
            .iter()
            .find(|i| i.code == code)
            .map(|i| i.message.clone())
            .unwrap_or_default()
    };
    assert!(msg(PlanIssueCode::UnknownPreset).contains("felt_piano, upright_piano"));
    assert!(msg(PlanIssueCode::Grid).contains("allowed grids here: eighth"));
    let off = issues
        .iter()
        .find(|i| i.code == PlanIssueCode::OffScale)
        .expect("off-scale issue");
    assert_eq!(off.path, "/layers/0/motif/notes/0/pitch");
    assert!(
        off.message.contains("A, B, C, D, E, F, G"),
        "{}",
        off.message
    );
}
