# Contract: `praxis` CLI JSON (FR-33)

With `--json`, every command prints exactly one JSON object on stdout and nothing else:

```json
{"ok": true, "command": "<name>", ...payload}
{"ok": false, "command": "<name>", "error": {"code": "<code>", "message": "<text>"}}
```

Exit code 0 on success, 1 on error. Human-readable output (without `--json`) may change freely;
the JSON payloads below only gain fields, never lose or rename them.

| Command | Input | Payload |
| --- | --- | --- |
| `render <score\|project> -o out.wav [--format 16\|24\|32f\|flac\|flac16\|flac24] [--sample-rate N] [--stems DIR] [--seed N] [--skip-missing]` | score JSON or `.praxis` | `output, stems[], sample_rate, frames, duration_s, is_loop, loudness_lufs, true_peak_dbtp, applied_gain_db, blocked_layers[], render_seconds, realtime_factor, format, skipped_layers[], warnings[], check{VerifyReport fields, passed, failed[]}` |
| `validate <score>` | score JSON | `valid, errors[], warnings[]` (issues: `code, path, message`) |
| `schema [--write]` | - | `score, patch, plan` schemas, or `written[]` |
| `generate (--prompt TEXT [--provider offline\|claude] \| --plan FILE\|-) [--seed N] -o project.praxis [--render out.wav]` | brief or agent-written ScorePlan | `project, score_summary{key, bpm, bars, layers[], sections[]}, provider, provider_note, attempts, repairs[], render?` |
| `edit <project> --prompt TEXT [--provider ...]` | project + instruction | `project, patch, scope, changed_bars, changed_layers` |
| `analyze <clip> [-o profile.json]` | audio/video file | the Sound Profile |
| `check <wav> --score <score>` | render + score | `VerifyReport` (`checks[], tempo_bpm, key, grid_deviation_ms, onsets_on_grid, integrated_lufs, true_peak_dbtp, seam_*`) and `passed` |
| `match <clip> [--prompt TEXT] [--bars N] [--key K] [--bpm N] -o project.praxis` | reference clip | `project, similarity, iterations[], originality` |
| `export-midi <score\|project> -o out.mid` | score | `output, tracks` |
| `packs list\|install <name>` | - | `packs[]` / `installed` |
| `key set\|clear\|status` | key on stdin only (set) | `stored` / `cleared` / `available, source` |
| `presets` | - | `presets[]` (name, kind, class, renderable here) |

The command layer is the library `praxis_cli::ops` (one function per command, typed request in,
payload JSON out); the MCP server and the desktop app call it directly. `edit` lands with WP-P2,
`match` with WP-M.

Error codes: `io`, `bad_json`, `bad_score`, `invalid_score`, `bad_format`, `render_failed`,
`no_api_key`, `provider_failed`, `pack_missing`, `decode_failed`, `patch_rejected`, `error`.

## Takes (WP-K2)

Edits, variations and the shared take history (ADR-0011). Library functions in
`praxis_cli::ops` (`edit`, `variations`, `ask`, `takes`, `restore`, `keep_variation`,
`open_view`); CLI commands below; MCP tools `praxis_edit`, `praxis_variations`, `praxis_ask`,
`praxis_takes`, `praxis_restore`, `praxis_keep_variation`, `praxis_open_view` (same payloads).

| Command | Input | Payload |
| --- | --- | --- |
| `edit <project> (--prompt TEXT [--provider offline\|claude] [--model M] \| --plan FILE\|-) [--seed N] [--render out.wav]` | project + instruction, or an EditPlan | `project, changed, patch, scope, changed_bars ([from, to] or null), changed_layers[], changed_by_layer{layer: [from, to]}, summary, refused[], new_layers[], plan, provider, provider_note, attempts, rounds, repairs, seed, can_undo, render?` |
| `variations (<project\|score> \| --sound S) [--bars A-B] [--layer ID]... [-n N] [--seed N] [--out-dir DIR]` | a selection | `source, out_dir, file, scope, seed, base_seq, base_take, passed, variations[{index (1-based), seed, summary, patch, changed_bars, changed_layers, changed_by_layer, render}]` |
| `ask <sound> <text> [--bars A-B --layer ID] [--who NAME] [--patch FILE\|-] [--title T] [--provider ...] [--seed N] [--skip-missing] [--include-score]` | a request | `sound, project, revision, current, take, score_summary, can_undo, can_redo, provider_note, edit?{refused, new_layers, plan, changed_bars, attempts, repairs}, repairs? (take 1)` |
| `takes <sound> [--include-scores]` | - | `sound, project, revision, current, takes[], score (live, or null), can_undo, can_redo, views[]` |
| `restore <sound> <n>` | - | `sound, project, revision, current, take, restored, patch, score_summary, can_undo, can_redo` |
| `keep-variation <sound> <index> [--who NAME] [--skip-missing]` | - | as `ask`, plus `variation` |
| `open-view <sound> <overview\|rhythm\|notes\|mix\|export> [--who NAME]` | - | `sound, revision, view, by, take` |

`validate` also accepts a `.praxis` project (detected by `"format": "praxis-project"`) and
validates its score; its payload gains `input_kind` (`score` or `project`). New error code:
`pack_verify_failed` (`packs install`: the archive's size or blake3 differs from the index, or
a file is missing after unpacking). `key clear` deletes the keychain entry (payload gains
`existed`). The `render` payload gains `length_samples`, `tail_samples` and `warmup_cycles`.

### `edit` and `variations`

`edit` runs `praxis_planner::plan_edit` (an instruction) or `compose_edit` (`--plan`) on the
project's score and commits the patch to its undo history; it writes the project only when the
patch changes something. A request that changes nothing (every named layer locked) is not an
error: `changed` is false and `summary` says why. Planner failures are `patch_rejected` (or
`bad_json` when every issue is a schema issue) with the issues (`code, path, message`, `path`
into the EditPlan); `no_api_key` / `provider_failed` as for `generate`.

`variations` runs `praxis_planner::variations` (1 to 8, default 3), renders each variation to
`<i>.wav` in `--out-dir` (default `variations/` next to the source, or inside the sound folder)
and checks it, and writes `variations.json` there (`format: "praxis-variations"`, `source`,
`base_seq` = the project's `current_seq()`, `base_take`, `scope`, `seed`, `variations[{index,
seed, summary, patch, changed, path, checks}]`). Nothing is committed. Named layers are exact
ids: the planner is given a view of the score with every other layer locked (its layer
references also match presets and words, so `piano` would otherwise reach an `upright_piano`
melody) and each patch is then applied to the real score.

### Sound folders

A `sound` is a folder path (absolute, or containing a path separator), or a bare name that
resolves to `PRAXIS_HOME/sounds/<name>`; `PRAXIS_HOME` defaults to
`%USERPROFILE%\Documents\Praxis Sound` (`$HOME/Documents/Praxis Sound` elsewhere).

```text
<sound>/project.praxis       the project; take n's score is version `take-<n>`
<sound>/takes.json           {format: "praxis-takes", version: 1, revision, current, takes[], views[]}
<sound>/takes/<n>/mix.wav    take n's master, 24-bit WAV at 48 kHz
<sound>/variations/          <i>.wav and variations.json from `variations --sound`
<sound>/.lock                held by a writer during a read-modify-write
```

Every write goes to a temporary file in the folder and is renamed over the target; the project
is written before `takes.json`, and every `takes.json` write increments `revision`, so a watcher
that sees a new revision finds the project and audio it refers to. Writers take `.lock`
(created exclusively; waited for up to 10 minutes; a lock older than 30 minutes is broken).
`takes` reads without the lock.

### The take record

`takes.json` stores each take as below; command payloads return the same object with
`render.path` made absolute (and the take's `score` with `--include-score(s)`). The fields up to
`scope` are the TypeScript `Take` of `app/src/engine/EngineAdapter.ts`, field for field
(`render.path` is its `render.audioUrl`; `score` is the version `take-<n>`); the rest are extra.

```json
{
  "n": 2,
  "who": "Agent · cut-assistant",
  "ask": "change the last four bars to add a subtle melody (bars 13–16, Felt piano)",
  "title": "New melody, bars 13–16",
  "say": "Added a soft upright piano melody (`melody`) in bars 13-16. Bars 1-12 are unchanged. Layers `piano`, `kick` are untouched.",
  "patch": {"description": "...", "origin": "model", "scope": {"bars": {"start": 13, "end": 16}, "layers": ["melody"]}, "ops": []},
  "changed": {"melody": [13, 16]},
  "untouched": "Bars 4–12 sample-identical to take 1. Felt piano unchanged. Soft kick unchanged.",
  "render": {
    "path": "takes/2/mix.wav",
    "report": {
      "mock": false, "sample_rate": 48000, "length_samples": 1440000, "tail_samples": 0,
      "is_loop": true, "loudness_before_lufs": -16.9, "loudness_lufs": -16.0,
      "true_peak_dbtp": -1.0, "applied_gain_db": 0.97, "warmup_cycles": 1, "notes_rendered": 176,
      "blocked_layers": [], "render_seconds": 5.9, "warnings": [],
      "skipped_layers": [], "format": "wav24", "frames": 1440000, "duration_s": 30.0
    }
  },
  "checks": {"checks": [], "tempo_bpm": 128.0, "key": "a minor", "grid_deviation_ms": 1.7,
             "onsets_on_grid": 0.91, "integrated_lufs": -16.0, "true_peak_dbtp": -1.0,
             "seam_jump_ratio": 1.0, "seam_excess_db": -20.0, "passed": true,
             "failed": [], "layers_checked": ["piano", "kick", "melody"]},
  "opens": "notes",
  "scope": {"layer": "piano", "bars": {"start": 13, "end": 16}},
  "version": "take-2",
  "from": 1,
  "identical_bars": [[4, 12]],
  "provider": "offline-rules",
  "seed": 2
}
```

- `who`: `"You"` or `"Agent · <name>"` (a bare name is prefixed; the CLI defaults to `"You"`,
  the MCP tools to `"Agent · <MCP client name>"`).
- `patch`: null for take 1. `changed`: `{layer: [first, last]}` of the bars whose expanded notes
  differ; a layer whose sound settings (instrument, gain, pan, effects, mute, solo) changed, or
  that was added or removed, counts its whole extent; a tempo/key/metre/length/master change
  counts every layer. Empty for take 1.
- `identical_bars`: runs of bars whose master samples are bit-identical to take `from`
  (measured on the two WAVs). `untouched` says it in words; when no bar is bit-identical (a
  recirculating reverb on a changed layer leaves rounding-level differences), it says which bars
  are identical note for note instead. Each existing layer the take did not change is listed
  ("Soft kick unchanged.").
- `provider`: the planner that answered, `hand` for an explicit patch, `variation` for a kept
  variation. `seed`: defaults to the take number.
- `opens` / `scope`: omitted when absent.
- `views[]` in `takes.json`: `{revision, view, by, take}`; `by` is `"Praxis"` for the person's
  own request, else the agent's name.

### `ask`

- **New sound** (no takes yet): `text` is the brief, planned with `plan_brief` (provider as for
  `generate`; seed default 1). `say` is `PlanOutcome::summary()`. A `scope` or `patch` is an
  `error`, and so is a folder that already holds a `project.praxis` without `takes.json`.
- **Master gain freeze** (ADR-0007): after take 1's first render, the applied gain is committed
  as `master.frozen_gain_db` through an `origin: system` `set_master` patch (undoable), before
  version `take-1` is saved. Every later take renders with that gain, so outside the edit's tail
  window (contracts/patch.md) the master is bit-identical.
- **Edit** (takes exist): a scoped edit of the current score. Without `scope`: `plan_edit` on
  `text`. With `scope {layer, bars}`: the planner reads `"bars A-B of `<layer>`: <text>"` on a
  view of the score where every other layer is locked; its plan keeps its intent, its bars are
  replaced by `scope.bars` and its target layers by `[scope.layer]` (none for `add_layer` /
  `add_melody`, which add a layer inside the bars), and it is built with `compose_edit`. A result
  that changes another existing layer, or bars outside the scope, fails with `patch_rejected` and
  changes nothing; so does a scope naming an unknown layer or bars outside the score. `ask`
  records the scope appended to the request ("... (bars 13–16, Felt piano)").
- **Hand edit**: with `patch` (a ScorePatch) the patch is committed as given; `title` defaults
  to "Hand edit[, bars A–B]" and `say` to "Changed by hand: <description>.".
- An edit that changes nothing fails with `patch_rejected` (its summary is the message) and
  makes no take. The new take is rendered to `takes/<n>/mix.wav`, checked, saved as version
  `take-<n>` and becomes `current`.

### `restore`, `keep_variation`, `open_view`

- `restore {sound, n}`: `Project::restore_version("take-<n>")` (an undoable patch) and
  `current = n`. No take is added. `restored` is false (and `patch` null) when the score already
  equals take n. An unknown take is `error`; a patch the project refuses is `patch_rejected`.
- `keep_variation {sound, index}`: commits variation `index` of the sound's
  `variations/variations.json` as a new take (`provider: "variation"`, `say` = the variation's
  summary, `scope` set when the variations named one layer and bars). Fails with
  `patch_rejected` when the sound's current take or history moved since the variations were made.
- `open_view {sound, view, who}`: appends `{revision, view, by, take}` to `views` (the last 50
  are kept) and, when the current take was made by the same `who`, sets its `opens`.
