//! `praxis`: headless Praxis Sound (FR-33). Every command accepts `--json` and then prints
//! exactly one JSON object on stdout: `{"ok": true, "command": ..., ...}` or
//! `{"ok": false, "command": ..., "error": {"code", "message"}}`. See docs/contracts/cli.md.
//!
//! This file only parses arguments. Every command is implemented once in `praxis_cli::ops`,
//! which the MCP server (WP-X) calls too.

#![forbid(unsafe_code)]

use clap::{Parser, Subcommand};
use praxis_cli::ops::{self, KeyAction, ProviderChoice, RenderSettings, TakeScope, ViewName};
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
    /// Apply a scoped edit to a project (FR-7): an instruction, or an EditPlan you wrote.
    Edit {
        /// The project (.praxis); written back when the edit changes anything.
        project: PathBuf,
        /// The instruction, e.g. "change the last four bars to add a subtle melody".
        #[arg(long, conflicts_with = "plan")]
        prompt: Option<String>,
        /// An EditPlan JSON file (`-` reads stdin). No provider or key needed.
        #[arg(long)]
        plan: Option<PathBuf>,
        /// offline or claude. Default: claude when a key is available, else offline.
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long, default_value_t = ops::generate::DEFAULT_SEED)]
        seed: u64,
        /// Also render the edited score to this file and check it.
        #[arg(long = "render")]
        render_out: Option<PathBuf>,
        #[command(flatten)]
        render: RenderArgs,
    },
    /// Make N alternates of a selection (FR-8), each rendered to variations/<i>.wav and checked.
    /// Nothing is committed (keep one of a sound's with `keep-variation`).
    Variations {
        /// A project (.praxis) or score (.score.json). Or give --sound instead.
        input: Option<PathBuf>,
        /// A sound folder (or bare name): varies its current take.
        #[arg(long, conflicts_with = "input")]
        sound: Option<String>,
        /// Bars to vary, e.g. 13-16 (default: the whole score).
        #[arg(long)]
        bars: Option<String>,
        /// Layer ids to vary (repeatable; default: every unlocked layer playing there).
        #[arg(long = "layer")]
        layers: Vec<String>,
        /// How many (1 to 8).
        #[arg(short, default_value_t = 3)]
        n: usize,
        #[arg(long, default_value_t = ops::generate::DEFAULT_SEED)]
        seed: u64,
        /// Output folder (default: variations/ next to the input).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        #[command(flatten)]
        render: RenderArgs,
    },
    /// Make a take in a sound folder: a brief for a new sound, else a scoped edit of the current
    /// take. Renders takes/<n>/mix.wav, checks it and records it in takes.json.
    Ask {
        /// The sound folder, or a bare name under PRAXIS_HOME/sounds.
        sound: String,
        /// The request in words.
        text: String,
        /// "You" (default) or an agent's name.
        #[arg(long)]
        who: Option<String>,
        /// Hold the edit to these bars of --layer, e.g. 13-16.
        #[arg(long, requires = "layer")]
        bars: Option<String>,
        /// The layer the scope is on (with --bars).
        #[arg(long, requires = "bars")]
        layer: Option<String>,
        /// A hand edit: commit this ScorePatch JSON file (`-` reads stdin) instead of planning.
        #[arg(long)]
        patch: Option<PathBuf>,
        #[arg(long)]
        title: Option<String>,
        /// `claude` (the sound assistant; the default when an API key is available) or
        /// `offline` (the rule planner).
        #[arg(long)]
        provider: Option<String>,
        /// The assistant's model (default claude-opus-5-5).
        #[arg(long)]
        model: Option<String>,
        /// The assistant's effort: low, medium (default), high, xhigh or max.
        #[arg(long)]
        effort: Option<String>,
        /// Tool calls the assistant may make for this request (default 16).
        #[arg(long)]
        max_steps: Option<u32>,
        /// Default: the new take's number.
        #[arg(long)]
        seed: Option<u64>,
        #[arg(long)]
        skip_missing: bool,
        /// Include the take's full score in the output.
        #[arg(long)]
        include_score: bool,
        /// A reference clip the request is about; "make something like this" runs the match loop.
        #[arg(long, conflicts_with = "reference_library")]
        reference: Option<PathBuf>,
        /// A reference-library entry id instead of a clip.
        #[arg(long)]
        reference_library: Option<String>,
    },
    /// List a sound's takes, the current take and the live score.
    Takes {
        sound: String,
        /// Include each take's score.
        #[arg(long)]
        include_scores: bool,
    },
    /// Go back to take N (an undoable patch); it becomes the current take.
    Restore { sound: String, n: u32 },
    /// Commit a variation made with `variations --sound` as a new take.
    KeepVariation {
        sound: String,
        /// 1-based variation number.
        index: usize,
        #[arg(long)]
        who: Option<String>,
        #[arg(long)]
        skip_missing: bool,
    },
    /// Ask the app to show a panel view of a sound (overview, rhythm, notes, mix, export).
    OpenView {
        sound: String,
        view: String,
        #[arg(long)]
        who: Option<String>,
    },
    /// Measure a render against its Score. Exits 1 if any check fails.
    Check {
        /// Rendered audio (WAV or FLAC).
        audio: PathBuf,
        /// Score JSON or project.
        #[arg(long)]
        score: PathBuf,
    },
    /// Measure an audio or video clip: its complete Sound Profile.
    Analyze {
        clip: PathBuf,
        /// Also write the measurements to this JSON file.
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 4)]
        beats_per_bar: u8,
    },
    /// The reference library: saved, tagged Sound Profiles (never audio).
    Library {
        #[command(subcommand)]
        action: LibraryCmd,
    },
    /// Make something like a reference clip: a new project with its character, refined by
    /// rendering and measuring until the similarity stops improving (FR-14 to FR-18).
    Match {
        /// The reference clip (audio or video).
        clip: PathBuf,
        /// Mood brief, e.g. "melancholy".
        #[arg(long)]
        prompt: Option<String>,
        /// Take the timbre from this second clip; the reference gives the groove.
        #[arg(long)]
        timbre: Option<PathBuf>,
        #[arg(long)]
        bars: Option<u32>,
        /// Key, e.g. "A minor", "F#m", "Eb dorian".
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        bpm: Option<f64>,
        #[arg(long)]
        seed: Option<u64>,
        /// Most renders inside the match loop (default 40).
        #[arg(long)]
        max_iterations: Option<usize>,
        /// Also render the result to this file and check it.
        #[arg(long)]
        render: Option<PathBuf>,
        /// The new `.praxis` project.
        #[arg(short, long)]
        out: PathBuf,
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
    /// Time a render on this machine: a 2-minute, 8-layer piece (or a score you give) against
    /// the 20x real-time target. Writes nothing.
    Bench {
        /// Bench this score or project instead of the built-in piece.
        input: Option<PathBuf>,
        /// Length of the built-in piece in seconds.
        #[arg(long, default_value_t = 120.0)]
        seconds: f64,
        /// Timed runs after one warm-up render.
        #[arg(long, default_value_t = 3)]
        runs: usize,
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// Also time each layer rendered alone.
        #[arg(long)]
        per_layer: bool,
    },
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
enum LibraryCmd {
    /// Analyse a clip (or read a profile JSON from `analyze -o`) and save its profile.
    Add {
        input: PathBuf,
        /// Style name (default: the file name).
        #[arg(long)]
        name: Option<String>,
        /// A tag; repeat for more.
        #[arg(long = "tag")]
        tags: Vec<String>,
        #[arg(long, default_value_t = 4)]
        beats_per_bar: u8,
    },
    /// Every saved profile.
    List,
    /// One saved profile in full.
    Show { id: String },
    /// Add or remove tags.
    Tag {
        id: String,
        #[arg(long)]
        add: Vec<String>,
        #[arg(long)]
        remove: Vec<String>,
    },
    /// Delete a saved profile.
    Remove { id: String },
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
        Command::Edit { .. } => "edit",
        Command::Variations { .. } => "variations",
        Command::Ask { .. } => "ask",
        Command::Takes { .. } => "takes",
        Command::Restore { .. } => "restore",
        Command::KeepVariation { .. } => "keep-variation",
        Command::OpenView { .. } => "open-view",
        Command::Analyze { .. } => "analyze",
        Command::Library { .. } => "library",
        Command::Match { .. } => "match",
        Command::ExportMidi { .. } => "export-midi",
        Command::Packs { .. } => "packs",
        Command::Presets => "presets",
        Command::Bench { .. } => "bench",
        Command::Key { .. } => "key",
    }
}

/// `--reference CLIP` or `--reference-library ID` as `ask`'s reference.
fn reference_arg(clip: Option<PathBuf>, library: Option<String>) -> Option<ops::ReferenceArg> {
    match (clip, library) {
        (Some(c), _) => Some(ops::ReferenceArg {
            name: c
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| c.display().to_string()),
            clip: Some(c),
            library_id: None,
        }),
        (None, Some(id)) => Some(ops::ReferenceArg {
            name: id.clone(),
            clip: None,
            library_id: Some(id),
        }),
        (None, None) => None,
    }
}

fn read_stdin() -> Result<String, OpError> {
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .map_err(|e| OpError::new(ErrorCode::Io, format!("stdin: {e}")))?;
    Ok(s)
}

fn parse_provider(p: Option<&str>) -> Result<Option<ProviderChoice>, OpError> {
    match p {
        None => Ok(None),
        Some(p) => ProviderChoice::parse(p).map(Some).ok_or_else(|| {
            OpError::new(
                ErrorCode::Error,
                format!("unknown provider `{p}`; use offline or claude"),
            )
        }),
    }
}

/// `13-16` or `13` -> a bar range.
fn parse_bars(s: &str) -> Result<praxis_score::BarRange, OpError> {
    let bad = || {
        OpError::new(
            ErrorCode::Error,
            format!("bars `{s}`: give a range like 13-16 or one bar like 9"),
        )
    };
    let (a, b) = s.split_once(['-', '\u{2013}']).unwrap_or((s, s));
    let start: u32 = a.trim().parse().map_err(|_| bad())?;
    let end: u32 = b.trim().parse().map_err(|_| bad())?;
    if start == 0 || end < start {
        return Err(bad());
    }
    Ok(praxis_score::BarRange { start, end })
}

/// Whether a file is a `.praxis` project (by its `format` field).
fn is_project(path: &Path) -> Result<bool, OpError> {
    Ok(read_plan(path)?.get("format").and_then(Value::as_str) == Some("praxis-project"))
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
            let provider = parse_provider(provider.as_deref())?;
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
        Command::Edit {
            project,
            prompt,
            plan,
            provider,
            model,
            seed,
            render_out,
            render,
        } => ops::edit(&ops::EditRequest {
            project,
            prompt,
            plan: plan.as_deref().map(read_plan).transpose()?,
            provider: parse_provider(provider.as_deref())?,
            model,
            seed,
            render: render_out,
            render_settings: render.settings(None),
        }),
        Command::Variations {
            input,
            sound,
            bars,
            layers,
            n,
            seed,
            out_dir,
            render,
        } => {
            let (project, score) = match &input {
                Some(p) if is_project(p)? => (Some(p.clone()), None),
                Some(p) => (None, Some(p.clone())),
                None => (None, None),
            };
            ops::variations(&ops::VariationsRequest {
                project,
                score,
                sound,
                scope: praxis_score::patch::Scope {
                    bars: bars.as_deref().map(parse_bars).transpose()?,
                    layers,
                },
                n,
                seed,
                out_dir,
                render_settings: render.settings(None),
            })
        }
        Command::Ask {
            sound,
            text,
            who,
            bars,
            layer,
            patch,
            title,
            provider,
            model,
            effort,
            max_steps,
            seed,
            skip_missing,
            include_score,
            reference,
            reference_library,
        } => {
            let mut agent = praxis_cli::agent::AgentConfig::default();
            if let Some(e) = effort {
                if !["low", "medium", "high", "xhigh", "max"].contains(&e.as_str()) {
                    return Err(OpError::new(
                        ErrorCode::Error,
                        format!("unknown effort `{e}`; use low, medium, high, xhigh or max"),
                    ));
                }
                agent.effort = e;
            }
            if let Some(n) = max_steps {
                agent.max_steps = n.max(1);
            }
            // The assistant's steps, in words, on stderr (stdout stays one JSON object).
            use praxis_cli::agent::AgentEvent;
            let progress: praxis_cli::agent::Progress =
                std::sync::Arc::new(|e: &AgentEvent| match e {
                    AgentEvent::Step { words, .. } => eprintln!("  {words}"),
                    AgentEvent::StepDone { ok, words, .. }
                        if !*ok || words.starts_with("Checking") =>
                    {
                        eprintln!("    {words}")
                    }
                    _ => {}
                });
            let opts = ops::AskOptions {
                client: None,
                agent,
                progress: Some(progress),
            };
            let scope = match (bars, layer) {
                (Some(b), Some(layer)) => Some(TakeScope {
                    layer,
                    bars: parse_bars(&b)?,
                }),
                _ => None,
            };
            let patch = match patch.as_deref().map(read_plan).transpose()? {
                Some(v) => Some(serde_json::from_value(v).map_err(|e| {
                    OpError::new(
                        ErrorCode::BadJson,
                        format!("the patch is not a ScorePatch: {e}"),
                    )
                })?),
                None => None,
            };
            ops::ask_with(
                &ops::AskRequest {
                    sound,
                    text,
                    who,
                    scope,
                    patch,
                    title,
                    provider: parse_provider(provider.as_deref())?,
                    model,
                    seed,
                    skip_missing,
                    include_score,
                    reference: reference_arg(reference, reference_library),
                },
                &opts,
            )
        }
        Command::Takes {
            sound,
            include_scores,
        } => ops::takes(&ops::TakesRequest {
            sound,
            include_scores,
        }),
        Command::Restore { sound, n } => ops::restore(&ops::RestoreRequest { sound, n }),
        Command::KeepVariation {
            sound,
            index,
            who,
            skip_missing,
        } => ops::keep_variation(&ops::KeepVariationRequest {
            sound,
            index,
            who,
            ask: None,
            title: None,
            skip_missing,
            include_score: false,
        }),
        Command::OpenView { sound, view, who } => {
            let view: ViewName =
                serde_json::from_value(Value::String(view.clone())).map_err(|_| {
                    OpError::new(
                        ErrorCode::Error,
                        format!(
                            "unknown view `{view}`; use overview, rhythm, notes, mix or export"
                        ),
                    )
                })?;
            ops::open_view(&ops::OpenViewRequest { sound, view, who })
        }
        Command::Analyze {
            clip,
            out,
            beats_per_bar,
        } => ops::analyze(&ops::AnalyzeRequest {
            clip,
            out,
            beats_per_bar,
        }),
        Command::Library { action } => {
            let action = match action {
                LibraryCmd::Add {
                    input,
                    name,
                    tags,
                    beats_per_bar,
                } => ops::LibraryAction::Add {
                    input,
                    name,
                    tags,
                    beats_per_bar,
                },
                LibraryCmd::List => ops::LibraryAction::List,
                LibraryCmd::Show { id } => ops::LibraryAction::Show { id },
                LibraryCmd::Tag { id, add, remove } => ops::LibraryAction::Tag { id, add, remove },
                LibraryCmd::Remove { id } => ops::LibraryAction::Remove { id },
            };
            ops::library(&ops::LibraryRequest { action, dir: None })
        }
        Command::Match {
            clip,
            prompt,
            timbre,
            bars,
            key,
            bpm,
            seed,
            max_iterations,
            render,
            out,
        } => ops::match_clip(&ops::MatchClipRequest {
            clip,
            out,
            prompt,
            timbre,
            bars,
            key,
            bpm,
            seed,
            max_iterations,
            render,
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
        Command::Bench {
            input,
            seconds,
            runs,
            sample_rate,
            per_layer,
        } => ops::bench(&ops::BenchRequest {
            input,
            seconds,
            runs,
            sample_rate,
            per_layer,
        }),
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
    ops::crash::install("praxis");
    let cli = parse();
    let command = name(&cli.command);
    std::process::exit(emit(command, cli.json, run(cli.command)));
}
