# Brief: WP-P3 scoring depth and hit points (Phase 7)

**Start building only after WP-P4 is merged** (it owns the planner now); then merge
`build/integration` into your branch. Branch from `build/integration`. Read first:
`docs/BRIEFING.md` (all audio-quality rules), PRD FR-9, FR-10 and the Phase 7 exit,
`docs/contracts/planner.md` (plan schema, arranger guarantees), `docs/contracts/score.md`
(markers, sections, automation), the planner in `crates/praxis-planner/src/` (`plan.rs`,
`arrange.rs`, `motifs.rs`, `brief.rs`, `offline.rs`, `claude.rs`), and `crates/praxis-score/src/theory.rs`.

## Build

- **Chord progressions**: a curated library of progressions per mood and mode, voice-leading for
  chord layers, bass that follows roots with passing tones, borrowed chords only when asked.
- **Multi-section form**: intro, build, peak, release (and verse/chorus-like A/B forms) with
  section lengths solved to the target duration; layers enter and leave by section (layering, not
  rewriting); the motif develops across sections (register, density, rhythm variation) while
  staying recognisable.
- **Tension curves and transitions**: an intensity curve over the cue drives dynamics, density,
  filter automation and layer entries; transitions between sections are musical (a pickup, a
  drop to one layer, a held chord), never a stock riser unless the brief asks for one.
- **Emotion presets**: a small set (calm, tense, hopeful, melancholy, playful, dark, warm,
  epic-but-restrained) mapping to mode, tempo range, register, instruments and intensity shape.
- **Hit points (FR-10)**: markers at timestamps ("hits at 0:42, 1:05 and 1:38"), from the brief or
  a plan field, that the arrangement lands on: a section change, an accent or a new layer exactly on
  the marker's bar/beat, with tempo or bar lengths solved so each hit lands within one grid step.
- **2-minute cues** with evolving layers, both providers (offline rules and Claude), plus prompt
  suite cases for each feature.

## You own

`crates/praxis-planner/**` and the planner sections of `docs/contracts/planner.md`; chord helpers
in `crates/praxis-score/src/theory.rs` only as additions (no changed signatures); `demos/phase-7/**`.

## Rules

`PRAXIS_KEYCHAIN=off` for tests; live Claude runs allowed at most twice (under US$3 total; log token
usage); never play audio; 5 jobs / 5 test threads via the worktrees cargo config.

## Done when

A 2-minute score from a brief with three hit points lands each one on its marker (measured on the
render: the onset or section change within 20 ms of the marker) and passes every check; demo renders
in `demos/phase-7/` with a README saying what to listen for (the owner's listening review is the
phase exit); offline prompt suite passes with the new cases; one live run through Claude passes the
new cases; fmt, workspace clippy, `cargo test -p praxis-planner -p praxis-cli --release`, acceptance
1 and the wasm build pass. Report as in docs/BRIEFING.md.
