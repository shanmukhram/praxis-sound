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
    // The bell fixture's synth presets exist now (WP-C2): it renders.
    ok(
        &run(&[
            "--json",
            "render",
            s(&fixture("acceptance1_bell.score.json")),
            "-o",
            s(&dir.join("bell_ok.wav")),
        ]),
        "render",
    );
    // A preset no registry knows is a render_failed error, not a crash.
    let unknown = dir.join("unknown_preset.score.json");
    let text = std::fs::read_to_string(fixture("acceptance1_bell.score.json"))
        .unwrap()
        .replace("\"preset\": \"bell\"", "\"preset\": \"no_such_preset\"");
    std::fs::write(&unknown, text).unwrap();
    err(
        &run(&["--json", "render", s(&unknown), "-o", s(&out)]),
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

// ---------------------------------------------------------------- generate

const BRIEF: &str = "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt \
piano playing a repeating rhythmic motif on an eighth-note grid, with accents and overlapping \
sustain. Bars 9-16: the identical piano part, plus a soft muted kick layered underneath. No new \
melody, no fills. No risers, sweeps, impacts or pads anywhere.";

fn plan_file() -> PathBuf {
    repo().join("crates/praxis-planner/tests/prompts/acceptance1.plan.json")
}

fn assert_summary(v: &Value) {
    let sum = &v["score_summary"];
    has_fields(sum, &["key", "bpm", "bars", "layers", "sections"]);
    assert_eq!(sum["key"], json!("a minor"), "{sum}");
    assert_eq!(sum["bars"], json!(16));
    assert!(sum["layers"].as_array().is_some_and(|l| !l.is_empty()));
    has_fields(v, &["project", "provider", "attempts", "repairs"]);
    assert!(v["repairs"].is_array());
}

#[test]
fn generate_offline_from_the_acceptance_brief() {
    let dir = scratch("generate_offline");
    let project = dir.join("acc.praxis");
    let r = run(&[
        "--json",
        "generate",
        "--prompt",
        BRIEF,
        "--provider",
        "offline",
        "-o",
        s(&project),
    ]);
    let v = ok(&r, "generate");
    assert_summary(&v);
    assert_eq!(v["provider"], json!("offline-rules"));
    assert_eq!(v["attempts"], json!(1));
    assert!(
        v["provider_note"].is_null(),
        "explicit choice needs no note"
    );
    assert!(v.get("render").is_none());
    let text = std::fs::read_to_string(&project).expect("project written");
    let saved: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(saved["format"], json!("praxis-project"));
    assert_eq!(saved["score"]["bars"], json!(16));

    // The project is a valid input to every score-taking command.
    let mid = dir.join("acc.mid");
    ok(
        &run(&["--json", "export-midi", s(&project), "-o", s(&mid)]),
        "export-midi",
    );

    // Same brief and seed, same project.
    let again = dir.join("again.praxis");
    ok(
        &run(&[
            "--json",
            "generate",
            "--prompt",
            BRIEF,
            "--provider",
            "offline",
            "-o",
            s(&again),
        ]),
        "generate",
    );
    let a: Value = serde_json::from_str(&text).unwrap();
    let b: Value = serde_json::from_str(&std::fs::read_to_string(&again).unwrap()).unwrap();
    assert_eq!(a["score"], b["score"], "deterministic");

    if !have_piano("generate_offline_from_the_acceptance_brief (render)") {
        return;
    }
    let wav = dir.join("acc.wav");
    let v = ok(
        &run(&[
            "--json",
            "generate",
            "--prompt",
            BRIEF,
            "--provider",
            "offline",
            "-o",
            s(&project),
            "--render",
            s(&wav),
            "--skip-missing",
        ]),
        "generate",
    );
    let render = &v["render"];
    has_fields(render, RENDER_FIELDS);
    has_fields(&render["check"], VERIFY_FIELDS);
    report_failures("generate --render", &render["check"]);
    assert_eq!(
        check_named(&render["check"], "loop_seam")["passed"],
        json!(true)
    );
    assert!(wav.is_file());
    // Rendering the written project gives the same length.
    let wav2 = dir.join("acc2.wav");
    let r2 = ok(
        &run(&[
            "--json",
            "render",
            s(&project),
            "-o",
            s(&wav2),
            "--skip-missing",
        ]),
        "render",
    );
    assert_eq!(r2["frames"], render["frames"]);
}

#[test]
fn generate_without_a_key_falls_back_to_offline_and_says_so() {
    let dir = scratch("generate_fallback");
    let v = ok(
        &run(&[
            "--json",
            "generate",
            "--prompt",
            BRIEF,
            "-o",
            s(&dir.join("p.praxis")),
        ]),
        "generate",
    );
    assert_eq!(v["provider"], json!("offline-rules"));
    let note = v["provider_note"].as_str().expect("provider_note");
    assert!(note.contains("offline"), "{note}");
}

#[test]
fn generate_from_an_agent_written_plan() {
    let dir = scratch("generate_plan");
    let project = dir.join("plan.praxis");
    let v = ok(
        &run(&[
            "--json",
            "generate",
            "--plan",
            s(&plan_file()),
            "-o",
            s(&project),
        ]),
        "generate",
    );
    assert_summary(&v);
    assert_eq!(v["provider"], json!("direct-plan"));
    assert!(project.is_file());

    // The plan can also come on stdin.
    let plan_text = std::fs::read_to_string(plan_file()).unwrap();
    let v2 = ok(
        &run_with(
            &[
                "--json",
                "generate",
                "--plan",
                "-",
                "-o",
                s(&dir.join("stdin.praxis")),
            ],
            &[],
            Some(&plan_text),
        ),
        "generate",
    );
    assert_eq!(v2["score_summary"], v["score_summary"]);

    if !have_piano("generate_from_an_agent_written_plan (render)") {
        return;
    }
    let wav = dir.join("plan.wav");
    let v = ok(
        &run(&[
            "--json",
            "generate",
            "--plan",
            s(&plan_file()),
            "-o",
            s(&project),
            "--render",
            s(&wav),
            "--skip-missing",
        ]),
        "generate",
    );
    has_fields(&v["render"]["check"], VERIFY_FIELDS);
    report_failures("generate --plan --render", &v["render"]["check"]);
    assert!(wav.is_file());
}

#[test]
fn generate_errors() {
    let dir = scratch("generate_errors");
    let out = dir.join("x.praxis");
    err(
        &run(&[
            "--json",
            "generate",
            "--provider",
            "claude",
            "--prompt",
            BRIEF,
            "-o",
            s(&out),
        ]),
        "generate",
        "no_api_key",
    );
    err(
        &run(&[
            "--json",
            "generate",
            "--provider",
            "nope",
            "--prompt",
            BRIEF,
            "-o",
            s(&out),
        ]),
        "generate",
        "error",
    );
    err(
        &run(&["--json", "generate", "-o", s(&out)]),
        "generate",
        "error",
    );
    err(
        &run(&[
            "--json",
            "generate",
            "--prompt",
            "x",
            "--plan",
            s(&plan_file()),
            "-o",
            s(&out),
        ]),
        "generate",
        "error",
    );
    let garbage = dir.join("garbage.json");
    std::fs::write(&garbage, "{ nope").unwrap();
    err(
        &run(&["--json", "generate", "--plan", s(&garbage), "-o", s(&out)]),
        "generate",
        "bad_json",
    );
    let empty_plan = dir.join("empty.plan.json");
    std::fs::write(&empty_plan, "{}").unwrap();
    let v = err(
        &run(&[
            "--json",
            "generate",
            "--plan",
            s(&empty_plan),
            "-o",
            s(&out),
        ]),
        "generate",
        "invalid_score",
    );
    for i in v["error"]["issues"].as_array().expect("issues") {
        has_fields(i, &["code", "path", "message"]);
    }
    err(
        &run(&[
            "--json",
            "generate",
            "--plan",
            s(&dir.join("missing.json")),
            "-o",
            s(&out),
        ]),
        "generate",
        "io",
    );
    assert!(!out.exists());
}

/// The ops layer is callable directly with JSON requests, the way the MCP server will call it.
#[test]
fn ops_take_json_requests() {
    let dir = scratch("ops_json");
    let plan: Value = serde_json::from_str(&std::fs::read_to_string(plan_file()).unwrap()).unwrap();
    let req: praxis_cli::ops::GenerateRequest = serde_json::from_value(json!({
        "plan": plan,
        "out": dir.join("ops.praxis"),
    }))
    .expect("request from JSON");
    let payload = praxis_cli::ops::generate(&req).expect("generate");
    assert_eq!(payload["provider"], json!("direct-plan"));
    let env = praxis_cli::envelope("generate", &Ok(payload));
    assert_eq!(env["ok"], json!(true));
    assert_eq!(env["command"], json!("generate"));

    let req: praxis_cli::ops::ValidateRequest =
        serde_json::from_value(json!({ "input": dir.join("missing.json") })).unwrap();
    let e = praxis_cli::ops::validate(&req).unwrap_err();
    assert_eq!(e.code.as_str(), "io");
    let env = praxis_cli::envelope("validate", &Err(e));
    assert_eq!(env["error"]["code"], json!("io"));
}

// ---------------------------------------------------------------- export-midi

/// Minimal Standard MIDI File reader: (format, division, note-ons per track).
fn parse_smf(b: &[u8]) -> (u16, u16, Vec<usize>) {
    fn u16be(b: &[u8], i: usize) -> u16 {
        u16::from_be_bytes([b[i], b[i + 1]])
    }
    fn u32be(b: &[u8], i: usize) -> usize {
        u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize
    }
    fn vlq(b: &[u8], i: &mut usize) -> usize {
        let mut v = 0usize;
        loop {
            let c = b[*i];
            *i += 1;
            v = (v << 7) | (c & 0x7f) as usize;
            if c & 0x80 == 0 {
                return v;
            }
        }
    }
    assert_eq!(&b[..4], b"MThd");
    assert_eq!(u32be(b, 4), 6);
    let (format, ntrks, division) = (u16be(b, 8), u16be(b, 10), u16be(b, 12));
    let mut pos = 14;
    let mut notes = Vec::new();
    for _ in 0..ntrks {
        assert_eq!(&b[pos..pos + 4], b"MTrk", "track chunk at {pos}");
        let end = pos + 8 + u32be(b, pos + 4);
        let mut i = pos + 8;
        let mut status = 0u8;
        let mut count = 0;
        let mut ended = false;
        while i < end {
            vlq(b, &mut i);
            let mut st = b[i];
            if st & 0x80 != 0 {
                i += 1;
            } else {
                st = status;
            }
            match st {
                0xff => {
                    let kind = b[i];
                    i += 1;
                    let n = vlq(b, &mut i);
                    i += n;
                    ended |= kind == 0x2f;
                }
                0xf0 | 0xf7 => {
                    let n = vlq(b, &mut i);
                    i += n;
                }
                _ => {
                    status = st;
                    if st & 0xf0 == 0x90 && b[i + 1] > 0 {
                        count += 1;
                    }
                    i += if matches!(st & 0xf0, 0xc0 | 0xd0) {
                        1
                    } else {
                        2
                    };
                }
            }
        }
        assert_eq!(i, end, "track parsed to its end");
        assert!(ended, "track has end-of-track");
        notes.push(count);
        pos = end;
    }
    assert_eq!(pos, b.len(), "no trailing bytes");
    (format, division, notes)
}

#[test]
fn export_midi_reparses() {
    let dir = scratch("export_midi");
    let input = fixture("acceptance1.score.json");
    let mid = dir.join("acc.mid");
    let v = ok(
        &run(&["--json", "export-midi", s(&input), "-o", s(&mid)]),
        "export-midi",
    );
    has_fields(&v, &["output", "tracks"]);
    let bytes = std::fs::read(&mid).unwrap();
    assert_eq!(v["bytes"], json!(bytes.len()));
    let (format, division, notes) = parse_smf(&bytes);
    assert_eq!(format, 1);
    assert_eq!(division, 480);
    assert_eq!(v["tracks"], json!(notes.len()));

    // A conductor track, then one track per layer with exactly the notes the engine plays.
    let score: praxis_score::Score =
        serde_json::from_str(&std::fs::read_to_string(&input).unwrap()).unwrap();
    assert_eq!(notes.len(), score.layers.len() + 1);
    assert_eq!(notes[0], 0, "conductor track has no notes");
    let events = praxis_score::timeline::expand_notes(&score, 48_000);
    for (li, layer) in score.layers.iter().enumerate() {
        let expected = events.iter().filter(|e| e.layer == li).count();
        assert!(expected > 0, "layer {} has notes", layer.id);
        assert_eq!(notes[li + 1], expected, "notes in layer {}", layer.id);
    }

    err(
        &run(&[
            "--json",
            "export-midi",
            s(&dir.join("missing.json")),
            "-o",
            s(&mid),
        ]),
        "export-midi",
        "io",
    );
}

// ---------------------------------------------------------------- analyze

#[test]
fn analyze_shapes() {
    let dir = scratch("analyze");
    err(
        &run(&["--json", "analyze", s(&dir.join("missing.wav"))]),
        "analyze",
        "io",
    );
    let not_audio = dir.join("not-audio.wav");
    std::fs::write(&not_audio, "this is not audio").unwrap();
    err(
        &run(&["--json", "analyze", s(&not_audio)]),
        "analyze",
        "decode_failed",
    );

    if !have_piano("analyze_shapes (success)") {
        return;
    }
    let wav = dir.join("acc.wav");
    ok(
        &run(&[
            "--json",
            "render",
            s(&fixture("acceptance1.score.json")),
            "-o",
            s(&wav),
            "--skip-missing",
        ]),
        "render",
    );
    let profile = dir.join("profile.json");
    let v = ok(
        &run(&["--json", "analyze", s(&wav), "-o", s(&profile)]),
        "analyze",
    );
    has_fields(
        &v,
        &[
            "complete", "missing", "source", "tempo", "key", "loudness", "onsets_s",
        ],
    );
    assert_eq!(v["complete"], json!(false));
    assert_eq!(v["source"]["sample_rate"], json!(48_000));
    assert_eq!(v["key"]["name"], json!("a minor"), "{}", v["key"]);
    assert!(!v["onsets_s"].as_array().unwrap().is_empty());
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&profile).unwrap()).unwrap();
    assert_eq!(saved["key"], v["key"]);
}

// ---------------------------------------------------------------- packs

#[test]
fn packs_shapes() {
    let v = ok(&run(&["--json", "packs", "list"]), "packs");
    let packs = v["packs"].as_array().expect("packs[]");
    let piano = packs
        .iter()
        .find(|p| p["name"] == PACK)
        .expect("the piano pack is in the index");
    has_fields(
        piano,
        &["name", "version", "license", "installed", "presets"],
    );

    err(
        &run(&["--json", "packs", "install", "no-such-pack"]),
        "packs",
        "pack_missing",
    );

    // An archive that does not match the index is refused before anything is unpacked.
    let dir = scratch("packs");
    let fake = dir.join("fake.7z");
    std::fs::write(&fake, b"not a pack").unwrap();
    let store = dir.join("store");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::copy(repo().join("packs/index.json"), store.join("index.json")).unwrap();
    let r = run_with(
        &["--json", "packs", "install", PACK, "--archive", s(&fake)],
        &[("PRAXIS_PACKS", s(&store))],
        None,
    );
    let v = r.json();
    assert_eq!(v["ok"], json!(false), "{v}");
    assert_eq!(r.code, 1);
    println!("packs install with a bad archive: {}", v["error"]);
    assert!(!store.join(PACK).join("manifest.json").exists());

    // No packs index at all.
    let none = dir.join("none");
    std::fs::create_dir_all(&none).unwrap();
    err(
        &run_with(
            &["--json", "packs", "list"],
            &[("PRAXIS_PACKS", s(&none))],
            None,
        ),
        "packs",
        "io",
    );
}

#[test]
fn presets_lists_the_catalogue() {
    let v = ok(&run(&["--json", "presets"]), "presets");
    let presets = v["presets"].as_array().unwrap();
    assert!(presets.iter().any(|p| p["name"] == "felt_piano"));
    for p in presets {
        has_fields(p, &["name", "kind", "available"]);
    }
}

// ---------------------------------------------------------------- key

fn assert_hidden(r: &Run, secret: &str) {
    assert!(
        !r.stdout.contains(secret) && !r.stderr.contains(secret),
        "key leaked: stdout {:?} stderr {:?}",
        r.stdout,
        r.stderr
    );
}

#[test]
fn key_never_echoes_the_key() {
    const SECRET: &str = "sk-ant-test-0123456789";
    // The keychain is disabled in tests, so `set` refuses; the key appears nowhere.
    for args in [&["--json", "key", "set"][..], &["key", "set"][..]] {
        let r = run_with(args, &[], Some(SECRET));
        assert_eq!(r.code, 1);
        assert_hidden(&r, SECRET);
    }
    err(
        &run_with(&["--json", "key", "set"], &[], Some(SECRET)),
        "key",
        "error",
    );
    // A key on the command line is rejected without echoing it.
    for args in [
        &["--json", "key", "set", SECRET][..],
        &["key", "set", SECRET][..],
    ] {
        let r = run(args);
        assert_ne!(r.code, 0);
        assert_hidden(&r, SECRET);
    }
    err(
        &run_with(&["--json", "key", "set"], &[], Some("  \n")),
        "key",
        "no_api_key",
    );
    err(&run(&["--json", "key", "clear"]), "key", "error");

    let v = ok(&run(&["--json", "key", "status"]), "key");
    assert_eq!(v["available"], json!(false));
    assert!(v["source"].is_null());
    // A key in the environment is reported by its source only.
    let r = run_with(
        &["--json", "key", "status"],
        &[("ANTHROPIC_API_KEY", SECRET)],
        None,
    );
    let v = ok(&r, "key");
    assert_eq!(v["source"], json!("env"));
    assert_hidden(&r, SECRET);
}
