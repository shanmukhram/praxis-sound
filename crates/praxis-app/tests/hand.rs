//! Building a sound by hand through the praxis-app commands, on real sound folders with real
//! renders and checks: a blank sound, layers and notes as patches that make no take, one take
//! for the batch, the next batch as the next take, and the Phase 3 exit sound (felt piano motif,
//! soft kick in bars 9-16) exported and checked.
//!
//! The soft kick is synthesised, so `a_blank_sound_built_by_hand` runs anywhere. The felt piano
//! needs the Upright Piano KW pack (`PRAXIS_PACKS`); without it that test says why and returns.

use praxis_app::commands::{self, ExportArgs};
use praxis_app::hand::{self, MakeTakeArgs, NewSoundArgs};
use praxis_app::sounds::list_sounds;
use praxis_app::watch::read_revision;
use praxis_cli::ops::{self, TakeScope};
use praxis_score::{BarRange, Key, Mode, PitchClass, Playback, TimeSignature};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("praxis-app-hand-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::path::absolute(dir).unwrap()
}

fn blank(title: &str) -> NewSoundArgs {
    NewSoundArgs {
        title: Some(title.into()),
        key: Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        },
        bpm: Some(128.0),
        seconds: None,
        time_signature: TimeSignature {
            numerator: 4,
            denominator: 4,
        },
        bars: 16,
        playback: Playback::Loop,
    }
}

fn take(sound: &str, ask: &str) -> MakeTakeArgs {
    MakeTakeArgs {
        sound: sound.into(),
        ask: ask.into(),
        who: None,
        title: None,
        say: None,
        scope: None,
        skip_missing: false,
    }
}

/// What the window sends when "Add a layer" picks the soft kick: an empty one-bar pattern and a
/// layer that plays it in every bar.
fn add_kick() -> Value {
    json!({
        "description": "Add Soft kick",
        "origin": "user",
        "scope": {"layers": ["soft_kick"]},
        "ops": [
            {"op": "add_pattern", "pattern": {"id": "soft_kick_p1", "grid": "eighth", "length_steps": 8, "notes": []}},
            {"op": "add_layer", "layer": {
                "id": "soft_kick", "name": "Soft kick", "class": "drums",
                "instrument": {"kind": "drum", "preset": "soft_kick", "params": {}},
                "clips": [{"bars": {"start": 1, "end": 16}, "pattern": "soft_kick_p1"}]
            }}
        ]
    })
}

/// The Rhythm view's "DON - - - don - - -" written into the kick's pattern.
fn kick_beats() -> Value {
    json!({
        "description": "Wrote DON - - - don - - -",
        "origin": "user",
        "scope": {"layers": ["soft_kick"]},
        "ops": [{"op": "replace_pattern", "pattern": {"id": "soft_kick_p1", "grid": "eighth", "length_steps": 8, "notes": [
            {"step": 0, "pitch": 36, "velocity": 0.8, "accent": "strong", "length": 1.0},
            {"step": 4, "pitch": 36, "velocity": 0.65, "accent": "normal", "length": 1.0}
        ]}}]
    })
}

/// "Plays in bars 9-16": clear bars 1-8 of the kick.
fn kick_from_bar_9() -> Value {
    json!({
        "description": "Soft kick plays in bars 9-16",
        "origin": "user",
        "scope": {"layers": ["soft_kick"]},
        "ops": [{"op": "clear_bars", "layer": "soft_kick", "bars": {"start": 1, "end": 8}}]
    })
}

fn assert_checked_take(t: &Value, n: u64) {
    assert_eq!(t["n"], n);
    assert_eq!(t["who"], "You");
    assert!(t["score"].is_object(), "the take comes back with its score");
    let path = t["render"]["path"].as_str().unwrap();
    assert!(Path::new(path).is_file(), "{path}");
    assert!(t["checks"]["checks"].is_array());
    assert!(t["checks"]["passed"].is_boolean());
    assert_eq!(t["provider"], "hand");
}

#[test]
fn a_blank_sound_built_by_hand() {
    let root = scratch("blank").join("sounds");
    std::fs::create_dir_all(&root).unwrap();

    // A blank sound: a folder with a project and an empty take history.
    let made = hand::new_sound(&root, &blank("Hand loop")).unwrap();
    let dir = PathBuf::from(&made.sound);
    assert_eq!(dir, root.join("hand-loop"));
    assert_eq!(made.revision, 1);
    assert_eq!(made.live.current, 0);
    assert_eq!(made.live.score["tempo"]["bpm"], 128.0);
    assert_eq!(made.live.score["layers"], json!([]));
    assert!(!made.live.can_undo);
    let sound = made.sound.clone();

    // It is a sound: listed (no take yet), its audio folders may be served, `takes` reads it.
    let list = list_sounds(&root);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].takes, 0);
    assert_eq!(list[0].name, "Hand loop");
    assert_eq!(list[0].sub, "A minor · 0:30 loop");
    assert_eq!(commands::audio_dirs(&sound).unwrap().len(), 3);
    let t = commands::takes(&sound).unwrap();
    assert_eq!(t["current"], 0);
    assert!(t["takes"].as_array().unwrap().is_empty());
    assert_eq!(t["score"]["key"]["tonic"], "A");
    let (live, gain) = commands::playback_score(&sound).unwrap();
    assert!(live.layers.is_empty() && gain.is_none());

    // Nothing to render yet: no take is made.
    let e = hand::make_take(&take(&sound, "")).unwrap_err();
    assert!(e.message.contains("add a layer first"), "{}", e.message);
    assert_eq!(read_revision(&dir), 1);

    // Hand edits are patches with no take: undoable, read by the real-time engine.
    commands::apply_patch(&sound, add_kick()).unwrap();
    let u = commands::undo(&sound).unwrap();
    assert!(u.moved);
    assert_eq!(u.live.score["layers"], json!([]));
    assert_eq!(u.live.current, 0);
    commands::redo(&sound).unwrap();
    commands::apply_patch(&sound, kick_beats()).unwrap();
    assert_eq!(read_revision(&dir), 1, "no take yet: takes.json untouched");

    // Take 1: the live score, rendered and checked, with the master gain frozen after it.
    let r = hand::make_take(&take(&sound, "Added Soft kick; wrote DON - - - don")).unwrap();
    let t1 = &r["take"];
    assert_checked_take(t1, 1);
    assert!(t1["patch"].is_null(), "take 1 of a new sound has no patch");
    assert_eq!(t1["title"], "Built by hand");
    assert!(
        t1["say"]
            .as_str()
            .unwrap()
            .starts_with("Built by hand: soft kick, 16 bars"),
        "{}",
        t1["say"]
    );
    assert_eq!(r["current"], 1);
    assert!(t1["score"]["master"]["frozen_gain_db"].is_number());
    println!("take 1 checks: {}", t1["checks"]["failed"]);

    // Nothing changed since take 1: refused, no take.
    let e = hand::make_take(&take(&sound, "again")).unwrap_err();
    assert_eq!(e.code, "patch_rejected");
    assert!(
        e.message.contains("nothing changed since take 1"),
        "{}",
        e.message
    );

    // The next batch (the kick moves to bars 9-16) is take 2: its patch is the exact diff and
    // the change is measured on the renders.
    commands::apply_patch(&sound, kick_from_bar_9()).unwrap();
    let mut req = take(&sound, "Soft kick plays in bars 9-16");
    req.scope = Some(TakeScope {
        layer: "soft_kick".into(),
        bars: BarRange { start: 1, end: 8 },
    });
    let r2 = hand::make_take(&req).unwrap();
    let t2 = &r2["take"];
    assert_checked_take(t2, 2);
    assert_eq!(t2["from"], 1);
    assert!(t2["patch"]["ops"].as_array().is_some_and(|o| !o.is_empty()));
    assert_eq!(t2["changed"], json!({"soft_kick": [1, 8]}));
    assert_eq!(t2["title"], "Hand edit, bars 1–8");
    assert_eq!(t2["scope"]["layer"], "soft_kick");
    assert!(
        t2["untouched"]
            .as_str()
            .unwrap()
            .contains("Bars 9–16 sample-identical to take 1"),
        "{}",
        t2["untouched"]
    );
    let clips = &t2["score"]["layers"][0]["clips"];
    assert_eq!(clips[0]["bars"], json!({"start": 9, "end": 16}), "{clips}");

    // The history reads like any other: undo across the take moves the current take back.
    let all = commands::takes(&sound).unwrap();
    assert_eq!(all["takes"].as_array().unwrap().len(), 2);
    let u = commands::undo(&sound).unwrap();
    assert!(u.moved);
    assert_eq!(u.live.current, 1);
}

#[test]
fn a_blank_sound_from_a_length_and_the_presets() {
    let root = scratch("length");
    let mut a = blank("");
    a.bpm = None;
    a.seconds = Some(30.0);
    a.playback = Playback::OneShot { tail_seconds: 4.0 };
    let made = hand::new_sound(&root, &a).unwrap();
    assert!(made.sound.ends_with("untitled-sound"), "{}", made.sound);
    assert_eq!(made.live.score["tempo"]["bpm"], 128.0);
    assert_eq!(made.live.score["playback"]["mode"], "one_shot");
    // A second untitled sound gets its own folder.
    let again = hand::new_sound(&root, &a).unwrap();
    assert!(again.sound.ends_with("untitled-sound-2"), "{}", again.sound);

    let p = hand::presets().unwrap();
    let list = p["presets"].as_array().unwrap();
    let find = |n: &str| list.iter().find(|x| x["name"] == n).cloned();
    let kick = find("soft_kick").expect("soft kick in the catalogue");
    assert_eq!(kick["kind"], "drum");
    assert_eq!(kick["available"], true);
    let piano = find("felt_piano").expect("felt piano in the catalogue");
    assert_eq!(piano["kind"], "sampler");
    assert!(piano["available"].is_boolean());
}

fn have_piano(test: &str) -> bool {
    let ok = praxis_instruments::Packs::discover().has_pack("upright-piano-kw");
    if !ok {
        println!("SKIP {test}: the upright-piano-kw pack is not installed (set PRAXIS_PACKS)");
    }
    ok
}

/// The acceptance-1 motif as the window writes it: the fixture's piano pattern.
fn motif() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/acceptance1.score.json");
    let score: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let p = score["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "piano_motif")
        .unwrap();
    p["notes"].clone()
}

#[test]
fn the_phase_3_exit_sound_by_hand() {
    if !have_piano("the_phase_3_exit_sound_by_hand") {
        return;
    }
    let root = scratch("exit");
    let made = hand::new_sound(&root, &blank("Calm piano by hand")).unwrap();
    let sound = made.sound.clone();
    // A felt piano layer with an empty two-bar pattern over every bar, then the motif.
    commands::apply_patch(
        &sound,
        json!({
            "description": "Add Felt piano", "origin": "user", "scope": {"layers": ["felt_piano"]},
            "ops": [
                {"op": "add_pattern", "pattern": {"id": "felt_piano_p1", "grid": "eighth", "length_steps": 16, "notes": []}},
                {"op": "add_layer", "layer": {
                    "id": "felt_piano", "name": "Felt piano", "class": "motif",
                    "instrument": {"kind": "sampler", "preset": "felt_piano", "params": {}},
                    "clips": [{"bars": {"start": 1, "end": 16}, "pattern": "felt_piano_p1"}]
                }}
            ]
        }),
    )
    .unwrap();
    commands::apply_patch(
        &sound,
        json!({
            "description": "Wrote the motif", "origin": "user", "scope": {"layers": ["felt_piano"]},
            "ops": [{"op": "replace_pattern", "pattern": {"id": "felt_piano_p1", "grid": "eighth", "length_steps": 16, "notes": motif()}}]
        }),
    )
    .unwrap();
    commands::apply_patch(&sound, add_kick()).unwrap();
    commands::apply_patch(&sound, kick_beats()).unwrap();
    commands::apply_patch(&sound, kick_from_bar_9()).unwrap();
    let r = hand::make_take(&take(&sound, "Felt piano motif, soft kick in bars 9-16")).unwrap();
    let t = &r["take"];
    assert_checked_take(t, 1);
    let failed = &t["checks"]["failed"];
    assert_eq!(t["checks"]["passed"], true, "failed checks: {failed}");

    let out = commands::export(&ExportArgs {
        sound: sound.clone(),
        take: Some(1),
        kinds: ["wav24", "flac", "stems", "midi"]
            .map(String::from)
            .to_vec(),
        destination: "exports".into(),
        base_name: "calm piano by hand".into(),
    })
    .unwrap();
    let kinds: Vec<&str> = out.files.iter().map(|f| f.kind.as_str()).collect();
    for k in ["wav24", "flac", "stems", "midi"] {
        assert!(kinds.contains(&k), "{k} missing from {kinds:?}");
    }
    let wav = out.files.iter().find(|f| f.kind == "wav24").unwrap();
    let check = ops::check(&ops::CheckRequest {
        audio: PathBuf::from(&wav.path),
        score: PathBuf::from(&sound).join("project.praxis"),
    })
    .unwrap();
    assert_eq!(check["passed"], true, "praxis check: {}", check["failed"]);
}
