//! Score v1 round-trip, schema and validation tests on the shipped fixtures.

use praxis_score::{Score, validate::validate};
use std::path::PathBuf;

fn fixture(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

const FIXTURES: &[&str] = &[
    "empty.score.json",
    "acceptance1_bell.score.json",
    "acceptance1.score.json",
];

#[test]
fn fixtures_validate_and_round_trip() {
    for f in FIXTURES {
        let s = praxis_score::from_json(&fixture(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
        let text = praxis_score::to_json(&s);
        let back: Score = serde_json::from_str(&text).unwrap();
        assert_eq!(s, back, "{f} round trip");
        assert_eq!(
            text,
            praxis_score::to_json(&back),
            "{f} canonical form is stable"
        );
    }
}

#[test]
fn fixtures_match_published_schema() {
    let schema = praxis_score::score_schema();
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    for f in FIXTURES {
        let v: serde_json::Value = serde_json::from_str(&fixture(f)).unwrap();
        let errs: Vec<String> = validator.iter_errors(&v).map(|e| e.to_string()).collect();
        assert!(errs.is_empty(), "{f}: {errs:?}");
    }
}

#[test]
fn published_schema_is_current() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/score.schema.json");
    let on_disk: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&p)
            .expect("schema/score.schema.json exists; run `praxis schema --write`"),
    )
    .unwrap();
    assert_eq!(
        on_disk,
        praxis_score::score_schema(),
        "regenerate with `cargo run -p praxis-cli -- schema --write`"
    );
}

#[test]
fn validator_catches_core_errors() {
    let mut s = praxis_score::from_json(&fixture("acceptance1_bell.score.json")).unwrap();
    // Off-scale note (C# in A minor).
    s.patterns[0].notes[0].pitch = 61;
    // Avoided class.
    s.layers[1].class = praxis_score::GeneratorClass::Riser;
    // Unknown effect parameter.
    s.layers[0].effects[0].params.insert("bogus".into(), 1.0);
    let r = validate(&s);
    use praxis_score::validate::IssueCode::*;
    let codes: Vec<_> = r.errors.iter().map(|i| i.code).collect();
    for c in [OffScale, AvoidedClass, UnknownParam] {
        assert!(codes.contains(&c), "missing {c:?} in {codes:?}");
    }
}

#[test]
fn acceptance_timeline() {
    let s = praxis_score::from_json(&fixture("acceptance1_bell.score.json")).unwrap();
    let sr = 48_000;
    assert_eq!(
        praxis_score::timeline::score_length_samples(&s, sr),
        30 * sr as u64
    );
    let ev = praxis_score::timeline::expand_notes(&s, sr);
    let piano = ev.iter().filter(|e| e.layer == 0).count();
    let kick = ev.iter().filter(|e| e.layer == 1).count();
    assert_eq!(piano, 18 * 8, "18 notes per 2 bars x 8 repeats");
    assert_eq!(kick, 2 * 8, "two kicks per bar in bars 9-16");
    // Every note starts on the eighth grid: 48000 * 60/128 / 2 = 11250 samples per step.
    for e in &ev {
        assert_eq!(e.start_sample % 11_250, 0, "{e:?}");
    }
    assert!(ev.iter().filter(|e| e.layer == 1).all(|e| e.bar >= 9));
}

#[test]
fn split_clip_keeps_phase() {
    // A 4-bar pattern on bars 1-8, split into 1-4 and 5-8 with the second continuing the phase,
    // expands to the same events as the unsplit clip; restarting the phase would not.
    let mut s = praxis_score::from_json(&fixture("acceptance1_bell.score.json")).unwrap();
    s.patterns[0].length_steps = 32;
    s.patterns[0].notes.push(praxis_score::Note {
        step: 20,
        pitch: 76,
        velocity: 0.5,
        accent: praxis_score::Accent::Normal,
        length: 1.0,
        offset: 0.0,
    });
    s.layers[0].clips[0].bars = praxis_score::BarRange { start: 1, end: 6 };
    let whole = praxis_score::timeline::expand_notes(&s, 48_000);
    let mut split = s.clone();
    let mut second = split.layers[0].clips[0].clone();
    split.layers[0].clips[0].bars.end = 3;
    second.bars = praxis_score::BarRange { start: 4, end: 6 };
    second.pattern_offset_steps = 24; // bar 4 is step 24 of the 32-step pattern
    split.layers[0].clips.push(second);
    assert!(validate(&split).is_valid(), "{:?}", validate(&split).errors);
    assert_eq!(whole, praxis_score::timeline::expand_notes(&split, 48_000));
}
