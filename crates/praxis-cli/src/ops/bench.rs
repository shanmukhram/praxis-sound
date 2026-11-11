//! `bench`: how fast this machine renders (PRD section 2, "Render speed": a 2-minute, 8-layer
//! piece at 20x real time or better).
//!
//! The default piece is built from acceptance test 1 (felt piano motif, soft kick) and grown to
//! eight layers with real instruments and effect chains: sampled felt piano, FM keys, warm bass,
//! pluck with a tempo-synced delay, granular texture, soft kick, closed hat and rim, each through
//! its own effects and the master bus (loudness normalisation and true-peak limiter). It renders
//! through the same `render_offline` and instrument registry as `praxis render`, so the number
//! is the one a user gets. Nothing is written to disk.
//!
//! Every timed run must produce the same samples as the first; `deterministic` says whether they
//! did (bit for bit).

use crate::error::{ErrorCode, OpError};
use crate::ops::render::instrument_unavailable;
use crate::ops::{finite, load_score};
use praxis_engine::{RenderOptions, render_offline};
use praxis_instruments::Registry;
use praxis_score::Score;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Instant;

/// The PRD's target: audio seconds rendered per wall-clock second.
pub const TARGET_REALTIME_FACTOR: f64 = 20.0;

/// Acceptance test 1, the template the bench piece grows from.
const TEMPLATE: &str = include_str!("../../../../fixtures/acceptance1.score.json");

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchRequest {
    /// Bench this score or project instead of the built-in piece.
    #[serde(default)]
    pub input: Option<PathBuf>,
    /// Length of the built-in piece in seconds (rounded to whole bars at 128 BPM).
    #[serde(default = "default_seconds")]
    pub seconds: f64,
    /// Timed runs after one untimed warm-up render (1 to 20).
    #[serde(default = "default_runs")]
    pub runs: usize,
    #[serde(default = "default_rate")]
    pub sample_rate: u32,
    /// Also time each layer rendered alone (once each), to see where the time goes.
    #[serde(default)]
    pub per_layer: bool,
}

fn default_seconds() -> f64 {
    120.0
}
fn default_runs() -> usize {
    3
}
fn default_rate() -> u32 {
    48_000
}

impl Default for BenchRequest {
    fn default() -> Self {
        BenchRequest {
            input: None,
            seconds: default_seconds(),
            runs: default_runs(),
            sample_rate: default_rate(),
            per_layer: false,
        }
    }
}

fn bad(msg: impl Into<String>) -> OpError {
    OpError::new(ErrorCode::Error, msg)
}

/// The built-in bench piece: eight layers playing for `seconds` (whole bars at 128 BPM), then a
/// 2-second tail.
pub fn bench_score(seconds: f64) -> Result<Score, OpError> {
    if !(seconds.is_finite() && (2.0..=1800.0).contains(&seconds)) {
        return Err(bad("seconds must be between 2 and 1800"));
    }
    let mut v: Value =
        serde_json::from_str(TEMPLATE).map_err(|e| bad(format!("bench template: {e}")))?;
    let bpm = v["tempo"]["bpm"].as_f64().unwrap_or(128.0);
    let bars = ((seconds * bpm / 60.0 / 4.0).round() as u32).max(1);
    v["title"] = json!("Praxis bench: 8 layers");
    v["bars"] = json!(bars);
    v["playback"] = json!({ "mode": "one_shot", "tail_seconds": 2.0 });
    v["chords"] = json!([]);
    v["markers"] = json!([]);
    v["sections"] = json!([{
        "id": "all", "name": "Everything", "bars": { "start": 1, "end": bars },
        "role": "main", "intensity": 0.5
    }]);
    let patterns = v["patterns"]
        .as_array_mut()
        .ok_or_else(|| bad("bench template has no patterns"))?;
    let hats: Vec<Value> = (0..8)
        .map(|s| {
            json!({ "step": s, "pitch": 42, "velocity": if s % 2 == 0 { 0.5 } else { 0.32 },
                    "accent": if s % 2 == 0 { "normal" } else { "soft" }, "length": 0.5 })
        })
        .collect();
    patterns.push(json!({ "id": "hats", "grid": "eighth", "length_steps": 8, "notes": hats }));
    patterns.push(
        json!({ "id": "rims", "grid": "eighth", "length_steps": 8, "notes": [
            { "step": 2, "pitch": 37, "velocity": 0.45, "accent": "normal", "length": 0.5 },
            { "step": 6, "pitch": 37, "velocity": 0.4, "accent": "soft", "length": 0.5 }
        ]}),
    );
    patterns.push(
        json!({ "id": "drone", "grid": "eighth", "length_steps": 16, "notes": [
            { "step": 0, "pitch": 57, "velocity": 0.4, "accent": "soft", "length": 16.0 }
        ]}),
    );

    let all = json!({ "start": 1, "end": bars });
    let layer = |id: &str,
                 class: &str,
                 kind: &str,
                 preset: &str,
                 pattern: &str,
                 transpose: i32,
                 gain_db: f64,
                 pan: f64,
                 effects: Value| {
        json!({
            "id": id, "name": id, "class": class,
            "instrument": { "kind": kind, "preset": preset, "params": {} },
            "clips": [{ "bars": all, "pattern": pattern, "transpose": transpose }],
            "gain_db": gain_db, "pan": pan, "effects": effects
        })
    };
    let room = |mix: f64| json!({ "id": "room", "kind": "reverb", "params": { "decay_s": 2.4, "mix": mix } });
    v["layers"] = json!([
        layer(
            "piano",
            "motif",
            "sampler",
            "felt_piano",
            "piano_motif",
            0,
            0.0,
            0.0,
            json!([room(0.2)])
        ),
        layer(
            "keys",
            "harmony",
            "synth",
            "soft_keys",
            "piano_motif",
            -12,
            -9.0,
            -0.3,
            json!([{ "id": "chorus", "kind": "chorus", "params": { "mix": 0.3 } }, room(0.25)])
        ),
        layer(
            "bass",
            "bass",
            "synth",
            "warm_bass",
            "piano_motif",
            -24,
            -6.0,
            0.0,
            json!([{ "id": "comp", "kind": "compressor", "params": { "threshold_db": -20.0, "ratio": 3.0 } },
                     { "id": "tone", "kind": "eq", "params": { "lp_hz": 4000.0, "hp_hz": 35.0 } }])
        ),
        layer(
            "pluck",
            "arp",
            "synth",
            "pluck",
            "piano_motif",
            12,
            -12.0,
            0.35,
            json!([{ "id": "echo", "kind": "delay", "params": { "time_beats": 0.75, "feedback": 0.3, "mix": 0.2 } }])
        ),
        layer(
            "texture",
            "texture",
            "synth",
            "granular_texture",
            "drone",
            0,
            -16.0,
            0.0,
            json!([{ "id": "wide", "kind": "width", "params": { "width": 1.4 } }, room(0.35)])
        ),
        layer(
            "kick",
            "drums",
            "drum",
            "soft_kick",
            "kick_soft",
            0,
            -4.0,
            0.0,
            json!([{ "id": "tone", "kind": "eq", "params": { "lp_hz": 2500.0 } }])
        ),
        layer(
            "hat",
            "percussion",
            "drum",
            "hat_closed",
            "hats",
            0,
            -14.0,
            0.2,
            json!([{ "id": "tone", "kind": "eq", "params": { "hp_hz": 400.0 } }])
        ),
        layer(
            "rim",
            "percussion",
            "drum",
            "rim",
            "rims",
            0,
            -12.0,
            -0.2,
            json!([room(0.15)])
        ),
    ]);
    let score: Score = serde_json::from_value(v).map_err(|e| bad(format!("bench piece: {e}")))?;
    let report = praxis_score::validate::validate(&score);
    if !report.is_valid() {
        return Err(OpError::new(
            ErrorCode::InvalidScore,
            format!(
                "bench piece does not validate: {}",
                serde_json::to_string(&report.errors).unwrap_or_default()
            ),
        ));
    }
    Ok(score)
}

/// `{piece, audio_s, runs[], best_s, median_s, realtime_factor, best_realtime_factor, target,
/// passed, deterministic, warmup_s, build, machine}`. `realtime_factor` is from the median run.
pub fn bench(req: &BenchRequest) -> Result<Value, OpError> {
    if !(1..=20).contains(&req.runs) {
        return Err(bad("runs must be between 1 and 20"));
    }
    if req.sample_rate == 0 {
        return Err(OpError::new(
            ErrorCode::BadFormat,
            "sample rate must be above 0",
        ));
    }
    let (score, piece) = match &req.input {
        Some(p) => (load_score(p)?, p.display().to_string()),
        None => (
            bench_score(req.seconds)?,
            "built-in 8-layer piece".to_string(),
        ),
    };
    let registry = Registry::with_default_packs();
    let missing: Vec<Value> = score
        .layers
        .iter()
        .filter_map(|l| {
            instrument_unavailable(&registry, l.instrument.kind, &l.instrument.preset).map(
                |reason| json!({ "layer": l.id, "preset": l.instrument.preset, "reason": reason }),
            )
        })
        .collect();
    if !missing.is_empty() {
        // A bench with a stand-in instrument would measure something else; refuse instead.
        return Err(OpError::new(
            ErrorCode::PackMissing,
            format!(
                "cannot bench: {} (install packs with `praxis packs install upright-piano-kw`)",
                serde_json::to_string(&missing).unwrap_or_default()
            ),
        )
        .with_issues(missing));
    }
    let opts = RenderOptions {
        sample_rate: req.sample_rate,
        ..RenderOptions::default()
    };
    let render_with = |score: &Score, opts: &RenderOptions| {
        let t0 = Instant::now();
        let out = render_offline(score, &registry, opts).map_err(|e| {
            OpError::new(ErrorCode::RenderFailed, format!("bench render failed: {e}"))
        })?;
        Ok::<_, OpError>((t0.elapsed().as_secs_f64(), out))
    };
    let render = |score: &Score| render_with(score, &opts);

    // Warm-up: decodes and caches the samples, as a running app would have done already.
    let (warmup_s, first) = render(&score)?;
    let audio_s = first.master.len() as f64 / req.sample_rate as f64;
    let mut times = Vec::with_capacity(req.runs);
    let mut deterministic = true;
    for _ in 0..req.runs {
        let (secs, out) = render(&score)?;
        deterministic &= out.master.left.len() == first.master.left.len()
            && out
                .master
                .left
                .iter()
                .zip(&first.master.left)
                .chain(out.master.right.iter().zip(&first.master.right))
                .all(|(a, b)| a.to_bits() == b.to_bits());
        times.push(secs);
    }
    let mut sorted = times.clone();
    sorted.sort_by(f64::total_cmp);
    let best = sorted[0];
    let median = sorted[sorted.len() / 2];
    let rtf = |s: f64| if s > 0.0 { audio_s / s } else { 0.0 };
    let mut per_layer = Vec::new();
    if req.per_layer {
        for l in &score.layers {
            let alone = RenderOptions {
                only_layers: vec![l.id.clone()],
                ..opts.clone()
            };
            let (secs, _) = render_with(&score, &alone)?;
            per_layer
                .push(json!({ "layer": l.id, "preset": l.instrument.preset, "seconds": secs }));
        }
    }
    let report = &first.report;
    Ok(json!({
        "per_layer": per_layer,
        "piece": piece,
        "layers": score.layers.iter().map(|l| json!({
            "id": l.id, "kind": l.instrument.kind, "preset": l.instrument.preset,
            "effects": l.effects.iter().map(|e| e.kind).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "bars": score.bars,
        "bpm": score.tempo.bpm,
        "sample_rate": req.sample_rate,
        "audio_s": audio_s,
        "notes_rendered": report.notes_rendered,
        "loudness_lufs": finite(report.loudness_lufs),
        "true_peak_dbtp": finite(report.true_peak_dbtp),
        "warmup_s": warmup_s,
        "runs": times,
        "best_s": best,
        "median_s": median,
        "realtime_factor": rtf(median),
        "best_realtime_factor": rtf(best),
        "target": TARGET_REALTIME_FACTOR,
        "passed": rtf(median) >= TARGET_REALTIME_FACTOR,
        "deterministic": deterministic,
        "build": if cfg!(debug_assertions) { "debug" } else { "release" },
        "threads": 1,
        "machine": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cpu": std::env::var("PROCESSOR_IDENTIFIER").ok(),
            "logical_cpus": std::thread::available_parallelism().map(|n| n.get()).ok(),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_piece_is_two_minutes_of_eight_layers() {
        let s = bench_score(120.0).unwrap();
        assert_eq!(s.layers.len(), 8);
        assert_eq!(s.bars, 64);
        assert_eq!(s.tempo.bpm, 128.0);
        let bar_s = praxis_score::timeline::TimeMap::new(&s, 48_000).bar_s();
        assert!((bar_s * s.bars as f64 - 120.0).abs() < 1e-9);
        for l in &s.layers {
            assert_eq!(l.clips[0].bars.start, 1, "{} plays from bar 1", l.id);
            assert_eq!(l.clips[0].bars.end, 64, "{} plays to the end", l.id);
            assert!(!l.effects.is_empty(), "{} has an effect chain", l.id);
        }
    }

    #[test]
    fn bad_requests_are_refused() {
        assert!(bench_score(0.5).is_err());
        assert!(bench_score(f64::NAN).is_err());
        let req = BenchRequest {
            runs: 0,
            ..BenchRequest::default()
        };
        assert!(bench(&req).is_err());
    }

    #[test]
    fn a_short_bench_renders_the_same_samples_every_run() {
        let registry = Registry::with_default_packs();
        if instrument_unavailable(
            &registry,
            praxis_score::InstrumentKind::Sampler,
            "felt_piano",
        )
        .is_some()
        {
            println!("SKIP: the upright-piano-kw pack is not installed (set PRAXIS_PACKS)");
            return;
        }
        let v = bench(&BenchRequest {
            seconds: 4.0,
            runs: 2,
            ..BenchRequest::default()
        })
        .unwrap();
        assert_eq!(v["deterministic"], true);
        assert_eq!(v["layers"].as_array().unwrap().len(), 8);
        assert!(v["audio_s"].as_f64().unwrap() > 3.5);
        assert!(v["realtime_factor"].as_f64().unwrap() > 0.0);
    }
}
