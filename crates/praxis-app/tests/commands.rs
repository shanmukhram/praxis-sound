//! Every praxis-app command on a real sound folder: real planner, real instruments, real renders
//! and checks, through the shared command layer (no mock, no window).
//!
//! `a_sound_from_first_take_to_export` follows the app: a new sound from the owner's brief (take
//! 1), the sounds list, a mute that makes no take, undo and redo, a scoped request (take 2),
//! undo across a take (current moves back), going back to take 1, an agent's view request, the
//! take watcher, the live render and every export kind.
//!
//! Renders need the Upright Piano KW pack (`PRAXIS_PACKS`); without it the tests say why and
//! return.

use praxis_app::commands::{self, ExportArgs};
use praxis_app::sounds::{list_sounds, new_sound_path};
use praxis_app::watch::{read_revision, watch};
use praxis_cli::ops::{self, TakeScope, ViewName};
use praxis_score::BarRange;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

const BRIEF: &str = "a calm 30-second felt piano loop in A minor";

fn have_piano(test: &str) -> bool {
    let ok = praxis_instruments::Packs::discover().has_pack("upright-piano-kw");
    if !ok {
        println!("SKIP {test}: the upright-piano-kw pack is not installed (set PRAXIS_PACKS)");
    }
    ok
}

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("praxis-app-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::path::absolute(dir).unwrap()
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

fn ask(sound: &str, text: &str, scope: Option<TakeScope>) -> ops::AskRequest {
    ops::AskRequest {
        sound: sound.into(),
        text: text.into(),
        who: Some("You".into()),
        scope,
        patch: None,
        title: None,
        provider: Some(ops::ProviderChoice::Offline),
        model: None,
        seed: None,
        skip_missing: false,
        include_score: false,
    }
}

fn mute(layer: &str, on: bool) -> Value {
    json!({
        "description": if on { format!("Mute {layer}") } else { format!("Unmute {layer}") },
        "origin": "user",
        "ops": [{"op": "update_layer", "id": layer, "update": {"muted": on}}]
    })
}

/// The fields the TypeScript `Take` reads, present and typed (app/src/engine/tauri/protocol.ts).
fn assert_take_shape(t: &Value) {
    for f in [
        "n",
        "who",
        "ask",
        "title",
        "say",
        "changed",
        "untouched",
        "render",
        "checks",
    ] {
        assert!(t.get(f).is_some(), "take lacks `{f}`: {t}");
    }
    assert!(t["n"].is_u64());
    assert!(t["score"].is_object(), "take comes back with its score");
    let path = t["render"]["path"].as_str().unwrap();
    assert!(
        Path::new(path).is_absolute() && Path::new(path).is_file(),
        "{path}"
    );
    let r = &t["render"]["report"];
    for f in [
        "sample_rate",
        "length_samples",
        "is_loop",
        "loudness_lufs",
        "true_peak_dbtp",
    ] {
        assert!(r.get(f).is_some(), "render report lacks `{f}`");
    }
    assert!(t["checks"]["checks"].is_array());
    assert!(t["checks"]["passed"].is_boolean());
}

#[test]
fn a_sound_from_first_take_to_export() {
    if !have_piano("a_sound_from_first_take_to_export") {
        return;
    }
    let root = scratch("flow").join("sounds");
    std::fs::create_dir_all(&root).unwrap();

    // A new sound gets a folder named from the request; nothing exists until the first take.
    let dir = new_sound_path(&root, BRIEF);
    assert_eq!(dir, root.join("calm-30-second-felt-piano-loop"));
    assert!(list_sounds(&root).is_empty());
    let sound = s(&dir);

    // Take 1: planned, rendered with the felt piano, checked.
    let r = commands::ask(ask(&sound, BRIEF, None)).unwrap();
    let t1 = &r["take"];
    assert_take_shape(t1);
    assert_eq!(t1["n"], 1);
    assert_eq!(t1["who"], "You");
    assert!(t1["patch"].is_null());
    let checks = &t1["checks"];
    let failed: Vec<&Value> = checks["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["passed"] != true)
        .collect();
    assert!(checks["passed"] == true, "take 1 checks failed: {failed:?}");
    assert_eq!(t1["score"]["key"]["tonic"], "A", "{}", t1["score"]["key"]);
    let layers: Vec<&str> = t1["score"]["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["id"].as_str().unwrap())
        .collect();
    let piano = layers[0].to_string();
    let bars = t1["score"]["bars"].as_u64().unwrap() as u32;
    let last4 = bars.saturating_sub(3).max(1);
    println!("take 1: layers {layers:?}, {}", t1["say"]);

    // The sounds list shows it.
    let list = list_sounds(&root);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].path, sound);
    assert_eq!(list[0].takes, 1);
    assert!(list[0].sub.starts_with("A minor · "), "{}", list[0].sub);
    assert!(list[0].sub.ends_with("loop"), "{}", list[0].sub);

    // `takes`: the whole history with scores and the undo state.
    let all = commands::takes(&sound).unwrap();
    assert_eq!(all["current"], 1);
    assert_eq!(all["takes"].as_array().unwrap().len(), 1);
    assert_take_shape(&all["takes"][0]);
    let rev1 = all["revision"].as_u64().unwrap();
    assert_eq!(read_revision(&dir), rev1);

    // A mute makes no take: project history only, takes.json untouched.
    let m = commands::apply_patch(&sound, mute(&piano, true)).unwrap();
    assert!(m.live.can_undo);
    assert_eq!(m.live.current, 1);
    assert_eq!(m.live.score["layers"][0]["muted"], true);
    assert!(m.inverse["ops"].is_array());
    assert_eq!(read_revision(&dir), rev1);
    let u = commands::undo(&sound).unwrap();
    assert!(u.moved);
    assert_eq!(u.live.score["layers"][0]["muted"], false);
    assert!(u.live.can_redo);
    let rd = commands::redo(&sound).unwrap();
    assert!(rd.moved);
    assert_eq!(rd.live.score["layers"][0]["muted"], true);
    commands::undo(&sound).unwrap();

    // A refused patch carries the TypeScript PatchError shape.
    let bad = commands::apply_patch(&sound, mute("no-such-layer", true)).unwrap_err();
    assert_eq!(bad.code, "patch_rejected");
    assert_eq!(bad.patch_error.as_ref().unwrap()["kind"], "unknown_id");

    // Take 2: a scoped request touches only those bars of that layer.
    let scope = TakeScope {
        layer: piano.clone(),
        bars: BarRange {
            start: last4,
            end: bars,
        },
    };
    let r2 = commands::ask(ask(&sound, "make it a little softer", Some(scope))).unwrap();
    let t2 = &r2["take"];
    assert_take_shape(t2);
    assert_eq!(t2["n"], 2);
    let changed = t2["changed"].as_object().unwrap();
    assert!(changed.keys().all(|k| *k == piano), "{changed:?}");
    if let Some(b) = changed.get(&piano) {
        assert!(b[0].as_u64().unwrap() >= u64::from(last4), "{b}");
    }
    println!("take 2: {} / {}", t2["say"], t2["untouched"]);
    assert_eq!(r2["current"], 2);

    // Undo across a take moves `current` back (new revision for the watcher); redo returns.
    let rev2 = read_revision(&dir);
    let u = commands::undo(&sound).unwrap();
    assert!(u.moved);
    assert_eq!(u.live.current, 1);
    assert_eq!(commands::takes(&sound).unwrap()["current"], 1);
    assert!(read_revision(&dir) > rev2);
    let rd = commands::redo(&sound).unwrap();
    assert_eq!(rd.live.current, 2);

    // Going back to take 1 is an undoable patch.
    let back = commands::restore(&ops::RestoreRequest {
        sound: sound.clone(),
        n: 1,
    })
    .unwrap();
    assert_eq!(back["current"], 1);
    assert_eq!(back["restored"], true);
    assert_eq!(back["can_undo"], true);

    // An agent opens a view; the watcher sees the new revision.
    let from = read_revision(&dir);
    let (tx, rx) = mpsc::channel();
    let handle = watch(dir.clone(), from, Duration::from_millis(25), move |rev| {
        let _ = tx.send(rev);
    });
    std::thread::sleep(Duration::from_millis(80));
    let v = commands::open_view(&ops::OpenViewRequest {
        sound: sound.clone(),
        view: ViewName::Notes,
        who: Some("cut-assistant".into()),
    })
    .unwrap();
    assert_eq!(v["by"], "cut-assistant");
    let seen = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(seen > from);
    drop(handle);
    let views = commands::takes(&sound).unwrap()["views"].clone();
    assert_eq!(views.as_array().unwrap().last().unwrap()["view"], "notes");

    // The live score renders and checks.
    let live = commands::render_live(&sound).unwrap();
    assert!(Path::new(live["output"].as_str().unwrap()).is_file());
    assert!(live["check"]["checks"].is_array());

    // Every export kind of take 2, into a folder inside the sound.
    let out = commands::export(&ExportArgs {
        sound: sound.clone(),
        take: Some(2),
        kinds: ["wav16", "flac", "midi", "project", "stems"]
            .map(String::from)
            .to_vec(),
        destination: "exports".into(),
        base_name: "calm piano / take 2".into(),
    })
    .unwrap();
    assert!(out.ok);
    let kinds: Vec<&str> = out.files.iter().map(|f| f.kind.as_str()).collect();
    for k in ["wav16", "flac", "midi", "project", "stems"] {
        assert!(kinds.contains(&k), "{k} missing from {kinds:?}");
    }
    for f in &out.files {
        assert!(Path::new(&f.path).is_file(), "{}", f.path);
        assert!(f.path.starts_with(&s(&dir.join("exports"))), "{}", f.path);
    }
    assert!(out.files[0].path.ends_with("calm-piano---take-2-wav16.wav"));
    let none = commands::export(&ExportArgs {
        sound: sound.clone(),
        take: Some(9),
        kinds: vec!["wav16".into()],
        destination: "exports".into(),
        base_name: "x".into(),
    });
    assert!(none.is_err());
}

#[test]
fn project_calls_on_a_sound_without_takes_say_so() {
    let root = scratch("empty");
    let dir = root.join("nothing-yet");
    let sound = s(&dir);
    let e = commands::apply_patch(&sound, mute("piano", true)).unwrap_err();
    assert_eq!(e.code, "error");
    assert!(e.message.contains("no take yet"), "{}", e.message);
    assert!(commands::undo(&sound).is_err());
    let t = commands::takes(&sound).unwrap();
    assert_eq!(t["current"], 0);
    assert!(t["takes"].as_array().unwrap().is_empty());
    assert!(t["score"].is_null());
    let bad = commands::apply_patch(&sound, json!({"ops": "not a list"})).unwrap_err();
    assert_eq!(bad.code, "bad_json");
    assert!(list_sounds(&root).is_empty());
}
