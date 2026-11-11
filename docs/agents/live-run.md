# Live run of the sound assistant (WP-AG)

The assistant was built and tested without a key: a scripted model and replayed API streams
drive the real loop, tools, renders and take history (`crates/praxis-cli/tests/agent.rs`). This
is the first run against the real API, once the owner has given the key. It costs roughly
US$1-3 in total (see "Cost" at the end).

## Setup

```bash
praxis key set < key.txt
```

```bash
set PRAXIS_PACKS=C:\Users\admin\Desktop\praxis sound\packs
```

Use a fresh sound folder, e.g. `C:\Users\admin\Praxis Sound\sounds\live-1`, and run
each request below in order with `praxis ask live-1 "<request>" --json`. The assistant's steps
print on stderr; the JSON on stdout has the take and `agent.steps`, `agent.reply` and
`agent.usage`. After each request, open the sound in the app (or `praxis takes live-1`) and
listen.

## Requests and what each should do

1. **"A calm 16-bar felt piano loop in A minor, 30 seconds, seamless, with a soft kick coming in
   at bar 9. No fills or risers."**
   Steps: `compose_sound` with a ScorePlan (felt_piano motif on an eighth grid, sustain over
   1.0, soft_kick from bar 9), `render_check`, maybe a fix and another `render_check`. Take 1,
   all 8 checks passed. Reply: one or two sentences (bars, key, tempo, what plays where).

2. **"Make the piano muffled."**
   Steps: `read_sound` (summary or the piano layer), `set_params` on `piano` lowering `tone`
   and/or raising `felt` (or an `eq` with a lower `lp_hz` through `apply_patch`), `render_check`.
   Take 2: `changed` lists only `piano`; every note identical (`praxis takes live-1
   --include-scores`, compare the piano patterns); the render_check result shows the piano's
   centroid lower than take 1's. It must NOT rewrite the melody (this was the one-shot planner's
   failure). The loudness check may fail because the master gain is frozen; a good answer then
   raises the piano's `gain_db` a little and checks again.

3. **"A bit brighter than before."**
   Steps: `set_params` with `tone` between the original 0.5 and the muffled value (it reads
   "before" from the history; no need to read the take list). Take 3, notes still identical,
   centroid between takes 1 and 2.

4. **"Undo what you just did to the piano."**
   Either `undo_redo` (take 2 becomes current, no new take; the turn then has no take and the
   reply says take 2 is current again) or a `set_params` back to take 2's values (a new take).
   Both are right; the reply must say which. With `undo_redo`: `praxis ask` exits 0 with `ok:
   true`, `take: null`, `no_take: "undo"`.

5. **"Make the kick a little softer in the second half."** (with the kick present only in bars
   9-16, "second half" is the same bars)
   Steps: `compose_edit` with `change_dynamics` softer on the kick, or `set_params`/`apply_patch`
   on the kick's `gain_db`. Take: only `kick` changed; the `render_check` shows its RMS and low-band
   energy lower.

6. **In the app, select bars 13-16 of the piano and ask "add a subtle melody here".**
   (CLI: `praxis ask live-1 "add a subtle melody here" --layer piano --bars 13-16`.)
   Steps: `compose_edit` with `add_melody` in bars 13-16 (or `apply_patch` adding a layer whose
   clips are in 13-16). Take: one new layer in bars 13-16; `identical_bars` covers bars 4-12 (bars
   1-3 hold the new layer's tail wrapped round the loop).

7. **Lock the kick in the app, then ask "make the kick louder".**
   (CLI: `praxis ask live-1 "Lock the kick" --patch lock.json` with a `set_lock` op: exit 0,
   `take: null`, `no_take: "live"`, no take and no render, `revision` one higher; undoable.)
   The patch on the kick is refused by the tools; the reply says the kick is locked and nothing
   changed. No take: exit 0, `ok: true`, `take: null`, `no_take: "refused"`.

8. **"Make it better."**
   Too vague: the assistant should call `ask_user` with one short question (or make one small,
   clearly described change). No take for a question (exit 0, `no_take: "question"`); the next
   request answers it and the assistant continues from there.

9. **"Give me three alternatives for the last four bars of the piano and keep the one whose
   checks all pass."**
   Steps: `make_variations` (piano, bars 13-16, count 3), `keep_variation`. One take. If no bar
   is bit-identical (the piano's reverb), `untouched` reads "Bars ... identical note for note to
   take N; only reverb rounding differs (at most -1xx dBFS)".

10. **"Show me the notes."**
    Steps: `open_view` notes. The app shows "Notes opened by Assistant". No take (exit 0,
    `no_take: "view"`).

11. **In the app, select bars 5-8 of the piano and ask "make these bars muffled".**
    (CLI: `praxis ask live-1 "make these bars muffled" --layer piano --bars 5-8`.)
    Steps: an unscoped `apply_patch` adding an `instrument.tone` lane (step curve: the old value,
    a lower one from bar 5, the old value again from bar 9), or `set_params`-like change held to
    the bars by automation; `render_check`. Take: `changed` is `{"piano": [5, 8]}`; the scope
    is not refused; bars 1-4 and 11-16 of the master are identical (bit-identical or below
    -120 dBFS). If the assistant instead refuses ("a tone change applies to the whole layer"),
    record it: the prompt should have led it to automation.

## What to look at

- **Every turn without a take** (requests 4, 7, 8, 10) exits 0 with `ok: true`, `take: null`,
  `reply` and `no_take`; in the app it shows as the assistant's reply (labelled "Assistant"),
  never as an error.
- **Titles** (`praxis takes live-1`): 3-6 words saying what changed ("Muffled felt piano"),
  never a clipped sentence ("I made a 16-bar"). The assistant gives them through `finish_turn`;
  check that most turns end with it (in `agent.steps` it is not listed; in `conversation.json`
  it is the last `tool_use` of the turn).
- **The app** shows the steps under the request while the assistant works ("Reading the
  sound", "Changing the felt piano's tone", "Rendering", "Checking: 8 of 8 passed"), replaced
  by the take or the reply.
- **Key safety**: `PRAXIS_CLAUDE_URL` must be unset; set to anything but a loopback address it
  is refused before any request.
- `agent.steps` reads like a sensible plan; no tool fails more than once in a row.
- `agent.usage`: from the second request of a turn on, `cache_read_input_tokens` is around
  10 000 or more and `cache_creation_input_tokens` small; if `cache_read_input_tokens` stays 0,
  the prefix is not caching (report it).
- Replies are one or two plain sentences and match what changed.
- `conversation.json` grows by one turn per request; nothing in it or anywhere in the folder
  contains the key.
- If any request returns HTTP 400, record the message: the likeliest causes are a tool schema the
  strict grammar rejects, the compaction or fallback beta, or the cache markers.

## Cost

Measured from the requests the tests send (`request_sizes_for_the_cost_estimate` and the
`conversation.json` of `a_conversation_new_sound_muffled_then_brighter`), converted at about
3.5 bytes per token (no `count_tokens` call was possible without a key, so the token numbers
are estimates; the live run's `agent.usage` gives the real ones):

- Cached prefix (tools 8.2 KB + system prompt 28.2 KB): about 10 000 tokens.
- "Make the piano muffled": 4 requests; the turn adds about 6 KB of history (the request with
  the sound's state 0.5 KB, the piano layer 2.6 KB, the measurements 2.3 KB, the calls). Input
  over the 4 requests is about 53 000 tokens, almost all of it the repeated prefix and history.
  Output (thinking at effort medium, the calls, the reply): an estimated 1 500-2 500 tokens.

At Claude Opus 5.5 prices ($4 input, $20 output per million tokens; cache reads $0.20 per
million; cache writes 1.25x for five minutes, 2x for one hour):

| Request | With cache hits | Without cache hits |
| --- | --- | --- |
| A typical edit turn ("make the piano muffled", 4 requests) | about $0.06-0.08 | about $0.25 |
| A new sound (3 requests, the ScorePlan written as output) | about $0.08-0.12 | about $0.20 |
| First request after an hour idle (prefix rewritten to the one-hour cache) | +$0.08 | - |

Most of a cached turn's cost is output tokens; most of an uncached turn's is the repeated
prefix, so the cache matters: check `cache_read_input_tokens` in the live run.
