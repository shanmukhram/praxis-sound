# Brief: WP-U3 real engine adapter (desktop) and Tauri shell

Start only after WP-K (the CLI's shared command layer) is merged. Branch `wp/u3-adapter` from
`build/integration`. Read: `docs/BRIEFING.md`, `docs/adr/0002-ui-shell.md`, `docs/adr/0010-agents-first-class.md`,
`docs/adr/0011-conversation-first-workstation.md`, `docs/contracts/cli.md`, `app/src/engine/EngineAdapter.ts`
(the take model), and the merged command layer in `crates/praxis-cli/src/` (ops).

## Goal
The approved workstation (app/) runs against a mock. Make it real on Windows: every request
becomes a take planned by `praxis-planner`, rendered by the engine with the real instruments
(felt piano, soft kick), checked by `verify_render`, with real audio playback. Agents and the app
must share one take history (the same command layer the MCP server uses).

## You own
`crates/praxis-app/**` (new Tauri 2 crate; the orchestrator adds it to the workspace members, not
default-members), `app/src/engine/tauri/**` (the `TauriEngine` adapter), and a switch in
`app/src/main.ts` that picks Tauri when running inside it, the mock otherwise.

## Build
- Tauri commands are thin wrappers over the command-layer ops: `ask`, `takes`, `restore`, `edit`,
  `render`, `export`, `open_view`. A take is the planner's outcome, the patch, the render paths and
  the verify report, mapped exactly onto the TypeScript `Take` type.
- Playback through the WebView `<audio>` from the rendered file at first. The real-time engine
  (WP-R) replaces it later.
- A project store per sound (`.praxis` files in a user folder) with undo/redo through the score
  crate's `Project`.
- Events: a take made by an agent (through MCP, WP-X) appears in the app's thread live.
- Before adding Tauri, check its current version, licence and footprint, and record them in your
  report (the orchestrator adds the entry to THIRD_PARTY.md).

## Tests
- Rust tests for each command.
- An app test with a fake Tauri bridge.
- A manual run on Windows, with screenshots: "a calm 30-second felt piano loop in A minor" makes
  take 1 with real audio and passing checks.

## Report
As in docs/BRIEFING.md.
