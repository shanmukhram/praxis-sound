# Brief: WP-Z performance, release and audit (Phase 8, last)

**Start building only when the orchestrator says every other package is merged.** Until then you
may read and plan, nothing else. Read first: `docs/BRIEFING.md`, PRD section 2 (every success
criterion) and Phase 8, `docs/PLAN.md` ("Success-criteria checks"), `THIRD_PARTY.md`,
`docs/NEEDS_OWNER.md`, the CLI, the app and the release profile in the root `Cargo.toml`.

## Build

- **Measure every PRD success criterion on release builds** and write
  `docs/release/success-criteria.md` with the numbers and how each was measured: correctness
  (acceptance 1 from one prompt), determinism, edit isolation, loop seam, render speed
  (`praxis bench`: a 2-minute, 8-layer piece, target 20x real time), real-time preview (xruns at
  256 frames requested; note the shared-mode 480-frame callbacks), footprint (installer under 30
  MB without packs, cold start under 1 s, idle memory under 200 MB), reference analysis and match
  (from WP-D2 and WP-M), loudness. Fix what misses, or explain exactly why it cannot be met.
- **Installer** for Windows (Tauri's NSIS bundle). Building it downloads the NSIS tools from
  Tauri's official sources; the owner approved downloads (2026-11-11). Record them in
  `THIRD_PARTY.md`. Sign nothing (no certificate; say so).
- **Pack downloader** in the app (install the felt-piano pack from `packs/index.json` with its
  blake3 check, with progress, using the existing `praxis packs install` logic).
- **CLI documentation** (`docs/cli/`: every command, its JSON, examples) and the agent guide
  checked against the code.
- **Crash reporting**: a local crash log (panic hook writing a report to `PRAXIS_HOME/logs`, never
  uploaded), shown to the user with how to send it.
- **Licence audit**: `cargo deny` (already installed) with a `deny.toml` allowing only permissive
  licences (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, CC0, Unlicense, MPL refused), the npm
  dependency licences, and every sample/pack in `THIRD_PARTY.md`.
- Run the full three-OS CI by hand (`gh workflow run ci --ref build/integration`) if the Actions
  minutes allow; otherwise record that it is blocked by billing.

## You own

`docs/release/**`, `docs/cli/**`, `deny.toml`, `crates/praxis-cli/src/ops/bench.rs` plus its lines
in `ops/mod.rs` and `main.rs`, the bundle settings in `crates/praxis-app/tauri.conf.json`, crash
reporting modules you add, the pack-downloader UI files you add under `app/src/ui/`, and
`THIRD_PARTY.md`.

## Rules

`PRAXIS_KEYCHAIN=off`; never play audio except the opt-in device test after asking; 5 jobs / 5 test
threads via the worktrees cargo config.

## Done when

`docs/release/success-criteria.md` shows every criterion passing on release builds (or a precise,
agreed exception), the installer is built and under 30 MB, `cargo deny check`
passes, and fmt, workspace clippy, the full workspace release tests and the wasm build pass.
Report as in docs/BRIEFING.md.
