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
| `render <score\|project> -o out.wav [--format 16\|24\|32f\|flac\|flac16\|flac24] [--sample-rate N] [--stems DIR] [--seed N] [--skip-missing]` | score JSON or `.praxis` | `output, stems[], sample_rate, frames, duration_s, is_loop, loudness_lufs, true_peak_dbtp, applied_gain_db, blocked_layers[], render_seconds, realtime_factor, format, skipped_layers[], warnings[], check{VerifyReport fields, passed, failed[], inconclusive[], summary}` |
| `validate <score>` | score JSON | `valid, errors[], warnings[]` (issues: `code, path, message`) |
| `schema [--write]` | - | `score, patch, plan` schemas, or `written[]` |
| `generate (--prompt TEXT [--provider offline\|claude] \| --plan FILE\|-) [--seed N] -o project.praxis [--render out.wav]` | brief or agent-written ScorePlan | `project, score_summary{key, bpm, bars, layers[], sections[]}, provider, provider_note, attempts, repairs[], render?` |
| `edit <project> --prompt TEXT [--provider ...]` | project + instruction | `project, patch, scope, changed_bars, changed_layers` |
| `analyze <clip> [-o profile.json] [--beats-per-bar N]` | audio/video file | the complete Sound Profile (see "Sound Profile and library") |
| `library add <clip\|profile.json> [--name N] [--tag T]... \| list \| show <id> \| tag <id> [--add T]... [--remove T]... \| remove <id>` | clip, profile or id | see "Sound Profile and library" |
| `check <wav> --score <score>` | render + score | `VerifyReport` (`checks[], tempo_bpm, key, grid_deviation_ms, onsets_on_grid, integrated_lufs, true_peak_dbtp, seam_*`) and `passed` |
| `match <clip> [--prompt TEXT] [--timbre CLIP] [--bars N] [--key K] [--bpm N] [--seed N] [--max-iterations N] [--render out.wav] -o project.praxis` | reference clip | `project, similarity, iterations[], originality` (see "Match (WP-M)") |
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
| `ask <sound> <text> [--bars A-B --layer ID] [--who NAME] [--patch FILE\|-] [--title T] [--provider ...] [--seed N] [--skip-missing] [--include-score] [--reference CLIP \| --reference-library ID]` | a request | `sound, project, revision, current, take, score_summary, can_undo, can_redo, provider_note, edit?{refused, new_layers, plan, changed_bars, attempts, repairs}, repairs? (take 1)` |
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
`%USERPROFILE%\Praxis Sound` (`$HOME/Praxis Sound` elsewhere; not Documents, which Windows'
Controlled Folder Access protects).

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
  differ, plus the bars where its effects or automation sound different: an automation lane
  counts where its value differs from the old one (the old lane, or the fixed value); a new
  effect counts only where it is heard (nowhere when bypassed; where its `mix` is above 0 for
  `filter`, `compressor`, `delay`, `reverb`, `convolution_reverb` and `chorus`, which pass their
  input through unchanged at `mix` 0; the fixed `mix` must be 0 too for a one-shot); gain and pan
  lanes glide over one 128-sample block, so a change of theirs starting at a bar line also counts
  the bar before. A layer whose other sound settings (instrument, gain, pan, mute, solo) changed,
  whose existing effects changed or were removed, or that was added or removed, counts its whole
  extent; a tempo/key/metre/length/master change counts every layer. Empty for take 1.
- `identical_bars`: runs of bars whose master samples are bit-identical to take `from`
  (measured on the two WAVs). `untouched` says it in words; when no bar is bit-identical (a
  recirculating reverb on a changed layer leaves rounding-level differences), it says which bars
  are identical note for note instead; when each bar's largest difference is measured below
  -120 dBFS (the patch contract's rounding level), it says so with the residue: "Bars 3–12
  identical note for note to take 4; only reverb rounding differs (at most -131 dBFS)." (bars
  holding the change's audible tail are left out). Each existing layer the take did not change is listed
  ("Soft kick unchanged.").
- `provider`: the planner that answered, `hand` for an explicit patch, `variation` for a kept
  variation, `match` for a take the match loop made. `seed`: defaults to the take number.
- `reference`: `{name, clip?, library_id?}` when the request came with a reference (omitted
  otherwise).
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
- **Lock, mute, solo**: a `patch` whose ops only lock or unlock layers (`set_lock`) or set
  `muted` / `solo` (`update_layer` with nothing else) changes how the live score plays, as in the
  app: it is committed to the project (undoable) and `revision` grows, with no render and no
  take. Payload `{sound, project, revision, current, take: null, no_take: "live", reply,
  score_summary, can_undo, can_redo, provider_note: null}`; `reply` is "<description>; no take
  was made.". Exit code 0.
- An edit that changes nothing fails with `patch_rejected` (its summary is the message) and
  makes no take. The new take is rendered to `takes/<n>/mix.wav`, checked, saved as version
  `take-<n>` and becomes `current`.

- **Reference** (`AskRequest.reference: Option<ReferenceArg {name, clip?, library_id?}>`,
  CLI `--reference CLIP` / `--reference-library ID`, MCP `praxis_ask` `reference {name, clip?,
  library_id?}`): a dropped clip (measured with `analyze`'s code) or a library style. When the
  words ask for something like it ("like this", "something like", "similar", "in the style of",
  "same vibe/feel/style/character", "match this/it", or no words at all) the request runs the
  match loop (no model): on a new sound it makes take 1, on a sound with takes the matched sound
  replaces the current one as one undoable patch (locked layers refuse it); the take's
  `provider` is `match`, its `say` names the result and its similarity, the words are the mood.
  Otherwise the reference goes along: the planner reads the brief followed by `Reference
  "<name>": <analyze summary>. Around <bpm> BPM in <key>.` (new sounds), and the assistant's turn
  starts with a synthetic `analyze_clip` call (`input.path` is the clip or `library:<id>`) whose
  result is `{reference, profile}`, the profile trimmed to under 3 kB (no `pitch.midi`,
  `envelope.frames_db`, `spectral.centroid_track_hz`, `onsets_s`, `tempo.beats_s` or `features`;
  floats to three decimals; `summary` added). The take records `reference`. Plain words that
  mention a clip are still accepted without the field.

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

## Assistant agent (WP-AG)

A request in words on a sound (`ask` without `patch`) is carried out by the sound assistant
(`praxis_cli::agent`) when it can be: `provider` unset and an Anthropic API key available
(`ANTHROPIC_API_KEY` or the keychain), or `provider: "claude"` (without a key that is
`no_api_key`). `provider: "offline"`, or no key, keeps the offline planner described above. A
hand edit (`patch`) never reaches a model. Library: `ops::ask` (default options) and
`ops::ask_with(req, &AskOptions {client, agent: AgentConfig, progress})`; `AskRequest` is
unchanged.

**The turn.** One Claude Messages API tool loop (`claude-opus-5-5`, `output_config.effort`
`medium` unless set, adaptive thinking left at the model default, `tool_choice: auto`, every tool
`strict`, streaming, server-side refusal fallbacks and server-side compaction as betas) with 17
tools: `read_sound`, `catalogue`, `get_schema`, `compose_sound` (a ScorePlan), `compose_edit` (an
EditPlan), `apply_patch` (any ScorePatch op; locks and scope enforced by `apply`), `set_params`
(instrument parameters of one layer), `render_check` (the checks plus loudness, true peak, onsets
on the grid, per-layer spectral centroid, low-band and high-band energy and RMS, each against the current
take, and the bars bit-identical to it), `make_variations`, `keep_variation`, `restore_take`,
`undo_redo`, `open_view`, `analyze_clip`, `ask_user`, `discard_changes`, `finish_turn {reply,
title, no_take}` (ends the turn with the reply and the take's title; called alone, after the
other results). A sound
change held to the scope's bars by automation (a lane at the old value outside them, or a new
effect at zero mix there) passes the scope check, which reads `changed` above. Changes collect in a
draft of the score; with a `scope` every change is checked against it (another existing layer,
or bars outside it, is refused to the model). The turn ends when the model answers without a
tool call, calls `ask_user` or `finish_turn`, declines (`stop_reason: refusal`), or uses up 16 tool calls or 10
minutes (it is then told to wrap up and gets one more response). The system prompt is
`docs/agents/praxis-skill/assistant.md`, five sections of `SKILL.md` and the preset and effect
tables generated from code.

**The take.** One request makes at most one take: the draft, committed as one patch (`origin:
model`) through the same take path as above, with `who` `"Agent · Assistant"`, `provider`
`"agent:<model>"`, `say` and `agent.reply` the assistant's reply (`finish_turn.reply`, else the text of
its final message, else a summary of the change) and `title` the one it gave with `finish_turn` (3-6 words,
tidied; one that is empty, starts with "I" or runs past 8 words is not used), else a summary of
what changed ("Felt piano sound", "New upright piano, bars 13–16", "Kick level"); never the
reply clipped. `keep_variation`
inside a turn is that turn's take; `finish_turn`'s title and reply then replace the variation's
default title ("Variation 2, bars 13–16") and `say` (a new `takes.json` revision). `restore_take` and `undo_redo` move the history at once and
make no take.

**Payload.** As `ask`, plus `agent {outcome, reply, question, model, turn, steps[{tool, words,
ok, result}], usage{requests, input_tokens, output_tokens, cache_creation_input_tokens,
cache_read_input_tokens}}`. `outcome`: `take`, `question`, `no_change`, `out_of_steps`,
`declined`, `failed`. A turn that makes no take is a reply, not an error (exit code 0): `take:
null`, `reply` (for a question, the question) and `no_take`: `question` (`ask_user`; the next
request answers it), `refused` (a locked layer, the model declining, or `finish_turn` with
`refused`), `undo` (`undo_redo` or `restore_take` moved the history), `view` (only a view was
opened), `nothing_to_change` (an answer, advice, or out of steps before any change); with
`revision`, `current`, `score_summary`, `can_undo`, `can_redo` as after the turn. Only a real
failure is an error: `message` is the reply and `issues[0]` is `{code: "agent", path: "",
message, agent}`, with `provider_failed` (the API failed), `no_api_key`, or the code of a failed
final render.

**Progress.** `AgentEvent` (`started`, `thinking`, `said`, `step {n, tool, words}`, `step_done
{n, tool, ok, words}`, `take_made {n}`, `finished {outcome, reply}`) through
`AskOptions::progress`; `words` are plain ("Reading the sound", "Changing the Felt piano's tone
and felt", "Rendering", "Checking: 8 of 8 passed"). `praxis ask` prints the steps on stderr.

**Files in the sound folder.**

- `conversation.json`: `{format: "praxis-conversation", version: 1, messages[], turns[]}`.
  `messages` is the API history, append-only, assistant content exactly as returned (thinking
  blocks with signatures, compaction blocks); an unanswered user message (an `ask_user` turn, a
  failed request) gets the next request's text added to it. `turns[]`: `{n, request, who, scope?,
  model, from_take, outcome, take, reply, steps[{tool, input, ok, words, result}], usage}`.
  Written atomically at the end of each turn.
- `.agent-lock`: held for a whole turn, so two turns on one sound run one after the other.
- `agent/draft.wav`, `agent/current.wav`: the last `render_check` renders (not takes).

**API address.** `PRAXIS_CLAUDE_URL` (a local stand-in for the API, used by tests) is accepted only
with an `http://` or `https://` loopback host: `localhost`, `127.x.y.z` or `[::1]`, no user
info. Anything else fails with `error` ("PRAXIS_CLAUDE_URL must point at this machine ...")
before a request is sent or a file written, for the assistant and the planner's Claude provider
alike; the HTTP client follows no redirects. The key can never be sent elsewhere by an
environment variable.

**App.** The desktop shell's `ask` command sends each `AgentEvent` to the window as
`praxis://agent` with the sound: `{sound, event: "step", n, tool, words}` and so on; the thread
shows the steps under the request while the turn runs.

**CLI.** `praxis ask <sound> <text> [--provider offline|claude] [--model M] [--effort
low|medium|high|xhigh|max] [--max-steps N]`. **MCP.** `praxis_ask` takes `effort` and
`max_steps` too.

**Also new:** `ops::undo_redo(UndoRedoRequest {sound, action: undo|redo})` -> `{sound, moved,
description, revision, current, score_summary, can_undo, can_redo}`: one step of the project
history, as the app's undo and redo; landing on a take's score makes it current.

## Sound Profile and library (WP-D2)

`analyze` returns the complete `SoundProfile` of docs/contracts/sound-profile.md (every field;
`complete: true`, `missing: []`), plus `source.path`, `source.source_kind` (`audio` or
`video`), `key.name` (e.g. `"a minor"`), `feature_names` (the names of `features`, in order),
`loop_seam {passed, jump_ratio, excess_db}`, a one-line `summary` in plain words (e.g. `"music
(93% sure) · 30.0 s · 128 BPM · a minor · -16.0 LUFS · tonal"`; tempo only for music, key only
for tonal clips) and, with `-o`, `output`. The `-o` file holds the same payload and reads back as
a `SoundProfile` (extra fields are ignored). Library functions: `ops::analyze`,
`ops::analyze::{profile_clip, profile_decoded, payload, summary}`. The payload of a 30 s clip is
about 120 kB (pitch track at 10 ms, envelope and centroid tracks up to 2000 frames each).

`library` keeps profiles, never audio, in `PRAXIS_HOME/library/<id>.profile.json` (one
`SoundProfile` per file; the style name in `style_name`, tags in `tags`). Library function
`ops::library(LibraryRequest {action, dir})` with `LibraryAction::{Add {input, name, tags,
beats_per_bar}, List, Show {id}, Tag {id, add, remove}, Remove {id}}` (`dir` overrides the
folder; the CLI and MCP leave it unset). Every payload has `action` and `library` (the folder).

| Action | Payload |
| --- | --- |
| `add` | `id, already, entry`. `input` is a clip (analysed) or a profile JSON (`.json`, as `analyze -o` writes). The id is the name (default: the file name) in lowercase letters, digits and dashes, made unique with `-2`, `-3` .... A clip whose decoded PCM hash is already kept is not added again: that entry gains the new tags (and name, when given) and `already` is `true`. |
| `list` | `count, entries[] {id, path, name, tags, kind, duration_s, tempo_bpm, key, file_name, pcm_hash, summary}` in id order, `unreadable[] {id, error}` |
| `show` | `id, entry, profile` (the analyze payload of the saved profile, without `source.path` and `loop_seam`) |
| `tag` | `id, entry` after removing `remove` and adding `add` (trimmed, duplicates dropped, order kept) |
| `remove` | `id, removed: true` |

An unknown id fails with `io` ("no library entry ..."); an id with anything but lowercase
letters, digits and dashes fails with `error`.

MCP: `praxis_analyze {clip, out?, beats_per_bar?}` returns the analyze payload;
`praxis_library {action: add|list|show|tag|remove, input?, name?, tags?, id?, add?, remove?,
beats_per_bar?}` mirrors the CLI (same payloads; a missing `input` or `id` fails with
`bad_json`).

## Match (WP-M)

`ops::match_clip(MatchClipRequest {clip, out, prompt?, timbre?, bars?, key?, bpm?, seed?,
max_iterations?, render?})`; CLI `match`; MCP `praxis_match {clip, project, prompt?, timbre?,
bars?, key?, bpm?, seed?, max_iterations?, render?}` (same payload). The clip (and `timbre`) is
measured with `analyze`'s code and dropped: only its Sound Profile reaches the matcher
(docs/contracts/sound-profile.md, "Matching"). `key` is free text ("A minor", "F#m", "Eb
dorian"); `bars` 1-512, `bpm` 40-220, `max_iterations` 10-200 (default 40), `seed` default 0.

Payload: `project, similarity, iterations[] {n, stage, tried, similarity, best, accepted},
originality {passed, melodic_similarity, rhythmic_similarity, reason, reference},
originality_attempts, kind (music|sfx|ambience), made_with, similarities[] {reference, role,
similarity}, params{}, notes[], reference {file_name, duration_s, kind, tempo_bpm, key},
score_summary, render?` (the `render` payload with its checks when `--render` is given).

Errors: a missing clip `io`; an unreadable clip `decode_failed`; an unreadable key or a value out
of range `error`; every attempt too close to the reference's melody `error` ("every attempt was
too close to the reference's melody: ..."); a render failure `render_failed` / `pack_missing`.

The assistant's tool `match_reference {path, timbre_path, mood, key, bpm, bars}` (`""` / `0` for
none; `path` may be `library:<id>`) runs the same matcher and puts the result in the draft like
`compose_sound` (every unlocked layer replaced); its result has `summary, kind, made_with,
similarity, similarities, iterations [[stage, similarity], ...], originality, notes, score`.
