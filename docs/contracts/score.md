# Contract: Score v1

Code: `crates/praxis-score/src/types.rs` (types), `params.rs` (effect parameters),
`validate.rs` (issue codes), `timeline.rs` (time mapping). Published schema:
`schema/score.schema.json` (regenerate with `praxis schema --write`; CI fails if stale).
Only the orchestrator changes these files.

## Units and conventions

| Thing | Convention |
| --- | --- |
| Bars | 1-based; `BarRange {start, end}` is inclusive. `bars 9-16` = `{9, 16}` |
| Tempo | Constant quarter-note BPM, 20..=400. Tempo maps are v2 |
| Pitch | MIDI note number (60 = C4). Drum layers use the GM drum map (36 kick, 38 snare, 42 closed hat, ...) |
| Steps | `Note.step` is 0-based within the pattern; `length` in steps, >0, may exceed 1.0 (overlap, FR-4) |
| Offset | `Note.offset` -0.5..=0.5 of a step (swing/humanise) |
| Velocity | (0, 1]; accents multiply it: strong x1.25, bright x1.1, soft x0.7 (clamped to 1) |
| Brightness | `bright` accent (TIN) also sets `NoteOn.brightness = 1`, which instruments map to tone |
| Time | Seconds from the start of bar 1 (`Marker.time_s`) |
| Automation | `AutoPoint.bar` is a fractional 1-based bar position: 1.0 is time zero |
| Maps | `BTreeMap` everywhere, so JSON field order is stable |

## Semantics

- **Patterns repeat.** A clip plays its pattern from the clip's first bar, looping every
  `length_steps`. `length_steps` must be a whole number of bars for the time signature.
- **Clips on one layer never overlap** (validator error `overlapping_clips`).
- **Layer class is the avoid-list key.** `constraints.avoid` lists generator classes that must not
  sound. The validator rejects such layers (`avoided_class`) and the engine also refuses to render
  them (it reports them in `RenderReport.blocked_layers`). A preset's intrinsic class
  (`PresetInfo.class`) is checked too, so a pad preset cannot hide in a "melody" layer.
- **Locked layers** (`locked: true`) are immutable to every patch except a user `set_lock` unlock.
- **In key.** Pitched layers (any class except drums, percussion, tick) must use only notes of
  `key` after transpose (`off_scale` error). Mode `chromatic` disables the check (sound design).
- **Seeds.** Score `seed` plus layer `seed_offset` plus the note's stable key seed each note.
  Variations (FR-8) change `seed_offset`, never the note keys.
- **Loudness.** `master.loudness` sets the target (default -16 LUFS integrated, -1 dBTP ceiling).
  `master.frozen_gain_db`, when present, is applied instead of re-measuring; this keeps scoped edits
  from changing the level of untouched bars. Normalising again is an explicit action.
- **Playback.** `loop`: the render is exactly `bars` long and tails wrap into the start.
  `one_shot`: bars plus `tail_seconds`.

## Validation codes

`validate()` returns `{errors, warnings}`; each issue has `code`, `path` (JSON pointer) and
`message`. Codes in `IssueCode` never change meaning; new ones may be added. Quality lints
(`isolated_notes`, `no_motif_repetition`, `missing_sustain_overlap`, `too_many_elements`,
`unrequested_effect_class`) are warnings produced by `lint::lint` (WP-A) and drive the planner's
repair loop.

## Versioning

`schema_version` is 1. Readers reject other versions. A v2 adds a migration function
`migrate(value) -> Score` and keeps v1 files loading.
