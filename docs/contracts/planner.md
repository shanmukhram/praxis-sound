# Contract: planner

Frozen: `crates/praxis-planner/src/provider.rs` (`Provider`, `ToolRequest`, `ProviderError`).
Everything else in praxis-planner belongs to WP-P, which documents its plan schema here when it
lands (append a section; do not change the provider types).

## Design (ADR-0006)

1. **Brief -> plan.** The provider fills one forced tool call whose input schema is the
   `ScorePlan` (high level: key, tempo or target duration, bars, time signature, loop/one-shot,
   emotion, avoid-list, sections with roles and intensities, layers with role, preset and entry
   bar, motifs as spoken rhythm plus scale degrees, chord progression).
2. **Validate the plan** (in key, on grid, presets exist, banned classes absent, restraint rules)
   and **repair**: send the issues back to the provider (max 3 rounds), then fall back to
   deterministic fixes (snap to scale, quantise, drop banned layers).
3. **Arrange.** A deterministic arranger expands the plan into a full `Score`: duration solver
   (FR-2), motif patterns with accents and overlap from spoken rhythm (FR-4, FR-5), layering by
   section (a section adds layers; existing parts are reused unchanged, FR-6), chords, master
   defaults (loudness -16 LUFS, -1 dBTP), and an avoid-list that always includes the trailer
   classes not explicitly requested.
4. **Validate the Score** (`validate` + `lint`); lint warnings trigger arranger fixes.
5. **Edits** (FR-7): the provider returns an `EditPlan` (scope + intent), turned into a
   `ScorePatch` with the bars and layers named; locked layers are never in scope. Applied with
   `praxis_score::apply::apply`.
6. **Variations** (FR-8): N alternates of a selection, each a patch with its own `seed_offset`
   and arranger seed.

## Providers

- `offline::RuleProvider`: a deterministic brief parser (keys, tempo, bars, durations, emotion
  words, instruments, "no X" lists, spoken rhythm in quotes, section phrases like "bars 9-16 add
  a soft kick"). Used in CI and when no key is configured. It must pass acceptance test 1.
- `claude::ClaudeProvider`: Claude Messages API with a forced tool call; model id configurable,
  default `claude-opus-5-5`; key from `ANTHROPIC_API_KEY` or the OS keychain (`keyring`).
  Recorded/replayed fixtures make its tests run offline.
