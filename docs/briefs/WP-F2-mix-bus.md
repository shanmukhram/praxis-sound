# Brief: WP-F2 engine mix bus

Branch: `wp/f2-mix-bus`. Read first: `docs/BRIEFING.md` (binding), `PRD.md` (section 2:
loudness within 0.5 LU, true peak never above the ceiling; loop seam), `docs/PLAN.md` (WP-F2),
`docs/contracts/engine.md`, `docs/contracts/dsp.md`, `docs/contracts/patch.md` (audio isolation),
`docs/adr/0007-edit-isolation-and-loudness.md`, frozen `crates/praxis-engine/src/{api.rs,instrument.rs}`,
`crates/praxis-score/src/params.rs`, and the merged engine (`crates/praxis-engine/src/`) and
effects (`crates/praxis-dsp/src/fx/`, `meter/`).

## Goal and why it matters
"Output is mixed: balanced levels, controlled low end, loudness target met, no clipping" and
"Film-grade output ... a proper mix bus, loudness targets and seamless loops" (PRD goal 4). You
wire WP-B2's effects into the engine: layer effect chains from `Effect` specs (FR-23), effect
parameter automation (FR-24), compressor sidechain keyed by another layer, latency compensation,
and the master bus (FR-25): master effects, loudness normalisation to `master.loudness.target_lufs`
measured with `praxis_dsp::meter::LoudnessMeter`, `frozen_gain_db` handling, and the true-peak
limiter at `true_peak_dbtp`, all inside the steady-state loop wrap so loops stay seamless.

## You own
Engine modules for effects, sidechain, master and loudness (`crates/praxis-engine/src/effects.rs`
and any new `master.rs`, `sidechain.rs`, `latency.rs`), edits inside `render.rs` needed to call
them, `pub mod` lines in `crates/praxis-engine/src/lib.rs`, tests `crates/praxis-engine/tests/mix_*.rs`,
golden records `tests/golden/mix_*.json`.

## Build
- `build_effect`: map `EffectKind` to `praxis_dsp::fx::create(kind_index, ...)`, set every param
  from the spec (defaults from `params.rs`), `set_tempo` for delays, impulse responses for
  `convolution_reverb` from a named IR (built-in synthetic IRs `room_small`, `room_medium`,
  `hall` generated deterministically from seeded noise with exponential decay; real IRs later).
  Bypassed effects skip. Unknown impulse -> warning, effect skipped.
- Automation targets `effects.<id>.<param>` and `gain_db`/`pan`, evaluated per block, values
  clamped to spec ranges.
- Sidechain: render the key layer's instrument output first; feed `set_sidechain` before each
  `process`; detect cycles (error).
- Latency compensation so stems stay aligned with the grid.
- Master: sum -> master effects -> gain (frozen or measured) -> limiter. Measurement: integrated
  loudness of the captured cycle (loops) or the whole render (one-shots) before the limiter;
  gain = target - measured; apply gain then limit; re-measure and, if the limiter pulled loudness
  more than 0.3 LU below target, iterate (max 3) with a corrected gain. Report before/after
  loudness, true peak, applied gain. Loops: the limiter and master effects run over warm-up cycles
  like the layers, so the captured cycle is steady state.
- Default master chain when `master.effects` is empty: none (the planner adds a gentle bus EQ/
  glue compressor when it writes scores); the limiter and normalisation always run unless
  `normalize` is false (then limiter only).

## Tests
Loudness of renders within 0.5 LU of target for several targets (-23, -16, -14) and signals;
true peak <= ceiling + 0.05 dB on hot material; frozen gain leaves untouched bars bit-identical
after a bars 13-16 edit (master-level isolation test with the documented tail window); sidechain
ducking measurable on the piano when the kick plays; latency compensation keeps a delayed path
aligned to the sample; loop seam still clean with reverb + limiter on (no click at the wrap,
reverb tail present at sample 0); determinism golden `mix_acceptance1_bell`.

## Report
As in docs/BRIEFING.md "Report back", including measured LUFS/true-peak numbers.
