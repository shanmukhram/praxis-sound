//! `render`: score or project -> audio file (WAV or FLAC), optional stems, measurements.
//!
//! Agents cannot listen, so every render also runs `praxis_analyze::verify::verify_render` on
//! the master and returns the report under `check` (ADR-0010 point 3).

use crate::error::{ErrorCode, OpError, io_error};
use crate::ops::{ensure_parent, finite, load_score};
use praxis_analyze::verify::{Tolerances, verify_render};
use praxis_audio_io::encode::{EncodeError, StemFormat, write_flac, write_stems};
use praxis_audio_io::{WavFormat, write_wav};
use praxis_engine::{
    InstrumentError, InstrumentFactory, RenderError, RenderOptions, render_offline,
};
use praxis_instruments::Registry;
use praxis_score::Score;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// How to render. Shared by `render` and `generate --render`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderSettings {
    /// `16`, `24`, `32f` (WAV) or `flac` / `flac24` / `flac16`.
    pub format: String,
    pub sample_rate: u32,
    /// Directory for per-layer stems, written in the same format as the master.
    pub stems: Option<PathBuf>,
    /// Overrides the score seed.
    pub seed: Option<u64>,
    /// Render only the layers whose instrument is available (preset built, pack installed) and
    /// list the others in `skipped_layers`, instead of failing.
    pub skip_missing: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings {
            format: "24".into(),
            sample_rate: 48_000,
            stems: None,
            seed: None,
            skip_missing: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenderRequest {
    /// Score JSON or `.praxis` project.
    pub input: PathBuf,
    pub out: PathBuf,
    #[serde(flatten)]
    pub settings: RenderSettings,
}

pub fn render(req: &RenderRequest) -> Result<Value, OpError> {
    let score = load_score(&req.input)?;
    render_score(&score, &req.out, &req.settings)
}

/// Parses the `--format` value.
pub fn parse_format(format: &str) -> Result<StemFormat, OpError> {
    if let Some(f) = WavFormat::parse(format) {
        return Ok(StemFormat::Wav(f));
    }
    match format {
        "flac" | "flac24" => Ok(StemFormat::Flac { bits: 24 }),
        "flac16" => Ok(StemFormat::Flac { bits: 16 }),
        _ => Err(OpError::new(
            ErrorCode::BadFormat,
            format!("unknown format `{format}`; use 16, 24, 32f, flac, flac24 or flac16"),
        )),
    }
}

fn format_name(f: StemFormat) -> &'static str {
    match f {
        StemFormat::Wav(WavFormat::Pcm16) => "wav16",
        StemFormat::Wav(WavFormat::Pcm24) => "wav24",
        StemFormat::Wav(WavFormat::Float32) => "wav32f",
        StemFormat::Flac { bits: 16 } => "flac16",
        StemFormat::Flac { .. } => "flac24",
    }
}

/// A layer that cannot be rendered on this machine, and why.
pub fn unavailable_layers(score: &Score, registry: &Registry) -> Vec<Value> {
    score
        .layers
        .iter()
        .filter_map(|l| {
            instrument_unavailable(registry, l.instrument.kind, &l.instrument.preset).map(
                |reason| {
                    json!({
                        "layer": l.id,
                        "preset": l.instrument.preset,
                        "kind": l.instrument.kind,
                        "reason": reason,
                    })
                },
            )
        })
        .collect()
}

/// `None` when the preset is built and its pack (if any) is installed.
pub fn instrument_unavailable(
    registry: &Registry,
    kind: praxis_score::InstrumentKind,
    preset: &str,
) -> Option<String> {
    let Some(info) = registry.describe(kind, preset) else {
        return Some(format!("preset `{preset}` is not built yet"));
    };
    if let Some(pack) = pack_of(&info.source)
        && !registry.packs().has_pack(pack)
    {
        return Some(format!(
            "pack `{pack}` is not installed (praxis packs install {pack})"
        ));
    }
    None
}

/// The pack a sampled preset loads from, from its source note ("..., pack `name`").
fn pack_of(source: &str) -> Option<&str> {
    let rest = &source[source.find("pack `")? + 6..];
    Some(&rest[..rest.find('`')?])
}

fn render_error(e: RenderError) -> OpError {
    match e {
        RenderError::Instrument(InstrumentError::PackMissing(p)) => OpError::new(
            ErrorCode::PackMissing,
            format!(
                "instrument pack `{p}` is not installed: run `praxis packs install {p}`, or \
                 pass --skip-missing to render the other layers"
            ),
        ),
        RenderError::Instrument(e @ InstrumentError::UnknownPreset { .. }) => OpError::new(
            ErrorCode::RenderFailed,
            format!("{e}; pass --skip-missing to render the other layers"),
        ),
        RenderError::InvalidScore(m) => OpError::new(ErrorCode::InvalidScore, m),
        other => OpError::new(ErrorCode::RenderFailed, other.to_string()),
    }
}

fn encode_error(path: &Path, e: EncodeError) -> OpError {
    match e {
        EncodeError::UnsupportedBits(_) | EncodeError::UnsupportedRate(_) => {
            OpError::new(ErrorCode::BadFormat, e.to_string())
        }
        EncodeError::Io { .. } => io_error(path, e),
        other => OpError::new(ErrorCode::Io, format!("{}: {other}", path.display())),
    }
}

/// Renders `score` to `out` and returns the render payload (docs/contracts/cli.md) plus the
/// verify report under `check`.
pub fn render_score(
    score: &Score,
    out: &Path,
    settings: &RenderSettings,
) -> Result<Value, OpError> {
    render_score_master(score, out, settings).map(|(v, _)| v)
}

/// [`render_score`], also returning the rendered master (before encoding).
pub(crate) fn render_score_master(
    score: &Score,
    out: &Path,
    settings: &RenderSettings,
) -> Result<(Value, praxis_dsp::StereoBuffer), OpError> {
    let mut score = score.clone();
    if let Some(s) = settings.seed {
        score.seed = s;
    }
    let format = parse_format(&settings.format)?;
    if settings.sample_rate == 0 {
        return Err(OpError::new(
            ErrorCode::BadFormat,
            "sample rate must be above 0",
        ));
    }
    let registry = Registry::with_default_packs();
    let skipped = if settings.skip_missing {
        unavailable_layers(&score, &registry)
    } else {
        Vec::new()
    };
    let any_solo = score.layers.iter().any(|l| l.solo);
    let only_layers: Vec<String> = if skipped.is_empty() {
        Vec::new()
    } else {
        score
            .layers
            .iter()
            .filter(|l| !skipped.iter().any(|s| s["layer"] == l.id.as_str()))
            .filter(|l| !l.muted && (l.solo || !any_solo))
            .map(|l| l.id.clone())
            .collect()
    };
    if !skipped.is_empty() && only_layers.is_empty() {
        return Err(OpError::new(
            ErrorCode::RenderFailed,
            format!(
                "no layer can be rendered on this machine: {}",
                serde_json::to_string(&skipped).unwrap_or_default()
            ),
        )
        .with_issues(skipped));
    }
    let opts = RenderOptions {
        sample_rate: settings.sample_rate,
        stems: settings.stems.is_some(),
        only_layers,
        ..RenderOptions::default()
    };
    let t0 = std::time::Instant::now();
    let output = render_offline(&score, &registry, &opts).map_err(render_error)?;
    let render_s = t0.elapsed().as_secs_f64();

    ensure_parent(out)?;
    match format {
        StemFormat::Wav(f) => {
            write_wav(out, &output.master, settings.sample_rate, f).map_err(|e| io_error(out, e))?
        }
        StemFormat::Flac { bits } => write_flac(out, &output.master, settings.sample_rate, bits)
            .map_err(|e| encode_error(out, e))?,
    }
    let mut stem_files = Vec::new();
    if let Some(dir) = &settings.stems {
        let stems: Vec<(String, _)> = output
            .stems
            .iter()
            .map(|s| (s.layer_id.clone(), s.audio.clone()))
            .collect();
        let paths = write_stems(dir, &stems, settings.sample_rate, format)
            .map_err(|e| encode_error(dir, e))?;
        stem_files = paths.iter().map(|p| p.display().to_string()).collect();
    }

    let r = &output.report;
    let duration_s = output.master.len() as f64 / settings.sample_rate as f64;
    // Check what was rendered: layers skipped for a missing instrument are left out.
    let mut checked = score.clone();
    checked
        .layers
        .retain(|l| !skipped.iter().any(|s| s["layer"] == l.id.as_str()));
    let report = verify_render(
        &checked,
        &output.master,
        settings.sample_rate,
        &Tolerances::default(),
    );
    let mut check = serde_json::to_value(&report).unwrap_or(Value::Null);
    check["passed"] = json!(report.passed());
    check["layers_checked"] = json!(
        checked
            .layers
            .iter()
            .map(|l| l.id.clone())
            .collect::<Vec<_>>()
    );
    check["failed"] = json!(
        report
            .checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
    );
    let payload = json!({
        "output": out.display().to_string(),
        "format": format_name(format),
        "stems": stem_files,
        "sample_rate": settings.sample_rate,
        "frames": output.master.len(),
        "duration_s": duration_s,
        "is_loop": r.is_loop,
        "length_samples": r.length_samples,
        "tail_samples": r.tail_samples,
        "warmup_cycles": r.warmup_cycles,
        "loudness_lufs": finite(r.loudness_lufs),
        "loudness_before_lufs": finite(r.loudness_before_lufs),
        "true_peak_dbtp": finite(r.true_peak_dbtp),
        "applied_gain_db": r.applied_gain_db,
        "blocked_layers": r.blocked_layers,
        "skipped_layers": skipped,
        "notes_rendered": r.notes_rendered,
        "warnings": r.warnings,
        "seed": score.seed,
        "render_seconds": render_s,
        "realtime_factor": if render_s > 0.0 { duration_s / render_s } else { 0.0 },
        "check": check,
    });
    Ok((payload, output.master))
}
