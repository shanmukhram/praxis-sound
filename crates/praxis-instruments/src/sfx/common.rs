//! Building blocks shared by the sound-effect voices: the four macros, key-aware tuning,
//! an additive harmonic oscillator, a phasor that can lock to a note's length (for drones that
//! loop exactly) and smooth envelope shapes that end in exact silence.
//!
//! Everything here is `Copy`, allocation-free and deterministic: envelope positions are whole
//! samples counted on the voice's own clock, so output never depends on the block size.

use crate::synth::common::{Params, param};
use praxis_dsp::filter::DcBlocker;
use praxis_engine::{PresetInfo, PresetParam};
use praxis_score::{GeneratorClass, InstrumentKind, Key};
use std::f64::consts::{FRAC_PI_2, TAU};

/// Filter coefficients (one `tan` each) are updated every this many samples.
pub const UPDATE: u32 = 16;

/// Most harmonics an additive [`Tone`] sums.
pub const MAX_HARMONICS: usize = 12;

/// Highest frequency any tuned partial reaches (restraint: no harsh top end).
pub const PARTIAL_CEILING_HZ: f64 = 9_000.0;

/// Ceiling for noise filters and output low-passes.
pub const NOISE_CEILING_HZ: f64 = 11_000.0;

/// How much a bright accent (`NoteOn.brightness` 1) adds to the `brightness` macro.
pub const ACCENT_BRIGHTNESS: f32 = 0.25;

/// Lowest and highest MIDI pitch of the low ("weight") layer: E1..D#2, 41..78 Hz.
pub const LOW_LAYER_MIDI: (u8, u8) = (28, 39);

/// The four macros every sound effect has, each `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Macros {
    pub length: f32,
    pub brightness: f32,
    pub weight: f32,
    pub pitch: f32,
}

impl Macros {
    /// Macros from resolved parameter values (in [`macro_params`] order).
    pub fn from_params(p: &Params) -> Macros {
        Macros {
            length: p[0],
            brightness: p[1],
            weight: p[2],
            pitch: p[3],
        }
    }

    /// Applies a parameter change by index (`length`, `brightness`, `weight`, `pitch`).
    pub fn set(&mut self, index: usize, value: f32) {
        let v = if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.5
        };
        match index {
            0 => self.length = v,
            1 => self.brightness = v,
            2 => self.weight = v,
            3 => self.pitch = v,
            _ => {}
        }
    }

    /// Brightness for one note: the macro plus the bright-accent flag.
    pub fn note_brightness(&self, accent: f32) -> f32 {
        (self.brightness + ACCENT_BRIGHTNESS * accent.clamp(0.0, 1.0)).clamp(0.0, 1.0)
    }
}

/// Default macro values (`length`, `brightness`, `weight`, `pitch`).
pub const DEFAULTS: [f64; 4] = [0.5, 0.5, 0.5, 0.5];

/// The four macro parameters, each `0..1`, with their defaults.
pub fn macro_params(defaults: [f64; 4]) -> Vec<PresetParam> {
    vec![
        param("length", 0.0, 1.0, defaults[0], ""),
        param("brightness", 0.0, 1.0, defaults[1], ""),
        param("weight", 0.0, 1.0, defaults[2], ""),
        param("pitch", 0.0, 1.0, defaults[3], ""),
    ]
}

/// Registry description of one sound effect.
pub fn info(
    name: &str,
    class: GeneratorClass,
    pitched: bool,
    sustaining: bool,
    range: (u8, u8),
) -> PresetInfo {
    PresetInfo {
        kind: InstrumentKind::Sfx,
        name: name.to_string(),
        class,
        pitched,
        sustaining,
        range,
        params: macro_params(DEFAULTS),
        source: "praxis sfx (procedural)".into(),
    }
}

/// Corner of the DC blockers on low layers, Hz.
pub const DC_HZ: f32 = 20.0;

/// A DC blocker for a low layer (decaying sines and low-passed noise carry DC otherwise).
pub fn dc_blocker(sample_rate: f32) -> DcBlocker {
    let mut d = DcBlocker::new(sample_rate);
    d.set_cutoff(DC_HZ);
    d
}

/// Exponential map of `p` in `0..=1` onto `lo..=hi` (equal ratios per equal step).
#[inline]
pub fn expmap(p: f32, lo: f64, hi: f64) -> f64 {
    lo * (hi / lo).powf(p.clamp(0.0, 1.0) as f64)
}

/// Seconds to whole samples, at least `min`.
#[inline]
pub fn to_samples(seconds: f64, sr: f64, min: u64) -> u64 {
    let s = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    ((s * sr).round() as u64).max(min)
}

/// Equal-tempered frequency of a (possibly fractional) MIDI pitch.
#[inline]
pub fn midi_hz(pitch: f64) -> f64 {
    440.0 * 2f64.powf((pitch - 69.0) / 12.0)
}

/// Whole steps the `pitch` macro moves a tuned part: `round((pitch - 0.5) * 2 * max)`, so 0.5 is
/// no change and 0 / 1 are `-max` / `+max` steps.
#[inline]
pub fn pitch_steps(pitch: f32, max: i32) -> i32 {
    ((pitch.clamp(0.0, 1.0) as f64 - 0.5) * 2.0 * max as f64).round() as i32
}

/// Continuous colour shift of untuned parts (noise bands): one octave down at `pitch` 0, one up
/// at 1.
#[inline]
pub fn colour_shift(pitch: f32) -> f64 {
    2f64.powf((pitch.clamp(0.0, 1.0) as f64 - 0.5) * 2.0)
}

/// Moves `pitch` by `steps` notes of the key's scale (snapping it into the key first), clamped
/// to MIDI 0..=127. In a chromatic key a step is a semitone.
pub fn step_in_key(key: &Key, pitch: u8, steps: i32) -> u8 {
    let mut p = key.snap(pitch.min(127)) as i32;
    let dir = steps.signum();
    for _ in 0..steps.unsigned_abs().min(64) {
        let mut q = p + dir;
        while (0..=127).contains(&q) && !key.contains(q as u8) {
            q += dir;
        }
        if !(0..=127).contains(&q) {
            break;
        }
        p = q;
    }
    p as u8
}

/// Pitch class (0..12) of the key's tonic.
#[inline]
pub fn tonic_pc(key: &Key) -> i32 {
    key.tonic.semitone() as i32
}

/// Pitch class of the key's fifth: a perfect fifth above the tonic when the scale has one,
/// otherwise the nearest scale note to it (the diminished fifth of Locrian).
pub fn fifth_pc(key: &Key) -> i32 {
    let t = tonic_pc(key);
    let p = 60 + t + 7;
    (key.snap(p as u8) as i32).rem_euclid(12)
}

/// The tonic nearest `pitch` (ties resolve downward).
pub fn tonic_near(key: &Key, pitch: u8) -> u8 {
    let p = pitch.min(127) as i32;
    let below = p - (p - tonic_pc(key)).rem_euclid(12);
    let above = below + 12;
    let t = if p - below <= above - p { below } else { above };
    t.clamp(0, 127) as u8
}

/// Pitch class `pc` placed in the low-layer register ([`LOW_LAYER_MIDI`]).
#[inline]
pub fn low_layer(pc: i32) -> u8 {
    let lo = LOW_LAYER_MIDI.0 as i32;
    (lo + (pc - lo).rem_euclid(12)) as u8
}

/// The lowest tonic or fifth at least `min_interval` semitones above `start`: where a riser
/// lands.
pub fn landing(key: &Key, start: u8, min_interval: i32) -> u8 {
    let t = tonic_pc(key);
    let f = fifth_pc(key);
    let mut p = start as i32 + min_interval;
    while p < 127 {
        let pc = p.rem_euclid(12);
        if pc == t || pc == f {
            break;
        }
        p += 1;
    }
    p.clamp(0, 127) as u8
}

/// `sin^2` rise: 0 at `x <= 0`, 1 at `x >= 1`, zero slope at both ends.
#[inline]
pub fn rise(x: f64) -> f64 {
    let s = (FRAC_PI_2 * x.clamp(0.0, 1.0)).sin();
    s * s
}

/// `cos^2` fall: 1 at `y <= 0`, 0 at `y >= 1` (exactly), zero slope at both ends.
#[inline]
pub fn fall(y: f64) -> f64 {
    if y >= 1.0 {
        return 0.0;
    }
    let c = (FRAC_PI_2 * y.max(0.0)).cos();
    c * c
}

/// Per-sample multiplier that decays by 60 dB in `t60_samples`.
#[inline]
pub fn t60_coeff(t60_samples: f64) -> f64 {
    if t60_samples <= 0.0 || !t60_samples.is_finite() {
        0.0
    } else {
        (-6.907_755_278_982_137 / t60_samples).exp()
    }
}

/// Equal-power pan gains scaled so the centre is unity on both sides.
#[inline]
pub fn pan_unity(pan: f64) -> (f64, f64) {
    let p = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * std::f64::consts::PI;
    (
        p.cos() * std::f64::consts::SQRT_2,
        p.sin() * std::f64::consts::SQRT_2,
    )
}

/// Sum of harmonics `w[h] * sin((h + 1) * theta)` by the Chebyshev recurrence (one `sin` and
/// one `cos` per call).
#[inline]
pub fn harmonics(theta: f64, w: &[f64]) -> f64 {
    let s1 = theta.sin();
    let c2 = 2.0 * theta.cos();
    let (mut prev, mut cur) = (0.0, s1);
    let mut out = 0.0;
    for wh in w {
        out += wh * cur;
        let next = c2 * cur - prev;
        prev = cur;
        cur = next;
    }
    out
}

/// Harmonic weights `h^-rolloff`, normalised to unit power, for harmonics that stay below
/// `ceiling_hz` when the fundamental is at `max_hz`. Returns the weights and how many are used.
pub fn harmonic_weights(
    rolloff: f64,
    max_hz: f64,
    ceiling_hz: f64,
    max_h: usize,
) -> ([f64; MAX_HARMONICS], usize) {
    let mut w = [0.0; MAX_HARMONICS];
    let mut n = 0;
    let mut power = 0.0;
    for (h, wh) in w.iter_mut().enumerate().take(max_h.min(MAX_HARMONICS)) {
        let k = (h + 1) as f64;
        if h > 0 && k * max_hz >= ceiling_hz {
            break;
        }
        *wh = k.powf(-rolloff);
        power += *wh * *wh;
        n = h + 1;
    }
    let norm = 1.0 / power.max(1e-12).sqrt();
    for wh in w.iter_mut().take(n) {
        *wh *= norm;
    }
    (w, n)
}

/// Additive tone: a phase accumulator in `f64` and fixed harmonic weights. Its frequency may
/// change every sample (glides, Doppler bends); the harmonic count is fixed per note so a glide
/// never switches partials on or off.
#[derive(Clone, Copy, Debug)]
pub struct Tone {
    phase: f64,
    w: [f64; MAX_HARMONICS],
    n: usize,
}

impl Default for Tone {
    fn default() -> Self {
        Tone {
            phase: 0.0,
            w: [0.0; MAX_HARMONICS],
            n: 0,
        }
    }
}

impl Tone {
    /// Restarts at phase 0 (so the tone starts at zero) with weights `h^-rolloff` for
    /// harmonics below the partial ceiling at the highest frequency `max_hz` it will play.
    pub fn start(&mut self, rolloff: f64, max_hz: f64, sr: f64, max_h: usize) {
        let ceiling = PARTIAL_CEILING_HZ.min(0.45 * sr);
        let (w, n) = harmonic_weights(rolloff, max_hz, ceiling, max_h);
        self.phase = 0.0;
        self.w = w;
        self.n = n;
    }

    /// The next sample at `hz` (non-finite or negative frequencies hold the phase).
    #[inline]
    pub fn next(&mut self, hz: f64, sr: f64) -> f64 {
        let y = harmonics(TAU * self.phase, &self.w[..self.n]);
        let inc = if hz.is_finite() && hz > 0.0 {
            hz / sr
        } else {
            0.0
        };
        self.phase = (self.phase + inc).fract();
        y
    }
}

/// A phase in cycles that is either locked to a whole number of cycles over a span of samples
/// (exactly periodic with that span: integer arithmetic, no drift) or free-running.
#[derive(Clone, Copy, Debug, Default)]
pub struct Phasor {
    k: u64,
    step: u64,
    span: u64,
    phase: f64,
    inc: f64,
    locked: bool,
}

impl Phasor {
    /// A phasor at `hz`, locked to `span` samples when a whole number of cycles over the span
    /// is within `tol_cents` of `hz`, free-running otherwise.
    pub fn new(hz: f64, sr: f64, span: u64, tol_cents: f64) -> Phasor {
        let hz = if hz.is_finite() { hz.max(0.0) } else { 0.0 };
        if span > 0 && hz > 0.0 {
            let n = (hz * span as f64 / sr).round();
            if n >= 1.0 && n < span as f64 {
                let locked_hz = n * sr / span as f64;
                if (1200.0 * (locked_hz / hz).log2()).abs() <= tol_cents {
                    return Phasor::cycles(n as u64, span);
                }
            }
        }
        Phasor {
            inc: hz / sr,
            ..Phasor::default()
        }
    }

    /// Exactly `cycles` cycles every `span` samples.
    pub fn cycles(cycles: u64, span: u64) -> Phasor {
        let span = span.max(1);
        Phasor {
            k: 0,
            step: cycles % span,
            span,
            phase: 0.0,
            inc: 0.0,
            locked: true,
        }
    }

    /// Whether the phasor is locked to its span.
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// Current phase in cycles, `0..1`.
    #[inline]
    pub fn phase(&self) -> f64 {
        if self.locked {
            self.k as f64 / self.span as f64
        } else {
            self.phase
        }
    }

    #[inline]
    pub fn advance(&mut self) {
        if self.locked {
            self.k += self.step;
            if self.k >= self.span {
                self.k -= self.span;
            }
        } else {
            self.phase = (self.phase + self.inc).fract();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::{Mode, PitchClass};

    fn a_minor() -> Key {
        Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        }
    }

    #[test]
    fn key_steps_stay_in_key() {
        let k = a_minor();
        // A3 (57) up two scale steps: B3, C4.
        assert_eq!(step_in_key(&k, 57, 2), 60);
        assert_eq!(step_in_key(&k, 57, -1), 55);
        assert_eq!(step_in_key(&k, 57, 7), 69);
        assert_eq!(step_in_key(&k, 57, -7), 45);
        // Off-key input snaps first (A#3 -> A3, ties resolve downward).
        assert_eq!(step_in_key(&k, 58, 0), 57);
        for p in 30..90u8 {
            for s in -7..=7 {
                assert!(k.contains(step_in_key(&k, p, s)));
            }
        }
    }

    #[test]
    fn tonic_fifth_and_landing() {
        let k = a_minor();
        assert_eq!(tonic_near(&k, 45), 45);
        assert_eq!(tonic_near(&k, 50), 45);
        assert_eq!(tonic_near(&k, 52), 57);
        assert_eq!(fifth_pc(&k), 4);
        let loc = Key {
            tonic: PitchClass::B,
            mode: Mode::Locrian,
        };
        assert_eq!(fifth_pc(&loc), 5);
        assert_eq!(landing(&k, 57, 5), 64); // A -> E
        assert_eq!(landing(&k, 52, 5), 57); // E -> A
        assert_eq!(landing(&k, 48, 5), 57); // C -> A
        assert_eq!(low_layer(9), 33);
        assert_eq!(low_layer(4), 28);
        assert_eq!(low_layer(3), 39);
    }

    #[test]
    fn pitch_steps_are_monotonic_and_centred() {
        assert_eq!(pitch_steps(0.5, 7), 0);
        assert_eq!(pitch_steps(0.0, 7), -7);
        assert_eq!(pitch_steps(1.0, 7), 7);
        let mut last = -100;
        for i in 0..=100 {
            let s = pitch_steps(i as f32 / 100.0, 7);
            assert!(s >= last);
            last = s;
        }
    }

    #[test]
    fn locked_phasor_is_exactly_periodic() {
        let sr = 48_000.0;
        let span = 96_000;
        let mut p = Phasor::new(110.0, sr, span, 1.5);
        assert!(p.is_locked());
        let start = p.phase();
        for _ in 0..span {
            p.advance();
        }
        assert_eq!(p.phase(), start);
        // 41.3 Hz over 0.1 s cannot lock within 1.5 cents.
        assert!(!Phasor::new(41.3, sr, 4_800, 1.5).is_locked());
    }

    #[test]
    fn envelopes_end_exactly() {
        assert_eq!(fall(1.0), 0.0);
        assert_eq!(rise(0.0), 0.0);
        assert!((rise(1.0) - 1.0).abs() < 1e-12);
        let (w, n) = harmonic_weights(1.0, 1000.0, 9000.0, 12);
        assert_eq!(n, 8);
        let p: f64 = w[..n].iter().map(|v| v * v).sum();
        assert!((p - 1.0).abs() < 1e-9);
    }
}
