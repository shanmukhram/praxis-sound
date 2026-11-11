# Brief: WP-D1b analyzer onset robustness

Branch `wp/d1b-onsets` from `build/integration`. Read first: `docs/BRIEFING.md`,
`docs/contracts/sound-profile.md` (render verification), `crates/praxis-analyze/src/verify.rs`,
`crates/praxis-analyze/src/features.rs` and `features/`, and `crates/praxis-cli/tests/acceptance1.rs`.

## Problem

Acceptance test 1 (felt piano + soft kick + room reverb, `demos/phase-2/acceptance1_mixed_loop.flac`)
passes the grid check at 90.9% against a 90% threshold. Two causes were seen:

1. The onset detector counts a soft felt-piano note's second swell (pitch 60, velocity 0.55),
   made more visible by the room reverb, as an extra onset off the grid.
2. Eight soft notes read about 12 ms late.

Find out which part of each is the detector and which is the audio. Use the stems
(`praxis render --stems`) and the score's note times to measure where each note's attack really
starts. If notes are truly late in the audio, report it with numbers (that would be a sampler
issue for another package); do not hide it in the analyzer.

## You own

`crates/praxis-analyze/src/features.rs`, `crates/praxis-analyze/src/features/**`, the body of
`crates/praxis-analyze/src/verify.rs` (types and thresholds are frozen), and
`crates/praxis-analyze/tests/verify_*.rs`, `tests/onset_*.rs`.

Do not change the check thresholds in contracts/sound-profile.md (90% within 10 ms), the reverb,
the instruments or the engine.

## Build

- A more robust onset detector for verification: for example a log-compressed multi-band
  spectral flux with adaptive thresholding and a minimum inter-onset interval, onset time refined
  backwards to the attack start (not the flux peak), and suppression of slow swells (energy rises
  much slower than an attack) and of reverb build-up.
- Keep the synthetic tests passing (click trains at known BPM, keys, deliberate seam clicks,
  clipping) and the tempo/key estimates unchanged or better.

## Done when

- New tests: a synthetic soft, slowly swelling sustained note under a reverb-like decay counts
  as one onset; a soft attack is timed within 2 ms of its true start; a real render of the
  felt piano stem from `fixtures/acceptance1.score.json` puts at least 97% of onsets on the grid
  (skip with a clear message when the piano pack is absent).
- `cargo test -p praxis-cli --release --test acceptance1 -- --nocapture` passes with a grid
  figure clearly above the 90.9% baseline (report the number and the median deviation), with
  `PRAXIS_PACKS` set to `C:\Users\admin\Desktop\praxis sound\packs`.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p praxis-analyze --release`, and the wasm build
  (`cargo build -p praxis-wasm --target wasm32-unknown-unknown`) pass.
- Report as in docs/BRIEFING.md, with before/after numbers for every acceptance check.
