# Praxis Sound workstation front end (`app/`)

The workstation UI (FR-28 to FR-32) as a TypeScript + Vite web app with canvas components and no
framework (ADR-0002). In this package it runs in a browser against a **mock engine adapter**; the
Tauri IPC adapter (desktop) and the wasm adapter (browser build) replace the mock later behind the
same `EngineAdapter` interface.

```bash
npm install
npm run dev          # http://localhost:5173 (loads fixtures/acceptance1.score.json)
npm test             # Vitest unit tests
npm run test:e2e     # Playwright smoke tests (starts the dev server itself)
npm run typecheck
npm run build        # dist/
npm run gen:types    # regenerate src/score/generated.ts from ../schema/*.schema.json
```

`?fixture=acceptance1_bell` (or `empty`) picks another fixture; `?theme=light|dark` forces a theme.
Playwright uses its bundled Chromium (`npx playwright install chromium` once), or an installed
browser with `PW_CHANNEL=msedge` / `PW_CHANNEL=chrome`.

## What is here

| Path | What it does |
| --- | --- |
| `scripts/gen-types.mjs` | Dependency-free generator: `schema/score.schema.json` + `schema/patch.schema.json` -> `src/score/generated.ts`. A unit test fails if the file is stale. |
| `src/score/generated.ts` | Generated Score, ScorePatch, PatchOp, ... types (do not edit). |
| `src/score/types.ts` | Normalised types (schema defaults filled), `HistoryEntry`/`Version`/`Project` mirrors of `project.rs`, helpers. |
| `src/score/theory.ts` | Keys, scales, grids, steps per bar, accents; ported from `theory.rs`/`timeline.rs`. |
| `src/score/timeline.ts` | Bar/second mapping and `expandNotes` (port of `timeline::expand_notes`, keeps pattern phase). |
| `src/score/patchBuilder.ts` | UI intent -> `ScorePatch` with exact scope: mute/solo/lock, tempo, clear bars, step and piano-roll edits in a bar range (`write_bars`, or `replace_pattern` when the range owns its pattern), restore-version patches. |
| `src/score/history.ts` | Local undo/redo/versions mirroring the project contract; operations are serialised. |
| `src/engine/EngineAdapter.ts` | The adapter interface, `PatchError` (mirrors `patch.rs`), `RenderReport` (mirrors `api.rs`, plus `mock`). |
| `src/engine/MockEngine.ts` | Mock adapter: patch rules, placeholder render to a WAV blob URL, WebAudio transport with loop, position and meter streams, export recording. |
| `src/engine/mock/apply.ts` | Mock `apply`: atomic, locks, scope, copy-on-write splits with `pattern_offset_steps`, exact inverses, validation. |
| `src/engine/mock/validate.ts` | Subset of `validate.rs` (ranges, ids, grid, overlap, off-scale, avoid-list). |
| `src/engine/mock/synth.ts` | **Mock preview** synthesis (sine tones, click/thump drums). Not Praxis instruments. |
| `src/engine/wav.ts` | WAV 16/24/32f encode and decode. |
| `src/ui/app.ts` | Shell, controller, layout, keyboard shortcuts. |
| `src/ui/timeline.ts` | Canvas timeline: ruler, sections, layer lanes, clips with note ticks, edit-range selection, playhead; DOM layer headers with M/S/L. |
| `src/ui/stepSequencer.ts` | Step grid per pitch lane, velocity row, accent row (DON/din/don/TIN), note inspector with length % (overlap above 100% shown). |
| `src/ui/pianoRoll.ts` | Canvas piano roll: key rows highlighted, tonic marked, off-scale notes flagged; add, select, resize, delete. |
| `src/ui/analysis.ts` | Waveform and log-frequency spectrogram (own FFT) of the adapter's render, render report. |
| `src/ui/sidePanel.ts` | History list, Undo/Redo, named versions, last patch JSON with scope. |
| `src/ui/exportDialog.ts` | WAV 16/24/32f, FLAC, stems, MIDI, project; calls `adapter.export`. |
| `tests/unit/*` | Grid maths, patch building and scope, mock engine rules and inverses, history, schemas. |
| `tests/e2e/smoke.spec.ts` | Playwright: acceptance 1 loads (16 bars, 2 layers, sections), M/S/L, scoped step edit in bars 13-16, undo/redo, versions, piano roll, export, transport. |
| `screenshots/` | Screenshots written by the e2e run. |

## How edits work

Every edit is a `ScorePatch`; nothing mutates the Score directly. Views call
`Controller.edit(score => patch)`, which waits for earlier edits to land, builds the patch from the
current score, and sends it through `History.commit` -> `EngineAdapter.applyPatch`. The adapter
returns the new score and the exact inverse (stored for undo) or a `PatchError`, shown in the
status bar and the patch panel. The scope bar shows the next edit's scope (layer + bars) and the
last patch's scope.

Step and piano-roll edits target the selected layer and bar range. If one clip covers the range and
the range is a whole number of pattern repeats, the editors show that pattern (an edit applies to
every repeat in the range); otherwise they show the range note for note. The first edit writes a
new pattern `<layer>_b<start>-<end>` with `write_bars`; later edits of the same range use
`replace_pattern` on it, so no orphan patterns pile up.

## Keyboard

Space play/stop · Home to start · L loop the selected bars · Ctrl+Z undo · Ctrl+Shift+Z or Ctrl+Y
redo · M / S mute / solo the selected layer · R render · E export · 1 / 2 step sequencer / piano roll.

## Mocked or not yet real

- **Audio is a placeholder.** The mock synthesises sine tones and simple drum clicks so transport,
  playhead, waveform and spectrogram work. It is labelled "mock" in the UI and the render report.
  Its loudness is an unweighted RMS estimate and its peak is sample peak, not LUFS/dBTP.
- **Export** only records the request and lists the files it would write.
- The mock implements `set_title`, `set_tempo`, `add_layer`, `remove_layer`, `update_layer`,
  `set_lock`, `add_pattern`, `replace_pattern`, `remove_pattern`, `write_bars`, `clear_bars`,
  `place_clip`, `set_clips`; other ops are rejected with a plain message.
- No Tauri shell (`crates/praxis-app`) and no wasm adapter yet; FR-30's loudness-matched A/B
  against a reference clip is not built (needs reference analysis).

## Contract notes (for the orchestrator)

1. **Mute/solo on a locked layer.** `muted`/`solo` are Score layer fields, so by rule 2 a patch
   that mutes a locked layer fails with `LockedLayer` (the acceptance-1 piano is locked). FR-28
   expects mute and solo on every layer. Proposal: exempt monitoring fields in rule 2 of
   `docs/contracts/patch.md`: "...except a user `set_lock` unlock, and a user `update_layer`
   whose update sets only `muted` and/or `solo`." The UI already sends exactly that patch.
2. **`add_layer` / `add_pattern` have no position**, so the inverse of removing a layer or pattern
   that is not last cannot restore order and rule 5 (structural equality) fails. Proposal in
   `patch.rs`: `AddLayer { layer, #[serde(default, skip_serializing_if = "Option::is_none")] index: Option<u32> }`
   and the same for `AddPattern` (insert position, default append). The mock appends today and a
   unit test documents the gap.
3. **`write_bars` with an existing pattern id** is not specified in `patch.md`; the mock returns
   `DuplicateId` (that variant exists). The UI never sends one.

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
