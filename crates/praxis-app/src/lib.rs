//! praxis-app: the Praxis Sound desktop app (ADR-0002, ADR-0011). A Tauri 2 shell around the
//! workstation front end in `app/`, whose `TauriEngine` adapter calls the commands here.
//!
//! Every command is a thin wrapper over the shared command layer (`praxis_cli::ops`), so a take
//! made in the app and a take an agent makes through the MCP server land in the same history:
//! the sound folder's `takes.json`, which [`watch`] follows so agent takes appear live.

#![forbid(unsafe_code)]

pub mod audio;
pub mod commands;
mod shell;
pub mod sounds;
pub mod watch;

pub use shell::{AppState, TAKES_EVENT, run};
