//! praxis-mcp: the Praxis Sound MCP server (ADR-0010), stdio transport, built on `rmcp`.
//!
//! Every tool calls the shared command layer `praxis_cli::ops` and returns the same JSON the
//! `praxis --json` CLI prints, so the CLI and the MCP tools never drift. Paths in, paths and
//! measurements out: audio is written to files the agent names, never returned inline.
//!
//! Tools: `praxis_plan_schema`, `praxis_presets`, `praxis_compose`, `praxis_generate`,
//! `praxis_render`, `praxis_check`, `praxis_analyze` (the full Sound Profile), `praxis_library`
//! (saved profiles), `praxis_match` (make something like a clip), `praxis_export`, `praxis_validate`,
//! `praxis_edit`, `praxis_variations`, and the shared take history (ADR-0011): `praxis_ask`,
//! `praxis_takes`, `praxis_restore`, `praxis_keep_variation`, `praxis_open_view`. A take an
//! agent makes is recorded as `"Agent · <MCP client name>"` unless the call names `who`.

#![forbid(unsafe_code)]

pub mod args;
pub mod guide;
pub mod server;

pub use server::PraxisServer;

/// Serves [`PraxisServer`] over stdin/stdout until the client disconnects.
pub async fn serve_stdio() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use rmcp::ServiceExt;
    let service = PraxisServer::new().serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
