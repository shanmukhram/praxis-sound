//! Prompt suite: every brief in tests/prompts/*.json runs through RuleProvider + repair +
//! arrange, and automatic validators check key, tempo, bars, grid, structure, layering and that
//! banned elements are absent.

mod common;

use common::{TRAILER, active_bars, check_acceptance1};
use praxis_planner::catalogue::class_name;
use praxis_planner::{Catalogue, RuleProvider, plan_brief};
use praxis_score::lint::lint;
use praxis_score::validate::{unpitched_class, validate};
use praxis_score::{GeneratorClass, Key, Mode, Score};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn prompts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("prompts")
}

/// Brief files (not the hand-written plan examples).
fn brief_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(prompts_dir())
        .expect("prompts dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension().is_some_and(|x| x == "json")
                && !p.to_string_lossy().ends_with(".plan.json")
        })
        .collect();
    files.sort();
    files
}

fn class_of(s: &str) -> GeneratorClass {
    serde_json::from_value(Value::String(s.to_string())).expect("class name")
}

fn layer_by_preset<'a>(score: &'a Score, preset: &str) -> Option<&'a praxis_score::Layer> {
    score.layers.iter().find(|l| l.instrument.preset == preset)
}

fn check_case(brief: &str, seed: u64, expect: &Value) -> Result<String, String> {
    let catalogue = Catalogue::standard();
    let provider = RuleProvider::new();
    let out = plan_brief(brief, &provider, &catalogue, seed).map_err(|e| e.to_string())?;
    let score = &out.score;

    // Generic validators.
    let report = validate(score);
    if !report.is_valid() {
        return Err(format!("score invalid: {:?}", report.errors));
    }
    if out.attempts != 1 || !out.repairs.is_empty() {
        return Err(format!(
            "not first try: attempts {}, repairs {:?}",
            out.attempts, out.repairs
        ));
    }
    let lint_ctx = praxis_planner::brief::lint_context(
        score,
        &praxis_planner::plan::PlanContext {
            catalogue: &catalogue,
            requested: &praxis_planner::brief::requested_classes(brief),
        },
    );
    let lints = lint(score, &lint_ctx);
    if !lints.is_empty() || !out.lint.is_empty() {
        return Err(format!("lint warnings: {lints:?}"));
    }
    let again = plan_brief(brief, &provider, &catalogue, seed).map_err(|e| e.to_string())?;
    if again.score != *score {
        return Err("not deterministic".into());
    }
    // On grid and in key.
    for p in &score.patterns {
        let spb = score
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
    for l in &score.layers {
        if score.constraints.avoid.contains(&l.class) {
            return Err(format!("layer {} has avoided class {:?}", l.id, l.class));
        }
        if unpitched_class(l.class) || score.key.mode == Mode::Chromatic {
            continue;
        }
        for c in &l.clips {
            let p = score.pattern(&c.pattern).ok_or("missing pattern")?;
            if let Some(n) = p.notes.iter().find(|n| !score.key.contains(n.pitch)) {
                return Err(format!("layer {} note {} is off the key", l.id, n.pitch));
            }
        }
    }
    let requested = praxis_planner::brief::requested_classes(brief);
    for c in TRAILER {
        if !requested.contains(&c) {
            if score.layers.iter().any(|l| l.class == c) {
                return Err(format!("unrequested {c:?} layer present"));
            }
            if !score.constraints.avoid.contains(&c) {
                return Err(format!("avoid-list lacks unrequested {c:?}"));
            }
        }
    }

    // Expectations.
    if let Some(k) = expect.get("key") {
        let key: Key = serde_json::from_value(k.clone()).map_err(|e| e.to_string())?;
        if score.key != key {
            return Err(format!("key {:?} != {key:?}", score.key));
        }
    }
    if let Some(b) = expect.get("bpm").and_then(Value::as_f64)
        && (score.tempo.bpm - b).abs() > 1e-6
    {
        return Err(format!("bpm {} != {b}", score.tempo.bpm));
    }
    if let Some(b) = expect.get("bars").and_then(Value::as_u64)
        && score.bars as u64 != b
    {
        return Err(format!("bars {} != {b}", score.bars));
    }
    if let Some(ts) = expect.get("time_signature").and_then(Value::as_array) {
        let want = (
            ts[0].as_u64().unwrap_or(0) as u8,
            ts[1].as_u64().unwrap_or(0) as u8,
        );
        let got = (
            score.time_signature.numerator,
            score.time_signature.denominator,
        );
        if got != want {
            return Err(format!("time signature {got:?} != {want:?}"));
        }
    }
    if let Some(pb) = expect.get("playback").and_then(Value::as_str) {
        let got = serde_json::to_value(&score.playback).map_err(|e| e.to_string())?;
        if got["mode"] != pb {
            return Err(format!("playback {} != {pb}", got["mode"]));
        }
    }
    if let Some(e) = expect.get("emotion").and_then(Value::as_str)
        && score.meta.emotion.as_deref() != Some(e)
    {
        return Err(format!("emotion {:?} != {e}", score.meta.emotion));
    }
    if let Some(n) = expect.get("layer_count").and_then(Value::as_u64)
        && score.layers.len() as u64 != n
    {
        return Err(format!(
            "layers {:?}, expected {n}",
            score
                .layers
                .iter()
                .map(|l| l.instrument.preset.as_str())
                .collect::<Vec<_>>()
        ));
    }
    if let Some(n) = expect.get("sections").and_then(Value::as_u64)
        && score.sections.len() as u64 != n
    {
        return Err(format!("{} sections, expected {n}", score.sections.len()));
    }
    for want in expect
        .get("layers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let preset = want["preset"].as_str().unwrap_or("");
        let l = layer_by_preset(score, preset).ok_or(format!("no `{preset}` layer"))?;
        if let Some(r) = want.get("bars").and_then(Value::as_array) {
            let (a, b) = (
                r[0].as_u64().unwrap_or(0) as u32,
                r[1].as_u64().unwrap_or(0) as u32,
            );
            let got = active_bars(score, &l.id);
            if got != (a..=b).collect::<BTreeSet<u32>>() {
                return Err(format!("`{preset}` plays bars {got:?}, expected {a}-{b}"));
            }
        }
        if let Some(g) = want.get("grid").and_then(Value::as_str) {
            let p = score
                .pattern(&l.clips[0].pattern)
                .ok_or("missing pattern")?;
            let got = serde_json::to_value(p.grid).map_err(|e| e.to_string())?;
            if got != g {
                return Err(format!("`{preset}` grid {got} != {g}"));
            }
        }
    }
    if let Some(m) = expect.get("spoken").and_then(Value::as_object) {
        for (preset, s) in m {
            let l = layer_by_preset(score, preset).ok_or(format!("no `{preset}` layer"))?;
            let p = score
                .pattern(&l.clips[0].pattern)
                .ok_or("missing pattern")?;
            if p.spoken.as_deref() != s.as_str() {
                return Err(format!("`{preset}` spoken {:?} != {s}", p.spoken));
            }
        }
    }
    for c in expect
        .get("absent_classes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let c = class_of(c.as_str().unwrap_or(""));
        if score.layers.iter().any(|l| l.class == c) {
            return Err(format!("class {} should be absent", class_name(c)));
        }
    }
    for c in expect
        .get("avoid_contains")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let c = class_of(c.as_str().unwrap_or(""));
        if !score.constraints.avoid.contains(&c) {
            return Err(format!("avoid-list lacks {}", class_name(c)));
        }
    }
    for c in expect
        .get("avoid_absent")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let c = class_of(c.as_str().unwrap_or(""));
        if score.constraints.avoid.contains(&c) {
            return Err(format!("avoid-list should not hold {}", class_name(c)));
        }
    }
    if let Some(chords) = expect.get("chords").and_then(Value::as_array) {
        let mut per_bar = Vec::new();
        for c in &score.chords {
            for _ in c.bars.start..=c.bars.end {
                per_bar.push(Value::String(c.symbol.clone()));
            }
        }
        if &per_bar != chords {
            return Err(format!("chords {per_bar:?} != {chords:?}"));
        }
    }
    if let Some(s) = expect.get("plan_note_contains").and_then(Value::as_str)
        && !out.plan.notes.iter().any(|n| n.contains(s))
    {
        return Err(format!("plan notes {:?} lack `{s}`", out.plan.notes));
    }
    if expect.get("acceptance1").and_then(Value::as_bool) == Some(true) {
        check_acceptance1(score)?;
    }
    Ok(format!(
        "{} bars @ {} BPM, {}, layers [{}]",
        score.bars,
        score.tempo.bpm,
        score.key.display(),
        score
            .layers
            .iter()
            .map(|l| l.instrument.preset.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

#[test]
fn prompt_suite_passes() {
    let files = brief_files();
    assert!(
        files.len() >= 20,
        "prompt suite needs at least 20 briefs, has {}",
        files.len()
    );
    let mut failures = Vec::new();
    let mut acceptance = 0;
    for f in &files {
        let text = std::fs::read_to_string(f).expect("read prompt");
        let case: Value = serde_json::from_str(&text).expect("prompt json");
        let brief = case["brief"].as_str().expect("brief");
        let seed = case["seed"].as_u64().unwrap_or(0);
        let name = f.file_stem().map(|s| s.to_string_lossy().to_string());
        if case["expect"]["acceptance1"] == true {
            acceptance += 1;
        }
        match check_case(brief, seed, &case["expect"]) {
            Ok(summary) => println!("PASS {name:?}: {summary}"),
            Err(e) => {
                println!("FAIL {name:?}: {e}");
                failures.push(format!("{name:?}: {e}"));
            }
        }
    }
    println!(
        "prompt suite: {}/{} passed ({acceptance} acceptance-1 phrasings)",
        files.len() - failures.len(),
        files.len()
    );
    assert!(acceptance >= 3, "need several acceptance-1 phrasings");
    assert!(failures.is_empty(), "failures:\n{}", failures.join("\n"));
}

#[test]
fn acceptance1_holds_for_many_seeds() {
    let brief = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt piano \
                 playing a repeating rhythmic motif on an eighth-note grid with accents and \
                 overlapping sustain. Bars 9-16: the identical piano part plus a soft muted kick \
                 underneath. No new melody, no fills, no risers, sweeps, impacts or pads.";
    let catalogue = Catalogue::standard();
    for seed in 0..12u64 {
        let out = plan_brief(brief, &RuleProvider::new(), &catalogue, seed).expect("plan");
        check_acceptance1(&out.score).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(out.score.seed, seed);
        assert_eq!(
            out.score
                .meta
                .provenance
                .get("provider")
                .map(String::as_str),
            Some("offline-rules")
        );
    }
}
