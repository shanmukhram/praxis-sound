# ADR-0010: Agents are first-class users (MCP server, plan-in, measured feedback)

Status: accepted (owner, 2026-11-10)

The owner builds Praxis Sound so that their AI agents, as well as people, produce sounds with it.
Agents cannot listen and are themselves language models. Decisions:

1. **MCP server.** A new crate `praxis-mcp` (binary entry `praxis mcp`, stdio transport) built on
   `rmcp` 3.9 (the official Rust MCP SDK, Apache-2.0, released 2026-11-05) exposes the product as
   tools: `praxis_plan_schema`, `praxis_presets`, `praxis_compose` (plan in -> project + render),
   `praxis_generate` (brief in, using the offline planner or Claude when a key exists),
   `praxis_edit` (scoped edit), `praxis_variations`, `praxis_render`, `praxis_check`,
   `praxis_analyze`, `praxis_match`, `praxis_export` (WAV/FLAC/stems/MIDI). Tool inputs and outputs
   reuse the CLI JSON shapes (docs/contracts/cli.md), so the CLI and MCP never drift.
2. **The calling agent can be the planner.** `praxis_compose` takes a ScorePlan the agent writes;
   Praxis validates, repairs deterministically and arranges it. No API key is needed for this path.
   Validation errors are actionable: JSON pointer, what is wrong, allowed values.
3. **Every render returns measurements.** Render responses include the `verify_render` report
   (detected tempo and key, grid alignment, LUFS, true peak, loop seam, clipping) and pass/fail
   against the Score, so an agent can check its own result and iterate without ears.
4. **Files, not blobs.** Tools write audio to paths the agent names (or a project folder) and
   return paths plus measurements; they never return audio bytes in the response.
5. **An agent guide.** `docs/agents/praxis-skill/SKILL.md` teaches agents the workflow (write plan
   -> compose -> read checks -> edit scoped bars -> export), with worked examples. It can be
   installed as a Claude Code skill.
