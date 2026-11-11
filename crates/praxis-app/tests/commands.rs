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
        reference: None,
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

    // The WebView may load only this sound's audio folders, canonicalized; a `..` path that
    // climbs out of the sound is refused.
    let grant = commands::audio_dirs(&sound).unwrap();
    let canon = std::fs::canonicalize(&dir).unwrap();
    let canon = canon.to_string_lossy();
    let canon = canon.strip_prefix(r"\\?\").unwrap_or(&canon).to_string();
    let want: Vec<PathBuf> = commands::AUDIO_DIRS
        .iter()
        .map(|d| Path::new(&canon).join(d))
        .collect();
    assert_eq!(grant, want);
    assert!(
        grant
            .iter()
            .all(|d| !d.to_string_lossy().starts_with(r"\\?\"))
    );
    assert!(grant[0].join("1").join("mix.wav").is_file());
    let climb = format!(r"{sound}\takes\..\..");
    assert!(commands::audio_dirs(&climb).is_err());
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

#[test]
fn only_a_real_sound_folder_gets_its_audio_served() {
    let root = scratch("scope");
    let refused = |p: &Path| {
        let e = commands::audio_dirs(&s(p)).unwrap_err();
        assert_eq!(e.code, "error", "{}", e.message);
        assert!(
            e.message.contains("not a Praxis sound folder"),
            "{}",
            e.message
        );
    };
    // Any folder with files in it, the drive root, and a folder that does not exist.
    let plain = root.join("documents");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("notes.txt"), "private").unwrap();
    refused(&plain);
    refused(Path::new("C:\\"));
    refused(&root.join("missing"));
    // A takes.json of another format, or one that is not JSON.
    let other = root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("takes.json"),
        r#"{"format":"something-else","version":1,"revision":1,"current":0,"takes":[]}"#,
    )
    .unwrap();
    refused(&other);
    std::fs::write(other.join("takes.json"), "not json").unwrap();
    refused(&other);
    // A real (empty) take history is a sound: its audio folders only.
    let sound = root.join("sound");
    std::fs::create_dir_all(&sound).unwrap();
    std::fs::write(
        sound.join("takes.json"),
        r#"{"format":"praxis-takes","version":1,"revision":1,"current":0,"takes":[],"views":[]}"#,
    )
    .unwrap();
    let grant = commands::audio_dirs(&s(&sound)).unwrap();
    assert_eq!(grant.len(), 3);
    for (d, name) in grant.iter().zip(commands::AUDIO_DIRS) {
        assert_eq!(d.file_name().unwrap(), name);
        assert!(d.parent().unwrap().ends_with("sound"));
    }
    // Reaching the plain folder through the sound with `..` is still refused.
    refused(&sound.join("..").join("documents"));
}

/// The assistant through the app's `ask` (WP-AG2): each step reaches the progress callback the
/// shell turns into `praxis://agent` events, and a turn without a take comes back as a reply.
/// A scripted model stands in for the API (no key, no network).
#[test]
fn the_assistant_reports_its_steps_and_replies_without_a_take() {
    use praxis_app::commands::{AGENT_EVENT, AgentProgress};
    use praxis_cli::agent::AgentEvent;
    use praxis_cli::agent::testing::{ScriptedModel, message, tool_use};
    use std::sync::{Arc, Mutex};

    let sound = scratch("assistant-reply").join("logo");
    let q = "Should it be a short rising sting or a soft chime?";
    let model = ScriptedModel::new(vec![
        message(
            vec![tool_use(
                "r1",
                "read_sound",
                json!({"part": "summary", "layer": "", "take": 0}),
            )],
            "tool_use",
        ),
        message(
            vec![tool_use("q1", "ask_user", json!({"question": q}))],
            "tool_use",
        ),
    ]);
    let seen: Arc<Mutex<Vec<AgentProgress>>> = Arc::default();
    let sink = seen.clone();
    let name = s(&sound);
    let progress: praxis_cli::agent::Progress = Arc::new(move |e: &AgentEvent| {
        sink.lock().unwrap().push(AgentProgress {
            sound: name.clone(),
            event: e.clone(),
        })
    });
    let mut req = ask(&s(&sound), "a sound for my app's logo", None);
    req.provider = None;
    let v = commands::ask_with(req, Some(progress), Some(model)).expect("a reply, not an error");
    assert_eq!(v["take"], Value::Null);
    assert_eq!(v["no_take"], json!("question"));
    assert_eq!(v["reply"], json!(q));

    let seen = seen.lock().unwrap().clone();
    let steps: Vec<Value> = seen
        .iter()
        .map(|p| serde_json::to_value(p).unwrap())
        .filter(|p| p["event"] == "step")
        .collect();
    assert_eq!(AGENT_EVENT, "praxis://agent");
    // The event the window gets: the sound, then the AgentEvent's own fields.
    assert_eq!(
        steps[0],
        json!({"sound": s(&sound), "event": "step", "n": 1, "tool": "read_sound", "words": "Reading the sound"})
    );
    assert_eq!(steps[1]["words"], json!("Asking you a question"));
    assert!(seen.iter().any(|p| matches!(
        &p.event,
        AgentEvent::Finished { outcome, .. } if outcome == "question"
    )));
}
