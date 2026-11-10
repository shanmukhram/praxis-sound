# Praxis Sound workstation front end (`app/`)

The conversation-first workstation (ADR-0011, FR-28 to FR-32) as a TypeScript + Vite web app with
no framework (ADR-0002). It follows the approved concept, `docs/design/workstation-concept.html`:
a sounds list, a conversation about one sound in which every request makes a playable **take**,
and a Sound panel whose change map shows what each take changed. In this package it runs in a
browser against a **mock engine adapter**; the Tauri IPC adapter (desktop) and the wasm adapter
(browser build) replace the mock later behind the same `EngineAdapter` interface.

```bash
npm install
npm run dev          # http://localhost:5173 (opens the acceptance-1 sound on take 3)
npm test             # Vitest unit tests
npm run test:e2e     # Playwright tests at 1440 and 390 (starts the dev server itself)
npm run typecheck
npm run build        # dist/
npm run gen:types    # regenerate src/score/generated.ts from ../schema/*.schema.json
```

`?sound=bell` or `?sound=new` opens another sound. Playwright uses its bundled Chromium
(`npx playwright install chromium` once), or an installed browser with `PW_CHANNEL=msedge` /
`PW_CHANNEL=chrome`. Screenshots from the e2e run land in `screenshots/` (desktop 1440, phone 390,
phone with the Sound sheet open).

## What you see

- **Sounds list** (left, 248 px): your sounds, New sound, and the connected agents.
- **Conversation** (middle): each request and its take. The current take is the largest block:
  its reply, waveform (the changed bars in amber), key / tempo / bars / length, the measured
  checks in plain words, what changed and what stayed identical, Try 3 variations and Export.
  Earlier takes fold into one-line rows with Go back. The composer takes words, a scope chip
  ("Bars 13–16 · Felt piano", set by dragging on the change map) and a dropped clip; suggestion
  chips above it come from the current sound. Undo / Redo sit in the header (Ctrl+Z,
  Ctrl+Shift+Z); Space plays the current take.
- **Sound panel** (right, 456 px): "Where each layer plays", always at the top, then Overview
  (layers with Mute / Solo / Lock in words; Mood, Brightness, Space, Energy), Rhythm (the step
  grid in DON / din / don / TIN), Notes (piano roll, moved notes in amber, What moved), Mix
  (loudness and peak against targets, spectrogram, layer levels) and Export. When the assistant or
  an agent opens a view, it says "Notes opened by cut-assistant".
- **Phone** (under 760 px): the conversation full width with a sticky composer; the Sound panel
  is a bottom sheet opened from the top bar.
- Scrollbars are themed everywhere: a slim grey thumb on a transparent track, `color-scheme: dark`.

## Code

| Path | What it does |
| --- | --- |
| `src/engine/EngineAdapter.ts` | The adapter interface: score, patches, render, transport, export, and the take model (`ask`, `edit`, `takes`, `state`, `restore`, `undo`/`redo`, `suggestions`, `agents`, `onTake`, `onState`, `onOpenView`, `openView`). `Take` and `VerifyReport` mirror the Rust `VerifyReport` and the CLI payloads, so a real adapter fills them from `praxis generate | edit | check`. |
| `src/engine/takes.ts` | Pure take helpers any adapter can use: `changedFromPatch` (bars each layer changed), `changedText`, `untouchedText`. |
| `src/engine/MockEngine.ts` | Mock adapter: a take session with undo history (going back is an undoable patch), the acceptance-1 demo conversation, mock agents, view events, a WebAudio transport and export recording. `identicalOutside` proves "sample-identical" by comparing renders outside the changed bars and their tails. |
| `src/engine/mock/planner.ts` | **Mock planner**: keyword rules (scoped lift / soften / accent, last four bars, darker / brighter, room, energy, kick level, glass melody, variations, a new sound from words) that build patches through the patch builder. |
| `src/engine/mock/verify.ts` | Mock checks shaped like `verify_render`: loudness, peak, clipping, loop seam and silence measured on the render; tempo, key and grid from the score (labelled as such). |
| `src/engine/mock/apply.ts`, `validate.ts`, `synth.ts` | Patch rules (locks, scope, copy-on-write, exact inverses, insert positions, write_bars ids), a validator subset, and the placeholder synth (seeded note phases, per-layer low-pass, loudness-target or held master gain). |
| `src/engine/wav.ts` | WAV 16/24/32f encode and decode. |
| `src/score/*` | Generated types, theory, timeline, patch builder, derived pattern ids (unchanged in spirit from WP-U1). |
| `src/ui/shell.ts` | The three-column shell: sounds list and agents, thread (take cards, compact takes, replies, making indicator), composer with scope and clip chips, suggestion chips, Sound panel with tabs, phone sheet, playback of each take's render. |
| `src/ui/changeMapModel.ts`, `src/ui/checks.ts` | Pure models: change-map cells from `take.changed`, the checks summary from the report. |
| `src/ui/views/*` | Panel views: change map (drag to scope), Overview, Rhythm, Notes, Mix and Export. |
| `src/ui/waveform.ts`, `src/ui/spectrogram.ts` | Waveform peaks from a take's render, highlighted from `changed`; FFT spectrogram. |
| `tests/unit/*` | Grid maths, patch building, mock rules and inverses, schemas, takes (demo, change map, scoped asks, refusals, restore and undo, checks summary). |
| `tests/e2e/workstation.spec.ts` | The brief's Playwright tests at 1440 and 390. |

## How a request becomes a take

The composer (or a chip, a Feel macro, an agent) calls `adapter.ask({text, scope?, clipFile?})`.
The planner turns it into a `ScorePatch`; when the person dragged bars, the patch scope is exactly
those bars and that layer. The engine applies it under the lock and scope rules, renders, checks
and returns a `Take`. A hand edit in Rhythm or Notes goes through `adapter.edit(patch, ...)` the
same way. Mute, solo and lock are patches on the live score (undoable, no take). A refused request
(for example, a change on a locked layer) gets a short reply in the thread and no take.

## Mocked or not yet real

- **The planner is a keyword mock.** It understands the requests listed above and says so for
  anything else; the real planner (WP-P/P2) understands plain language.
- **Audio is a placeholder.** Sine tones and simple drum clicks so playback, waveforms and the
  spectrogram work. Loudness is an unweighted RMS estimate and peak is sample peak, labelled in
  the Mix view. Reverb is not rendered.
- **Checks** measure the mock render for loudness, peak, clipping, loop seam and silence; tempo,
  key and grid come from the score.
- **Clips** are accepted and named, but analysis (WP-D) is not in the mock: the reply says so.
- **Export** records the request and lists the files it would write.
- **Agents** are two mock entries; `praxis.engine.agentAsk(name, text)` in the console stands in
  for the MCP server (WP-X). Switching sounds restarts that sound's conversation.
- The demo sound unlocks the felt piano so pointed edits on it work; lock it in Overview and such
  requests are refused, as the contract requires.

## Contract notes (for the orchestrator)

1. The UI needs an `edit(patch, ask, scope?)` call for hand edits from the Rhythm and Notes views
   (a take from an explicit patch); the brief lists only `ask`. It is in `EngineAdapter`.
2. `ask` returns `AskResult = {ok: true, take} | {ok: false, message, error?}` rather than a bare
   `Take`, so a refusal (lock rule, unplannable request) reaches the thread in words.
3. Undo/redo of takes lives in the adapter (`undo()`, `redo()`), next to `restore(n)`, because
   agents make takes too; the MCP `praxis_takes` stream should carry the same history.

## Dependencies (all dev-only; nothing is bundled into the app)

| Package | Version | Licence | Released |
| --- | --- | --- | --- |
| vite | 8.3.5 | MIT | 2026-10-21 |
| typescript | 7.0.2 | Apache-2.0 | 2026-07-08 |
| vitest | 5.0.5 | MIT | 2026-10-15 |
| @playwright/test | 1.64.1 | Apache-2.0 | 2026-10-14 |

Transitive tree (40 packages): 29 MIT, 7 Apache-2.0, 1 ISC (`picocolors`), 1 BSD-3-Clause
(`source-map-js`), and 2 MPL-2.0 (`lightningcss` and its platform binary, Vite's CSS minifier,
used at build time only and not shipped in `dist/`).
