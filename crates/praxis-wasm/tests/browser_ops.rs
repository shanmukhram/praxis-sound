//! The browser build's operations, run natively through the same code the wasm exports call:
//! planning, pack files in memory, synth stand-ins, renders with checks, patches, scoped edits,
//! and the real-time wrapper the AudioWorklet drives (equal to the offline render).
//!
//! Tests that need the Upright Piano KW pack find it through `PRAXIS_PACKS` or a `packs/` folder
//! above the crate that holds it, and say so when they skip.

use praxis_wasm::library::Library;
use praxis_wasm::ops::{self, RenderSettings};
use praxis_wasm::realtime::Realtime;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid, with accents and overlapping \
sustain. Bars 9-16: the identical piano part, plus a soft muted kick layered underneath. No new \
melody, no fills. No risers, sweeps, impacts or pads anywhere.";

const PIANO_PACK: &str = "upright-piano-kw";

fn pack_root() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PRAXIS_PACKS") {
        let p = PathBuf::from(p);
        return p.join(PIANO_PACK).join("pack.json").is_file().then_some(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|d| d.join("packs"))
        .find(|p| p.join(PIANO_PACK).join("pack.json").is_file())
}

/// Loads a preset's pack into the library the way the browser does: ask what is missing,
/// "fetch" it (here: read the file), repeat.
fn load_pack(lib: &mut Library, root: &Path, pack: &str, preset: &str) -> usize {
    let mut fetched = 0;
    for _ in 0..8 {
        let missing = lib.missing(pack, preset).expect("pack parses");
        if missing.is_empty() {
            return fetched;
        }
        for path in missing {
            let bytes = std::fs::read(root.join(&path)).expect("pack file exists");
            lib.insert(&path, bytes);
            fetched += 1;
        }
    }
    panic!("pack {pack} did not complete in 8 rounds");
}

fn planned(brief: &str) -> Value {
    let p = ops::plan_brief(brief, 1);
    assert_eq!(p["ok"], true, "{p}");
    p
}

fn score_text(p: &Value) -> String {
    p["score"].to_string()
}

#[test]
fn the_offline_planner_makes_the_acceptance_score() {
    let p = planned(BRIEF);
    let presets: Vec<&str> = p["score"]["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["instrument"]["preset"].as_str().unwrap())
        .collect();
    assert_eq!(presets.len(), 2, "{presets:?}");
    assert!(presets.contains(&"felt_piano") && presets.contains(&"soft_kick"));
    assert_eq!(p["score"]["tempo"]["bpm"], 128.0);
    assert!(!p["summary"].as_str().unwrap().is_empty());
    assert_eq!(ops::plan_brief("   ", 1)["ok"], false);
}

/// A tiny WAV (16-bit mono) for an in-memory pack.
fn wav(frames: usize) -> Vec<u8> {
    let data = frames * 2;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&48_000u32.to_le_bytes());
    b.extend_from_slice(&96_000u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data as u32).to_le_bytes());
    for i in 0..frames {
        let v = ((i as f32 * 0.05).sin() * 8000.0) as i16;
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

#[test]
fn missing_files_come_in_order_manifest_entry_includes_samples() {
    let mut lib = Library::new();
    assert_eq!(lib.missing("p", "felt_piano").unwrap(), vec!["p/pack.json"]);
    let manifest = json!({
        "name": "p", "version": "1", "license": "CC0-1.0", "source_url": "",
        "presets": { "felt_piano": "inst.sfz" }
    });
    lib.insert("p/pack.json", manifest.to_string().into_bytes());
    assert_eq!(lib.missing("p", "felt_piano").unwrap(), vec!["p/inst.sfz"]);
    assert!(lib.missing("p", "nothing").is_err());
    lib.insert(
        "p/inst.sfz",
        b"#include \"common.sfz\"\n<region> sample=a.wav lokey=0 hikey=60\n".to_vec(),
    );
    assert_eq!(lib.missing("p", "felt_piano").unwrap(), vec!["p/common.sfz"]);
    lib.insert(
        "p/common.sfz",
        b"<region> sample=b.wav lokey=61 hikey=127\n".to_vec(),
    );
    assert_eq!(
        lib.missing("p", "felt_piano").unwrap(),
        vec!["p/a.wav", "p/b.wav"]
    );
    lib.insert("p/a.wav", wav(64));
    lib.insert("p/b.wav", wav(64));
    assert!(lib.missing("p", "felt_piano").unwrap().is_empty());
    assert!(lib.complete("p", "felt_piano"));
    assert_eq!(lib.size().0, 5);
}

#[test]
fn without_the_pack_the_piano_plays_a_synth_and_says_so() {
    let p = planned(BRIEF);
    let mut lib = Library::new();
    let needs = ops::pack_needs(&mut lib, &score_text(&p));
    assert_eq!(needs["needs"][0]["pack"], PIANO_PACK, "{needs}");
    assert_eq!(needs["needs"][0]["complete"], false);
    let r = ops::render(&mut lib, &score_text(&p), &RenderSettings::default(), None)
        .expect("renders with a stand-in");
    let notes = r.report["fallbacks"].as_array().unwrap();
    assert_eq!(notes.len(), 1, "{}", r.report);
    assert!(notes[0].as_str().unwrap().contains("soft keys synth"), "{notes:?}");
    assert!(
        r.played
            .layers
            .iter()
            .any(|l| l.instrument.preset == "soft_keys")
    );
    assert_eq!(r.checks["checks"].as_array().unwrap().len(), 8);
    let presets = ops::presets(&mut lib);
    let felt = presets["presets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "felt_piano")
        .unwrap();
    assert_eq!(felt["available"], false);
}

#[test]
fn the_acceptance_take_passes_eight_of_eight_checks_with_the_pack() {
    let Some(root) = pack_root() else {
        eprintln!("skipped: the {PIANO_PACK} pack is not installed (set PRAXIS_PACKS)");
        return;
    };
    let p = planned(BRIEF);
    let mut lib = Library::new();
    let fetched = load_pack(&mut lib, &root, PIANO_PACK, "felt_piano");
    assert!(fetched > 60, "manifest, SFZ and samples: {fetched}");
    let t0 = std::time::Instant::now();
    let clock = move || t0.elapsed().as_secs_f64();
    let r = ops::render(
        &mut lib,
        &score_text(&p),
        &RenderSettings::default(),
        Some(&clock),
    )
    .expect("renders");
    assert_eq!(r.report["fallbacks"].as_array().unwrap().len(), 0);
    let checks = r.checks["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 8, "{}", r.checks);
    assert_eq!(r.checks["passed"], true, "{}", r.checks["summary"]);
    assert!(
        checks.iter().all(|c| c["passed"] == true),
        "{}",
        r.checks["summary"]
    );
    assert_eq!(r.checks["summary"], "8 of 8 checks passed");
    assert_eq!(r.audio.len(), 30 * 48_000);
}

#[test]
fn patches_apply_and_a_locked_layer_is_refused_in_the_apps_words() {
    let p = planned(BRIEF);
    let score = &p["score"];
    let layer = score["layers"][0]["id"].as_str().unwrap().to_string();
    let lock = json!({
        "description": "lock", "origin": "user", "scope": {},
        "ops": [{ "op": "update_layer", "id": layer, "update": { "locked": true } }]
    });
    let locked = ops::apply_patch(&score.to_string(), &lock.to_string());
    assert_eq!(locked["ok"], true, "{locked}");
    assert_eq!(
        locked["inverse"]["ops"][0]["update"]["locked"], false,
        "{locked}"
    );
    let mute = json!({
        "description": "mute", "origin": "model", "scope": {},
        "ops": [{ "op": "update_layer", "id": layer, "update": { "muted": true } }]
    });
    let refused = ops::apply_patch(&locked["score"].to_string(), &mute.to_string());
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"]["kind"], "locked_layer", "{refused}");
    assert_eq!(refused["error"]["layer"], layer.as_str());
    let bad = ops::apply_patch(&score.to_string(), "{\"ops\": 3}");
    assert_eq!(bad["error"]["kind"], "invalid");
}

#[test]
fn a_scoped_edit_stays_in_its_bars_and_layer() {
    let p = planned(BRIEF);
    let score = p["score"].to_string();
    let piano = p["score"]["layers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["instrument"]["preset"] == "felt_piano")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let scope = json!({ "layer": piano, "bars": { "start": 13, "end": 16 } }).to_string();
    let e = ops::plan_edit(&score, "make it softer", &scope, 2);
    assert_eq!(e["ok"], true, "{e}");
    assert_eq!(e["changed"], true, "{e}");
    assert_eq!(e["patch"]["scope"]["bars"]["start"], 13, "{}", e["patch"]);
    assert_eq!(e["patch"]["scope"]["bars"]["end"], 16);
    assert!(e["title"].as_str().unwrap().starts_with("Softer"), "{}", e["title"]);
    // Unscoped: the rules read the bars from the words.
    let w = ops::plan_edit(&score, "make the last four bars softer", "null", 2);
    assert_eq!(w["ok"], true, "{w}");
    assert_eq!(w["changed"], true);
    let none = ops::plan_edit(&score, "  ", "null", 2);
    assert_eq!(none["ok"], false);
    let outside = json!({ "layer": piano, "bars": { "start": 15, "end": 40 } }).to_string();
    assert_eq!(ops::plan_edit(&score, "softer", &outside, 2)["ok"], false);
}

#[test]
fn analyse_measures_a_render() {
    let p = planned(BRIEF);
    let mut lib = Library::new();
    let r = ops::render(&mut lib, &score_text(&p), &RenderSettings::default(), None).unwrap();
    let a = ops::analyse(&r.audio.left, &r.audio.right, 48_000, 4);
    assert_eq!(a["ok"], true, "{a}");
    let bpm = a["tempo"]["bpm"].as_f64().unwrap();
    assert!((bpm - 128.0).abs() < 2.0, "{bpm}");
    assert!(!a["onsets_s"].as_array().unwrap().is_empty());
    let v = ops::verify(&score_text(&p), &r.audio.left, &r.audio.right, 48_000);
    assert_eq!(v["ok"], true);
    assert_eq!(v["checks"].as_array().unwrap().len(), 8);
    assert_eq!(ops::analyse(&[], &[], 48_000, 4)["ok"], false);
}

/// Plays `frames` frames through the real-time wrapper in 128-frame blocks, as the worklet does.
fn play(rt: &mut Realtime, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    while l.len() < frames {
        let n = rt.render(128.min(frames - l.len()));
        l.extend_from_slice(&rt.left()[..n]);
        r.extend_from_slice(&rt.right()[..n]);
    }
    (l, r)
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[test]
fn the_worklet_engine_plays_a_one_shot_as_the_offline_render() {
    let p = planned("A warm logo sting in D major, 4 seconds");
    let score = score_text(&p);
    let mut lib = Library::new();
    let off = ops::render(&mut lib, &score, &RenderSettings::default(), None).unwrap();
    let gain = off.report["applied_gain_db"].as_f64().unwrap();
    let mut rt = Realtime::new(48_000);
    let info = rt.load(&score, Some(gain));
    assert_eq!(info["ok"], true, "{info}");
    assert!(rt.play());
    let n = off.audio.len();
    let (l, r) = play(&mut rt, n);
    // Equal everywhere but the last limiter lookahead (realtime.rs docs): compare up to 10 ms
    // before the end.
    let cut = n - 480;
    let d = max_diff(&l[..cut], &off.audio.left[..cut]).max(max_diff(&r[..cut], &off.audio.right[..cut]));
    assert!(d <= 1e-6, "max difference {d}");
    assert!(rt.status()[0] > 0.0, "position moves");
}

#[test]
fn the_worklet_engine_loops_as_the_offline_loop() {
    let p = planned(BRIEF);
    let score = score_text(&p);
    let mut lib = Library::new();
    let off = ops::render(&mut lib, &score, &RenderSettings::default(), None).unwrap();
    let gain = off.report["applied_gain_db"].as_f64().unwrap();
    let mut rt = Realtime::new(48_000);
    assert_eq!(rt.load(&score, Some(gain))["ok"], true);
    rt.play();
    let n = off.audio.len();
    let (l, r) = play(&mut rt, 2 * n);
    // The second cycle has the first cycle's tails ringing into it, like the offline loop wrap.
    let d = max_diff(&l[n..], &off.audio.left).max(max_diff(&r[n..], &off.audio.right));
    let peak = off.audio.left.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    eprintln!("loop cycle 2 vs offline: max difference {d:e} (peak {peak})");
    assert!(d <= 1e-4, "max difference {d}");
}
