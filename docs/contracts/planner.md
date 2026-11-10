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

## Plan schema (WP-P)

Code: `crates/praxis-planner/src/plan.rs` (`ScorePlan`, `validate_plan`, issue codes),
`repair.rs` (repair loop and deterministic fixes), `arrange.rs` (plan -> Score),
`offline.rs` (`RuleProvider`), `claude.rs` (`ClaudeProvider`), `brief.rs` (entry points),
`catalogue.rs` (preset facts), `motifs.rs` (default rhythms and degree templates). The JSON
Schema is `praxis_planner::plan_schema()` (schemars); the tool is named `plan_score`.
Entry points: `plan_brief(brief, provider, catalogue, seed)` and
`compose_plan(plan_json, catalogue, seed)`, both returning `PlanOutcome {score, plan, attempts,
repairs, rounds, provider_id, lint}`.

### Fields

| Field | Meaning |
| --- | --- |
| `title` | Short title (optional) |
| `emotion` | Exactly one emotion word (restraint) |
| `key` | `{tonic, mode}` as in the Score |
| `time_signature` | Default 4/4 |
| `tempo_bpm`, `duration_s`, `bars` | Any two fix the third (FR-2): bars + duration solves the tempo (16 bars in 30 s = 128 BPM); tempo + duration solves the bars (nearest whole bar, `timeline::duration::solve_bars`). If all three are given they must agree within 0.5 BPM. The resolved tempo must be 40-220 BPM, bars 1-512 |
| `playback` | `loop` or `one_shot` (default; 4 s tail) |
| `avoid` | Classes the brief bans. The Score's avoid-list is `avoid` plus every trailer class (riser, sweep, impact, boom, pad, whoosh, fill) not explicitly requested |
| `requested` | Trailer classes explicitly asked for. With a brief (`plan_brief`) this is derived from the brief's words and the field is ignored; mood words ("cinematic", "epic", "mysterious") never count. With `compose_plan` the field decides |
| `sections[]` | `{name, bars {start, end}, role, intensity 0..1, layers [ids]}`; in bar order, covering every bar once. Sections add or remove whole layers |
| `layers[]` | `{id, name, class, preset {kind, name}, register low/mid/high, dynamics soft/normal/strong, gain_db?, motif}`; at most 5 |
| `layers[].motif` | `{grid, length_bars 1..8, spoken, degrees[], octave?, sustain_steps?, follow_chords, notes[]}`. `spoken` uses the FR-5 tokens (`praxis_score::rhythm`); a rhythm whose length does not divide the motif restarts every bar. `degrees` (0 = tonic, 7 = an octave up) are cycled over the hits and restart with each motif repetition; with `follow_chords` they are relative to each bar's chord root, and chords/harmony/pad layers stack triads. Optional explicit `notes` `{step, pitch, accent, velocity?, length?}` replace the generated notes |
| `chords[]` | `{bars, symbol}`; roots (and slash basses) must be in the key; bars without a chord use the tonic triad |
| `notes[]` | Planner remarks (unmapped instruments, assumptions); copied to `meta.provenance.planner_notes` |

### Issue codes and repairs

`validate_plan` returns `PlanIssue {code, path, message}`: `path` is a JSON pointer into the plan
and the message names the allowed values (preset names, the scale's pitch classes, the grids that
divide the bar, the spoken tokens). Codes: `schema`, `timing`, `time_signature`, `bars`,
`emotion`, `bad_id`, `duplicate_id`, `no_layers`, `too_many_layers`, `unknown_preset`,
`preset_kind`, `class_mismatch`, `avoided_class`, `unrequested_class`, `grid`, `motif_length`,
`spoken`, `degrees`, `octave`, `sustain`, `off_scale`, `off_grid`, `note_range`, `velocity`,
`gain`, `sections`, `unknown_layer`, `inactive_layer`, `intensity`, `chord`.

The loop: provider call -> parse (field-level `schema` issues with pointers) -> validate -> on
issues, the issues go back to the provider in a user message (at most 3 repair rounds; a provider
that returns the same plan twice is not asked again) -> deterministic `fix_plan`: solve or default
the timing, reduce the emotion to one word, a catalogue preset for unknown ones, the preset's
class wins for drums, effects, pads and textures, drop banned and unrequested layers, a grid that
fits the bar, clamp lengths, degrees and octaves, sustain 1.6 on sustaining presets, quantise and
snap explicit notes (`Key::snap`), rebuild sections without gaps or overlaps, in-key chords.
Every fix is recorded as `Repair {code, path, action}`; `fix_plan` is idempotent. Provenance
keys: `planner`, `provider`, `attempts`, `repairs`, `repair_codes`, `seed`, `planner_notes`.

### Arranger guarantees

One pattern per layer (`<layer>_motif`); clips are the contiguous runs of the sections a layer
plays in, phased to the global bar grid (`pattern_offset_steps`), so a later section replays the
identical pattern. Sustaining presets get note lengths above 1.0 step and a small room reverb
(`decay_s` 1.8, `size` 0.35, `predelay_ms` 12, `mix` 0.18); kicks get an EQ high-pass at 30 Hz
and, when soft, a low-pass at 2.5 kHz; hats and shakers a 300 Hz high-pass. Master defaults:
-16 LUFS, -1 dBTP. Banned layers are never generated. The Score passes `validate::validate` with
zero errors and is a pure function of (plan, seed).

### Catalogue

`Catalogue::standard()` mirrors docs/contracts/instruments.md (the planner cannot depend on
praxis-instruments); `Catalogue::to_json()` publishes it. Classes of the presets the instruments
contract lists as "various" are the planner's reading of their names: soft_keys chords, pluck
arp, mallet / glass / bell motif, sub_bass and warm_bass bass, fm_keys melody, granular_texture
pad, noise_bed texture (unpitched).

### Providers

- `RuleProvider` (`offline-rules`) parses the brief in the first user message. Layering rule:
  once a layer enters it plays to the end unless a later phrase removes it ("drop", "remove",
  "without", "only in bars ...").
- `ClaudeProvider` (`claude:<model>`): `POST /v1/messages`, `anthropic-version: 2026-10-21`,
  one tool `plan_score` with `strict: true` and the schema rewritten for strict mode
  (`additionalProperties: false`, no numeric, length or format keywords). Claude Opus 5.5 (the
  default), Sonnet 5.5 and Fable/Mythos 5.1+ reject a forced `tool_choice` with a 400, so for
  them the request sends `tool_choice: auto`, names the tool in the system prompt, checks that a
  call was made and asks once more if not; older models get
  `{"type": "tool", "name": "plan_score"}`. `output_config.effort` defaults to `medium`. On
  models that support it, `fallbacks: "default"` (beta `server-side-fallback-2026-07-01`) re-runs
  a refused request on a fallback model.

### For agents

An agent that is itself a language model can skip the brief and write the plan: read
`plan_schema()` and `Catalogue::to_json()`, write a ScorePlan, and call
`compose_plan(plan_json, catalogue, seed)`. Start from
`crates/praxis-planner/tests/prompts/acceptance1.plan.json`. Keep it small: one emotion word,
one to three layers, a spoken rhythm per layer (DON strong, din soft, don normal, TIN bright, `-`
rest, `~` tie) on a grid that divides the bar, scale degrees rather than MIDI pitches, and
sections that list which layer ids play (add layers in later sections; never rewrite earlier
ones). Give `bars` plus `duration_s` or `tempo_bpm`. Add no risers, sweeps, impacts, booms, pads,
whooshes or fills unless the user asked for one, and then list it in `requested`. `compose_plan`
repairs what it can deterministically and lists each change in `repairs`; if it returns
`PlanError::Invalid`, each issue's `path` points into your plan and its message lists the allowed
values, so fix those fields and call again.
