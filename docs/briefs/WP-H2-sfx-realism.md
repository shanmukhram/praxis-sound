# Brief: WP-H2 make the sound-effect presets sound real (Phase 6 prep)

Branch from `build/integration`. Read first: `docs/BRIEFING.md` (audio-quality rules: restraint,
in key, nothing that sounds like a stock trailer effect), WP-H's report notes in `demos/sfx/README.md`,
`crates/praxis-instruments/src/sfx.rs` and `src/sfx/`, its tests, and the DSP available in
`crates/praxis-dsp` (filters, noise, envelopes, FM, wavetable, the effects).

## Why

WP-H's six presets (whoosh, impact, riser, drone, glitch, tick) meet every measured spec, but the
earshot listening tools heard every demo as code-synthesized (realness 0.01-0.07) and the drone as
an electrical hum. Phase 6 ends with the owner's blind listening review, and the match loop (WP-M)
fits these presets to real reference clips, so they must be convincing.

## Build

- Rework the timbre of each preset toward real recorded effects, with physical modelling ideas
  rather than plain oscillators: layered, band-limited noise with moving resonances and air
  absorption for whoosh; a body resonance with modal partials, a transient with a real attack
  shape and a room tail for impact; a riser built from evolving filtered noise and a slowly
  detuning tonal bed; a drone from several slowly drifting, beating partials with filtered noise
  breath instead of a static hum; glitch from granular slices; tick from a short modal click.
  Gentle randomness through the note seed only. Keep every existing measured property and test
  (pitch, length, macro monotonicity, riser landing, determinism, zero allocations).
- Listen with the earshot tools if your session has them (`realness`, `describe`) after each
  change, on renders written to files; never play audio on the speakers. Aim for realness well
  above 0.5 on each demo, and report before/after per preset.
- Re-render `demos/sfx/` and update its README.

## You own

`crates/praxis-instruments/src/sfx.rs`, `src/sfx/**`, `tests/sfx_*.rs`, `demos/sfx/**`.

## Rules

`PRAXIS_KEYCHAIN=off` on every run; never play audio; 5 jobs / 5 test threads via the worktrees
cargo config, never override.

## Done when

All sfx tests pass (existing and any new), realness and describe readings reported before and
after per preset, demos re-rendered; fmt, workspace clippy, `cargo test -p praxis-instruments
--release` and the wasm build pass. Report as in docs/BRIEFING.md.
