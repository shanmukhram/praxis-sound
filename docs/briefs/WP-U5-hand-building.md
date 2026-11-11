# Brief: WP-U5 build a sound by hand in the window (closes Phase 3)

Branch from `build/integration`. Read first: `docs/BRIEFING.md` (all rules), PRD Phase 3 exit
("acceptance test 1 can be built by hand in the window, played without dropouts and exported")
and FR-28, FR-29, FR-31, `docs/adr/0011-conversation-first-workstation.md` (the approved design:
depth on demand, plain words, the style rules, phone layout), `docs/design/workstation-concept.html`
and `docs/design/screenshots/`, `docs/contracts/patch.md` and `crates/praxis-score/src/patch.rs`
(the ops a hand edit can use), `app/src/engine/EngineAdapter.ts`, the UI in `app/src/ui/`, the
desktop adapter `app/src/engine/tauri/` and shell `crates/praxis-app/`, and WP-U4's report notes in
`crates/praxis-app/README.md`. The taste-skill design guidance is installed as Markdown; use it for
any new control so it matches the approved look.

## Why

WP-U4's exit run found the window cannot build acceptance test 1 by hand: there is no way to add a
layer, pick its instrument, set key, tempo, bars or loop/one-shot, and entering a motif costs one
rendered take per click. There is also no seek or loop-range control (the engine supports both).

## You own

`app/src/ui/**`, `app/src/engine/**` (adapter interface additions, MockEngine, Tauri adapter),
`app/tests/**`, `app/screenshots/**`, and `crates/praxis-app/**`. Do not edit `crates/praxis-cli`
(WP-AG is working there): anything the desktop needs beyond the existing ops (for example starting
a blank sound) goes in `crates/praxis-app/src/commands.rs` on the public `Sound`, `SoundLock` and
`Project` API, as WP-U3 did for undo and redo.

## Build (all through ScorePatches, so undo, locks and the shared take history keep working)

- **Start a blank sound** without a request: key, mode, tempo (or bars + target length, using the
  duration solver), time signature, bars, loop or one-shot.
- **Sound settings** in the Overview: change key, tempo, bars, loop/one-shot later.
- **Layers by hand**: add a layer (instrument picked from the presets the engine has, grouped and
  named in plain words, with the felt piano and soft kick easy to find), remove, rename, change
  instrument, set which bars it plays (sections), gain and pan.
- **Writing notes without a take per click**: edits in the Rhythm and Notes views change the live
  score immediately (heard at once through the real-time engine) and are batched; one "Make a
  take" action renders, checks and records them as a single take (with an automatic take after a
  pause or when leaving the view if the owner prefers; pick one and say why). Spoken-rhythm entry
  (DON / din / don / TIN) for a whole bar at once.
- **Transport**: seek by clicking the waveform or change map, a loop-range control that does not
  clash with dragging to set an edit's scope, and play/stop from the keyboard.
- Phone (390 px) keeps working: controls fit the bottom sheet.

## Never play audio on the owner's speakers

Tests use the mock engine or the fake output. The manual run below uses the fake output or Windows
output muted; if you think you need audible playback, stop and ask the owner through your report
instead.

## Done when

- Vitest and Playwright tests (1440 and 390) for each control, plus Rust tests for any new
  `praxis-app` command; existing tests stay green.
- The Phase 3 exit, by hand, in the release app with screenshots: a blank sound (A minor, 128 BPM,
  16 bars, loop); a felt-piano layer with the motif entered in Rhythm/Notes; a soft-kick layer in
  bars 9-16; play 60 s through the real-time engine (fake output or muted) with 0 xruns; export
  WAV, FLAC, stems and MIDI; `praxis check` passes on the WAV.
- `cargo fmt --all --check`, `cargo clippy -p praxis-app --all-targets -- -D warnings`,
  `cargo test -p praxis-app`, `npx tsc --noEmit`, `npx vitest run`, Playwright e2e.
- Report as in docs/BRIEFING.md, with screenshots of each new control at 1440 and 390.
