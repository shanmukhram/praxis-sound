---
name: praxis-sound
description: Make music loops, short scores, stings and tuned sound effects as audio files with Praxis Sound, and check them by measurement instead of listening. Use when a task needs background music, a seamless loop, a sting or logo sound, a tonal bed, a sound effect that sits in key under music, or stems/MIDI of any of these; or when you must check a render's tempo, key, grid, loudness, peak or loop seam. Works through the praxis-mcp MCP server (praxis_* tools) or the `praxis --json` CLI.
---

# Praxis Sound for agents

Praxis Sound turns a plan into layered, editable audio. Everything is a Score (key, tempo, bars,
sections, layers, patterns); the engine renders only the Score, so any result can be changed bar
by bar and layer by layer. You cannot listen, and you do not need to: every render returns
measurements and pass/fail checks against the Score.

You are the planner. Write the plan yourself (a ScorePlan) and Praxis validates it, repairs what
it can deterministically, arranges it and renders it. No API key is needed for that path.

## When to use it

- Background music, a seamless loop, a tonal bed, a short score for a clip.
- A sting, logo sound or a one-shot that resolves.
- A sound effect that sits under music and must be in its key.
- Stems or MIDI of any of the above, for someone else to mix.
- Checking an existing render against its Score (tempo, key, grid, loudness, peak, seam).

Not for: speech, sung vocals, or copying a reference track.

## Setup

1. Build the server: `cargo build --release -p praxis-mcp` (gives
   `target/release/praxis-mcp.exe`).
2. Register it with Claude Code: copy `docs/agents/mcp.json.example` into the project's
   `.mcp.json` (fix the paths), or run
   `claude mcp add praxis --env PRAXIS_PACKS=<repo>/packs -- <repo>/target/release/praxis-mcp.exe`.
   `PRAXIS_PACKS` points at the instrument packs (the felt and upright pianos need
   `upright-piano-kw`). `ANTHROPIC_API_KEY` is optional and only used by `praxis_generate`.
3. Install this guide as a skill: copy the `praxis-skill` folder to
   `~/.claude/skills/praxis-sound/` (or `.claude/skills/praxis-sound/` in a project).

## Tools

With the MCP server (`praxis-mcp`):

| Tool | Use |
| --- | --- |
| `praxis_plan_schema` | The ScorePlan schema, a how-to, the spoken-rhythm tokens and a full example. Call first. |
| `praxis_presets` | Preset names a plan may use and whether each renders on this machine (`available`). |
| `praxis_compose` | Your plan -> `.praxis` project; with `render`, also the WAV and its checks. |
| `praxis_generate` | A plain-language brief -> project (offline rule planner, or Claude with a key). |
| `praxis_render` | Project or score -> WAV/FLAC (+ stems), with checks. |
| `praxis_check` | Measure an existing render against its project. |
| `praxis_analyze` | Measure any audio or video clip (tempo, key, loudness, onsets, seam). |
| `praxis_export` | WAV 16/24/32f, FLAC 16/24, stems, or MIDI. |
| `praxis_validate` | Validate a `.score.json` or a `.praxis` project's score. |
| `praxis_edit` | Scoped edit of a project: an instruction (`prompt`) or an EditPlan you wrote (`plan`). |
| `praxis_variations` | N alternates of some bars and layers, each rendered and checked; nothing committed. |
| `praxis_ask` | Make a take in a sound's shared history: the brief on a new sound, then scoped edits. |
| `praxis_takes` | A sound's takes, the current one, the live score; `revision` grows on every change. |
| `praxis_restore` | Go back to take `n` (an undoable patch; no new take). |
| `praxis_keep_variation` | Keep variation `index` of a sound as a new take. |
| `praxis_open_view` | Show the person a panel view (overview, rhythm, notes, mix, export). |

Without MCP, the CLI does the same with `--json`: `praxis generate --plan plan.json -o
x.praxis --render x.wav --skip-missing --json`, `praxis check x.wav --score x.praxis --json`,
`praxis presets --json`, `praxis schema --json` (the `plan` field), `praxis export-midi`,
`praxis edit x.praxis --prompt "..."`, `praxis variations x.praxis --bars 13-16 --layer piano`,
`praxis ask <sound> "<text>" [--bars 13-16 --layer piano] [--who name]`, `praxis takes <sound>`,
`praxis restore <sound> <n>`, `praxis keep-variation <sound> <i>`, `praxis open-view <sound> notes`.

Every response is the CLI's JSON: `{"ok": true, "command": ..., ...payload}` or
`{"ok": false, "command": ..., "error": {"code", "message", "issues"?}}` (MCP sets `isError`).
Paths go in, paths and numbers come out; audio is never returned inline. Relative paths resolve
against the server's working directory and come back absolute.

## The plan-first workflow

1. `praxis_plan_schema` once per session. Start from its `example`.
2. `praxis_presets`. Use presets with `available: true` where you can; others are skipped at
   render time with `skip_missing: true` (and listed in `render.skipped_layers`).
3. Write the plan (rules below) and call `praxis_compose` with `plan`, `project` (a `.praxis`
   path) and `render` (a `.wav` path). Add `skip_missing: true` while some presets are not built.
4. Read `repairs` (what Praxis changed in your plan) and `render.check`.
5. If a check failed, fix the plan (table below) and compose again. Keep the `seed`. A check
   in `inconclusive[]` did not fail: leave it (see "Reading the result").
6. `praxis_export` for the deliverable: FLAC, 16-bit WAV, stems (`stems: "<dir>"`) or MIDI.
7. Tell the user what you made in measured terms: "16 bars, 30.0 s loop in A minor at 128 BPM;
   8 of 8 checks passed: in key, on the beat, -15.6 LUFS, peak -3.2 dBTP, seamless loop."

## Writing a plan

- **Timing.** Give `bars` plus `duration_s` or `tempo_bpm`; any two fix the third (16 bars in
  30 s = 128 BPM). Tempo 40-220 BPM.
- **One emotion word** (`"calm"`, `"tense"`, `"hopeful"`). Mood words are not a licence for
  trailer effects.
- **Key** `{"tonic": "A", "mode": "minor"}`. Modes: major, minor, harmonic_minor,
  melodic_minor, dorian, phrygian, lydian, mixolydian, locrian, major_pentatonic,
  minor_pentatonic, chromatic (sound design only; disables the key check).
- **`playback`**: `"loop"` (seamless; tails wrap into bar 1) or `"one_shot"` (rings out, 4 s
  tail).
- **Layers**: one to three (five at most). Each has `id`, `class` (melody, motif, harmony,
  chords, arp, bass, drums, percussion, ...), `preset {kind, name}`, `register` (low/mid/high),
  `dynamics` (soft/normal/strong), optional `gain_db`, and a `motif`.
- **Motif** = rhythm first, then notes:
  - `grid`: quarter, eighth, sixteenth, eighth_triplet, sixteenth_triplet.
  - `spoken`: one token per grid step. `DON` strong, `don` normal, `din` soft, `TIN` bright,
    `-` rest, `~` tie (extends the previous note). Eight eighth-steps make a 4/4 bar.
  - `length_bars` (1-8): how long the motif is before it repeats. Make it divide the section.
  - `degrees`: scale degrees cycled over the hits (0 = tonic, 2 = third, 4 = fifth, 7 = the
    tonic an octave up, negative goes below). Never MIDI numbers; degrees cannot leave the key.
  - `sustain_steps`: note length in steps. Above 1.0 so notes connect; 1.6 is a good default
    for piano, keys, mallets and bells.
  - `follow_chords: true` makes degrees relative to each bar's chord root (use with `chords`).
  - Explicit `notes[]` (`step`, MIDI `pitch`, `accent`) replace the generated notes. Avoid them
    unless you need an exact line: off-key or off-grid notes get snapped and listed in `repairs`.
- **Sections** cover every bar once, in order, and list the layer ids that play:
  `{"name", "bars": {"start", "end"}, "role", "intensity", "layers": [...]}`. Later sections
  add layers; the earlier part keeps playing unchanged.
- **Restraint**: no riser, sweep, impact, boom, pad, whoosh or fill unless the user asked. If
  they did, put the class in `requested` (e.g. `"requested": ["impact"]`), otherwise the layer
  is dropped and a repair says so.
- **`chords`** (optional): `[{"bars": {"start": 1, "end": 4}, "symbol": "Am"}]`; roots must be
  in the key. Bars without a chord use the tonic triad.

## Reading the result

`praxis_compose` returns `project`, `score_summary` (key, bpm, bars, duration_s, layers with the
bars each plays, sections, avoid-list), `repairs[]` (`{code, path, action}`, one per change
Praxis made to your plan), `lint`, `plan` (the repaired plan), and with a render, `render`:

- `output`, `duration_s`, `frames`, `is_loop`, `format`, `stems[]`
- `loudness_lufs`, `true_peak_dbtp`, `applied_gain_db`
- `skipped_layers[]` (`{layer, preset, reason}`): not rendered on this machine
- `warnings[]`: engine notes (effects or mix stages not built yet)
- `check`: `passed`, `failed[]` (names), `inconclusive[]` (names), `summary` (the take's line
  in plain words), `layers_checked[]`, and `checks[]`, each
  `{name, passed, measured, expected, inconclusive}` in words and numbers.

**Inconclusive means "can't tell from these notes", not failed.** Tempo and key can be
inconclusive when the material itself cannot decide them: a sparse or uneven rhythm that also
fits a related tempo, or notes with no third, so minor and major sound alike. Such a check has
`passed: false` and `inconclusive: true`, is left out of `passed` and `failed[]`, and its
`measured` ends with `can't tell: <reason>`. Do not change the sound to "fix" it; report it in
the person's words ("key: can't tell, there is no third"). A wrong tempo or key heard clearly
still fails.

The checks (docs/contracts/sound-profile.md):

| Check | Passes when |
| --- | --- |
| `tempo` | detected BPM within 2 of the Score's (an octave error fails) |
| `key` | detected tonic and major/minor family match the Score (relative major/minor fails) |
| `grid` | at least 90% of onsets within 10 ms of a step of the finest grid |
| `loudness` | integrated loudness within 0.5 LU of the target (-16 LUFS by default) |
| `true_peak` | at most -1 dBTP (+0.05) |
| `clipping` | no sample at or above full scale |
| `loop_seam` | loops only: no jump, click, transient or drop at the wrap point |
| `silence` | every bar that has notes is audible |

Skipped layers are left out of the checks, so a passing check covers what was rendered. Say so
when you report a result with skipped layers.

## Fixing failures

**Plan errors** (`invalid_score` from `praxis_compose`): each `error.issues[]` entry has a JSON
pointer `path` into your plan and a message listing the allowed values (preset names, the
scale's pitch classes, grids that divide the bar, spoken tokens). Fix those fields and call
again. Unknown or missing tool arguments are `bad_json` with the field named.

**Repairs** are not errors, but read them: they say what Praxis changed.

| Repair or issue code | Meaning | What to do |
| --- | --- | --- |
| `off_scale` | an explicit note was outside the key and was snapped | use `degrees` instead of `notes`, or pick an in-key pitch |
| `off_grid` | an explicit note step was not on the grid and was quantised | put `step` on a whole grid step |
| `grid` | the grid does not divide the bar | use a grid that does (6/8: eighth) |
| `spoken` | unknown rhythm token | only DON don din TIN - ~ |
| `unknown_preset` / `preset_kind` / `class_mismatch` | preset name, kind or class wrong | copy name, kind and class from `praxis_presets` |
| `unrequested_class` / `avoided_class` | a trailer effect you did not request, or a banned class | remove it, or list it in `requested` if the user asked |
| `sections` | gaps or overlaps | cover every bar once, in order |

**Render checks:**

| Failed check | Usual cause | Fix |
| --- | --- | --- |
| `key` | the line does not establish the tonic (detector hears the relative major/minor) | start and end phrases on degree 0 or 7, outline the tonic triad (0, 2, 4), add a bass on degree 0 |
| `grid` | too few onsets (a short sting: one stray onset is more than 10%), or ties and long sustain blurring attacks | use more hits per bar on the grid (8 eighths rather than 3 notes), avoid `~` runs in the middle of a phrase |
| `tempo` | sparse or syncopated rhythm, so the detector locks to half or double time | put clear accents (`DON`) on the beats; give the main layer a hit on every beat |
| `loudness` | the master is not normalised yet (see `warnings`: the mix bus is not built), so level follows dynamics | change `dynamics` or `gain_db` on the main layer, roughly dB for dB toward -16 LUFS, then re-render |
| `true_peak` / `clipping` | too hot, often `dynamics: "normal"` or `"strong"` on piano with no limiter yet | `dynamics: "soft"` or a negative `gain_db` |
| `loop_seam` | the last bar does not lead into bar 1: a strong downbeat right after a quiet tail, or a note cut at the wrap | make `length_bars` divide the loop so the last bar matches, soften bar 1's first accent (`DON` -> `don`) or give the last bar a pickup into it; keep `playback: "loop"` |
| `silence` | a layer's bars are inaudible (gain too low, or it was skipped) | raise `gain_db`; check `skipped_layers` |

`pack_missing` / `render_failed` with a preset reason: the instrument is not available here.
Pass `skip_missing: true`, or swap to an available preset from `praxis_presets`.

## Taste rules (the owner's; follow them by default)

1. **Rhythm first.** Every layer is a repeating motif on a grid with deliberate accents. Isolated
   notes with silence between them sound out of time.
2. **Notes connect.** `sustain_steps` above 1.0 wherever the instrument would ring.
3. **Arrangement is layering.** A new section adds a layer under or over what plays. Never
   rewrite the existing part unless asked.
4. **Restraint.** One emotion, few elements, room to breathe. No risers, sweeps, impacts, booms,
   pads, whooshes or fills unless the brief asks. "Cinematic" is a mood, not an instruction.
5. **Always in key.** Effects under music get `degrees` like any layer (an impact on degree 0 is
   tuned to the tonic).
6. **Seamless loops.** `playback: "loop"`, the last bar leads into the first, no fades.
7. **Real instruments.** Prefer sampled presets (felt_piano, upright_piano) to synths standing
   in for them.
8. **Mixed output.** -16 LUFS, peak under -1 dBTP, no clipping: the checks enforce it.

## Changing a sound

Edits change only what you scope; everything else stays as it was, down to the sample.

- **A project file**: `praxis_edit {project, prompt}` ("change the last four bars to add a
  subtle melody", "make the kick a little softer in bars 9-16", "thin out the piano in the second
  half"). Or write the EditPlan yourself (`plan`; start from
  `crates/praxis-planner/tests/prompts/edit_last_four_melody.plan.json`): `intent`,
  `scope.phrase` or `scope.bars`, `scope.keep`, `layers` by id, `params`. The response gives the
  `patch`, `changed_bars`, `changed_layers`, a plain `summary` and `refused` (locked layers are
  never changed; say so if the person asked for one). The edit is undoable in the project.
- **Alternates**: `praxis_variations {project|sound, bars, layers, n}` renders
  `variations/<i>.wav` with checks for each; pick by the checks and the summary.
- **Arrangement is layering**: prefer adding a layer in the bars to rewriting a part.

## Working on a sound with the person (takes)

A sound is a folder the person's app shows as a conversation: `project.praxis`, `takes.json`
and `takes/<n>/mix.wav`. A bare name (`"rain loop"`) lives in
`PRAXIS_HOME/sounds/<name>` (default `%USERPROFILE%/Praxis Sound/sounds/<name>`); a path is used as
given. Your takes appear in the person's thread as "Agent · <your name>" (default: your MCP
client's name; set `who` to choose).

1. `praxis_takes {sound}` first: see what exists, which take is current, the layer ids.
2. `praxis_ask {sound, text}` on a new sound makes take 1 from the brief. After that, `text` is a
   change to the current take. To change exactly some bars of one layer, pass
   `scope: {layer, bars: {start, end}}`: nothing outside it changes (a new layer inside those bars
   is allowed); a request that would reach further is refused with `patch_rejected`.
3. Read the take: `checks.passed` / `checks.failed` / `checks.inconclusive` (can't tell, not
   failed), `say` (what was done), `changed`
   (`{layer: [from, to]}`), `untouched` and `identical_bars` (bars measured bit-identical to the
   take before). The master gain is frozen after take 1, so untouched bars stay sample-identical
   except where a changed layer's tail wraps round a loop.
4. `praxis_open_view {sound, view: "notes"}` points the person at what you changed.
5. `praxis_restore {sound, n}` goes back to a take (undoable). `praxis_variations {sound, ...}`
   then `praxis_keep_variation {sound, index}` makes an alternate a take.

Before acting on a sound, compare `revision` with what you saw last: the person (or another
agent) may have made a take since.

### The app's own assistant

When an API key is set, `praxis_ask` (and the app's conversation) hands a request in words to
the sound assistant: Claude with Praxis's tools and the sound's conversation history
(`conversation.json` in the sound folder). Its takes show as "Agent · Assistant", and the
`praxis_ask` payload gains `agent.steps` (what it did) and `agent.reply`. Pass
`provider: "offline"` to use the rule planner instead, or drive the edits yourself with
`praxis_edit`, `praxis_compose` and a `patch`. How the assistant works, and the rules it
follows, are in `assistant.md` next to this file; they hold for any agent changing a sound:
change the instrument or effects for tone ("muffled", "brighter"), not the notes; make the
smallest change; measure it; answer in a sentence or two.

## Worked example 1: the acceptance-1 loop

Brief: "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt piano playing a
repeating rhythmic motif on an eighth-note grid, with accents and overlapping sustain. Bars 9-16:
the identical piano part, plus a soft muted kick layered underneath. No fills, no risers, sweeps,
impacts or pads."

Plan (this is `praxis_plan_schema`'s `example`):

```json
{
  "title": "Calm felt piano loop",
  "emotion": "calm",
  "key": { "tonic": "A", "mode": "minor" },
  "time_signature": { "numerator": 4, "denominator": 4 },
  "bars": 16,
  "duration_s": 30,
  "playback": "loop",
  "avoid": ["fill"],
  "sections": [
    { "name": "Piano alone", "bars": { "start": 1, "end": 8 }, "role": "intro", "intensity": 0.3, "layers": ["piano"] },
    { "name": "Piano and kick", "bars": { "start": 9, "end": 16 }, "role": "main", "intensity": 0.4, "layers": ["piano", "kick"] }
  ],
  "layers": [
    {
      "id": "piano",
      "class": "motif",
      "preset": { "kind": "sampler", "name": "felt_piano" },
      "dynamics": "soft",
      "motif": {
        "grid": "eighth",
        "length_bars": 2,
        "spoken": "DON din don din TIN din don din",
        "degrees": [0, 4, 7, 4, 9, 4, 7, 4, -2, 2, 5, 2, 7, 2, 6, 4],
        "sustain_steps": 1.6
      }
    },
    {
      "id": "kick",
      "class": "drums",
      "preset": { "kind": "drum", "name": "soft_kick" },
      "register": "low",
      "dynamics": "soft",
      "motif": { "grid": "eighth", "spoken": "DON - - - don - - -" }
    }
  ],
  "chords": [ { "bars": { "start": 1, "end": 16 }, "symbol": "Am" } ]
}
```

Call:

```json
{ "name": "praxis_compose", "arguments": {
  "plan": { "...": "the plan above" },
  "project": "out/calm-loop.praxis",
  "render": "out/calm-loop.wav",
  "skip_missing": true } }
```

Result (measured on 2026-11-11, felt piano rendered, `soft_kick` not built yet so skipped):
`repairs: []`, 128 BPM solved from 16 bars in 30 s, `frames` 1 440 000 (30.0 s at 48 kHz),
`is_loop: true`, and `check.passed: true`:

```
tempo      PASS  128.00 bpm (confidence 0.80)   expected 128.00 +- 2.00
key        PASS  a minor (confidence 0.86)      expected a minor
grid       PASS  93.8% of 128 onsets on grid, median deviation 1.75 ms
loudness   PASS  -15.58 LUFS                     expected -16.00 +- 0.50
true_peak  PASS  -3.21 dBTP                      expected <= -0.95
clipping   PASS  sample peak -3.22 dBFS
loop_seam  PASS  jump 0.17x typical, excess -5.14 dB
silence    PASS  16/16 bars audible
```

Report it as: "30.0 s seamless loop, A minor, 128 BPM, piano alone for 8 bars then piano and a
soft kick. 8 of 8 checks passed on the rendered piano (the kick preset is not built on this
machine yet, so it is in the project but not in the WAV)." Then `praxis_export` with
`format: "flac"` and `stems: "out/stems"`, or `format: "midi"`.

## Worked example 2: a short sting with a tuned impact under music

Brief: "A two-bar resolving sting in D minor: a piano pickup that lands on bar 2 with a low
impact under it, in key." The user asked for the impact, so it goes in `requested`, and it gets
degree 0 so it is tuned to D.

First plan: piano at `dynamics: "normal"`, bar 2 a single held `DON ~ ~ ~ ~ ~ ~ ~`. Result:
`check.failed: ["grid", "loudness", "true_peak", "clipping"]`: loudness -14.43 LUFS, true peak
+0.05 dBTP, 3 clipped samples, and 85.7% of only 7 onsets on the grid.

Fixes, one render each:

1. `dynamics: "soft"` -> peak -3.33 dBTP, no clipping; loudness -16.79 LUFS (0.8 LU low).
2. `gain_db: -0.5` on the piano -> -16.18 LUFS,
   passes. Grid still 88.9% of 9 onsets: one stray onset is more than 10% of a short clip.
3. More hits on the grid in bar 2 (`DON - don - DON - - -`) and the line ends on the tonic ->
   90.9% of 11 onsets, every check passes.

Final plan (`docs/agents/examples/sting.plan.json`):

```json
{
  "title": "Resolve sting",
  "emotion": "resolute",
  "key": { "tonic": "D", "mode": "minor" },
  "bars": 2,
  "tempo_bpm": 96,
  "playback": "one_shot",
  "requested": ["impact"],
  "sections": [
    { "name": "Piano pickup", "bars": { "start": 1, "end": 1 }, "role": "intro", "intensity": 0.4, "layers": ["piano"] },
    { "name": "Landing", "bars": { "start": 2, "end": 2 }, "role": "peak", "intensity": 0.7, "layers": ["piano", "hit"] }
  ],
  "layers": [
    {
      "id": "piano",
      "class": "motif",
      "preset": { "kind": "sampler", "name": "felt_piano" },
      "dynamics": "soft",
      "gain_db": -0.5,
      "motif": {
        "grid": "eighth",
        "length_bars": 2,
        "spoken": "DON din don din TIN din don din DON - don - DON - - -",
        "degrees": [4, 2, 0, 2, 4, 5, 6, 5, 7, 4, 0],
        "sustain_steps": 1.6
      }
    },
    {
      "id": "hit",
      "class": "impact",
      "preset": { "kind": "sfx", "name": "impact" },
      "register": "low",
      "dynamics": "soft",
      "motif": { "grid": "eighth", "spoken": "DON - - - - - - -", "degrees": [0] }
    }
  ]
}
```

Measured (2026-11-11): 9.0 s (2 bars at 96 BPM = 5.0 s plus the 4 s one-shot tail), tempo 95.85
BPM, key D minor (confidence 0.80), grid 90.9%, -16.44 LUFS, -2.83 dBTP, no clipping, both bars
audible. The `impact` preset is not built yet, so `skipped_layers` lists `hit` and the checks
cover the piano only; the project already holds the tuned impact on bar 2, beat 1, and it will
render once the preset lands. Say exactly that when you report it.

## Error codes

`io` (path unreadable or unwritable), `bad_json` (malformed JSON or tool arguments), `bad_score`
(not a Score/project), `invalid_score` (plan or score fails validation; see `issues`),
`bad_format` (unknown audio format), `render_failed`, `no_api_key` (praxis_generate with
`provider: "claude"` and no key), `provider_failed`, `pack_missing` (install the pack or
`skip_missing`), `pack_verify_failed` (a pack archive does not match the index), `decode_failed`
(not audio), `patch_rejected` (an edit, restore or kept variation cannot be applied: a locked
layer, a scope it would leave, a stale variation; or an edit that changes nothing), `error`.
