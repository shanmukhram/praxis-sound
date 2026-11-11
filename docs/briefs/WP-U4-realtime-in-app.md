# Brief: WP-U4 real-time playback in the desktop app (Phase 3 exit)

Branch from `build/integration`. Read first: `docs/BRIEFING.md` (all rules), PRD Phase 3 exit,
`docs/adr/0011-conversation-first-workstation.md`, `docs/contracts/engine.md` ("Real time"), the
real-time engine in `crates/praxis-engine/src/realtime/` (start at `mod.rs`), the device layer
`crates/praxis-audio-io/src/device.rs` and `examples/play.rs` (how WP-R drove it), the desktop
shell `crates/praxis-app/` (README, `shell.rs`, `commands.rs`) and the adapter
`app/src/engine/tauri/TauriEngine.ts` with the transport part of
`app/src/engine/EngineAdapter.ts` (`play`, `stop`, `seek`, `setLoop`, `onPosition`, `onMeters`).

## Goal

The desktop app plays through the real-time engine instead of an `<audio>` element: play, stop,
seek and loop ranges are sample-accurate, mute / solo / gain changes are heard immediately, a new
take or a hand edit is swapped in while playing without a click, and the level meters move. Then
pass the PRD's Phase 3 exit: **acceptance test 1 built by hand in the window, played without
dropouts, and exported.**

## You own

- `crates/praxis-app/**` (add `praxis-engine`, `praxis-audio-io` and `praxis-instruments` to its
  `Cargo.toml`; all are workspace deps already).
- `app/src/engine/tauri/**` and `app/tests/unit/tauri*.test.ts`.
- `app/src/ui/shell.ts`, only for the "agents connected" fix below.

Do not edit the engine, audio-io or CLI crates. If the real-time API is missing something you
need, stop and say so in your report with the exact change. Do not enable cpal's `realtime`
feature: it pulls in `audio_thread_priority` (MPL-2.0), which the owner ruled out.

## Build

- An audio service in `praxis-app` that owns the cpal output stream on its own thread and the
  `EngineControl` side of the real-time engine. Graphs are prepared off the audio thread with the
  sound's frozen gain and the same instrument registry the renders use (`PRAXIS_PACKS`).
- Tauri commands `transport_play`, `transport_stop`, `transport_seek`, `transport_set_loop`, layer
  mute / solo / gain, and a graph swap whenever the live score changes (new take, restore, hand
  edit, undo, redo). Position and meters are emitted as events about 30 times a second.
- `TauriEngine` uses those instead of `<audio>`; the waveform playhead follows the engine's
  position. If no output device can be opened, fall back to the `<audio>` file playback and show
  a plain message, never a crash.
- Device errors and xruns are counted and visible (a small status line or the Mix view).
- `shell.ts`: re-read the adapter's connected agents whenever the take state changes, so the
  sidebar stops saying "0 agents connected" while an agent is working.

## Done when

- Rust tests for the audio service without a device (drive the engine through the same command
  path with a fake output) and with one when `PRAXIS_DEVICE_TEST=1`; app unit tests with the fake
  bridge for transport, meters and swap-on-take.
- A manual Phase 3 exit run in the release app on this Windows machine, with screenshots in
  `crates/praxis-app/screenshots/`:
  1. Build acceptance test 1 **by hand** in the window: a new sound, a felt-piano layer with the
     motif entered in the Rhythm / Notes views, a soft-kick layer in bars 9-16, A minor, 128 BPM,
     16 bars, loop. (If something the PRD needs for this is impossible in the UI, say exactly
     what, and do the rest.)
  2. Play it through the real-time engine for at least 60 s with loop on; report xruns (must be
     0), callback sizes and the meters.
  3. Mute and solo while playing, make a scoped edit by asking, and confirm the swap is heard
     without a click (earshot tools if your session has them, on a recorded export).
  4. Export WAV, FLAC, stems and MIDI, and run `praxis check` on the WAV (all checks pass).
- `cargo fmt --all --check`, `cargo clippy -p praxis-app --all-targets -- -D warnings`,
  `cargo test -p praxis-app`, `npx tsc --noEmit`, `npx vitest run` and the Playwright e2e in
  `app/` pass.
- Report as in docs/BRIEFING.md, including anything the owner should listen for in their own
  real-hardware check (NEEDS_OWNER item 3).
