# Brief: WP-AG the sound assistant agent (Claude with every Praxis tool)

Branch from `build/integration`. Read first: `docs/BRIEFING.md` (all rules: API keys only from
the environment or the keychain, never in files or logs), `PRD.md` (FR-1, FR-7, FR-8, FR-34),
`docs/adr/0006-two-level-planner.md`, `docs/adr/0010-agents-first-class.md`,
`docs/adr/0011-conversation-first-workstation.md`, `docs/contracts/cli.md` (including "Takes"),
`docs/contracts/planner.md`, `docs/contracts/patch.md`, `docs/contracts/score.md`,
`docs/agents/praxis-skill/SKILL.md` (the guide written for outside agents), the command layer
`crates/praxis-cli/src/ops/` (especially `takes.rs`, `sound.rs`, `edit.rs`), the MCP tool list in
`crates/praxis-mcp/src/server.rs`, and the existing Claude client in
`crates/praxis-planner/src/claude.rs` (raw HTTP over `ureq`, key handling, strict schemas,
fallbacks).

## What the owner asked for (2026-11-11)

"When the user talks we want it to actually make the changes, understanding what the user said;
it should have the context of what it did earlier in that conversation, and it should have all the
tools necessary to build it. The agent should be able to use the engine and produce, with all the
details about the app and the tools it has to use."

Today a request goes to a one-shot planner (offline rules or one Claude tool call) that can only
fill a ScorePlan or an EditPlan, remembers nothing, and misreads anything outside its intents
(for example "make the piano muffled" rewrote the melody). Replace that, when a key is set, with
an agent loop.

## You own

- `crates/praxis-cli/src/agent/**` (new module: the agent), declared from `crates/praxis-cli/src/lib.rs`.
- `crates/praxis-cli/src/ops/**` where the agent plugs in (`ask` routing, a conversation log in
  the sound folder), `crates/praxis-cli/src/main.rs` (CLI flags), `crates/praxis-cli/tests/**`,
  `crates/praxis-cli/Cargo.toml` (existing workspace deps only).
- `crates/praxis-mcp/**` (only so `praxis_ask` can choose the agent and report its steps).
- `docs/agents/**` (the knowledge the agent is given lives here, shared with outside agents).
- Append an "Assistant agent" section to `docs/contracts/cli.md`.

Do not edit the planner, engine, instruments or app crates. WP-U4 owns `crates/praxis-app` and
`app/` right now; the app gets the agent automatically because it already calls `ops::ask`.
Showing the agent's live steps in the window is a follow-up for after WP-U4 merges: provide the
progress callback, not the UI. WP-P2b is adding tone and space intents to the planner's edits in
parallel; the agent can use them through `compose_edit` once merged, but must not depend on them.

## Build

1. **The loop.** Claude Messages API over raw HTTP (`ureq`, the same client pattern and key lookup
   as `claude.rs`; there is no official Rust SDK). Model `claude-opus-5-5` by default
   (configurable). Adaptive thinking (the model default; do not send `budget_tokens` or
   `disabled`), `output_config.effort` set explicitly (default `medium`, configurable),
   `tool_choice` `auto` only (forced tool choice returns a 400 on this model), `strict: true`
   tool schemas, server-side refusal fallbacks (`fallbacks: "default"` with the
   `server-side-fallback-2026-07-01` beta, as `claude.rs` already does), streaming for long
   turns, check `stop_reason` (`refusal`, `max_tokens`) before running tools, parallel tool
   results returned in one user message, `is_error: true` results for failures. A step budget
   per request (default 16 tool calls) and a wall-clock cap; the agent finishes or explains.
2. **The tools: everything the engine and app can do.** One table of tools, each with a JSON
   Schema from the existing request types and a description written for a model. At least:
   read the sound (score summary, one layer, full score JSON, the take list, what changed
   between takes, `identical_bars`); the preset catalogue with every preset's parameters, ranges
   and what they do (felt piano `tone`, `felt`, `release_s`, synth and drum parameters, effect
   parameters from `praxis_score::params`); compose a new sound from a ScorePlan; compose an
   EditPlan; **apply a ScorePatch directly** (any op in `patch.rs`: instrument parameters,
   effects, automation, notes; locks and scope are still enforced by `apply`); render and check
   (the verify report plus measurements an agent can use instead of ears: loudness, true peak,
   per-layer spectral centroid and low-band energy, onsets on grid); variations, keep a
   variation, restore an earlier take, undo/redo; open a panel view; analyze a dropped clip; and
   `ask_user` (a clarifying question that ends the turn without a take). Every state-changing
   tool works on the sound folder through the same ops as the CLI and MCP, so the take history
   stays shared.
3. **What the agent knows.** A stable system prompt built from files in `docs/agents/` (extend
   the existing guide rather than writing a second one): what Praxis Sound is, the Score model,
   how edits stay scoped, the owner's taste rules from `docs/BRIEFING.md` (rhythm first, notes
   connect, layering, restraint, always in key, seamless loops, real instruments, mixed output),
   the catalogue and parameter tables (generated from code so they never go stale), and how to
   work: read the state, make the smallest change that does what was asked, render, check the
   measurements, fix if a check fails, then reply in one or two plain sentences saying what
   changed and what stayed identical. Mark the prompt and the tool list for prompt caching
   (stable content first, nothing time-varying in it).
4. **Memory of the conversation.** Each sound folder keeps `conversation.json`: every user
   request, the agent's replies, a compact record of its tool calls and results, and the takes it
   made. Each new request sends that history (with the assistant content kept exactly as
   returned, thinking blocks included, so preserved thinking keeps working) plus the current
   state, so "like before but softer" or "undo what you just did to the kick" work. Use
   server-side compaction for long conversations (beta `compact-2026-01-12`; keep the
   compaction blocks the API returns). Writes are atomic like `takes.json`.
5. **Routing.** `ops::ask` uses the agent when a key is available (`provider` auto, or `claude`
   explicitly), and the offline planner otherwise or when asked (`offline`). One request makes
   at most one take (plus any variations it offers), recorded with `who` = "Assistant". The CLI
   gets `praxis ask --agent` behaviour by default when a key exists; MCP `praxis_ask` reports the
   steps the agent took.
6. **Progress.** A callback (`Fn(AgentEvent)`) with the agent's steps in plain words ("Reading
   the sound", "Changing the felt piano's tone", "Rendering", "Checking: 8 of 8 passed"), for the
   app to show later.

## Tests and done when

- Without a key or network: a scripted fake model (and recorded replay fixtures of real API
  responses, as `tests/edit_claude_replay.rs` does for the planner) drives the loop through:
  a new sound; "make the piano muffled" (the agent applies a patch to the felt piano's `tone` /
  `felt`; every note event identical; the piano stem's measured centroid drops); "make it a bit
  brighter than before" in the same conversation (uses the history: undoes part of the previous
  change); a scoped edit with bars 1-12 bit-identical; a locked layer refused out loud; a tool
  error recovered from; the step budget reached and explained; `ask_user` ending the turn.
  Requests are checked for the API rules above (no forced tool choice, no `budget_tokens`,
  strict schemas, cache markers on the stable prefix, history appended unchanged).
- No key ever appears in a file, log, error message or test output.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p praxis-cli -p praxis-mcp --release`, the acceptance test, and the wasm build
  pass. Set `PRAXIS_PACKS` to `C:\Users\admin\Desktop\praxis sound\packs`.
- A live run is **not** part of this package (the owner gives the key after it merges). Write
  `docs/agents/live-run.md`: the exact requests to try live and what each should do.
- Report as in docs/BRIEFING.md, including an estimate of tokens and cost per typical request
  measured from the replay fixtures (with and without cache hits).
