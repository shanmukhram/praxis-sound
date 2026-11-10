//! The `praxis` binary, end to end: every command's JSON success and error shapes
//! (docs/contracts/cli.md), through the built executable.
//!
//! Renders need the Upright Piano KW pack. Point `PRAXIS_PACKS` at a packs directory that has it
//! installed; tests that need it print a message and skip when it is absent. The keychain is
//! always disabled (`PRAXIS_KEYCHAIN=off`) and `ANTHROPIC_API_KEY` removed, so no test reads or
//! changes a real key.

use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_praxis");
const PACK: &str = "upright-piano-kw";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> PathBuf {
    repo().join("fixtures").join(name)
}

/// A fresh scratch directory for one test.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("praxis-cli-{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    /// The single JSON object on stdout.
    fn json(&self) -> Value {
        let lines: Vec<&str> = self
            .stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        assert_eq!(
            lines.len(),
            1,
            "exactly one JSON line on stdout, got {:?} (stderr {:?})",
            self.stdout,
            self.stderr
        );
        serde_json::from_str(lines[0]).expect("stdout is JSON")
    }
}

fn run_with(args: &[&str], env: &[(&str, &str)], stdin: Option<&str>) -> Run {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .current_dir(repo())
        .env("PRAXIS_KEYCHAIN", "off")
        .env_remove("ANTHROPIC_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("run praxis");
    {
        let mut pipe = child.stdin.take().expect("stdin");
        if let Some(s) = stdin {
            pipe.write_all(s.as_bytes()).expect("write stdin");
        }
    }
    let out = child.wait_with_output().expect("praxis output");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn run(args: &[&str]) -> Run {
    run_with(args, &[], None)
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf-8 path")
}

/// Asserts a success envelope for `command` and returns it.
fn ok(r: &Run, command: &str) -> Value {
    let v = r.json();
    assert_eq!(v["ok"], json!(true), "{command}: {v}");
    assert_eq!(v["command"], json!(command), "{v}");
    v
}

/// Asserts an error envelope with `code`, exit 1, and returns it.
fn err(r: &Run, command: &str, code: &str) -> Value {
    let v = r.json();
    assert_eq!(v["ok"], json!(false), "{command}: {v}");
    assert_eq!(v["command"], json!(command), "{v}");
    assert_eq!(v["error"]["code"], json!(code), "{v}");
    assert!(v["error"]["message"].is_string(), "{v}");
    assert_eq!(r.code, 1, "error exit code: {v}");
    v
}

fn has_fields(v: &Value, fields: &[&str]) {
    for f in fields {
        assert!(v.get(*f).is_some(), "missing `{f}` in {v}");
    }
}

/// True when the piano pack is installed where the binary looks (`PRAXIS_PACKS`, else near the
/// repo). Prints why a test is skipped otherwise.
fn have_piano(test: &str) -> bool {
    let r = run(&["--json", "packs", "list"]);
    let v = r.json();
    let installed = v["packs"]
        .as_array()
        .map(|ps| {
            ps.iter()
                .any(|p| p["name"] == PACK && p["installed"] == json!(true))
        })
        .unwrap_or(false);
    if !installed {
        println!(
            "SKIP {test}: the `{PACK}` pack is not installed; set PRAXIS_PACKS to a packs \
             directory that has it"
        );
    }
    installed
}

fn check_named<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["checks"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["name"] == name))
        .unwrap_or_else(|| panic!("no `{name}` check in {report}"))
}

/// Prints every failing check, so loudness or grid failures are visible in the test output.
fn report_failures(label: &str, report: &Value) {
    for c in report["checks"].as_array().into_iter().flatten() {
        if c["passed"] != json!(true) {
            println!(
                "{label}: check `{}` FAILED: expected {}, measured {}",
                c["name"], c["expected"], c["measured"]
            );
        }
    }
}

const RENDER_FIELDS: &[&str] = &[
    "output",
    "stems",
    "sample_rate",
    "frames",
    "duration_s",
    "is_loop",
    "loudness_lufs",
    "true_peak_dbtp",
    "applied_gain_db",
    "blocked_layers",
    "render_seconds",
    "realtime_factor",
    "check",
];

const VERIFY_FIELDS: &[&str] = &[
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
];

// ---------------------------------------------------------------- validate

#[test]
fn validate_success_and_errors() {
    let v = ok(
        &run(&["--json", "validate", s(&fixture("acceptance1.score.json"))]),
        "validate",
    );
    assert_eq!(v["valid"], json!(true));
    assert!(v["errors"].as_array().is_some_and(Vec::is_empty));
    assert!(v["warnings"].is_array());

    let dir = scratch("validate");
    // Parses as a Score but breaks a rule: invalid_score with {code, path, message} issues.
    let mut score: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("acceptance1.score.json")).unwrap())
            .unwrap();
    score["bars"] = json!(0);
    let bad = dir.join("bad.score.json");
    std::fs::write(&bad, score.to_string()).unwrap();
    let v = err(
        &run(&["--json", "validate", s(&bad)]),
        "validate",
        "invalid_score",
    );
    let issues = v["error"]["issues"].as_array().expect("issues");
    assert!(!issues.is_empty());
    for i in issues {
        has_fields(i, &["code", "path", "message"]);
    }

    let not_score = dir.join("not.score.json");
    std::fs::write(&not_score, r#"{"bars": 4}"#).unwrap();
    err(
        &run(&["--json", "validate", s(&not_score)]),
        "validate",
        "bad_score",
    );
    err(
        &run(&["--json", "validate", s(&dir.join("missing.json"))]),
        "validate",
        "io",
    );
}

// ---------------------------------------------------------------- schema

#[test]
fn schema_prints_score_patch_and_plan() {
    let v = ok(&run(&["--json", "schema"]), "schema");
    for k in ["score", "patch", "plan"] {
        assert!(v[k].is_object(), "`{k}` schema: {v}");
    }
}

// ---------------------------------------------------------------- render + check

#[test]
fn render_errors() {
    let dir = scratch("render_errors");
    let out = dir.join("x.wav");
    let acc = fixture("acceptance1.score.json");
    err(
        &run(&[
            "--json",
            "render",
            s(&dir.join("missing.json")),
            "-o",
            s(&out),
        ]),
        "render",
        "io",
    );
    let garbage = dir.join("garbage.json");
    std::fs::write(&garbage, "not json").unwrap();
    err(
        &run(&["--json", "render", s(&garbage), "-o", s(&out)]),
        "render",
        "bad_json",
    );
    let not_score = dir.join("not.score.json");
    std::fs::write(&not_score, r#"{"bars": 4}"#).unwrap();
    err(
        &run(&["--json", "render", s(&not_score), "-o", s(&out)]),
        "render",
        "bad_score",
    );
    err(
        &run(&[
            "--json",
            "render",
            s(&acc),
            "-o",
            s(&out),
            "--format",
            "mp3",
        ]),
        "render",
        "bad_format",
    );
    // No pack installed in an empty packs directory.
    let empty = dir.join("no-packs");
    std::fs::create_dir_all(&empty).unwrap();
    err(
        &run_with(
            &["--json", "render", s(&acc), "-o", s(&out)],
            &[("PRAXIS_PACKS", s(&empty))],
            None,
        ),
        "render",
        "pack_missing",
    );
    // The bell fixture's presets are not built yet: render_failed, not a crash.
    err(
        &run(&[
            "--json",
            "render",
            s(&fixture("acceptance1_bell.score.json")),
            "-o",
            s(&out),
        ]),
        "render",
        "render_failed",
    );
    // Argument errors still give one JSON object when --json is present.
    err(&run(&["--json", "render"]), "render", "error");
    assert!(!out.exists(), "no output written on failure");
}

#[test]
fn render_and_check_the_fixture() {
    if !have_piano("render_and_check_the_fixture") {
        return;
    }
    let dir = scratch("render_check");
    let score = fixture("acceptance1.score.json");
    let wav = dir.join("acc.wav");
    let stems = dir.join("stems");
    // The kick preset is not built yet, so render what exists and say what was skipped.
    let r = run(&[
        "--json",
        "render",
        s(&score),
        "-o",
        s(&wav),
        "--skip-missing",
        "--stems",
        s(&stems),
    ]);
    assert_eq!(r.code, 0, "{}", r.stdout);
    let v = ok(&r, "render");
    has_fields(&v, RENDER_FIELDS);
    assert_eq!(v["sample_rate"], json!(48_000));
    assert_eq!(v["is_loop"], json!(true));
    assert!(
        (v["duration_s"].as_f64().unwrap() - 30.0).abs() < 1e-6,
        "{v}"
    );
    assert_eq!(v["frames"], json!(1_440_000));
    assert!(wav.is_file());
    for stem in v["stems"].as_array().unwrap() {
        assert!(Path::new(stem.as_str().unwrap()).is_file(), "stem {stem}");
    }
    assert!(!v["stems"].as_array().unwrap().is_empty());
    for sk in v["skipped_layers"].as_array().unwrap() {
        println!("render: skipped layer {sk}");
    }
    // Every render carries the verify report.
    let rc = &v["check"];
    has_fields(rc, VERIFY_FIELDS);
    report_failures("render", rc);
    assert_eq!(check_named(rc, "loop_seam")["passed"], json!(true), "{rc}");
    assert_eq!(check_named(rc, "tempo")["passed"], json!(true), "{rc}");
    assert_eq!(check_named(rc, "key")["passed"], json!(true), "{rc}");

    // `check` on the file: same report, `passed`, exit 1 iff a check fails.
    let r = run(&["--json", "check", s(&wav), "--score", s(&score)]);
    let c = ok(&r, "check");
    has_fields(&c, VERIFY_FIELDS);
    report_failures("check", &c);
    assert_eq!(r.code, if c["passed"] == json!(true) { 0 } else { 1 });
    assert_eq!(check_named(&c, "loop_seam")["passed"], json!(true), "{c}");
    let grid = check_named(&c, "grid");
    println!(
        "check: grid passed={} measured {} (onsets_on_grid {})",
        grid["passed"], grid["measured"], c["onsets_on_grid"]
    );
}

#[test]
fn render_flac() {
    if !have_piano("render_flac") {
        return;
    }
    let dir = scratch("render_flac");
    let score = fixture("acceptance1.score.json");
    let flac = dir.join("acc.flac");
    let v = ok(
        &run(&[
            "--json",
            "render",
            s(&score),
            "-o",
            s(&flac),
            "--format",
            "flac",
            "--skip-missing",
        ]),
        "render",
    );
    assert_eq!(v["format"], json!("flac24"));
    let bytes = std::fs::read(&flac).unwrap();
    assert_eq!(&bytes[..4], b"fLaC");
    // `check` decodes FLAC too.
    let c = run(&["--json", "check", s(&flac), "--score", s(&score)]).json();
    assert_eq!(c["ok"], json!(true), "{c}");
    assert!(
        (c["duration_s"].as_f64().unwrap() - 30.0).abs() < 1e-3,
        "{c}"
    );
}

#[test]
fn check_errors() {
    let dir = scratch("check_errors");
    let score = fixture("acceptance1.score.json");
    err(
        &run(&[
            "--json",
            "check",
            s(&dir.join("missing.wav")),
            "--score",
            s(&score),
        ]),
        "check",
        "io",
    );
    let not_audio = dir.join("not-audio.wav");
    std::fs::write(&not_audio, "this is not audio").unwrap();
    err(
        &run(&["--json", "check", s(&not_audio), "--score", s(&score)]),
        "check",
        "decode_failed",
    );
    err(
        &run(&[
            "--json",
            "check",
            s(&not_audio),
            "--score",
            s(&dir.join("missing.json")),
        ]),
        "check",
        "io",
    );
}
