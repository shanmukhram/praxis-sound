//! `praxis-mcp`: the Praxis Sound MCP server on stdio. Configure it in an MCP client (see
//! docs/agents/mcp.json.example). Stdout carries only the protocol; diagnostics go to stderr.

#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    if std::env::args().skip(1).any(|a| a == "--help" || a == "-h") {
        eprintln!(
            "praxis-mcp {}: the Praxis Sound MCP server (stdio).\n\
             Run it from an MCP client; see docs/agents/mcp.json.example.\n\
             Environment: PRAXIS_PACKS (instrument packs folder), ANTHROPIC_API_KEY (optional, \
             for praxis_generate with Claude).",
            env!("CARGO_PKG_VERSION")
        );
        return std::process::ExitCode::SUCCESS;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("praxis-mcp: cannot start the async runtime: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    match runtime.block_on(praxis_mcp::serve_stdio()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("praxis-mcp: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
