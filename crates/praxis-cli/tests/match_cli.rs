//! `praxis match` end to end through the built executable (docs/contracts/cli.md): a reference
//! from the matcher's reference set is rendered to a WAV, matched, and the payload carries
//! `project, similarity, iterations[], originality`. The keychain is off and no API key is set.

use praxis_match::refset;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_praxis");

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("praxis-match-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn praxis(args: &[&str]) -> (i32, Value) {
    let out = Command::new(BIN)
        .args(args)
        .current_dir(repo())
        .env("PRAXIS_KEYCHAIN", "off")
        .env_remove("ANTHROPIC_API_KEY")
        .output()
        .expect("run praxis");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_else(|| panic!("no output; stderr {}", String::from_utf8_lossy(&out.stderr)));
    (
        out.status.code().unwrap_or(-1),
        serde_json::from_str(line).expect("stdout is JSON"),
    )
}

/// Writes one reference of the set as a score file and renders it to a WAV with `praxis render`.
fn reference_wav(dir: &Path, name: &str) -> PathBuf {
    let r = refset::references()
        .into_iter()
        .find(|r| r.name == name)
        .expect("reference");
    let score = dir.join(format!("{name}.score.json"));
    std::fs::write(&score, praxis_score::to_json(&r.score)).unwrap();
    let wav = dir.join(format!("{name}.wav"));
    let (code, v) = praxis(&[
        "render",
        score.to_str().unwrap(),
        "-o",
        wav.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 0, "{v}");
    wav
}

#[test]
fn match_writes_a_project_with_similarity_iterations_and_originality() {
    let dir = scratch("cli");
    let wav = reference_wav(&dir, "heavy_impact");
    let project = dir.join("like-impact.praxis");
    let (code, v) = praxis(&[
        "match",
        wav.to_str().unwrap(),
        "-o",
        project.to_str().unwrap(),
        "--max-iterations",
        "16",
        "--json",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["command"], "match");
    assert!(project.is_file());
    let sim = v["similarity"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&sim), "{sim}");
    let its = v["iterations"].as_array().unwrap();
    assert!(its.len() >= 3);
    for it in its {
        for k in ["n", "stage", "tried", "similarity", "best", "accepted"] {
            assert!(it.get(k).is_some(), "{k} in {it}");
        }
    }
    assert_eq!(v["originality"]["passed"], true, "{v}");
    for k in ["melodic_similarity", "rhythmic_similarity", "reason"] {
        assert!(v["originality"].get(k).is_some(), "{k}");
    }
    assert!(v["made_with"].is_string());
    assert!(
        v["reference"]["file_name"]
            .as_str()
            .unwrap()
            .ends_with(".wav")
    );
    // The project holds a Score, never the reference's audio.
    let text = std::fs::read_to_string(&project).unwrap();
    assert!(text.contains("\"praxis-project\""));
    assert!(text.len() < 200_000, "{} bytes", text.len());
}

#[test]
fn match_errors_are_coded() {
    let dir = scratch("errors");
    let missing = dir.join("missing.wav");
    let (code, v) = praxis(&[
        "match",
        missing.to_str().unwrap(),
        "-o",
        dir.join("x.praxis").to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "io", "{v}");
    let wav = reference_wav(&dir, "heavy_impact");
    let (code, v) = praxis(&[
        "match",
        wav.to_str().unwrap(),
        "--key",
        "H sideways",
        "-o",
        dir.join("x.praxis").to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 1);
    assert_eq!(v["error"]["code"], "error", "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("key"));
}
