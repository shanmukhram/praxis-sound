//! `praxis`: headless Praxis Sound (FR-33). Every command accepts `--json` and then prints
//! exactly one JSON object on stdout: `{"ok": true, "command": ..., ...}` or
//! `{"ok": false, "command": ..., "error": {"code", "message"}}`. See docs/contracts/cli.md.
//!
//! This file only parses arguments. Every command is implemented once in `praxis_cli::ops`,
//! which the MCP server (WP-X) calls too.
//!
//! Not here yet: `edit` (needs WP-P2's scoped edits) and `match` (WP-M).

#![forbid(unsafe_code)]

use clap::{Parser, Subcommand};
use praxis_cli::ops::{self, KeyAction, ProviderChoice, RenderSettings};
use praxis_cli::{ErrorCode, OpError, emit};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(
    name = "praxis",
    version,
    about = "Praxis Sound: score-driven music and sound design"
)]
struct Cli {
    /// Print one JSON object on stdout instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Args, Debug)]
struct RenderArgs {
    /// 16, 24 or 32f (WAV), or flac / flac24 / flac16.
    #[arg(long, default_value = "24")]
    format: String,
    #[arg(long, default_value_t = 48_000)]
    sample_rate: u32,
    /// Directory for per-layer stems (same format as the master).
    #[arg(long)]
    stems: Option<PathBuf>,
    /// Render only layers whose instrument is available here; list the rest in skipped_layers.
    #[arg(long)]
    skip_missing: bool,
}

impl RenderArgs {
    fn settings(self, seed: Option<u64>) -> RenderSettings {
        RenderSettings {
            format: self.format,
            sample_rate: self.sample_rate,
            stems: self.stems,
            seed,
            skip_missing: self.skip_missing,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Render a score (or project) file to audio, with measurements.
    Render {
        /// Score JSON (.score.json) or project (.praxis).
        input: PathBuf,
        /// Output audio path.
        #[arg(short, long)]
        out: PathBuf,
        /// Override the score seed.
        #[arg(long)]
        seed: Option<u64>,
        #[command(flatten)]
        render: RenderArgs,
    },
    /// Validate a score file and report issues.
    Validate { input: PathBuf },
    /// Print or write the published JSON Schemas (score, patch, and the ScorePlan as `plan`).
    Schema {
        /// Write schema/score.schema.json and schema/patch.schema.json.
        #[arg(long)]
        write: bool,
    },
    /// Plan a Score from a brief (or compose an agent-written ScorePlan) into a project.
    Generate {
        /// The brief.
        #[arg(long, conflicts_with = "plan")]
        prompt: Option<String>,
        /// A ScorePlan JSON file (`-` reads stdin). No provider or key needed.
        #[arg(long)]
        plan: Option<PathBuf>,
        /// offline or claude. Default: claude when a key is available, else offline.
        #[arg(long)]
        provider: Option<String>,
        /// Claude model.
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value_t = ops::generate::DEFAULT_SEED)]
        seed: u64,
        /// Project file to write (.praxis).
        #[arg(short, long)]
        out: PathBuf,
        /// Also render to this file and check the render.
        #[arg(long = "render")]
        render_out: Option<PathBuf>,
        #[command(flatten)]
        render: RenderArgs,
    },
    /// Measure a render against its Score. Exits 1 if any check fails.
    Check {
        /// Rendered audio (WAV or FLAC).
        audio: PathBuf,
        /// Score JSON or project.
        #[arg(long)]
        score: PathBuf,
    },
    /// Measure an audio or video clip (the Sound Profile features available so far).
    Analyze {
        clip: PathBuf,
        /// Also write the measurements to this JSON file.
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 4)]
        beats_per_bar: u8,
    },
    /// Export a score (or project) as a Standard MIDI File.
    ExportMidi {
        input: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// List or install instrument packs from packs/index.json.
    Packs {
        #[command(subcommand)]
        action: PacksCmd,
    },
    /// The planner's presets and whether each can render here.
    Presets,
    /// Store or clear the Anthropic API key in the OS keychain (read from stdin).
    Key {
        #[command(subcommand)]
        action: KeyCmd,
    },
}

#[derive(Subcommand, Debug)]
enum PacksCmd {
    /// Every pack in the index and whether it is installed.
    List,
    /// Download (or take --archive), verify and unpack a pack.
    Install {
        name: String,
        /// Use this already-downloaded archive instead of downloading.
        #[arg(long)]
        archive: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum KeyCmd {
    /// Read the key from stdin and store it in the OS keychain.
    Set,
    /// Remove the stored key.
    Clear,
    /// Whether a key is available (never prints it).
    Status,
}

fn name(c: &Command) -> &'static str {
    match c {
        Command::Render { .. } => "render",
        Command::Validate { .. } => "validate",
        Command::Schema { .. } => "schema",
        Command::Generate { .. } => "generate",
        Command::Check { .. } => "check",
        Command::Analyze { .. } => "analyze",
        Command::ExportMidi { .. } => "export-midi",
        Command::Packs { .. } => "packs",
        Command::Presets => "presets",
        Command::Key { .. } => "key",
    }
}

fn read_stdin() -> Result<String, OpError> {
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .map_err(|e| OpError::new(ErrorCode::Io, format!("stdin: {e}")))?;
    Ok(s)
}

fn read_plan(path: &Path) -> Result<Value, OpError> {
    let text = if path == Path::new("-") {
        read_stdin()?
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| OpError::new(ErrorCode::Io, format!("{}: {e}", path.display())))?
    };
    serde_json::from_str(&text)
        .map_err(|e| OpError::new(ErrorCode::BadJson, format!("{}: {e}", path.display())))
}

fn run(command: Command) -> Result<Value, OpError> {
    match command {
        Command::Render {
            input,
            out,
            seed,
            render,
        } => ops::render(&ops::RenderRequest {
            input,
            out,
            settings: render.settings(seed),
        }),
        Command::Validate { input } => ops::validate(&ops::ValidateRequest { input }),
        Command::Schema { write } => ops::schema(&ops::SchemaRequest { write }),
        Command::Generate {
            prompt,
            plan,
            provider,
            model,
            seed,
            out,
            render_out,
            render,
        } => {
            let provider = match provider.as_deref() {
                None => None,
                Some(p) => Some(ProviderChoice::parse(p).ok_or_else(|| {
                    OpError::new(
                        ErrorCode::Error,
                        format!("unknown provider `{p}`; use offline or claude"),
                    )
                })?),
            };
            let plan = plan.as_deref().map(read_plan).transpose()?;
            ops::generate(&ops::GenerateRequest {
                prompt,
                plan,
                provider,
                model,
                seed,
                out,
                render: render_out,
                render_settings: render.settings(None),
            })
        }
        Command::Check { audio, score } => ops::check(&ops::CheckRequest { audio, score }),
        Command::Analyze {
            clip,
            out,
            beats_per_bar,
        } => ops::analyze(&ops::AnalyzeRequest {
            clip,
            out,
            beats_per_bar,
        }),
        Command::ExportMidi { input, out } => {
            ops::export_midi(&ops::ExportMidiRequest { input, out })
        }
        Command::Packs { action } => match action {
            PacksCmd::List => ops::packs_list(),
            PacksCmd::Install { name, archive } => {
                ops::packs_install(&ops::PacksInstallRequest { name, archive })
            }
        },
        Command::Presets => ops::presets(),
        Command::Key { action } => {
            let action = match action {
                KeyCmd::Set => KeyAction::Set(read_stdin()?),
                KeyCmd::Clear => KeyAction::Clear,
                KeyCmd::Status => KeyAction::Status,
            };
            ops::key(&ops::KeyRequest { action })
        }
    }
}

/// Argument errors: with `--json` anywhere on the line, the usual JSON error object (code
/// `error`), so callers that asked for JSON always get JSON. `--help`/`--version` print text.
fn parse() -> Cli {
    match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            let wants_json = std::env::args().any(|a| a == "--json");
            let informational = matches!(
                e.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            );
            if informational {
                e.exit();
            }
            let command = std::env::args()
                .skip(1)
                .find(|a| !a.starts_with('-'))
                .unwrap_or_else(|| "praxis".into());
            // clap echoes unexpected arguments; for `key` one of them may be the key itself.
            let msg = if command == "key" {
                "usage: praxis key set|clear|status (`set` reads the key from stdin; it is never taken from arguments)"
                    .to_string()
            } else {
                e.render().to_string()
            };
            if !wants_json {
                if command == "key" {
                    eprintln!("error: {msg}");
                    std::process::exit(2);
                }
                e.exit();
            }
            let err = OpError::new(ErrorCode::Error, msg.trim().to_string());
            std::process::exit(emit(&command, true, Err(err)));
        }
    }
}

fn main() {
    let cli = parse();
    let command = name(&cli.command);
    std::process::exit(emit(command, cli.json, run(cli.command)));
}
