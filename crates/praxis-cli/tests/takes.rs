//! The take history, edits and variations through `praxis_cli::ops` (the functions the CLI, the
//! MCP server and the desktop app share), with real instruments and real renders.
//!
//! - `frozen_gain_keeps_bars_1_to_12_bit_identical`: the acceptance-1 brief as take 1, then
//!   "change the last four bars to add a subtle melody" as take 2. The master gain is frozen
//!   after take 1, and the take-2 master is bit-identical to take 1 over bars 1-12 outside the
//!   tail window of docs/contracts/patch.md (the new layer's notes, release and effect tail,
//!   wrapped to the loop start, widened by the master chain's tail and limiter settling, and by
//!   the limiter lookahead before bar 13).
//! - The other tests check every op's JSON payload and the files a sound folder holds.
//!
//! Renders need the Upright Piano KW pack (`PRAXIS_PACKS`); without it the tests print why and
//! return.

use praxis_cli::ErrorCode;
use praxis_cli::ops::{self, TakeScope, ViewName};
use praxis_engine::InstrumentFactory;
use praxis_score::patch::{Origin, PatchOp, Scope};
use praxis_score::project::Project;
use praxis_score::timeline::{TimeMap, expand_notes};
use praxis_score::{BarRange, Score};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid with accents and overlapping \
sustain. Bars 9-16: the identical piano part plus a soft muted kick underneath. No new melody, \
no fills, no risers, sweeps, impacts or pads.";
const EDIT: &str = "change the last four bars to add a subtle melody";
const SR: u32 = 48_000;

fn have_piano(test: &str) -> bool {
    let ok = praxis_instruments::Packs::discover().has_pack("upright-piano-kw");
    if !ok {
        println!("SKIP {test}: the upright-piano-kw pack is not installed (set PRAXIS_PACKS)");
    }
    ok
}

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("praxis-takes-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::path::absolute(dir).unwrap()
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

fn ask(sound: &Path, text: &str) -> ops::AskRequest {
    ops::AskRequest {
        sound: s(sound),
        text: text.into(),
        who: None,
        scope: None,
        patch: None,
        title: None,
        provider: Some(ops::ProviderChoice::Offline),
        model: None,
        seed: None,
        skip_missing: false,
        include_score: false,
    }
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn project_of(sound: &Path) -> Project {
    Project::from_json(&std::fs::read_to_string(sound.join("project.praxis")).unwrap()).unwrap()
}

/// The fields of the app's TypeScript `Take` (app/src/engine/EngineAdapter.ts), the render
/// report's `RenderReport` fields and the checks' `VerifyReport` fields.
fn assert_take_shape(t: &Value) {
    for f in [
        "n",
        "who",
        "ask",
        "title",
        "say",
        "patch",
        "changed",
        "untouched",
        "render",
        "checks",
        "version",
        "from",
        "identical_bars",
        "provider",
        "seed",
    ] {
        assert!(t.get(f).is_some(), "take misses `{f}`: {t:#}");
    }
    assert!(
        t["render"]["path"]
            .as_str()
            .is_some_and(|p| Path::new(p).is_absolute())
    );
    for f in [
        "mock",
        "sample_rate",
        "length_samples",
        "tail_samples",
        "is_loop",
        "loudness_before_lufs",
        "loudness_lufs",
        "true_peak_dbtp",
        "applied_gain_db",
        "warmup_cycles",
        "notes_rendered",
        "blocked_layers",
        "render_seconds",
        "warnings",
    ] {
        assert!(
            t["render"]["report"].get(f).is_some(),
            "report misses `{f}`"
        );
    }
    assert_eq!(t["render"]["report"]["mock"], json!(false));
    for f in [
        "checks",
        "tempo_bpm",
        "key",
        "grid_deviation_ms",
        "onsets_on_grid",
        "integrated_lufs",
        "true_peak_dbtp",
        "seam_jump_ratio",
        "seam_excess_db",
        "passed",
    ] {
        assert!(t["checks"].get(f).is_some(), "checks miss `{f}`");
    }
    assert!(t["who"] == "You" || t["who"].as_str().is_some_and(|w| w.starts_with("Agent · ")));
}

fn decode(p: &str) -> praxis_dsp::StereoBuffer {
    praxis_audio_io::decode::decode_file(Path::new(p))
        .expect("decode take")
        .audio
}

/// The end of the edit's master window after the loop start (contracts/patch.md, "Audio-level
/// isolation"), for the layers the edit changed, and the limiter lookahead.
fn tail_window(before: &Score, after: &Score, changed: &[String]) -> (usize, usize) {
    let len = praxis_score::timeline::score_length_samples(after, SR);
    let registry = praxis_instruments::Registry::with_default_packs();
    let block = praxis_engine::RenderOptions::default().block_size;
    let mut stem_end = 0usize;
    for id in changed {
        let overhang = [before, after]
            .iter()
            .filter_map(|sc| {
                let li = sc.layers.iter().position(|l| &l.id == id)?;
                expand_notes(sc, SR)
                    .into_iter()
                    .filter(|n| n.layer == li && n.bar >= 13)
                    .map(|n| (n.start_sample + n.length_samples).saturating_sub(len))
                    .max()
            })
            .max()
            .unwrap_or(0) as usize;
        let layer = after.layer(id).expect("changed layer");
        let ctx = praxis_engine::InstrumentContext {
            sample_rate: SR as f32,
            max_block: block,
            key: after.key,
            bpm: after.tempo.bpm,
            seed: 0,
        };
        let inst = registry
            .create(&layer.instrument, &ctx)
            .expect("instrument")
            .tail_samples();
        let mut w = Vec::new();
        let chain = praxis_engine::effects::EffectChain::build(
            &layer.effects,
            &praxis_engine::effects::BuiltinEffects,
            SR as f32,
            block,
            after.tempo.bpm as f32,
            id,
            &mut w,
        );
        stem_end = stem_end.max(overhang + inst + chain.tail_samples() + chain.latency_samples());
    }
    let master_tail = praxis_engine::master::isolation_tail_samples(
        &after.master,
        SR,
        block,
        after.tempo.bpm as f32,
    ) as usize;
    let lookahead =
        praxis_engine::master::limiter_latency_samples(&after.master.loudness, SR, block) as usize;
    (stem_end + master_tail, lookahead)
}

#[test]
fn frozen_gain_keeps_bars_1_to_12_bit_identical() {
    if !have_piano("frozen_gain_keeps_bars_1_to_12_bit_identical") {
        return;
    }
    let sound = scratch("frozen-gain").join("loop");
    let one = ops::ask(&ask(&sound, BRIEF)).expect("take 1");
    let t1 = &one["take"];
    assert_eq!(t1["n"], json!(1));
    assert!(
        t1["checks"]["passed"] == json!(true),
        "take 1 checks: {:#}",
        t1["checks"]
    );
    let gain = t1["render"]["report"]["applied_gain_db"].as_f64().unwrap();

    // The gain is frozen by an undoable system patch, and take 1's version holds it.
    let p = project_of(&sound);
    let freeze = p.undo.last().expect("freeze patch");
    assert_eq!(freeze.patch.origin, Origin::System);
    assert!(matches!(freeze.patch.ops[..], [PatchOp::SetMaster { .. }]));
    assert_eq!(p.score.master.frozen_gain_db, Some(gain));
    assert_eq!(
        p.version("take-1").unwrap().score.master.frozen_gain_db,
        Some(gain)
    );
    let before = p.score.clone();

    let mut req = ask(&sound, EDIT);
    req.who = Some("cut-assistant".into());
    let two = ops::ask(&req).expect("take 2");
    let t2 = &two["take"];
    assert_eq!(t2["n"], json!(2));
    assert_eq!(t2["from"], json!(1));
    assert_eq!(t2["who"], json!("Agent · cut-assistant"));
    assert_eq!(
        t2["render"]["report"]["applied_gain_db"].as_f64(),
        Some(gain)
    );
    let after = project_of(&sound).score;
    let changed: Vec<String> = t2["changed"].as_object().unwrap().keys().cloned().collect();
    assert_eq!(changed.len(), 1, "only the new melody changed: {changed:?}");
    assert_eq!(t2["changed"][&changed[0]], json!([13, 16]));
    assert!(before.layer(&changed[0]).is_none(), "a new layer");

    // Bit-identical over bars 1-12 outside the window.
    let a = decode(t1["render"]["path"].as_str().unwrap());
    let b = decode(t2["render"]["path"].as_str().unwrap());
    assert_eq!(a.len(), b.len());
    let tm = TimeMap::new(&after, SR);
    let start13 = tm.bar_start_sample(13) as usize;
    let (window_end, lookahead) = tail_window(&before, &after, &changed);
    assert!(
        window_end < start13 - lookahead,
        "the window covers bars 1-12"
    );
    let differs = |i: usize| {
        a.left[i].to_bits() != b.left[i].to_bits() || a.right[i].to_bits() != b.right[i].to_bits()
    };
    let last_before_window = (0..window_end).rev().find(|&i| differs(i));
    let first_after_window = (window_end..a.len()).find(|&i| differs(i));
    let bar = |i: usize| i as f64 / tm.bar_s() / SR as f64 + 1.0;
    println!(
        "take 2 vs take 1: frozen gain {gain:.3} dB; tail window wraps to sample {window_end} \
         (bar {:.2}); last difference inside it at {:?} (bar {:.2}); first difference after it at \
         {:?} (bar {:.2}); bar 13 starts at {start13}, limiter lookahead {lookahead}; identical \
         bars measured {}",
        bar(window_end),
        last_before_window,
        last_before_window.map_or(0.0, bar),
        first_after_window,
        first_after_window.map_or(0.0, bar),
        t2["identical_bars"]
    );
    let first = first_after_window.expect("bars 13-16 differ");
    assert!(
        first >= start13 - lookahead,
        "bars 1-12 differ outside the window at sample {first} (bar {:.2})",
        bar(first)
    );
    // The take record states it: every whole bar after the window up to bar 12 is identical.
    let first_whole = (1..=16u32)
        .find(|&b| tm.bar_start_sample(b) as usize >= window_end)
        .unwrap();
    let runs = t2["identical_bars"].as_array().unwrap();
    assert!(
        runs.iter()
            .any(|r| r[0].as_u64().unwrap() <= first_whole as u64 && r[1] == json!(12)),
        "identical_bars {runs:?} should cover bars {first_whole}-12"
    );
    assert!(
        t2["untouched"]
            .as_str()
            .unwrap()
            .contains("sample-identical to take 1")
    );
}

#[test]
fn takes_restore_variations_and_views() {
    if !have_piano("takes_restore_variations_and_views") {
        return;
    }
    let sound = scratch("history").join("loop");
    let empty = ops::takes(&ops::TakesRequest {
        sound: s(&sound),
        include_scores: false,
    })
    .unwrap();
    assert_eq!(empty["takes"], json!([]));
    assert_eq!(empty["current"], json!(0));
    assert!(empty["score"].is_null());

    // A scope on an empty sound is refused.
    let mut scoped = ask(&sound, BRIEF);
    scoped.scope = Some(TakeScope {
        layer: "piano".into(),
        bars: BarRange { start: 1, end: 4 },
    });
    assert_eq!(ops::ask(&scoped).unwrap_err().code, ErrorCode::Error);

    let one = ops::ask(&ask(&sound, BRIEF)).unwrap();
    assert_take_shape(&one["take"]);
    assert_eq!(one["take"]["patch"], Value::Null);
    assert_eq!(one["take"]["changed"], json!({}));
    assert_eq!(one["take"]["version"], json!("take-1"));
    assert!(one["take"]["say"].as_str().unwrap().contains("128 BPM"));
    assert!(sound.join("takes/1/mix.wav").is_file());
    let file = read_json(&sound.join("takes.json"));
    assert_eq!(file["format"], json!("praxis-takes"));
    assert_eq!(file["version"], json!(1));
    assert_eq!(file["revision"], json!(1));
    assert_eq!(file["takes"][0]["render"]["path"], json!("takes/1/mix.wav"));
    assert!(!sound.join(".lock").exists());

    // A scoped request changes exactly those bars of that layer.
    let mut edit = ask(&sound, "give it a new motif");
    edit.scope = Some(TakeScope {
        layer: "piano".into(),
        bars: BarRange { start: 9, end: 12 },
    });
    edit.who = Some("Agent · arranger".into());
    let two = ops::ask(&edit).unwrap();
    let t2 = &two["take"];
    assert_take_shape(t2);
    assert_eq!(t2["changed"], json!({"piano": [9, 12]}), "{t2:#}");
    assert_eq!(
        t2["scope"],
        json!({"layer": "piano", "bars": {"start": 9, "end": 12}})
    );
    assert!(t2["ask"].as_str().unwrap().contains("bars 9–12"));
    assert_eq!(t2["patch"]["scope"]["bars"], json!({"start": 9, "end": 12}));
    println!(
        "scoped take: {} / {} / {}",
        t2["title"], t2["say"], t2["untouched"]
    );
    assert_eq!(two["revision"], json!(2));

    // A scope on a layer that is not there, and a scope outside the bars, are refused.
    let mut bad = ask(&sound, "give it a new motif");
    bad.scope = Some(TakeScope {
        layer: "nope".into(),
        bars: BarRange { start: 9, end: 12 },
    });
    assert_eq!(ops::ask(&bad).unwrap_err().code, ErrorCode::PatchRejected);
    bad.scope = Some(TakeScope {
        layer: "piano".into(),
        bars: BarRange { start: 15, end: 20 },
    });
    assert_eq!(ops::ask(&bad).unwrap_err().code, ErrorCode::PatchRejected);

    // Going back is an undoable patch and sets `current`; no take is added.
    let r = ops::restore(&ops::RestoreRequest {
        sound: s(&sound),
        n: 1,
    })
    .unwrap();
    assert_eq!(r["restored"], json!(true));
    assert_eq!(r["current"], json!(1));
    assert_eq!(r["take"]["n"], json!(1));
    assert!(r["patch"]["ops"].as_array().is_some_and(|o| !o.is_empty()));
    let p = project_of(&sound);
    assert_eq!(p.score, p.version("take-1").unwrap().score);
    assert!(p.undo.last().unwrap().patch.description.contains("take-1"));
    assert_eq!(
        ops::restore(&ops::RestoreRequest {
            sound: s(&sound),
            n: 9
        })
        .unwrap_err()
        .code,
        ErrorCode::Error
    );

    // Variations of the current take (take 1), none committed until one is kept.
    let seq = project_of(&sound).current_seq();
    let v = ops::variations(&ops::VariationsRequest {
        project: None,
        score: None,
        sound: Some(s(&sound)),
        scope: Scope {
            bars: Some(BarRange { start: 13, end: 16 }),
            layers: vec!["piano".into()],
        },
        n: 2,
        seed: 3,
        out_dir: None,
        render_settings: Default::default(),
    })
    .unwrap();
    let vs = v["variations"].as_array().unwrap();
    assert_eq!(vs.len(), 2);
    for (i, x) in vs.iter().enumerate() {
        assert_eq!(x["index"], json!(i + 1));
        assert_eq!(x["changed_bars"], json!([13, 16]));
        assert_eq!(x["changed_layers"], json!(["piano"]));
        assert!(sound.join(format!("variations/{}.wav", i + 1)).is_file());
        assert!(x["render"]["check"]["passed"].is_boolean());
    }
    assert_eq!(project_of(&sound).current_seq(), seq, "nothing committed");
    let kept = ops::keep_variation(&ops::KeepVariationRequest {
        sound: s(&sound),
        index: 2,
        who: None,
        ask: None,
        title: None,
        skip_missing: false,
        include_score: true,
    })
    .unwrap();
    let t3 = &kept["take"];
    assert_take_shape(t3);
    assert_eq!(t3["n"], json!(3));
    assert_eq!(t3["from"], json!(1));
    assert_eq!(t3["changed"], json!({"piano": [13, 16]}));
    assert_eq!(t3["scope"]["layer"], json!("piano"));
    assert!(t3["score"]["layers"].is_array(), "include_score");
    // The variations were made from take 1; take 3 is current now, so keeping again is stale.
    let stale = ops::keep_variation(&ops::KeepVariationRequest {
        sound: s(&sound),
        index: 1,
        who: None,
        ask: None,
        title: None,
        skip_missing: false,
        include_score: false,
    })
    .unwrap_err();
    assert_eq!(stale.code, ErrorCode::PatchRejected);

    // A mute sent as a patch changes how the live score plays: no take (WP-AG2).
    let mute: Value = json!({
        "description": "mute the kick",
        "origin": "user",
        "ops": [{"op": "update_layer", "id": "kick", "update": {"muted": true}}]
    });
    let mut muting = ask(&sound, "mute the kick");
    muting.patch = Some(serde_json::from_value(mute).unwrap());
    let muted = ops::ask(&muting).unwrap();
    assert_eq!(muted["take"], Value::Null);
    assert_eq!(muted["no_take"], json!("live"));
    assert_eq!(muted["current"], json!(3));

    // A hand edit from a panel view: an explicit patch (the kick quieter) becomes take 4.
    let kick_update: Value = json!({
        "description": "quieter kick",
        "origin": "user",
        "ops": [{"op": "update_layer", "id": "kick", "update": {"gain_db": -9.0}}]
    });
    let mut hand = ask(&sound, "quieter kick");
    hand.patch = Some(serde_json::from_value(kick_update).unwrap());
    let four = ops::ask(&hand).unwrap();
    assert_eq!(four["take"]["n"], json!(4));
    assert_eq!(four["take"]["provider"], json!("hand"));
    assert_eq!(four["take"]["title"], json!("Hand edit"));
    assert_eq!(four["take"]["changed"], json!({"kick": [9, 16]}));

    // A view request, recorded with a new revision.
    let before_rev = read_json(&sound.join("takes.json"))["revision"]
        .as_u64()
        .unwrap();
    let view = ops::open_view(&ops::OpenViewRequest {
        sound: s(&sound),
        view: ViewName::Notes,
        who: Some("arranger".into()),
    })
    .unwrap();
    assert_eq!(view["by"], json!("arranger"));
    assert_eq!(view["view"], json!("notes"));
    assert_eq!(view["revision"], json!(before_rev + 1));

    let all = ops::takes(&ops::TakesRequest {
        sound: s(&sound),
        include_scores: true,
    })
    .unwrap();
    let list = all["takes"].as_array().unwrap();
    assert_eq!(
        list.iter()
            .map(|t| t["n"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(all["current"], json!(4));
    assert_eq!(all["can_undo"], json!(true));
    assert_eq!(all["views"][0]["view"], json!("notes"));
    for t in list {
        assert_take_shape(t);
        assert!(t["score"]["layers"].is_array());
    }
    println!(
        "take record (the shape WP-U3 maps onto the TypeScript Take):\n{}",
        serde_json::to_string_pretty(&list[1]).unwrap()
    );
}

#[test]
fn edit_and_variations_on_a_project() {
    if !have_piano("edit_and_variations_on_a_project") {
        return;
    }
    let dir = scratch("edit");
    let project = dir.join("song.praxis");
    ops::generate(&ops::GenerateRequest {
        prompt: Some(BRIEF.into()),
        plan: None,
        provider: Some(ops::ProviderChoice::Offline),
        model: None,
        seed: 1,
        out: project.clone(),
        render: None,
        render_settings: Default::default(),
    })
    .unwrap();
    let v = ops::edit(&ops::EditRequest {
        project: project.clone(),
        prompt: Some(EDIT.into()),
        plan: None,
        provider: Some(ops::ProviderChoice::Offline),
        model: None,
        seed: 7,
        render: Some(dir.join("edited.wav")),
        render_settings: Default::default(),
    })
    .unwrap();
    for f in [
        "project",
        "patch",
        "scope",
        "changed_bars",
        "changed_layers",
        "summary",
        "refused",
        "changed",
        "plan",
        "provider",
    ] {
        assert!(v.get(f).is_some(), "edit misses `{f}`");
    }
    assert_eq!(v["changed"], json!(true));
    assert_eq!(v["changed_bars"], json!([13, 16]));
    assert_eq!(v["scope"]["bars"], json!({"start": 13, "end": 16}));
    assert_eq!(v["provider"], json!("offline-rules"));
    assert!(v["render"]["check"]["passed"].is_boolean());
    let p = Project::from_json(&std::fs::read_to_string(&project).unwrap()).unwrap();
    assert_eq!(p.undo.len(), 1, "the edit is undoable");

    // An agent-written EditPlan (no provider).
    let plan: Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../praxis-planner/tests/prompts/edit_last_four_melody.plan.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let w = ops::edit(&ops::EditRequest {
        project: project.clone(),
        prompt: None,
        plan: Some(plan),
        provider: None,
        model: None,
        seed: 2,
        render: None,
        render_settings: Default::default(),
    })
    .unwrap();
    assert_eq!(w["provider"], json!("direct-plan"));
    assert_eq!(w["changed_bars"], json!([13, 16]));
    // A plan that is not an EditPlan.
    let e = ops::edit(&ops::EditRequest {
        project: project.clone(),
        prompt: None,
        plan: Some(json!({"intent": "explode"})),
        provider: None,
        model: None,
        seed: 2,
        render: None,
        render_settings: Default::default(),
    })
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::BadJson);

    // Variations of a project: files next to it, nothing committed.
    let undo = Project::from_json(&std::fs::read_to_string(&project).unwrap())
        .unwrap()
        .undo
        .len();
    let vars = ops::variations(&ops::VariationsRequest {
        project: Some(project.clone()),
        score: None,
        sound: None,
        scope: Scope {
            bars: Some(BarRange { start: 5, end: 8 }),
            layers: vec![],
        },
        n: 2,
        seed: 1,
        out_dir: None,
        render_settings: Default::default(),
    })
    .unwrap();
    assert_eq!(vars["variations"].as_array().unwrap().len(), 2);
    assert!(dir.join("variations/1.wav").is_file() && dir.join("variations/2.wav").is_file());
    let file = read_json(&dir.join("variations/variations.json"));
    assert_eq!(file["format"], json!("praxis-variations"));
    assert_eq!(file["variations"][1]["index"], json!(2));
    assert_eq!(
        Project::from_json(&std::fs::read_to_string(&project).unwrap())
            .unwrap()
            .undo
            .len(),
        undo
    );
    let too_many = ops::variations(&ops::VariationsRequest {
        project: Some(project),
        score: None,
        sound: None,
        scope: Scope::default(),
        n: 0,
        seed: 1,
        out_dir: None,
        render_settings: Default::default(),
    })
    .unwrap_err();
    assert_eq!(too_many.code, ErrorCode::Error);
}
