# Brief: WP-M "make something like this": match loop and originality guard (Phase 6)

**Start building only after WP-D2 and WP-H2 are merged**; then merge `build/integration` into your
branch. Read first: `docs/BRIEFING.md`, PRD FR-14..FR-18 and the Phase 6 exit, PRD design rule
"matching is analysis by synthesis", `docs/contracts/sound-profile.md` (similarity, originality),
the profile code from WP-D2, the sfx presets (`crates/praxis-instruments/src/sfx/`), the planner's
ScorePlan and arranger, and the empty `crates/praxis-match`.

## Build (in `praxis-match`, wasm-safe)

- **Profile to Score (FR-14)**: for music, a ScorePlan with the reference's character (tempo or a
  requested one, key or a requested one, groove/onset pattern -> spoken rhythm, spectral balance ->
  instrument choice and brightness, envelope, reverb -> space, density, sections) in any key, tempo
  or length the user asks; arranged by the existing arranger. For sfx, the matching sfx family
  with its macros (length, brightness, weight, pitch) set from the profile. Ambience: noise beds
  and textures.
- **Match loop (FR-15)**: render, analyse with the same features, adjust parameters (macros,
  filters, reverb, gains, rhythm density) with a simple, deterministic optimiser (coordinate
  descent or Nelder-Mead over a small parameter set), repeat until the similarity stops improving;
  return each iteration's similarity so the user sees it.
- **Mix and match (FR-17)**: groove from clip A, timbre from clip B, mood from text.
- **Originality guard (FR-18)**: every result is checked with `originality::check` against its
  reference; a close melodic copy is rejected and re-generated with a different motif; reference
  audio never reaches the engine or an export.
- **CLI / MCP / assistant**: `praxis match <clip> [--prompt] [--bars] [--key] [--bpm] -o project`
  per contracts/cli.md (`project, similarity, iterations[], originality`), `praxis_match` in MCP,
  and a `match_reference` tool for the assistant agent.

## Test set and exit

Build a reference test set with the engine (no downloads): music references in several styles and
sfx references (whooshes, impacts, risers, drones) rendered with settings the matcher does not
know. Phase 6 exit, measured: every generated sound is closer (similarity) to its own reference
than to any other clip in the set, and passes the originality check. Write demo pairs to
`demos/phase-6/` with a README for the owner's blind listening review (shuffle reference/result
order, keep the answer key in a separate file).

## You own

`crates/praxis-match/**`, `crates/praxis-cli/src/ops/match.rs` plus its lines in `ops/mod.rs` and
`main.rs`, `crates/praxis-cli/src/agent/` for the one new tool, `crates/praxis-mcp/src/` for
`praxis_match`, `crates/praxis-cli/tests/` new files, `demos/phase-6/**`, and the match sections of
`docs/contracts/{sound-profile,cli}.md`.

## Rules

`PRAXIS_KEYCHAIN=off` everywhere (no live Claude calls in this package); never play audio; 5 jobs /
5 test threads via the worktrees cargo config.

## Done when

The ranking test (each result closest to its own reference) and originality pass on the test set;
iterations show similarity rising; demos written; fmt, workspace clippy, `cargo test -p praxis-match
-p praxis-cli -p praxis-mcp --release`, acceptance 1, wasm build pass. Report as in docs/BRIEFING.md.
