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
- `claude::ClaudeProvider`: Claude Messages API with one tool call (`tool_choice: auto` plus a
  check on models that reject forced tool use, such as claude-opus-5-5); model id configurable,
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
| `form[]` | Scoring depth (WP-P3). `{role, name?, weight (default 1), bars?, intensity?, layers[], transition auto/none/pickup/drop/hold}`, at most 12 parts. With a form, `sections` is derived (see "Scoring depth") and any given sections are ignored |
| `hits[]` | `{time_s, label?}`: hit points in seconds from the start, at most 8, at least 1 s apart, inside the piece (0.5 s before its end) |
| `progression` | `{chords [symbols in the key or Roman numerals], bars_per_chord 1..8}`: 1-16 chords that repeat from each section's start; borrowed chords (`bVII`) are rejected because Score v1 keeps every pitched note in the key |
| `tempo_range` | `{min, max}` BPM (40-220): where hit points may move the tempo; without it, within 4% of the resolved tempo |

### Issue codes and repairs

`validate_plan` returns `PlanIssue {code, path, message}`: `path` is a JSON pointer into the plan
and the message names the allowed values (preset names, the scale's pitch classes, the grids that
divide the bar, the spoken tokens). Codes: `schema`, `timing`, `time_signature`, `bars`,
`emotion`, `bad_id`, `duplicate_id`, `no_layers`, `too_many_layers`, `unknown_preset`,
`preset_kind`, `class_mismatch`, `avoided_class`, `unrequested_class`, `grid`, `motif_length`,
`spoken`, `degrees`, `octave`, `sustain`, `off_scale`, `off_grid`, `note_range`, `velocity`,
`gain`, `sections`, `unknown_layer`, `inactive_layer`, `intensity`, `chord`, and for scoring
depth `form`, `hit`, `progression` (plus `timing` for a bad `tempo_range`). Quality codes
(`PlanIssueCode::is_quality`; never produced by `validate_plan`, never a reason to reject a plan):
`unrequested_layer`, `layer_bars`, `isolated_notes`, `no_motif_repetition`,
`missing_sustain_overlap`, `too_many_elements`, `unrequested_effect_class`.

The loop: provider call -> parse (field-level `schema` issues with pointers) -> validate -> on
issues, the issues go back to the provider in a user message (at most 3 repair rounds; a provider
that returns the same plan twice is not asked again) -> deterministic `fix_plan`: solve or default
the timing, reduce the emotion to one word, a catalogue preset for unknown ones, the preset's
class wins for drums, effects, pads and textures, drop banned and unrequested layers, a grid that
fits the bar, clamp lengths, degrees and octaves, sustain 1.6 on sustaining presets, quantise and
snap explicit notes (`Key::snap`), rebuild sections without gaps or overlaps, in-key chords.
Every fix is recorded as `Repair {code, path, action}`; `fix_plan` is idempotent. Provenance
keys: `planner`, `provider`, `attempts`, `repairs`, `repair_codes`, `seed`, `planner_notes`.

**Quality rounds (model providers only, WP-P4).** For a provider whose id is not
`offline-rules`, `plan_brief` runs `repair::plan_with_quality_repair`: every plan that passes
`validate_plan` is also checked by `brief::quality_issues`, and its issues go back to the model in
the same repair rounds (the same limit of 3, the same message format: pointer, code, what is
wrong and what to do). The checks:
- `unrequested_layer` (`/layers/<i>`): a layer whose instrument the brief does not name. The
  brief's instruments are read with the rule planner's vocabulary (`offline::brief_instruments`:
  instrument families, class words such as "bass" or "melody", drum-kit words such as "drums" or
  "beat" covering every drum, and catalogue-less instruments such as "strings"). Judged only when
  the brief names an instrument and the plan covers at least one named instrument, so a plan made
  wholly of substitutes is a reading, not an extra.
- `layer_bars` (`/sections`): a layer that does not play every bar when the brief has no bar
  phrase ("bars 9-16", "second half", "from bar 5", "last 4 bars") and no timing word ("intro",
  "outro", "enters", "joins", "later", "then", "drop", ...).
- The lints of the arranged Score (`praxis_score::lint`) with the layer mapped back to the plan:
  `isolated_notes` and `no_motif_repetition` (`/layers/<i>/motif`), `missing_sustain_overlap`
  (`/layers/<i>/motif/sustain_steps`), `too_many_elements` (`/layers`),
  `unrequested_effect_class` (`/layers/<i>`), each with the lint's message plus a plain
  instruction (for `isolated_notes`: keep the layer on a repeating grid, add hits or `~` ties and
  let sustain connect each note to the next).

A plan with only quality issues is valid: when the rounds run out, or the model answers with the
same plan again, the valid plan with the fewest quality issues is kept (the latest on a tie) and
its lints stay in `PlanOutcome.lint`; the deterministic `fix_plan` runs only when no answer was
valid, then `finish` lengthens sustain as before. `rounds` lists quality issues like any other.
The offline provider is never asked again, so its plans and the offline suites are unchanged.

### Arranger guarantees

One pattern per layer (`<layer>_motif`); clips are the contiguous runs of the sections a layer
plays in, phased to the global bar grid (`pattern_offset_steps`), so a later section replays the
identical pattern. Sustaining presets get note lengths above 1.0 step and a small room reverb
(`decay_s` 1.8, `size` 0.35, `predelay_ms` 12, `mix` 0.18); kicks get an EQ high-pass at 30 Hz
and, when soft, a low-pass at 2.5 kHz; hats and shakers a 300 Hz high-pass. Master defaults:
-16 LUFS, -1 dBTP. Banned layers are never generated. The Score passes `validate::validate` with
zero errors and is a pure function of (plan, seed). Scored plans (below) keep these guarantees
except "one pattern per layer": there a layer also has its sparse and lifted forms and one-bar
transition and hit patterns.

### Scoring depth (WP-P3, FR-9 and FR-10)

Code: `crates/praxis-planner/src/scoring.rs` (`realise`, `validate`, `fix`, `summary`),
`scoring/emotion.rs` (presets), `scoring/harmony.rs` (progressions, voice-leading),
`scoring/form.rs` (form solver), `scoring/hits.rs` (tempo solver, landings),
`scoring/develop.rs` (the arranger's scored path), `scoring/words.rs` (what the rule planner reads).

A plan is **scored** when it has a `form`, `hits` or a `progression`
(`ScorePlan::is_scored`); every other plan arranges exactly as before. `scoring::realise(plan,
seed)` runs first (in `finish`, and in `arrange` itself, so `quality_issues` sees the real
Score) and turns the scoring fields into ordinary ones; it is idempotent, and the
`PlanOutcome.plan` is the realised plan, which arranges to the same Score again:

1. **Hits choose the tempo.** Every tempo in `tempo_range` (or the resolved tempo +-4%) is tried
   in 0.01 BPM steps. Bars are fixed when the plan gives no `duration_s`; with `duration_s` they
   are the whole bars nearest it. A candidate is skipped when a hit falls within half a beat (at
   least 0.5 s) of the end. The best has, in order: the fewest hits more than 12 ms from a
   sixteenth step (sixteenth-triplet when the first layer's grid is a triplet), the strongest
   metric positions (bar line 0, beat 1, half beat 2, other step 3, summed), the smallest largest
   distance in whole ms, the tempo nearest the resolved one. The plan then has `tempo_bpm`,
   `bars`, no `duration_s`, `tempo_range` pinned to the tempo, and a note saying so.
2. **The form becomes sections.** Parts share the bars by weight (fixed `bars` first), in phrases
   of 4 bars from 32 bars, 2 from 12, else 1, by largest remainder; each part gets at least one
   bar. Each hit asks for a section to start at its bar (the next bar when it falls in the second
   half of its bar); an exhaustive search assigns as many hits as possible to part boundaries in
   order, then moves them least, and the parts between are re-spaced by weight. A part's
   intensity is its own or its role's usual level (intro 0.3, verse 0.45, main 0.55, build 0.6,
   peak 0.85, breakdown 0.35, release 0.4, transition 0.5, outro 0.3). A part without `layers`
   gets the first `1 + round((n-1) (x - min) / (max - min))` plan layers, with `x` its intensity
   and min/max the form's quietest and loudest (layering: the first layer alone at the quietest,
   all at the loudest). Section names are the part's name, else the role's (numbered when
   repeated).
3. **Harmony.** A `progression` becomes `chords`, restarting at every section start with
   `bars_per_chord` bars per chord; a one-shot's last bar is the tonic chord. A form with no
   progression and no chords gets one of its emotion preset's progressions (chosen by seed), with
   a note.

The arranger's scored path (`scoring::develop`):
- **Chords** (chords, harmony and pad layers following chords): one close-position triad per bar,
  each chosen to move the voices least from the previous one (the pattern's wrap included),
  around the layer's octave.
- **Bass** (bass layers following chords): when the next bar's root is a third or more away, the
  last beat plays the scale step next to it, approached from the current root's side; the note
  before stops there.
- **Density by intensity**: below 0.4 a layer plays its sparse form (`<layer>_sparse`: the notes
  on beats and the strong and bright ones, sustaining notes ringing to the next kept onset; only
  when that leaves no more than a beat of silence); up to 0.75 the motif as written; above it
  melodic layers (motif, melody, arp) play the lifted form (`<layer>_lifted`: strong and bright
  notes doubled an octave up, in range, softer). Every form keeps the motif's pitches and accents.
- **Dynamics**: each bar's clip `velocity_scale` is 0.8 + 0.3 x intensity (0.6-1.1, steps of
  0.05); a build ramps bar by bar to the next part's intensity. Gain lanes are not used, so a
  later `change_dynamics` edit still works.
- **Filter**: pitched, sustaining layers other than bass and drone get a `tension` low-pass first
  in their chain (resonance 0) and a smooth lane on `effects.tension.cutoff_hz`: 1800 x 2^(3x) Hz
  (1.2-16 kHz) at each section start, held to half a bar before the next (a build ramps to it).
- **Transitions** into section k (the last bar of the section before, never a bar with a hit and
  never when that section is one bar long): `drop` keeps only the lead layer (the first plan layer
  playing); `hold` turns pitched layers into one held chord (their first onset of the bar, the
  whole bar long), drums rest; `pickup` lets a layer entering at k play the last beat of its part
  in that bar. `auto`: drop into a peak after a part with two or more layers, hold into a release
  or outro that is quieter than the part before, pickup into a build, main or verse where a layer
  enters, else none. Never a riser.
- **Hits**: in each hit's bar the anchor layer (chords, else motif or melody, else arp, else bass;
  playing there, else the first pitched layer) gets a one-bar pattern `<layer>_hit<k>` on the hit
  grid: its notes of that bar, less any within a step of the hit, plus a strong chord of the bar's
  harmony (velocity 1, two beats long on sustaining presets) on the hit's step with `offset` (the
  only off-grid notes a planner Score has; at most half a step, usually under 12 ms) so its onset
  is exactly the hit. A playing bass adds its root and a playing kick (or kit) a kick. The Score
  gets a `hit` marker per hit (`hit-<k>`, the hit's time, its label or "Hit k").

**Emotion presets** (`scoring::emotion::PRESETS`): calm, tense, hopeful, melancholy, playful,
dark, warm and epic-but-restrained, each with synonyms, a default key, a tempo range and usual
tempo, the lead layer's register, instruments in entry order, a default form (role, weight,
intensity per part), three progressions and bars per chord. The system prompt lists them.

**The rule planner** reads a brief as scored when it has hit timecodes after "hit(s)", "hit
points" or "markers" ("hits at 0:42, 1:05 and 1:38", "at 30 s and 52.5 seconds"; read and blanked
before anything else, so they never count as the length), two or more different form words
(intro, build, peak/climax, release, outro/ending, verse, chorus, bridge, breakdown, in order),
an A/B form (A/B, A/B/A, ABAB, AABA), or "cue", "evolving", "progression", "tension curve",
"score for", "underscore". One form word alone ("an 8-bar build") is not a form. Then: the emotion
preset of the mood word (an otherwise unknown preset synonym such as "epic" selects its preset);
its key when the brief gives none, its usual tempo when the brief gives none (and its tempo range
for the hits); its instruments when the brief names none; the brief's form words, else the
preset's form (unless the brief places its layers in bars itself); the brief's chords or Roman
numerals after "progression", else a preset progression; pitched layers follow the chords, the
motif layer with a two-bar chord-relative motif (`motifs::SCORED_MOTIF`), the lead layer in the
preset's register. "Borrowed chords" adds a note that the progression stays in the key.

**Model providers**: rules 10-13 of the system prompt describe `form`, `hits`, `progression`,
`tempo_range` and the presets. A plan with a form is exempt from the `layer_bars` quality check
(the form says when layers enter).

**The summary** (`PlanOutcome::summary`) adds "Form: intro (bars 1-8), ..." and "Hits land at
0:42 (bar 18), ...".

**Contract gap**: borrowed chords (bVII, iv in major) need notes outside the key, which Score v1's
`off_scale` check rejects; supporting them needs a per-chord scale or an allowed-chromatic flag on
`ChordSpan` (not done).

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
  one tool `plan_score` (not `strict`: the plan schemas are too large for the API's strict grammar, a live 400 found 2026-11-11; `validate_plan` and the repair loop keep answers correct) with the schema rewritten as for strict mode
  (`additionalProperties: false`, no numeric, length or format keywords). Claude Opus 5.5 (the
  default), Sonnet 5.5 and Fable/Mythos 5.1+ reject a forced `tool_choice` with a 400, so for
  them the request sends `tool_choice: auto`, names the tool in the system prompt, checks that a
  call was made and asks once more if not; older models get
  `{"type": "tool", "name": "plan_score"}`. `output_config.effort` defaults to `medium`. On
  models that support it, `fallbacks: "default"` (beta `server-side-fallback-2026-07-01`) re-runs
  a refused request on a fallback model. The system prompt is sent as one text block; it and the
  tool definition carry `cache_control: {"type": "ephemeral"}`, so the schema (the same in every
  request) and the system prompt (the same for every brief, resent in every repair round) are
  read from the prompt cache. Each 200 response's `usage` is added to `ClaudeProvider::usage()`
  (`claude::Usage`: requests, input, output, cache write and cache read tokens; `cost_usd(model)`
  at list price, `since(earlier)` for one call's share); the live suites print it per case.

### System prompt (model providers)

`brief::system_prompt` states the owner's rules as numbered plain sentences: rhythm first (one
repeating motif per layer, no note followed by more than a beat of silence: a slow part holds
notes with `~` ties or sustain; a spoken rhythm in the brief is copied exactly, not padded with
rests), notes connect, layering by sections, restraint (only the instruments the brief names, one
layer per named instrument, no layer the brief did not ask for, one layer when the brief names
one instrument, one to three when it names none, no trailer classes unless asked, mood words are
not a licence for effects), every layer from bar 1 to the end unless the brief says when, the
brief's own mood word as the emotion, always in key, the duration rule, and the catalogue (an
instrument it lacks goes in `notes`, never replaced by other layers).

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

### Prompt suites

`tests/prompt_suite.rs` (every brief in `tests/prompts/*.json`) and `tests/edit_prompt_suite.rs`
(every `edit_*.json`) run offline on `RuleProvider`; with `PRAXIS_SUITE_PROVIDER=claude` they run
on one shared `ClaudeProvider` (paid) and print each case's token usage and cost and the totals.
Live runs report repair rounds instead of failing on them and skip the determinism check; every
other check is the same. Checks that read a model's words fairly without loosening anything else
(`tests/common/mod.rs`): an expected emotion accepts the close synonyms listed in
`EMOTION_SYNONYMS` ("ominous" for dark, "suspenseful" for tense, ...); an expected spoken rhythm
is compared after reducing an exact repetition to one period and dropping trailing rests
(`spoken_core`; hits, accents, ties and inner rests must match); an edit case may give
`expect_any`, a list of acceptable outcomes. Layer counts, presets, bars, keys, tempo, grid,
banned classes and isolation are compared exactly. `edit_not_understood` ("make it pop") accepts
`not_understood` (nothing changes) or a whole-score `change_tone` brighter that keeps every note
and clip, the one reading of "pop" that is safe and that the edit tools can express.
`PRAXIS_SUITE_FILTER=<text>` runs only the brief cases whose file name contains it (a live run of
just the `scoring_` cases). Scoring cases add `check_scoring` expectations: `section_roles`,
`section_names`, `hits`, `hits_within_ms`, `duration_within`, `bpm_range`,
`layers_enter_in_order`, `voice_leading_max`, `progression_start`, `bass_passing_tones`,
`drop_before`, `hold_before`. The on-grid check allows a nudge (`offset`) only on the strong
accents of `_hit` patterns.

## Edits (WP-P2)

Code: `crates/praxis-planner/src/edit.rs` (entry points, repair loop, summary),
`edit/plan.rs` (`EditPlan`, `edit_schema()`, issue codes), `edit/scope.rs` (bar phrases),
`edit/resolve.rs` (plan -> exact bars and layers), `edit/build.rs` (resolved plan ->
`ScorePatch`), `edit/material.rs` (new material through the arranger), `edit/rules.rs`
(offline reader), `variations.rs`. The tool is named `edit_score`; its input schema is
`praxis_planner::edit_schema()` (schemars).

Entry points:
- `plan_edit(score, instruction, provider, catalogue, seed) -> Result<EditOutcome, EditError>`;
  `plan_project_edit(&mut project, ...)` does the same and commits the patch (undoable).
- `compose_edit(edit_plan_json, score, catalogue, seed)`: an EditPlan written directly, no
  provider, no repair rounds.
- `variations(score, scope, n, seed) -> Result<Vec<Variation>, EditError>` (standard catalogue)
  and `variations_with(..., catalogue)`.

`EditOutcome {patch, inverse, score, scope, bars, summary, refused, new_layers, plan, attempts,
rounds, repairs, provider_id}`; `changed()` is false when the patch has no ops; `understood()` is
false for a `not_understood` plan (empty patch, unchanged Score, a summary that starts "Nothing
changed:" and lists what edits are possible, `edit::EDIT_CAPABILITIES`). `bars` is always the bars
the edit was allowed to change, also when the patch itself has no bar scope (tone and space).
`Variation {index, seed, patch, inverse, score, summary}`.

### EditPlan fields

| Field | Meaning |
| --- | --- |
| `intent` | `add_layer`, `add_melody`, `rewrite_motif`, `change_instrument`, `change_dynamics`, `remove_layer`, `thin_out`, `change_tone`, `change_space`, `not_understood` |
| `scope.bars` | Explicit `{start, end}` (1-based, inclusive); wins over `phrase` |
| `scope.phrase` | Resolved against the Score: "last N bars", "first N bars", "first half", "second half" (the half is `bars / 2`, rounded down), "bars A-B", "bar N", "from bar N", "until bar N", "whole" |
| `scope.keep` | Bars the instruction keeps, as a phrase. Alone, the scope is the bars outside it (one range); with a phrase that overlaps it, the scope is narrowed (never widened) and the summary says so |
| `layers[]` | Existing layers by id, name, preset or a word of it, class, or role ("drums", "melody"). A reference equal to a layer id (ignoring case, spaces and dashes) selects only that layer (`resolve::layers_matching`), so `piano` never also reaches an `upright_piano` layer. Empty for `add_*`; for `rewrite_motif`, `change_dynamics`, `thin_out` and `change_tone` empty means every unlocked layer that plays in the bars; for `change_space` it means the layers playing there that have a reverb, plus (for `wetter`) the pitched, sustaining, non-drum ones |
| `params` | `preset`, `class`, `name`, `register`, `dynamics`, `gain_db` (-60..=24), `change` (softer/louder), `amount_db` (0 < x <= 24; default 4 softer, 3 louder), `grid`, `spoken`, `degrees`, `length_bars` (1..=8), `tone` (`darker`/`brighter`, required for `change_tone`), `space` (`wetter`/`drier`/`dry`, required for `change_space`), `amount` (`a_little`/`normal`/`much`, default `normal`) |
| `notes[]` | Remarks |

### Patch rules

The patch has `origin: model`, `scope.bars` = the resolved bars (unset only when they are the
whole score) and `scope.layers` = the target layers plus any new layer; it is applied with
`praxis_score::apply::apply`, so locks, scope and isolation are enforced there too.
- `add_layer` / `add_melody`: `add_pattern` + `add_layer` with one clip over the bars, phased
  to the global bar grid. Material is a one-layer ScorePlan in the Score's key, metre, tempo and
  chords run through `validate_plan`, `fix_plan` and the arranger, so it has accents, sustain
  overlap and the arranger's effects. `add_melody` defaults: `upright_piano`, class melody, high
  register, a two-bar spoken melody template following the chords; "subtle" means soft dynamics
  and -6 dB.
- `remove_layer`: whole score -> `remove_layer` (+ `remove_pattern` of patterns only it played);
  bars -> `clear_bars`.
- `change_dynamics`: whole score -> `update_layer {gain_db}`; bars -> `place_clip` of the same
  pattern and phase with `velocity_scale` times `10^(dB/20)` (clamped to 0.05..=2).
- `thin_out`: a new pattern keeping accented (strong, bright) notes and beat steps (or the
  downbeats if that removes nothing); sustaining presets ring into the next kept note.
- `rewrite_motif`: a new pattern from the layer's rhythm with one to three steps changed (bar
  downbeats kept) and new chord-relative degrees, in the layer's instrument, octave, register and
  dynamics; placed with `place_clip` over the bars.
- `change_instrument`: whole score -> `update_layer {instrument, class?, name?}` (+ `set_clips`
  octave transposition into the preset's range); bars -> `clear_bars` on the layer and a new
  layer `<layer>_<preset>` that plays the same patterns and phase in those bars. Drum <-> pitched
  swaps are refused.
- `change_tone` (notes, patterns and clips never change): whole score -> one `update_layer` per
  target. In order: a darker edit first lowers an `air` EQ an earlier brighter edit added (a
  brighter edit first opens or removes an un-automated `tone_lp` low-pass); otherwise the
  preset's tone controls move (`catalogue::ToneControls`: `tone` on every pitched synth, the
  sampled pianos and the drums, plus `felt` on `felt_piano`) by 0.15 / 0.3 / 0.45 for
  a_little / normal / much, `tone` down and `felt` up for darker (the reverse for brighter),
  clamped to 0..=1; at the end of their range, or on a preset without them (the sound effects),
  darker adds or lowers a `tone_lp` filter (low-pass, resonance 0; new at 6000 / 3000 / 1500 Hz,
  existing x0.75 / x0.5 / x0.33, never below 250 Hz) and brighter adds or raises an `air` EQ
  high shelf at 5 kHz (+2 / +4 / +6 dB, at most +12). "Make the piano muffled" on the felt
  piano: `tone` 0.5 -> 0.2, `felt` 0.7 -> 1.0. Bars -> `apply` refuses `update_layer` sound
  changes in a bar-scoped patch, so the patch has no `scope.bars` (only `scope.layers`) and its
  one `update_layer` inserts a filter `tone_b<start>_<end>` first in the chain at `mix` 0 (a
  low-pass at 3000 / 1500 / 800 Hz for darker, a high-pass tilt at 800 Hz for brighter) plus a
  `step` lane on `effects.<id>.mix` that is 0 outside the bars and 1 (darker) or 0.35 / 0.6 /
  0.85 (brighter) inside them. A filter at `mix` 0 passes its input through exactly, so the
  layer, and with a frozen master gain the master, is bit-identical outside the window of
  contracts/patch.md. A second edit of the same bars deepens that filter, or removes it when it
  went the other way.
- `change_space` (notes never change): whole score -> `update_layer {effects}`. `wetter` adds
  the arranger's room reverb `room` (decay 1.8 s, predelay 12 ms, size 0.35, mix 0.18; for
  much 2.6 s, size 0.6, mix 0.26) when the layer has none, otherwise raises each reverb's `mix`
  by 0.06 / 0.12 / 0.2 (at most 0.6), `decay_s` by x(1 + 2.5 step) (at most 8 s) and `size` by
  the step; `drier` scales `mix` by 0.75 / 0.55 / 0.3 and shortens decay and size; `dry`
  removes every reverb (and lanes on them). Bars -> no bar scope, as for tone: a `step` lane on
  each reverb's `mix` holding its own value outside the bars (`dry` is 0 inside); with no
  reverb, `wetter` adds `room` at `mix` 0 and the lane sets it inside the bars. A layer with no
  reverb to reduce is a no-change outcome that says so.
- `not_understood`: no patch; see `understood()` above. `rewrite_motif` is never a fallback.

Bar edits never mutate an existing pattern.

### Locks, repair and widening

Locked layers are never targets. A locked layer the plan names is listed in `refused` and the
summary says it was left alone because it is locked; if nothing else is left to change, the
outcome has an empty patch and the unchanged Score (not an error).

Issue codes (`EditIssue {code, path, message}`, path a JSON pointer into the plan): `schema`,
`scope`, `keep`, `unknown_layer`, `no_target`, `unknown_preset`, `avoided_class`, `material`,
`rejected` (apply refused the patch), `widened`. In `plan_edit` issues go back to the provider
(at most 3 repair rounds; a provider that returns the same plan twice is not asked again). The
bars of the first plan whose scope resolved, and the target layers of the first plan that
resolved completely, bind every later round: a repaired plan outside them gets `widened`.

### Providers

`RuleProvider` answers only `plan_score`, so `plan_edit` routes requests for a provider whose id
is `offline-rules` to `edit::EditRules` (`parse_edit`). It reads clauses split at punctuation
and "but/then/while", bar phrases, keep clauses ("keep", "leave", "don't touch"), and intents in
this order: add (+ melody words, or nothing named) -> `add_melody`; add -> `add_layer`; remove,
drop, without, mute -> `remove_layer`; swap, replace, instead, change X to Y -> `change_instrument`;
thin, sparser, simpler -> `thin_out`; softer/louder words -> `change_dynamics` ("a little" 2 dB,
"much" 8/6 dB); new-notes words (rewrite, vary, variation, different, new, rework, "change /
redo the melody, motif, rhythm, notes, pattern ...", a quoted spoken rhythm, a grid word with a
named layer) -> `rewrite_motif`; otherwise `not_understood` (never a rewrite). Two readings come
before all of these, unless the instruction adds a named instrument or a melody ("add a bell with
lots of reverb" stays `add_layer`): space -> `change_space` (a reverb noun: reverb, room, space,
ambience, echo, with no / without / remove / cut / off or "dry" -> `dry`, less / reduce / lower /
down, "too much" or a drier word -> `drier`, otherwise `wetter`; with no reverb noun: dry ->
`dry`; drier, closer, intimate, up front, "too wet" -> `drier`; wetter, bigger, hall, cathedral,
spacious, distant, far away -> `wetter`); then tone ->
`change_tone` (muffled, muted, darker, warmer, warmth, duller, mellow, rounder, "a softer tone",
harsh, brittle, shrill, tinny, "cut the highs" -> `darker`; brighter, clearer, crisper, airy,
"more air / treble", muddy, murky, boomy -> `brighter`; "less" or "not" up to two words before a
quality flips it; "tone down" stays dynamics). Amounts: little, slightly, bit, touch, tad ->
`a_little`; much, lot, very, really, completely, hall -> `much`. Layers a bar-less keep clause
names ("keep the piano but make it sound muffled") are the targets of a tone or space edit that
names none.

The app's Feel sliders (`app/src/ui/views/overview.ts` `MACROS`) send fixed phrases; each maps
to one intent (tested in `tests/edit_tone_space.rs` and `edit/rules.rs`):

| Slider | Down | Up |
| --- | --- | --- |
| Mood | "Make the mood calmer" -> `change_dynamics` softer | "Make the mood more tense" -> `change_dynamics` louder |
| Brightness | "Make it a little darker" -> `change_tone` darker, a_little | "Make it a little brighter" -> `change_tone` brighter, a_little |
| Space | "Make it drier" -> `change_space` drier | "Give it more room" -> `change_space` wetter |
| Energy | "Calmer, less energy" -> `change_dynamics` softer | "More energy" -> `change_dynamics` louder |

Mood and Energy have no intent of their own; the nearest existing one that keeps the notes is
dynamics (every unlocked layer, default 4 dB softer / 3 dB louder). Rewriting or thinning notes
for a slider nudge would break "never rewrite a part unless asked". Model providers get the same forced-tool request with
a system prompt that lists the catalogue, the presets with tone controls, the restraint rule
(change only what the user names; one added layer per instrument asked for, none they did not
ask for, playing through every bar of the scope), the tone / space / not_understood rules and
the Score (layers, bars they play, locks);
`tests/edit_claude_replay.rs` replays `ClaudeProvider` through a repair round, a widened
answer, a `change_tone` answer and a `not_understood` answer.

### Variations

`variations(score, scope, n, seed)`: `scope` is a `praxis_score::patch::Scope` (bars unset = the
whole score; empty layers = every unlocked layer playing in the bars). Variation `i` tries seeds
`derive_seed(seed, "variation{i}", k)` for k = 0..16 and keeps the first whose selected notes
differ from the original and from every earlier variation; it fails with `material` when it
cannot find N distinct ones, and with `no_target` when every selected layer is locked.
Whole-score variations also set each varied layer's `seed_offset`; bar-scoped ones cannot (a
`seed_offset` changes every bar, which `apply` refuses in a bar scope).

### For agents (edits)

Read `edit_schema()`, write an EditPlan and call `compose_edit(plan_json, score, catalogue,
seed)`. Start from `crates/praxis-planner/tests/prompts/edit_last_four_melody.plan.json`. Name
bars with `scope.phrase` or `scope.bars`, put what must stay in `scope.keep`, name existing
layers by id, and prefer adding a layer in the bars to rewriting a part (layering). Check
`refused` and `summary`: a locked layer is never changed. On `EditError::Invalid` fix the fields
the issue paths point at and call again; never widen the bars to make an issue go away.
