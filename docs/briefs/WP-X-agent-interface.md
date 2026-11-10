# Brief: WP-X agent interface (MCP server + agent guide)

Branch: `wp/x-agents`. Read first: `docs/BRIEFING.md` (binding), `PRD.md`, `docs/PLAN.md`
(WP-X), `docs/adr/0010-agents-first-class.md`, `docs/contracts/cli.md`, `docs/contracts/planner.md`
(including the "For agents" section), `docs/contracts/sound-profile.md` (verify report).

## Goal and why it matters
The owner's AI agents are primary users: they generate, edit and export sounds through Praxis
Sound. Agents cannot listen, so every action returns measurements they can act on, and an agent
can write the plan itself (no API key needed).

## You own
`crates/praxis-mcp/**`, `docs/agents/**`, and the shared command layer if WP-K has placed it in a
module you are told to extend (coordinate through the orchestrator; do not edit praxis-cli files
you were not given).

## Build
- rmcp-based stdio server with the tools in ADR-0010; input schemas generated with schemars;
  outputs are the CLI JSON payloads plus, for renders, the full verify report.
- Long renders report progress notifications; errors use the CLI error codes.
- `docs/agents/praxis-skill/SKILL.md`: when to use Praxis, the plan-first workflow, how to read
  checks and fix failures (off-scale, off-grid, loudness, seam), scoped edits, variations, export;
  two worked examples (acceptance-1 loop; a short sting with a tuned impact under music).
- `docs/agents/mcp.json.example` for Claude Code.

## Tests
Spawn the server binary over stdio in an integration test: initialize, list tools, call each
tool's happy path and one error path; the acceptance-1 plan through `praxis_compose` returns a
project path, WAV path and a check report with every check passing (sampler tests skip if the
piano pack is absent; then use the bell fixture plan).

## Report
As in docs/BRIEFING.md "Report back".
