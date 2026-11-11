# Brief: WP-R real-time engine and device output (Phase 3)

Branch `wp/r-realtime` from `build/integration`. Read first: `docs/BRIEFING.md` (audio-thread
rules especially), `docs/contracts/engine.md` (sections "Offline render" and "Real time"),
`docs/contracts/audio-io.md` (`device`), `docs/adr/0007-edit-isolation-and-loudness.md`, and the
offline renderer in `crates/praxis-engine/src/` (`render.rs`, `graph.rs`, `sequencer.rs`,
`master.rs`, `loopwrap.rs`).

## Goal

The same engine that renders offline plays in real time on Windows through cpal, with a
transport the app can drive, edits swapped in while playing, and no dropouts at a 256-frame
buffer (PRD FR-27, success criterion "Real-time preview").

## You own

- `crates/praxis-engine/src/realtime/**` (new) and `crates/praxis-engine/tests/realtime_*.rs`.
  You may add exactly one line `pub mod realtime;` (and its `pub use`) to
  `crates/praxis-engine/src/lib.rs`.
- `crates/praxis-audio-io/src/device.rs` (+ `src/device/**`), `crates/praxis-audio-io/tests/device_*.rs`,
  `crates/praxis-audio-io/examples/**`, and dev-dependencies in `crates/praxis-audio-io/Cargo.toml`
  (internal crates only; `cpal` and `rtrb` are already workspace deps).
- In the existing engine modules you may only widen visibility (`pub(crate)`) or split a
  function so the per-block path can be shared with the real-time engine. Offline output must
  stay bit-identical: the engine golden and determinism tests must pass unchanged.

## Build

- `RealtimeEngine` owned by the audio callback. Commands in through an `rtrb` queue (play, stop,
  seek, set loop range, layer gain / mute / solo, swap prepared graph); position, meters
  (peak/RMS per channel) and an xrun/late-callback counter out through another. Graphs are
  prepared off the audio thread (`prepare(score, factory, ...)`) and swapped by moving a boxed
  graph through the queue; the old graph goes back through a return queue to be dropped off the
  audio thread. A short crossfade on swap so an edit while playing does not click.
- Callbacks of any size: the engine keeps its fixed internal block (128) and events still land on
  their exact sample.
- Master: the real-time path uses the frozen or last applied gain and the same limiter; it never
  re-measures loudness while playing.
- Loop playback wraps at the loop end with tails continuing, like a looping player.
- `praxis_audio_io::device`: open the default (or a named) output at a requested buffer size
  (256 default), list devices, run a callback that pulls from the engine, count xruns.
- An example `cargo run -p praxis-audio-io --release --example play -- <score or project>` that
  plays a score through the speakers with the real instruments (packs via `PRAXIS_PACKS`), for
  the owner's hardware check at the Phase 3 exit.

## Done when

- Equivalence: with the master gain fixed, pulling the real-time engine in 256-frame (and odd,
  e.g. 97-frame) callbacks from bar 1 of `fixtures/acceptance1_bell.score.json` (one-shot
  playback) reproduces the offline render sample for sample (bit-identical, or explain exactly
  why not and bound the difference).
- No allocation on the audio thread: an allocation-counting test (see
  `crates/praxis-dsp/tests/core_alloc.rs` for the pattern) around process, every command, and a
  graph swap.
- A 60 s simulated soak at 256 frames / 48 kHz with the acceptance-1 score (felt piano when the
  pack is present, bell otherwise): the worst callback's render time stays well under the
  5.33 ms budget in release (report max and 99th percentile); a real-device soak test that runs
  only when `PRAXIS_DEVICE_TEST=1` and reports xruns = 0.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p praxis-engine --release`, `cargo test -p praxis-audio-io --release`, the wasm
  build (`cargo build -p praxis-wasm --target wasm32-unknown-unknown`; the engine's realtime
  module must stay wasm-safe: no threads, no `std::time`) pass.
- You ran the `play` example on this Windows machine and say what you heard reported (device,
  buffer size, xruns). Report as in docs/BRIEFING.md.
