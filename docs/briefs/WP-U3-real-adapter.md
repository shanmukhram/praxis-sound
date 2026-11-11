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
`crates/praxis-app/**` (new Tauri 2 crate), `app/src/engine/tauri/**` (the `TauriEngine`
adapter), and a switch in `app/src/main.ts` that picks Tauri when running inside it, the mock
otherwise. Authorised outside those: in the root `Cargo.toml`, add `crates/praxis-app` to
`[workspace] members` only (not `default-members`) and the audited Tauri crates to
`[workspace.dependencies]`; in `app/package.json` (+ lock), the `@tauri-apps/api` and
`@tauri-apps/cli` packages and a `tauri` script. Check Tauri's current version and licence first.

## Running in parallel (wave 3, 2026-11-11)
- **WP-K2** (`docs/briefs/WP-K2-edits-takes.md`) is adding the shared take history to the
  command layer: `praxis_cli::ops::{ask, takes, restore, edit, variations, keep_variation,
  open_view}` over a sound folder (`project.praxis`, `takes.json` with a `revision`,
  `takes/<n>/mix.wav`). **Do not build a second take store in praxis-app.** Build the shell, the
  IPC bridge, the adapter, playback and the fake-bridge tests first; wire the take commands to
  those ops. The orchestrator will message you when WP-K2 is merged; then merge
  `build/integration` into your branch and finish the wiring. Agent takes appear live by
  watching `takes.json`'s `revision`.
- WP-R (real-time engine) and WP-D1b (analyzer) work in other crates. Playback stays the
  WebView `<audio>` element for this package.

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
