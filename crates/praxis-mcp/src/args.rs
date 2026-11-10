//! Tool inputs. Each struct derives a JSON Schema (published in `tools/list`) and converts into
//! the matching `praxis_cli::ops` request, so the MCP tools and the CLI share one implementation.
//!
//! Relative paths resolve against the server's working directory and are made absolute before
//! the command runs, so every path in a response is absolute.

use praxis_cli::ops::{
    AnalyzeRequest, CheckRequest, ExportMidiRequest, GenerateRequest, ProviderChoice,
    RenderRequest, RenderSettings, ValidateRequest,
};
use praxis_cli::{ErrorCode, OpError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// The seed used when none is given (the CLI's default), so the same plan gives the same project.
pub const DEFAULT_SEED: u64 = praxis_cli::ops::generate::DEFAULT_SEED;

/// Makes `p` absolute against the current directory (no filesystem access).
pub fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

fn abs_opt(p: &Option<PathBuf>) -> Option<PathBuf> {
    p.as_deref().map(absolute)
}

/// Parses a tool call's arguments into `T`. Unknown fields are rejected (a misspelt `render`
/// would otherwise be ignored silently); failures are `bad_json` errors naming the field.
pub fn parse<T: DeserializeOwned + JsonSchema + 'static>(
    args: Map<String, Value>,
) -> Result<T, OpError> {
    let schema = rmcp::handler::server::common::schema_for_input::<T>()
        .map_err(|e| OpError::new(ErrorCode::Error, e))?;
    let empty = Map::new();
    let props = match schema.get("properties") {
        Some(Value::Object(p)) => p,
        _ => &empty,
    };
    let unknown: Vec<&String> = args.keys().filter(|k| !props.contains_key(*k)).collect();
    if !unknown.is_empty() {
        let allowed: Vec<&String> = props.keys().collect();
        let issues = unknown
            .iter()
            .map(|k| {
                serde_json::json!({
                    "code": "unknown_field",
                    "path": format!("/{k}"),
                    "message": format!("unknown field `{k}`; allowed fields: {allowed:?}"),
                })
            })
            .collect();
        return Err(OpError::new(
            ErrorCode::BadJson,
            format!("unknown field(s) {unknown:?}; allowed fields: {allowed:?}"),
        )
        .with_issues(issues));
    }
    serde_json::from_value(Value::Object(args))
        .map_err(|e| OpError::new(ErrorCode::BadJson, format!("invalid arguments: {e}")))
}

/// No arguments.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct NoArgs {}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PlanSchemaArgs {
    /// Also return the full Score and ScorePatch schemas (large; only needed to edit a Score
    /// file by hand).
    #[serde(default)]
    pub include_score: bool,
}

/// How to render audio. Shared by compose, generate, render and export.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct AudioOptions {
    /// Master file format: "24" (24-bit WAV, the default), "16", "32f" (float WAV), "flac" or
    /// "flac24", "flac16".
    #[serde(default)]
    #[schemars(extend("enum" = ["16", "24", "32f", "flac", "flac24", "flac16", null]))]
    pub format: Option<String>,
    /// Sample rate in Hz (default 48000).
    #[serde(default)]
    pub sample_rate: Option<u32>,
    /// Directory for one file per layer (stems), in the master's format.
    #[serde(default)]
    pub stems: Option<PathBuf>,
    /// Render only the layers whose instrument is available on this machine and list the others
    /// in `skipped_layers` (and leave them out of the checks), instead of failing with
    /// `pack_missing` / `render_failed`.
    #[serde(default)]
    pub skip_missing: bool,
}

impl AudioOptions {
    fn settings(&self, seed: Option<u64>) -> RenderSettings {
        let d = RenderSettings::default();
        RenderSettings {
            format: self.format.clone().unwrap_or(d.format),
            sample_rate: self.sample_rate.unwrap_or(d.sample_rate),
            stems: abs_opt(&self.stems),
            seed,
            skip_missing: self.skip_missing,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComposeArgs {
    /// The ScorePlan you wrote (JSON object). Get its schema, a how-to and a complete example
    /// from `praxis_plan_schema`, and preset names from `praxis_presets`.
    pub plan: Map<String, Value>,
    /// The `.praxis` project file to write (parent folders are created).
    pub project: PathBuf,
    /// Arranger and render seed (default 1). Same plan + seed = identical project.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Also render the master to this audio file and return the render report with its checks.
    #[serde(default)]
    pub render: Option<PathBuf>,
    #[serde(flatten)]
    pub audio: AudioOptions,
}

impl ComposeArgs {
    pub fn request(&self) -> GenerateRequest {
        GenerateRequest {
            prompt: None,
            plan: Some(Value::Object(self.plan.clone())),
            provider: None,
            model: None,
            seed: self.seed.unwrap_or(DEFAULT_SEED),
            out: absolute(&self.project),
            render: abs_opt(&self.render),
            render_settings: self.audio.settings(None),
        }
    }
}

/// Which planner turns a brief into a plan.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// The deterministic offline rule planner (no key, no network).
    Offline,
    /// Claude through the Anthropic API (needs ANTHROPIC_API_KEY or `praxis key set`).
    Claude,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GenerateArgs {
    /// Plain-language brief, e.g. "16 bars, A minor, 30 seconds, seamless loop, soft felt
    /// piano motif on an eighth grid". To write the plan yourself, use `praxis_compose`.
    pub prompt: String,
    /// The `.praxis` project file to write.
    pub project: PathBuf,
    /// Default: Claude when an API key is available, else offline (the response says which, in
    /// `provider` and `provider_note`).
    #[serde(default)]
    pub provider: Option<Provider>,
    /// Claude model id override.
    #[serde(default)]
    pub model: Option<String>,
    /// Planner, arranger and render seed (default 1).
    #[serde(default)]
    pub seed: Option<u64>,
    /// Also render the master to this audio file and return the render report with its checks.
    #[serde(default)]
    pub render: Option<PathBuf>,
    #[serde(flatten)]
    pub audio: AudioOptions,
}

impl GenerateArgs {
    pub fn request(&self) -> GenerateRequest {
        GenerateRequest {
            prompt: Some(self.prompt.clone()),
            plan: None,
            provider: self.provider.map(|p| match p {
                Provider::Offline => ProviderChoice::Offline,
                Provider::Claude => ProviderChoice::Claude,
            }),
            model: self.model.clone(),
            seed: self.seed.unwrap_or(DEFAULT_SEED),
            out: absolute(&self.project),
            render: abs_opt(&self.render),
            render_settings: self.audio.settings(None),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenderArgs {
    /// A `.praxis` project or a `.score.json` file.
    pub input: PathBuf,
    /// The audio file to write.
    pub out: PathBuf,
    /// Overrides the score's seed.
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(flatten)]
    pub audio: AudioOptions,
}

impl RenderArgs {
    pub fn request(&self) -> RenderRequest {
        RenderRequest {
            input: absolute(&self.input),
            out: absolute(&self.out),
            settings: self.audio.settings(self.seed),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckArgs {
    /// The rendered audio (WAV, FLAC, or anything `praxis_analyze` decodes).
    pub audio: PathBuf,
    /// The `.praxis` project or `.score.json` it was rendered from.
    pub score: PathBuf,
}

impl CheckArgs {
    pub fn request(&self) -> CheckRequest {
        CheckRequest {
            audio: absolute(&self.audio),
            score: absolute(&self.score),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeArgs {
    /// An audio or video file to measure.
    pub clip: PathBuf,
    /// Also write the measurements to this JSON file.
    #[serde(default)]
    pub out: Option<PathBuf>,
    /// Beats per bar for downbeat detection (default 4).
    #[serde(default)]
    pub beats_per_bar: Option<u8>,
}

impl AnalyzeArgs {
    pub fn request(&self) -> AnalyzeRequest {
        AnalyzeRequest {
            clip: absolute(&self.clip),
            out: abs_opt(&self.out),
            beats_per_bar: self.beats_per_bar.unwrap_or(4),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportArgs {
    /// A `.praxis` project or a `.score.json` file.
    pub input: PathBuf,
    /// The file to write (audio master, or the `.mid` file for "midi").
    pub out: PathBuf,
    /// "wav" (24-bit, the default), "16", "24", "32f", "flac", "flac24", "flac16" or "midi"
    /// (Standard MIDI File, one track per layer). Audio exports also return the checks.
    #[serde(default)]
    #[schemars(extend("enum" = ["wav", "16", "24", "32f", "flac", "flac24", "flac16", "midi", null]))]
    pub format: Option<String>,
    /// Audio only: a directory for per-layer stems in the same format.
    #[serde(default)]
    pub stems: Option<PathBuf>,
    /// Audio only: sample rate in Hz (default 48000).
    #[serde(default)]
    pub sample_rate: Option<u32>,
    /// Audio only: overrides the score's seed.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Audio only: render the layers that are available and list the others in `skipped_layers`.
    #[serde(default)]
    pub skip_missing: bool,
}

/// What an export runs: the CLI's `render` or its `export-midi`.
#[derive(Debug)]
pub enum Export {
    Audio(RenderRequest),
    Midi(ExportMidiRequest),
}

impl ExportArgs {
    pub fn request(&self) -> Result<Export, OpError> {
        let format = self.format.as_deref().unwrap_or("wav");
        if format == "midi" {
            if self.stems.is_some() {
                return Err(OpError::new(
                    ErrorCode::BadFormat,
                    "stems are audio files; drop `stems` for a MIDI export",
                ));
            }
            return Ok(Export::Midi(ExportMidiRequest {
                input: absolute(&self.input),
                out: absolute(&self.out),
            }));
        }
        let audio = AudioOptions {
            format: Some(if format == "wav" { "24" } else { format }.to_string()),
            sample_rate: self.sample_rate,
            stems: self.stems.clone(),
            skip_missing: self.skip_missing,
        };
        Ok(Export::Audio(RenderRequest {
            input: absolute(&self.input),
            out: absolute(&self.out),
            settings: audio.settings(self.seed),
        }))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValidateArgs {
    /// A `.score.json` file (the Score itself, not a `.praxis` project).
    pub input: PathBuf,
}

impl ValidateArgs {
    pub fn request(&self) -> ValidateRequest {
        ValidateRequest {
            input: absolute(&self.input),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => panic!("object expected"),
        }
    }

    #[test]
    fn compose_maps_onto_the_cli_request_with_cli_defaults() {
        let a: ComposeArgs = parse(obj(json!({
            "plan": {"emotion": "calm"},
            "project": "p.praxis",
            "render": "p.wav",
            "skip_missing": true
        })))
        .unwrap();
        let r = a.request();
        assert_eq!(r.seed, DEFAULT_SEED);
        assert!(r.out.is_absolute() && r.render.as_ref().unwrap().is_absolute());
        assert_eq!(r.plan, Some(json!({"emotion": "calm"})));
        assert!(r.prompt.is_none() && r.provider.is_none());
        let d = RenderSettings::default();
        assert_eq!(r.render_settings.format, d.format);
        assert_eq!(r.render_settings.sample_rate, d.sample_rate);
        assert!(r.render_settings.skip_missing);
    }

    #[test]
    fn unknown_and_missing_fields_are_bad_json() {
        let e = parse::<CheckArgs>(obj(json!({"audio": "a.wav", "scor": "s"}))).unwrap_err();
        assert_eq!(e.code, ErrorCode::BadJson);
        assert_eq!(e.issues[0]["path"], "/scor");
        let e = parse::<CheckArgs>(obj(json!({"audio": "a.wav"}))).unwrap_err();
        assert_eq!(e.code, ErrorCode::BadJson);
        assert!(e.message.contains("score"), "{}", e.message);
        let e = parse::<NoArgs>(obj(json!({"x": 1}))).unwrap_err();
        assert_eq!(e.code, ErrorCode::BadJson);
    }

    #[test]
    fn export_formats() {
        let ex = |f: &str| {
            parse::<ExportArgs>(obj(json!({"input": "s.praxis", "out": "o", "format": f})))
                .unwrap()
                .request()
        };
        match ex("wav").unwrap() {
            Export::Audio(r) => assert_eq!(r.settings.format, "24"),
            other => panic!("{other:?}"),
        }
        match ex("flac16").unwrap() {
            Export::Audio(r) => assert_eq!(r.settings.format, "flac16"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(ex("midi").unwrap(), Export::Midi(_)));
        let with_stems = parse::<ExportArgs>(obj(json!({
            "input": "s.praxis", "out": "o.mid", "format": "midi", "stems": "d"
        })))
        .unwrap()
        .request()
        .unwrap_err();
        assert_eq!(with_stems.code, ErrorCode::BadFormat);
    }
}
