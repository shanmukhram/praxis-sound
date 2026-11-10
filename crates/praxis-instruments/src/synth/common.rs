//! Small building blocks shared by the synth and drum voices: parameter tables, envelopes that
//! end in exact silence, decaying modal partials and pitch helpers.
//!
//! All of them are `Copy`/`Clone`, allocation-free and deterministic. Envelope times are rounded
//! to whole samples, so the same note gives the same samples at any block size.

// `next()` returning a bare value matches the praxis-dsp generator convention.
#![allow(clippy::should_implement_trait)]

use praxis_engine::{InstrumentError, PresetInfo, PresetParam};
use praxis_score::InstrumentRef;
use std::f64::consts::TAU;

/// Most parameters any preset declares.
pub const MAX_PARAMS: usize = 8;

/// Parameter values in [`PresetInfo::params`] order.
pub type Params = [f32; MAX_PARAMS];

/// Builds a [`PresetParam`].
pub fn param(name: &str, min: f64, max: f64, default: f64, unit: &str) -> PresetParam {
    PresetParam {
        name: name.to_string(),
        min,
        max,
        default,
        unit: unit.to_string(),
    }
}

/// Parameter values for `info` from the score's instrument spec: defaults for missing values,
/// clamped to the declared range. Unknown names are an error (the registry checks this too).
pub fn resolve_params(spec: &InstrumentRef, info: &PresetInfo) -> Result<Params, InstrumentError> {
    for name in spec.params.keys() {
        if !info.params.iter().any(|p| &p.name == name) {
            return Err(InstrumentError::UnknownParam {
                preset: spec.preset.clone(),
                param: name.clone(),
            });
        }
    }
    let mut out = [0.0f32; MAX_PARAMS];
    for (slot, p) in out.iter_mut().zip(info.params.iter()) {
        let v = spec
            .params
            .get(&p.name)
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(p.default);
        *slot = v.clamp(p.min, p.max) as f32;
    }
    Ok(out)
}

/// Equal-tempered frequency of a MIDI note (A4 = 69 = 440 Hz).
#[inline]
pub fn midi_hz(pitch: u8) -> f64 {
    440.0 * 2f64.powf((pitch as f64 - 69.0) / 12.0)
}

/// Frequency ratio of `cents`.
#[inline]
pub fn cents(c: f64) -> f64 {
    2f64.powf(c / 1200.0)
}

/// Seconds to whole samples (at least `min`).
#[inline]
pub fn samples(seconds: f32, sr: f64, min: u64) -> u64 {
    let s = if seconds.is_finite() {
        seconds.max(0.0) as f64
    } else {
        0.0
    };
    ((s * sr).round() as u64).max(min)
}

/// Per-sample multiplier that decays by 60 dB in `t60` seconds.
#[inline]
pub fn t60_coeff(t60: f64, sr: f64) -> f64 {
    if t60 <= 0.0 || !t60.is_finite() {
        0.0
    } else {
        (-6.907_755_278_982_137 / (t60 * sr)).exp()
    }
}

/// How much shorter notes ring as they rise: sqrt(middle C / f), clamped.
#[inline]
pub fn key_decay(hz: f64) -> f64 {
    (261.625_565 / hz.max(1.0)).sqrt().clamp(0.3, 2.5)
}

/// Overshoot of the exponential segments that end in exact silence: the curve aims 60 dB past
/// zero, so it sounds like a natural exponential decay but reaches 0 exactly on time.
const OVERSHOOT: f64 = 1e-3;

/// Linear attack ramp `0 -> 1` over a whole number of samples (at least 1).
#[derive(Clone, Copy, Debug, Default)]
pub struct Ramp {
    i: u32,
    n: u32,
}

impl Ramp {
    pub fn start(&mut self, seconds: f32, sr: f64) {
        self.i = 0;
        self.n = samples(seconds, sr, 1).min(u32::MAX as u64) as u32;
    }

    /// The next gain; the `n`-th call returns exactly 1.
    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.i < self.n {
            self.i += 1;
            self.i as f32 / self.n as f32
        } else {
            1.0
        }
    }

    pub fn is_done(&self) -> bool {
        self.i >= self.n
    }
}

/// Release gain: 1 until [`Release::start`], then an exponential-shaped fade that reaches
/// exactly 0 after the release time (so a voice can free itself without a step).
#[derive(Clone, Copy, Debug)]
pub struct Release {
    gain: f64,
    target: f64,
    coeff: f64,
    remaining: u64,
    stage: u8,
}

impl Default for Release {
    fn default() -> Self {
        Release {
            gain: 1.0,
            target: 0.0,
            coeff: 1.0,
            remaining: 0,
            stage: 0,
        }
    }
}

impl Release {
    /// Back to gain 1, not releasing.
    pub fn reset(&mut self) {
        *self = Release::default();
    }

    /// Starts the fade (ignored when already releasing).
    pub fn start(&mut self, seconds: f32, sr: f64) {
        if self.stage != 0 {
            return;
        }
        let n = samples(seconds, sr, 1);
        self.remaining = n;
        self.target = -OVERSHOOT * self.gain;
        self.coeff = (OVERSHOOT / (1.0 + OVERSHOOT)).powf(1.0 / n as f64);
        self.stage = 1;
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            0 => 1.0,
            1 => {
                self.gain = self.target + (self.gain - self.target) * self.coeff;
                self.remaining -= 1;
                if self.remaining == 0 {
                    self.gain = 0.0;
                    self.stage = 2;
                }
                self.gain as f32
            }
            _ => 0.0,
        }
    }

    pub fn gain(&self) -> f32 {
        self.gain as f32
    }

    pub fn is_releasing(&self) -> bool {
        self.stage != 0
    }

    /// The fade has reached 0.
    pub fn is_done(&self) -> bool {
        self.stage == 2
    }
}

/// One-shot envelope for struck and drum sounds: a linear attack to 1, then an
/// exponential-shaped decay that reaches exactly 0 after the decay time. Retriggering attacks
/// from the current level, so it never jumps (used for the bursts of a clap).
#[derive(Clone, Copy, Debug, Default)]
pub struct OneShot {
    level: f64,
    inc: f64,
    target: f64,
    coeff: f64,
    remaining: u64,
    decay_n: u64,
    /// 0 idle (never triggered), 1 attack, 2 decay, 3 finished.
    stage: u8,
}

impl OneShot {
    /// Starts (or restarts from the current level) an attack of `attack_s` and a decay of
    /// `decay_s` seconds.
    pub fn trigger(&mut self, attack_s: f32, decay_s: f32, sr: f64) {
        let a = samples(attack_s, sr, 1) as f64;
        let n = ((1.0 - self.level).max(0.0) * a).round().max(1.0);
        self.inc = (1.0 - self.level) / n;
        self.remaining = n as u64;
        self.decay_n = samples(decay_s, sr, 1);
        self.stage = 1;
    }

    /// Silent and not triggered.
    pub fn clear(&mut self) {
        *self = OneShot::default();
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            1 => {
                self.level += self.inc;
                self.remaining -= 1;
                if self.remaining == 0 {
                    self.level = 1.0;
                    self.stage = 2;
                    let n = self.decay_n;
                    self.remaining = n;
                    self.target = -OVERSHOOT;
                    self.coeff = (OVERSHOOT / (1.0 + OVERSHOOT)).powf(1.0 / n as f64);
                }
            }
            2 => {
                self.level = self.target + (self.level - self.target) * self.coeff;
                self.remaining -= 1;
                if self.remaining == 0 {
                    self.level = 0.0;
                    self.stage = 3;
                }
            }
            _ => {}
        }
        self.level as f32
    }

    pub fn level(&self) -> f32 {
        self.level as f32
    }

    /// Attack or decay still running.
    pub fn is_active(&self) -> bool {
        self.stage == 1 || self.stage == 2
    }
}

/// Held-note contour: a linear attack, then an exponential fall from 1 towards `sustain`
/// (60 dB of the excess in `decay_s`). Pair it with [`Release`] for the note-off.
#[derive(Clone, Copy, Debug, Default)]
pub struct Swell {
    ramp: Ramp,
    excess: f64,
    coeff: f64,
    sustain: f64,
}

impl Swell {
    pub fn start(&mut self, attack_s: f32, decay_s: f32, sustain: f32, sr: f64) {
        self.ramp.start(attack_s, sr);
        self.excess = 1.0;
        self.coeff = t60_coeff(decay_s.max(1e-3) as f64, sr);
        self.sustain = sustain.clamp(0.0, 1.0) as f64;
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        let a = self.ramp.next() as f64;
        let v = a * (self.sustain + (1.0 - self.sustain) * self.excess);
        if self.ramp.is_done() {
            self.excess *= self.coeff;
        }
        v as f32
    }

    /// Attack finished.
    pub fn attacked(&self) -> bool {
        self.ramp.is_done()
    }
}

/// A decaying sine partial computed as a rotating phasor in `f64` (exact frequency, no table).
#[derive(Clone, Copy, Debug, Default)]
pub struct Partial {
    re: f64,
    im: f64,
    cos_w: f64,
    sin_w: f64,
    amp: f64,
    decay: f64,
}

impl Partial {
    /// Starts at `phase` cycles with amplitude `amp`, decaying 60 dB in `t60` seconds. A partial
    /// at or above 0.45 x the sample rate (or with a zero amplitude) stays silent.
    pub fn start(&mut self, hz: f64, sr: f64, phase: f64, amp: f64, t60: f64) {
        if !(hz > 0.0 && hz < 0.45 * sr && amp > 0.0) {
            *self = Partial::default();
            return;
        }
        let w = TAU * hz / sr;
        let p = TAU * phase;
        *self = Partial {
            re: p.cos(),
            im: p.sin(),
            cos_w: w.cos(),
            sin_w: w.sin(),
            amp,
            decay: t60_coeff(t60, sr),
        };
    }

    /// The next sample.
    #[inline]
    pub fn next(&mut self) -> f64 {
        if self.amp == 0.0 {
            return 0.0;
        }
        let y = self.amp * self.im;
        let re = self.re * self.cos_w - self.im * self.sin_w;
        let im = self.re * self.sin_w + self.im * self.cos_w;
        self.re = re;
        self.im = im;
        self.amp *= self.decay;
        if self.amp < 1e-9 {
            self.amp = 0.0;
        }
        y
    }

    /// Current amplitude.
    #[inline]
    pub fn amp(&self) -> f64 {
        self.amp
    }
}

/// Balance gains for a small stereo offset `pan` in `-1..=1`: the centre is unity on both sides
/// and the far side dips (no level change for centred voices).
#[inline]
pub fn balance(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    ((1.0 - p).min(1.0), (1.0 + p).min(1.0))
}

/// Combined tone amount `0..=1` from the preset's `tone` parameter, the note velocity and the
/// bright-accent flag.
#[inline]
pub fn tone_amount(tone: f32, velocity: f32, brightness: f32) -> f32 {
    (tone + 0.35 * (velocity - 0.6) + 0.45 * brightness).clamp(0.0, 1.0)
}

/// Velocity curve for levels: `v^1.5`, so soft notes stay soft.
#[inline]
pub fn vel_gain(velocity: f32) -> f32 {
    velocity.clamp(0.0, 1.0).powf(1.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_reaches_zero_exactly_on_time() {
        let sr = 48_000.0;
        let mut r = Release::default();
        assert_eq!(r.next(), 1.0);
        r.start(0.01, sr);
        let mut last = 1.0;
        for i in 0..480 {
            let g = r.next();
            assert!(g <= last, "monotonic at {i}");
            last = g;
        }
        assert!(r.is_done());
        assert_eq!(r.gain(), 0.0);
    }

    #[test]
    fn oneshot_attack_and_decay_timing() {
        let sr = 48_000.0;
        let mut e = OneShot::default();
        e.trigger(0.001, 0.1, sr);
        let v: Vec<f32> = (0..48 + 4800 + 2).map(|_| e.next()).collect();
        assert!((v[47] - 1.0).abs() < 1e-6);
        assert!(v[48 + 4799] == 0.0);
        assert!(!e.is_active());
        // Retrigger mid-decay rises from the current level.
        let mut e = OneShot::default();
        e.trigger(0.001, 0.1, sr);
        for _ in 0..1000 {
            e.next();
        }
        let before = e.level();
        e.trigger(0.001, 0.1, sr);
        let after = e.next();
        assert!(after >= before && after - before < 0.05);
    }

    #[test]
    fn partial_frequency_is_exact() {
        let sr = 48_000.0;
        let mut p = Partial::default();
        p.start(1000.0, sr, 0.0, 1.0, 1e9);
        // 48 samples = one full cycle at 1 kHz: back to phase 0.
        let v: Vec<f64> = (0..49).map(|_| p.next()).collect();
        assert!(v[0].abs() < 1e-12 && v[48].abs() < 1e-9);
        assert!((v[12] - 1.0).abs() < 1e-6);
    }
}
