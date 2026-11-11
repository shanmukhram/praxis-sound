# Brief: WP-H procedural sound-effect patches (FR-16)

Branch from `build/integration`. Read first: `docs/BRIEFING.md` (all rules, especially the
audio-thread rules and the owner's audio-quality rules: restraint, always in key, nothing that
sounds like a stock trailer whoosh), `docs/contracts/instruments.md` (preset table and families),
`docs/contracts/engine.md` (instruments), `crates/praxis-engine/src/instrument.rs`, and how the
synth and drum families are built (`crates/praxis-instruments/src/synth.rs` + `synth/`,
`drums.rs` + `drums/`, their tests) so this family looks and tests the same way.

## Goal

Sound effects that sit under music: `whoosh`, `impact`, `riser`, `drone`, `glitch`, `tick`, each a
procedural patch (synthesis, noise, filters, envelopes, the in-house DSP) with four macro knobs,
`length`, `brightness`, `weight`, `pitch`, and tuned to the score's key (`ctx.key`). They are the
generators the match loop (WP-M) will fit to reference clips later, so every macro must move the
sound in a measurable, monotonic way.

## You own

`crates/praxis-instruments/src/sfx.rs`, `crates/praxis-instruments/src/sfx/**`,
`crates/praxis-instruments/tests/sfx_*.rs`, and `demos/sfx/**` (renders and a README).
Nothing else. The planner catalogue already lists these presets; do not edit it.

## Build

- One `PresetEntry` per preset with the contract's class (same name as the preset), `pitched`
  and `sustaining` set honestly, and the four macros as preset parameters (0..1, documented
  defaults). Per-note variation only through the note seed (`NoteOn.seed`), never a global stream.
- Musical behaviour:
  - `riser`: builds over `length` (in beats or seconds, document which) and ends exactly on the
    sample where the note's `length_samples` ends, so an arrangement can land it on a downbeat
    or hit point; pitched parts glide toward the key's tonic or fifth.
  - `impact`: a tuned low body (on the key's tonic) plus a short transient; `weight` sets body
    level and length.
  - `drone`: sustaining, tuned to the note pitch (in key), slow movement, loops cleanly.
  - `whoosh`: band-passed noise with a moving centre, Doppler-like pitch bend; `brightness`
    sets the centre range.
  - `glitch`: short stutters and bit/sample-rate reduction, seeded so it repeats exactly.
  - `tick`: short clicky transient, unpitched or tuned, usable as a clock.
- Restraint by default: moderate levels, no harsh top end, no DC, honest release tails.

## Done when

- Tests (`tests/sfx_*.rs`), measured on real renders through the engine's `Instrument` path:
  every preset renders without clicks at start and end, no NaN/inf, no DC offset; pitched
  presets land within 5 cents of the requested note; `length` sets the duration within 1%;
  `brightness` raises the spectral centroid monotonically; `weight` raises energy below 150 Hz
  monotonically; `riser` peaks within 5 ms of its end; same seed gives bit-identical output and a
  different seed a different take; zero allocations after construction (pattern in
  `tests/synth_alloc.rs`).
- Demo renders in `demos/sfx/` (one per preset at default macros, plus a riser landing on bar 5
  of a 120 BPM A-minor bar grid) with a README saying what each should sound like. Listen-check
  them with the earshot tools if your session has them (`describe`, `realness`), and report what
  they said.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p praxis-instruments --release`, and the wasm build
  (`cargo build -p praxis-wasm --target wasm32-unknown-unknown`) pass.
- Report as in docs/BRIEFING.md.
