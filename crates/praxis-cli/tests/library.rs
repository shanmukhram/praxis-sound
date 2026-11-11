//! `praxis analyze` returns the complete Sound Profile and `praxis library` keeps profiles (never
//! audio), through the command layer and the binary. The clips are engine renders of three
//! scores of the labelled set: music (a G-major pluck loop), a sound effect (an impact) and an
//! ambience (a noise bed); none needs a pack.

mod clipgen;

use praxis_cli::ops::{
    self, AnalyzeRequest, LibraryAction, LibraryRequest, RenderSettings, render::render_score,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("praxis-library-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn render(id: &str, dir: &Path) -> PathBuf {
    let text = std::fs::read_to_string(clipgen::clips_dir().join(format!("{id}.score.json")))
        .expect("score");
    let score = praxis_score::from_json(&text).expect("valid");
    let out = dir.join(format!("{id}.wav"));
    render_score(&score, &out, &RenderSettings::default()).expect("render");
    out
}

fn analyze(clip: &Path, out: Option<PathBuf>) -> Value {
    ops::analyze(&AnalyzeRequest {
        clip: clip.to_path_buf(),
        out,
        beats_per_bar: 4,
    })
    .expect("analyze")
}

#[test]
fn analyze_returns_the_complete_profile_for_music_sfx_and_ambience() {
    let dir = temp_dir("analyze");
    let cases = [
        ("m02_g_major_97_pluck", "music"),
        ("s05_impact", "sfx"),
        ("a01_noise_bed", "ambience"),
    ];
    for (id, kind) in cases {
        let wav = render(id, &dir);
        let json_out = dir.join(format!("{id}.profile.json"));
        let v = analyze(&wav, Some(json_out.clone()));
        eprintln!("{id}: {}", v["summary"].as_str().unwrap());
        eprintln!(
            "  class {} {:?} | tempo {} | key {} | lufs {:.1} lra {:.1} crest {:.1} | attack {:.3} decay {:.3} sustain {:.2} release {:.2}",
            v["class"]["kind"],
            v["class"]["scores"],
            v["tempo"]["bpm"],
            v["key"]["name"],
            v["loudness"]["integrated_lufs"].as_f64().unwrap(),
            v["loudness"]["range_lu"].as_f64().unwrap(),
            v["loudness"]["crest_db"].as_f64().unwrap(),
            v["envelope"]["attack_s"].as_f64().unwrap(),
            v["envelope"]["decay_s"].as_f64().unwrap(),
            v["envelope"]["sustain"].as_f64().unwrap(),
            v["envelope"]["release_s"].as_f64().unwrap(),
        );
        eprintln!(
            "  centroid {:.0} Hz rolloff {:.0} Hz flatness {:.3} tilt {:.1} dB/oct bands {:?}",
            v["spectral"]["centroid_hz"].as_f64().unwrap(),
            v["spectral"]["rolloff_hz"].as_f64().unwrap(),
            v["spectral"]["flatness"].as_f64().unwrap(),
            v["spectral"]["tilt_db_per_oct"].as_f64().unwrap(),
            v["spectral"]["bands_db"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| (b.as_f64().unwrap() * 10.0).round() / 10.0)
                .collect::<Vec<_>>(),
        );
        let voiced = v["pitch"]["midi"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| !m.is_null())
            .count();
        eprintln!(
            "  groove {} | tonality {:.2} | width {:.2} corr {:.2} | reverb {} | sections {} | voiced frames {voiced} of {} | onsets {}",
            v["groove"],
            v["tonality"].as_f64().unwrap(),
            v["stereo"]["width"].as_f64().unwrap(),
            v["stereo"]["correlation"].as_f64().unwrap(),
            v["reverb"],
            v["sections"].as_array().unwrap().len(),
            v["pitch"]["midi"].as_array().unwrap().len(),
            v["onsets_s"].as_array().unwrap().len(),
        );
        assert_eq!(v["complete"], true);
        assert_eq!(v["missing"], serde_json::json!([]));
        assert_eq!(v["class"]["kind"], kind, "{id}");
        assert_eq!(v["source"]["pcm_hash"].as_str().unwrap().len(), 64);
        assert_eq!(v["source"]["source_kind"], "audio");
        let names = v["feature_names"].as_array().unwrap();
        assert_eq!(names.len(), v["features"].as_array().unwrap().len());
        for field in [
            "source", "class", "tempo", "key", "loudness", "envelope", "spectral", "groove",
            "onsets_s", "pitch", "tonality", "stereo", "reverb", "sections", "features",
        ] {
            assert!(v.get(field).is_some(), "{id}: no {field}");
        }
        // The written file reads back as a SoundProfile.
        let text = std::fs::read_to_string(&json_out).unwrap();
        let p: praxis_analyze::SoundProfile = serde_json::from_str(&text).expect("profile JSON");
        assert_eq!(p.source.pcm_hash, v["source"]["pcm_hash"].as_str().unwrap());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn library_keeps_profiles_tags_and_never_audio() {
    let dir = temp_dir("lib");
    let lib = dir.join("library");
    let wav = render("s05_impact", &dir);
    let req = |action: LibraryAction| LibraryRequest {
        action,
        dir: Some(lib.clone()),
    };
    let add = ops::library(&req(LibraryAction::Add {
        input: wav.clone(),
        name: Some("Big hit".into()),
        tags: vec!["impact".into(), "dark".into(), "impact".into()],
        beats_per_bar: 4,
    }))
    .unwrap();
    assert_eq!(add["id"], "big-hit");
    assert_eq!(add["already"], false);
    assert_eq!(add["entry"]["tags"], serde_json::json!(["impact", "dark"]));
    assert_eq!(add["entry"]["kind"], "sfx");

    // The same clip again (as audio, then as the profile JSON) is kept once.
    let again = ops::library(&req(LibraryAction::Add {
        input: wav.clone(),
        name: None,
        tags: vec!["trailer".into()],
        beats_per_bar: 4,
    }))
    .unwrap();
    assert_eq!(again["already"], true);
    assert_eq!(again["id"], "big-hit");
    let profile_json = dir.join("hit.profile.json");
    analyze(&wav, Some(profile_json.clone()));
    let from_json = ops::library(&req(LibraryAction::Add {
        input: profile_json,
        name: None,
        tags: vec![],
        beats_per_bar: 4,
    }))
    .unwrap();
    assert_eq!(from_json["already"], true);

    // A second clip gets its own entry.
    let bed = render("a01_noise_bed", &dir);
    let add2 = ops::library(&req(LibraryAction::Add {
        input: bed,
        name: Some("Big hit".into()),
        tags: vec![],
        beats_per_bar: 4,
    }))
    .unwrap();
    assert_eq!(add2["id"], "big-hit-2");

    let list = ops::library(&req(LibraryAction::List)).unwrap();
    assert_eq!(list["count"], 2);
    assert_eq!(list["entries"][0]["id"], "big-hit");
    assert_eq!(
        list["entries"][0]["tags"],
        serde_json::json!(["impact", "dark", "trailer"])
    );

    let tag = ops::library(&req(LibraryAction::Tag {
        id: "big-hit".into(),
        add: vec!["cinematic".into()],
        remove: vec!["dark".into()],
    }))
    .unwrap();
    assert_eq!(
        tag["entry"]["tags"],
        serde_json::json!(["impact", "trailer", "cinematic"])
    );

    let show = ops::library(&req(LibraryAction::Show {
        id: "big-hit".into(),
    }))
    .unwrap();
    assert_eq!(show["profile"]["style_name"], "Big hit");
    assert_eq!(show["profile"]["complete"], true);

    // Only profile JSON files are in the library, and they hold no audio-sized data.
    for e in std::fs::read_dir(&lib).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        assert!(name.ends_with(".profile.json"), "{name}");
        assert!(e.metadata().unwrap().len() < 2_000_000, "{name}");
    }

    let rm = ops::library(&req(LibraryAction::Remove {
        id: "big-hit-2".into(),
    }))
    .unwrap();
    assert_eq!(rm["removed"], true);
    let err = ops::library(&req(LibraryAction::Show {
        id: "big-hit-2".into(),
    }))
    .unwrap_err();
    assert!(err.message.contains("no library entry"), "{}", err.message);
    let bad = ops::library(&req(LibraryAction::Show {
        id: "../secret".into(),
    }))
    .unwrap_err();
    assert!(bad.message.contains("not a library id"));
    let _ = std::fs::remove_dir_all(&dir);
}

fn praxis(home: &Path, args: &[&str]) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_praxis"))
        .args(args)
        .env("PRAXIS_HOME", home)
        .env("PRAXIS_KEYCHAIN", "off")
        .output()
        .expect("run praxis");
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("{e}: {text}"))
}

#[test]
fn the_binary_analyses_and_keeps_a_library() {
    let dir = temp_dir("bin");
    let home = dir.join("home");
    let wav = render("a01_noise_bed", &dir);
    let wav_s = wav.to_string_lossy().to_string();
    let a = praxis(&home, &["--json", "analyze", &wav_s]);
    assert_eq!(a["ok"], true, "{a}");
    assert_eq!(a["command"], "analyze");
    assert_eq!(a["complete"], true);
    assert_eq!(a["class"]["kind"], "ambience");
    let add = praxis(
        &home,
        &[
            "--json", "library", "add", &wav_s, "--name", "Rain bed", "--tag", "rain",
        ],
    );
    assert_eq!(add["ok"], true, "{add}");
    assert_eq!(add["id"], "rain-bed");
    assert!(home.join("library").join("rain-bed.profile.json").is_file());
    let list = praxis(&home, &["--json", "library", "list"]);
    assert_eq!(list["entries"][0]["name"], "Rain bed");
    let tag = praxis(
        &home,
        &[
            "--json", "library", "tag", "rain-bed", "--add", "night", "--remove", "rain",
        ],
    );
    assert_eq!(tag["entry"]["tags"], serde_json::json!(["night"]));
    let show = praxis(&home, &["--json", "library", "show", "rain-bed"]);
    assert_eq!(show["profile"]["class"]["kind"], "ambience");
    let missing = praxis(&home, &["--json", "library", "show", "nothing-here"]);
    assert_eq!(missing["ok"], false);
    let rm = praxis(&home, &["--json", "library", "remove", "rain-bed"]);
    assert_eq!(rm["removed"], true);
    let _ = std::fs::remove_dir_all(&dir);
}
