//! `praxis`: headless Praxis Sound (FR-33). Every command accepts `--json` and then prints
//! exactly one JSON object on stdout: `{"ok": true, "command": ..., ...}` or
//! `{"ok": false, "command": ..., "error": {"code", "message"}}`. See docs/contracts/cli.md.

#![forbid(unsafe_code)]

mod commands;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

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

#[derive(Subcommand, Debug)]
enum Command {
    /// Render a score (or project) file to audio.
    Render {
        /// Score JSON (.score.json) or project (.praxis).
        input: PathBuf,
        /// Output WAV path.
        #[arg(short, long)]
        out: PathBuf,
        /// 16, 24 or 32f.
        #[arg(long, default_value = "24")]
        format: String,
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// Directory for per-layer stems (WAV).
        #[arg(long)]
        stems: Option<PathBuf>,
        /// Override the score seed.
        #[arg(long)]
        seed: Option<u64>,
    },
    /// Validate a score file and report issues.
    Validate { input: PathBuf },
    /// Print or write the published JSON Schemas.
    Schema {
        /// Write schema/score.schema.json and schema/patch.schema.json.
        #[arg(long)]
        write: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    let name = match &cli.command {
        Command::Render { .. } => "render",
        Command::Validate { .. } => "validate",
        Command::Schema { .. } => "schema",
    };
    let result = match cli.command {
        Command::Render {
            input,
            out,
            format,
            sample_rate,
            stems,
            seed,
        } => commands::render(&input, &out, &format, sample_rate, stems.as_deref(), seed),
        Command::Validate { input } => commands::validate(&input),
        Command::Schema { write } => commands::schema(write),
    };
    std::process::exit(commands::emit(name, cli.json, result));
}
