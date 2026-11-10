# Brief: WP-C2 synth + drums

Branch: `wp/c2-synth-drums`. Read first: `docs/BRIEFING.md` (binding), `PRD.md`, `docs/PLAN.md`
(WP-C2), `docs/contracts/instruments.md`, `docs/contracts/engine.md`, `docs/contracts/dsp.md`,
frozen code `crates/praxis-engine/src/instrument.rs`, `crates/praxis-instruments/src/{registry.rs,packs.rs,lib.rs}`,
and the merged `crates/praxis-dsp/src/{osc,env,filter,noise,smooth,interp}.rs` you build on.

## Goal and why it matters
FR-21 (synthesis: subtractive, FM, wavetable, plucked and struck physical models, granular, noise)
and FR-22 (synthesised kick, snare, hats, toms). Acceptance test 1 needs a **soft muted kick**
layered under the piano, and Phase 1's test tone is a **bell**. The owner rejected thin, artificial
sounds: presets must be restrained, warm and in tune, with no clicks.

## You own
`crates/praxis-instruments/src/synth.rs`, `src/synth/**`, `src/drums.rs`, `src/drums/**`, tests
`crates/praxis-instruments/tests/synth_*.rs`, `tests/drums_*.rs`. Nothing else.

## Build
- A shared polyphonic voice framework inside `synth/` (preallocated voice pool, quietest-voice
  stealing with a short fade, per-voice seeded variation from `NoteOn.seed`, `brightness` mapped
  to tone) used by every synth preset.
- Synth presets (kind `synth`, names and classes exactly as in docs/contracts/instruments.md):
  `bell` (struck/FM bell, inharmonic partials, long ring, class motif), `soft_keys` (FM electric
  piano, melody), `pluck` (Karplus-Strong with damping, melody), `mallet` (modal struck bar:
  marimba/vibraphone partial ratios, motif), `sub_bass` (sine + slight 2nd harmonic, bass),
  `warm_bass` (subtractive saw/ladder, bass), `fm_keys` (harmony), `glass` (wavetable, motif),
  `granular_texture` (granular over an internal wavetable grain source, texture), `noise_bed`
  (filtered noise, ambience), `warm_pad` (class pad: it exists, but the avoid-list blocks it
  unless asked). Each declares params with ranges (`PresetParam`), `sustaining` correctly.
- Drum presets (kind `drum`): `soft_kick` (soft, muted: pitch-swept sine ~55 Hz settling, short
  felt-like click, low-passed around 2-3 kHz, short decay; restrained), `kick`, `snare`, `rim`,
  `clap`, `hat_closed`, `hat_open` (choked by closed hat in `kit`), `shaker`, `tom_low`, `tom_mid`,
  `tom_high`, and `kit` (GM drum map: 36 kick, 37 rim, 38 snare, 39 clap, 42 closed hat, 46 open
  hat, 41/45/48 toms, 70 shaker). Unpitched presets have `pitched: false`.
- All allocation-free after `create`, deterministic, `tail_samples` honest.

## Tests
Every preset: renders a note without NaN, bounded peak, no click at start/end (max adjacent-sample
jump relative to local RMS), determinism (two renders bit-identical), silence after tail. Pitched
presets: fundamental within 5 cents of the MIDI pitch (or documented inharmonic reference partial
for `bell`) at three pitches. `soft_kick`: spectral energy concentrated below 150 Hz, decay to
-40 dB within 400 ms, little energy above 4 kHz. Hats: energy mostly above 5 kHz. Kit: GM note
mapping and open-hat choke. Velocity changes level monotonically; `brightness` raises the
spectral centroid.

## Report
As in docs/BRIEFING.md "Report back", including measured pitch errors and the soft_kick numbers.
