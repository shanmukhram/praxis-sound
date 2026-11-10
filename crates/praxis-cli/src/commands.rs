//! Command implementations. Each returns a JSON value (the `ok` payload) or a coded error.

use anyhow::{Context, Result};
use praxis_audio_io::{WavFormat, write_wav};
use praxis_engine::{RenderOptions, render_offline};
use praxis_instruments::Registry;
use praxis_score::Score;
use serde_json::{Value, json};
use std::path::Path;

/// A failure with a stable machine-readable code (see docs/contracts/cli.md).
#[derive(Debug)]
pub struct CodedError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for CodedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CodedError {}

fn coded(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    CodedError {
        code,
        message: message.into(),
    }
    .into()
}

/// Prints the result and returns the process exit code.
pub fn emit(command: &str, as_json: bool, result: Result<Value>) -> i32 {
    match result {
        Ok(mut v) => {
            if as_json {
                if let Value::Object(m) = &mut v {
                    m.insert("ok".into(), json!(true));
                    m.insert("command".into(), json!(command));
                }
                println!("{v}");
            } else {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            0
        }
        Err(e) => {
            let (code, message) = match e.downcast_ref::<CodedError>() {
                Some(c) => (c.code, c.message.clone()),
                None => ("error", format!("{e:#}")),
            };
            if as_json {
                println!(
                    "{}",
                    json!({"ok": false, "command": command, "error": {"code": code, "message": message}})
                );
            } else {
                eprintln!("error [{code}]: {message}");
            }
            1
        }
    }
}

pub fn load_score(path: &Path) -> Result<Score> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| coded("io", format!("{}: {e}", path.display())))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| coded("bad_json", format!("{}: {e}", path.display())))?;
    let score_value = if value.get("format").and_then(Value::as_str) == Some("praxis-project") {
        value.get("score").cloned().unwrap_or(Value::Null)
    } else {
        value
    };
    let score: Score = serde_json::from_value(score_value)
        .map_err(|e| coded("bad_score", format!("{}: {e}", path.display())))?;
    let report = praxis_score::validate::validate(&score);
    if !report.is_valid() {
        return Err(coded(
            "invalid_score",
            serde_json::to_string(&report.errors).unwrap_or_default(),
        ));
    }
    Ok(score)
}

pub fn render(
    input: &Path,
    out: &Path,
    format: &str,
    sample_rate: u32,
    stems: Option<&Path>,
    seed: Option<u64>,
) -> Result<Value> {
    let mut score = load_score(input)?;
    if let Some(s) = seed {
        score.seed = s;
    }
    let fmt = WavFormat::parse(format).ok_or_else(|| {
        coded(
            "bad_format",
            format!("unknown format `{format}`; use 16, 24 or 32f"),
        )
    })?;
    let registry = Registry::with_default_packs();
    let opts = RenderOptions {
        sample_rate,
        stems: stems.is_some(),
        ..RenderOptions::default()
    };
    let t0 = std::time::Instant::now();
    let output = render_offline(&score, &registry, &opts)
        .map_err(|e| coded("render_failed", e.to_string()))?;
    let render_s = t0.elapsed().as_secs_f64();
    write_wav(out, &output.master, sample_rate, fmt)
        .with_context(|| format!("writing {}", out.display()))?;
    let mut stem_files = Vec::new();
    if let Some(dir) = stems {
        std::fs::create_dir_all(dir)?;
        for s in &output.stems {
            let p = dir.join(format!("{}.wav", s.layer_id));
            write_wav(&p, &s.audio, sample_rate, fmt)?;
            stem_files.push(p.display().to_string());
        }
    }
    let r = &output.report;
    let duration_s = output.master.len() as f64 / sample_rate as f64;
    Ok(json!({
        "output": out.display().to_string(),
        "stems": stem_files,
        "sample_rate": sample_rate,
        "frames": output.master.len(),
        "duration_s": duration_s,
        "is_loop": r.is_loop,
        "loudness_lufs": finite(r.loudness_lufs),
        "true_peak_dbtp": finite(r.true_peak_dbtp),
        "applied_gain_db": r.applied_gain_db,
        "blocked_layers": r.blocked_layers,
        "render_seconds": render_s,
        "realtime_factor": if render_s > 0.0 { duration_s / render_s } else { 0.0 },
    }))
}

fn finite(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}

pub fn validate(input: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(input)
        .map_err(|e| coded("io", format!("{}: {e}", input.display())))?;
    let score: Score =
        serde_json::from_str(&text).map_err(|e| coded("bad_score", e.to_string()))?;
    let report = praxis_score::validate::validate(&score);
    let valid = report.is_valid();
    let v = json!({ "valid": valid, "errors": report.errors, "warnings": report.warnings });
    if valid {
        Ok(v)
    } else {
        Err(coded("invalid_score", serde_json::to_string(&v)?))
    }
}

pub fn schema(write: bool) -> Result<Value> {
    let score = praxis_score::score_schema();
    let patch = praxis_score::patch_schema();
    if write {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join("score.schema.json"),
            serde_json::to_string_pretty(&score)? + "\n",
        )?;
        std::fs::write(
            dir.join("patch.schema.json"),
            serde_json::to_string_pretty(&patch)? + "\n",
        )?;
        return Ok(json!({ "written": ["schema/score.schema.json", "schema/patch.schema.json"] }));
    }
    Ok(json!({ "score": score, "patch": patch }))
}
