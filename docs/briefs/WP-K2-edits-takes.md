# Brief: WP-K2 edits, variations and the shared take history in the command layer

Branch `wp/k2-edits-takes` from `build/integration`. Read first: `docs/BRIEFING.md` (all rules
apply), `docs/adr/0010-agents-first-class.md`, `docs/adr/0011-conversation-first-workstation.md`,
`docs/contracts/cli.md`, `docs/contracts/planner.md` (sections "Edits" and "Variations"),
`docs/contracts/patch.md` (projects and audio-level isolation), the command layer in
`crates/praxis-cli/src/ops/`, the MCP server in `crates/praxis-mcp/src/`, and the take model the
app uses in `app/src/engine/EngineAdapter.ts` (`Take`, `TakeState`, `AskRequest`).

## Goal

People and agents share one history of **takes** per sound (ADR-0011). Today the command layer
can generate and render but cannot edit, make variations or keep takes. Add those to
`praxis_cli::ops` once, then expose them through the CLI and the MCP server, so the desktop app
(WP-U3, running in parallel) and agents call the same functions.

## You own

- `crates/praxis-cli/**` (ops, CLI subcommands, tests, its `Cargo.toml` within existing
  workspace deps)
- `crates/praxis-mcp/**`
- `docs/agents/**` (agent guide: add the edit / variations / takes workflow)
- Planner tidy-ups only: `crates/praxis-planner/src/offline.rs`, `crates/praxis-planner/src/brief.rs`
  (a plain-language `summary` for `PlanOutcome` if missing), `crates/praxis-planner/tests/prompt_suite.rs`,
  and a `praxis-engine` dev-dependency in `crates/praxis-planner/Cargo.toml`.
- Append a "Takes" section to `docs/contracts/cli.md` describing exactly what you built (append
  only; do not change existing rows except to add the new commands to the table).

Nobody else edits these files while you work. WP-U3 (Tauri app) consumes your ops; WP-R and
WP-D1b work in other crates.

## Build

1. **Edits.** `ops::edit {project, prompt, provider?, seed?}` -> `plan_project_edit`, saves the
   project, payload per contracts/cli.md (`project, patch, scope, changed_bars, changed_layers`
   plus `summary, refused[]`). `ops::variations {project|score, scope?, n, seed?}` -> N
   candidates, each rendered and checked (`variations/<i>.wav` next to the project), none
   committed until kept. CLI: `praxis edit`, `praxis variations`. MCP: `praxis_edit`,
   `praxis_variations`.
2. **Gain freeze.** Edits must leave untouched bars sample-identical (ADR-0007). After the first
   render of a new sound, store the applied gain in `master.frozen_gain_db` (as a project patch,
   so it is undoable) and render later takes with it. Prove it with a test: generate the
   acceptance-1 brief, edit "change the last four bars to add a subtle melody", render both, and
   show the master is bit-identical over bars 1-12 (outside the tail window in contracts/patch.md).
3. **Takes (the shared history).** A sound is a folder: `project.praxis`, `takes.json`
   (`{format: "praxis-takes", version: 1, revision, current, takes: [...]}`), and
   `takes/<n>/mix.wav`. A take record holds what the app's TypeScript `Take` needs, field for
   field (`n, who, ask, title, say, patch, changed, untouched, render {path, report}, checks,
   opens?, scope?`); the take's score is a project version named `take-<n>`. Writes are atomic
   (write to a temp file, then rename) and bump `revision`, so a watcher (the app) can pick up a
   take an agent made. Ops:
   - `ask {sound, text, who?, scope? {layer, bars}, provider?, seed?}`: on an empty sound,
     plan the brief (take 1); otherwise a scoped edit (honour `scope` exactly: those bars on that
     layer, nothing wider). Render, verify, record the take, return it.
   - `takes {sound}` -> the take list, `current` and the live score.
   - `restore {sound, n}`: going back is an undoable patch (`restore_version`), sets `current`.
   - `keep_variation {sound, ...}`: commits a variation as a new take.
   - `open_view {sound, view, who}`: records a view request (`overview|rhythm|notes|mix|export`)
     the app shows ("Notes opened by <who>").
   - Where sounds live: a `sound` is a folder path; a bare name resolves under
     `PRAXIS_HOME/sounds/<name>` (default `%USERPROFILE%\Documents\Praxis Sound`). Document it.
   CLI: `praxis ask|takes|restore`. MCP: `praxis_ask`, `praxis_takes`, `praxis_restore`,
   `praxis_keep_variation`, `praxis_open_view`.
4. **CLI tidy-ups** (handoff follow-up 4): `validate` accepts `.praxis` projects; a
   `pack_verify_failed` error code; `key clear` deletes the keychain entry.
5. **Planner tidy-ups** (handoff follow-up 3): route `edit_score` inside
   `RuleProvider::call_tool`; `prompt_suite.rs` skips `edit_*` plan files; the render-level
   "last four bars" isolation test in the planner's tests using `praxis-engine` as a dev-dep.

## Done when

- Rust tests for every new op (JSON payload shapes), the CLI binary tests for the new
  subcommands, an MCP stdio test that runs `praxis_ask` twice (new sound, then a scoped edit),
  `praxis_takes`, `praxis_restore`, and checks the take list and the bit-identical bars.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the tests
  of praxis-cli, praxis-mcp and praxis-planner in release, and
  `cargo test -p praxis-cli --release --test acceptance1` pass. Run with `PRAXIS_PACKS` pointing at
  the main checkout's `packs/` folder (`C:\Users\admin\Desktop\praxis sound\packs`) so the piano
  renders.
- Report as in docs/BRIEFING.md, including the exact take-record JSON you settled on (WP-U3 maps
  it onto the TypeScript `Take`).
