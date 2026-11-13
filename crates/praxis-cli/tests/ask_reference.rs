//! `ask` with a `reference` (a dropped clip or a library style), docs/contracts/cli.md "Takes":
//! "make something like this" runs the match loop; other requests give the planner the
//! reference's summary and key numbers and start the assistant's turn with a synthetic
//! `analyze_clip` call holding the trimmed profile. The take records the reference.
//!
//! The keychain is off (`PRAXIS_KEYCHAIN=off` in the test environment) and the assistant is a
//! scripted model, so nothing reaches a network.

use praxis_cli::agent::testing::{ScriptedModel, message, text};
use praxis_cli::agent::{AgentConfig, ModelClient};
use praxis_cli::ops::{self, AskOptions, ProviderChoice, ReferenceArg};
use praxis_engine::{RenderOptions, render_offline};
use praxis_instruments::Registry;
use praxis_match::refset;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("praxis-askref-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::path::absolute(dir).unwrap()
}

/// True when the Upright Piano KW pack is installed; otherwise prints why `test` is skipped (CI
/// does not install packs), like the other pack-dependent tests.
fn have_piano(test: &str) -> bool {
    let ok = praxis_instruments::Packs::discover().has_pack("upright-piano-kw");
    if !ok {
        println!("SKIP {test}: the upright-piano-kw pack is not installed (set PRAXIS_PACKS)");
    }
    ok
}

/// A reference of the matcher's set rendered to a WAV (the felt-piano references need the
/// Upright Piano KW pack; check [`have_piano`] first).
fn reference_wav(dir: &Path, name: &str) -> PathBuf {
    let r = refset::references()
        .into_iter()
        .find(|r| r.name == name)
        .unwrap();
    let out = render_offline(
        &r.score,
        &Registry::with_default_packs(),
        &RenderOptions::default(),
    )
    .unwrap();
    let wav = dir.join(format!("{name}.wav"));
    praxis_audio_io::wav::write_wav(
        &wav,
        &out.master,
        48_000,
        praxis_audio_io::wav::WavFormat::Float32,
    )
    .unwrap();
    wav
}

fn ask(sound: &Path, text: &str, clip: &Path) -> ops::AskRequest {
    ops::AskRequest {
        sound: sound.display().to_string(),
        text: text.into(),
        who: Some("You".into()),
        scope: None,
        patch: None,
        title: None,
        provider: Some(ProviderChoice::Offline),
        model: None,
        seed: Some(5),
        skip_missing: false,
        include_score: false,
        reference: Some(ReferenceArg {
            name: clip.file_name().unwrap().to_string_lossy().into_owned(),
            clip: Some(clip.to_path_buf()),
            library_id: None,
        }),
    }
}

fn takes(sound: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(sound.join("takes.json")).unwrap()).unwrap()
}

#[test]
fn like_this_runs_the_match_loop_and_the_take_records_the_reference() {
    let dir = scratch("like");
    let wav = reference_wav(&dir, "heavy_impact");
    let sound = dir.join("hit");
    let out = ops::ask(&ask(&sound, "make something like this", &wav)).expect("take 1");
    assert_eq!(out["take"]["n"], 1);
    let t = &takes(&sound)["takes"][0];
    assert_eq!(t["provider"], "match", "{t}");
    assert_eq!(t["reference"]["name"], "heavy_impact.wav");
    assert!(
        t["reference"]["clip"]
            .as_str()
            .unwrap()
            .ends_with("heavy_impact.wav")
    );
    assert!(t["say"].as_str().unwrap().contains("similarity"), "{t}");
    // A second "like this" on the same sound replaces it as one undoable take.
    let out = ops::ask(&ask(&sound, "another one like this, darker", &wav)).expect("take 2");
    assert_eq!(out["take"]["n"], 2);
    assert_eq!(takes(&sound)["takes"][1]["provider"], "match");
}

#[test]
fn other_requests_give_the_planner_the_reference_numbers() {
    // `lullaby_felt` is a felt-piano reference.
    if !have_piano("other_requests_give_the_planner_the_reference_numbers") {
        return;
    }
    let dir = scratch("planner");
    // The felt-piano lullaby profiles as music at 72 BPM in D major.
    let wav = reference_wav(&dir, "lullaby_felt");
    let sound = dir.join("lullaby");
    let mut req = ask(&sound, "a calm piano loop", &wav);
    req.include_score = true;
    let out = ops::ask(&req).expect("take 1");
    let t = &takes(&sound)["takes"][0];
    assert_eq!(t["provider"], "offline-rules", "{t}");
    assert_eq!(t["ask"], "a calm piano loop");
    assert_eq!(t["reference"]["name"], "lullaby_felt.wav");
    // The reference's tempo and key reached the planner through the words.
    let score = &out["take"]["score"];
    let bpm = score["tempo"]["bpm"].as_f64().unwrap();
    assert!((bpm - 72.0).abs() <= 2.0, "{bpm}");
    assert_eq!(score["key"]["tonic"], "D", "{}", score["key"]);
}

#[test]
fn the_trimmed_profile_is_small_and_the_wording_is_recognised() {
    let dir = scratch("trim");
    let wav = reference_wav(&dir, "bell_arp");
    let (p, _) = ops::analyze::profile_clip(&wav, 4).unwrap();
    let full = serde_json::to_string(&p).unwrap().len();
    let trimmed = serde_json::to_string(&ops::takes::trimmed_profile(&p))
        .unwrap()
        .len();
    println!("profile {full} bytes, trimmed {trimmed} bytes");
    assert!(trimmed < 3_000, "{trimmed} bytes");
    for yes in [
        "",
        "make something like this",
        "Something similar but darker",
        "in the style of this clip",
    ] {
        assert!(ops::takes::wants_like(yes), "{yes:?}");
    }
    for no in ["a calm piano loop", "make the bass louder"] {
        assert!(!ops::takes::wants_like(no), "{no:?}");
    }
}

#[test]
fn the_assistant_starts_with_the_reference_profile() {
    let dir = scratch("agent");
    let wav = reference_wav(&dir, "bell_arp");
    let sound = dir.join("bells");
    let model = ScriptedModel::new(vec![message(
        vec![text("I can see the reference; tell me what to make.")],
        "end_turn",
    )]);
    let mut req = ask(&sound, "what key is the reference in?", &wav);
    req.provider = Some(ProviderChoice::Claude);
    let client: Arc<dyn ModelClient> = model.clone();
    let opts = AskOptions {
        client: Some(client),
        agent: AgentConfig::default(),
        progress: None,
    };
    ops::ask_with(&req, &opts).expect("a reply");
    let first = &model.requests()[0].body["messages"];
    let msgs = first.as_array().unwrap();
    assert_eq!(msgs.len(), 3, "{first}");
    assert_eq!(msgs[1]["role"], "assistant");
    assert_eq!(msgs[1]["content"][0]["name"], "analyze_clip");
    let result = &msgs[2]["content"][0];
    assert_eq!(result["type"], "tool_result");
    let content = result["content"].as_str().unwrap();
    assert!(content.len() < 3_500, "{} bytes", content.len());
    let v: Value = serde_json::from_str(content).unwrap();
    assert_eq!(v["reference"], "bell_arp.wav");
    assert!(v["profile"]["summary"].is_string());
    assert!(v["profile"]["pitch"].get("midi").is_none());
}
