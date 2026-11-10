# Contract: DSP building blocks (praxis-dsp)

Frozen (orchestrator): `buffer.rs` (StereoBuffer), `rng.rs`, `util.rs`, `traits.rs` (Processor),
`biquad.rs`, `delayline.rs`.

## WP-B1 dsp-core (public API it must provide)

| Module | Items |
| --- | --- |
| `osc` | `Sine`, `PolyBlepSaw`, `PolyBlepSquare` (pulse width), `Triangle`, `Wavetable` (mip-mapped, from harmonic amplitudes), `FmOperator` (2/4-op FM pair helpers). Each: `new(sample_rate)`, `set_freq(hz)`, `next() -> f32`, `reset(phase)` |
| `env` | `Adsr` (linear attack, exponential decay/release, retrigger without click), `Ar` one-shot, `Dahdsr` (SFZ-style). `gate(on)`, `next() -> f32`, `is_idle()` |
| `filter` | `Svf` (Simper/Cytomic TPT: lp/hp/bp/notch, modulation-safe), `OnePole` lp/hp, `Ladder` (4-pole, soft-saturating), `DcBlocker` |
| `noise` | `White` (seeded PCG), `Pink` (Voss-McCartney or Kellet), `Brown` |
| `smooth` | `Smoothed` linear/exponential parameter smoothing |
| `interp` | `hermite4`, `lagrange` helpers, `Resampler` for variable-rate sample playback |

All are `Clone`, allocation-free after `new`, deterministic, and tested (frequency accuracy,
aliasing bounds for BLEP, envelope timing within 1 sample, filter responses).

## WP-B2 dsp-effects (fx/, meter/)

One type per `EffectKind`, each implementing `Processor`, constructed with
`new(sample_rate: f32, max_block: usize)`. **Parameter indices are the row order of
`praxis_score::params::effect_params(kind)`** and units are exactly as listed there.

| EffectKind | Type in `praxis_dsp::fx` |
| --- | --- |
| eq | `Eq4` |
| filter | `FilterFx` |
| compressor | `Compressor` (feed-forward, RMS/peak detector, soft knee, sidechain via `set_sidechain`) |
| saturation | `Saturation` (tanh/asymmetric with 2x oversampling, tilt tone) |
| delay | `Delay` (tempo sync needs `set_tempo(bpm)`; ping-pong; damped feedback) |
| reverb | `FdnReverb` (8-line FDN, Hadamard mix, modulated, decay from RT60) |
| convolution_reverb | `Convolver` (uniform partitioned FFT convolution; `set_impulse(&StereoBuffer)` off the audio thread) |
| chorus | `Chorus` |
| width | `Width` (M/S with mono-below crossover) |
| limiter | `TruePeakLimiter` (4x oversampled peak detection, lookahead, never exceeds ceiling) |
| gain | `GainFx` |

Plus `pub fn create(kind_index: usize, sample_rate, max_block) -> Box<dyn Processor>` in
`fx/mod.rs`, where `kind_index` is the position of the kind in this table (eq = 0 .. gain = 10).

`meter/`: `LoudnessMeter` (BS.1770-4 K-weighting, momentary/short-term/integrated with gating,
LRA), `true_peak_dbtp(&StereoBuffer, sr)` (4x oversampling), `rms_dbfs`. Must agree with the
`ebur128` crate within 0.1 LU on test signals (test against it as a dev reference).

## Conventions

- Stereo processors take `&mut [f32]` left/right of equal length.
- `tail_samples()` is honest (reverb: RT60 x sample rate capped at 30 s; delay: until -90 dB).
- `latency_samples()` is exact (the engine compensates).
