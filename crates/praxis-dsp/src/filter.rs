//! Filters for synthesis: [`Svf`] (Cytomic / Simper trapezoidal state-variable), [`OnePole`]
//! low/high-pass, [`Ladder`] (4-pole, saturating, resonance-compensated) and [`DcBlocker`].
//!
//! The SVF, one-pole and ladder are zero-delay-feedback (topology-preserving transform)
//! designs: their linear response is exactly the bilinear transform of the analog prototype,
//! pre-warped at the cutoff, and they stay stable and click-free when the cutoff or resonance
//! changes every sample. Cutoffs are in Hz (clamped to `5 Hz..0.49 * sample_rate`); states are
//! `f64`; NaN or infinite input is treated as silence and never reaches the state.

use crate::util::sanitize;
use std::f64::consts::PI;

#[inline]
fn sr_or_default(sample_rate: f32) -> f64 {
    if sample_rate.is_finite() && sample_rate > 0.0 {
        sample_rate as f64
    } else {
        48_000.0
    }
}

#[inline]
fn clamp_cutoff(hz: f32, sr: f64) -> f64 {
    let hz = if hz.is_finite() { hz as f64 } else { 1000.0 };
    hz.clamp(5.0, 0.49 * sr)
}

#[inline]
fn flush(x: f64) -> f64 {
    if x.is_finite() && x.abs() > 1e-30 {
        x
    } else {
        0.0
    }
}

#[inline]
fn input(x: f32) -> f64 {
    sanitize(x) as f64
}

/// Response of an [`Svf`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SvfMode {
    LowPass,
    HighPass,
    /// Band-pass with 0 dB gain at the centre frequency.
    BandPass,
    Notch,
    /// Resonant peak (low-pass minus high-pass): gain `2 * Q` at the cutoff.
    Peak,
    AllPass,
    /// Peaking EQ with `gain_db` at the centre.
    Bell {
        gain_db: f32,
    },
    LowShelf {
        gain_db: f32,
    },
    HighShelf {
        gain_db: f32,
    },
}

/// All simultaneous outputs of an [`Svf`] for one input sample.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SvfOutputs {
    pub low: f32,
    /// 0 dB-peak band-pass.
    pub band: f32,
    pub high: f32,
    pub notch: f32,
}

/// Cytomic (Andrew Simper) trapezoidal state-variable filter, 12 dB/octave.
///
/// `Q` is clamped to `0.1..=100` (0.707 = Butterworth). The filter never self-oscillates and
/// stays stable under per-sample cutoff and Q modulation. `set_cutoff` costs one `tan`.
#[derive(Clone, Debug)]
pub struct Svf {
    sr: f64,
    cutoff: f64,
    q: f64,
    mode: SvfMode,
    k: f64,
    a1: f64,
    a2: f64,
    a3: f64,
    m0: f64,
    m1: f64,
    m2: f64,
    ic1eq: f64,
    ic2eq: f64,
}

impl Svf {
    /// Low-pass at 1 kHz, Q 0.707.
    pub fn new(sample_rate: f32) -> Svf {
        let mut f = Svf {
            sr: sr_or_default(sample_rate),
            cutoff: 1000.0,
            q: std::f64::consts::FRAC_1_SQRT_2,
            mode: SvfMode::LowPass,
            k: 0.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            m0: 0.0,
            m1: 0.0,
            m2: 1.0,
            ic1eq: 0.0,
            ic2eq: 0.0,
        };
        f.update();
        f
    }

    pub fn set_mode(&mut self, mode: SvfMode) {
        self.mode = mode;
        self.update();
    }

    /// Cutoff (or centre) frequency in Hz.
    pub fn set_cutoff(&mut self, hz: f32) {
        self.cutoff = clamp_cutoff(hz, self.sr);
        self.update();
    }

    /// Quality factor, `0.1..=100`.
    pub fn set_q(&mut self, q: f32) {
        self.q = if q.is_finite() {
            (q as f64).clamp(0.1, 100.0)
        } else {
            0.707
        };
        self.update();
    }

    /// Cutoff and Q together (one coefficient update).
    pub fn set_cutoff_q(&mut self, hz: f32, q: f32) {
        self.cutoff = clamp_cutoff(hz, self.sr);
        self.q = if q.is_finite() {
            (q as f64).clamp(0.1, 100.0)
        } else {
            0.707
        };
        self.update();
    }

    pub fn cutoff(&self) -> f32 {
        self.cutoff as f32
    }

    fn update(&mut self) {
        let mut g = (PI * self.cutoff / self.sr).tan();
        let mut k = 1.0 / self.q;
        let gain_a = |db: f32| 10f64.powf(db as f64 / 40.0);
        let (m0, m1, m2) = match self.mode {
            SvfMode::LowPass => (0.0, 0.0, 1.0),
            SvfMode::HighPass => (1.0, -k, -1.0),
            SvfMode::BandPass => (0.0, k, 0.0),
            SvfMode::Notch => (1.0, -k, 0.0),
            SvfMode::Peak => (1.0, -k, -2.0),
            SvfMode::AllPass => (1.0, -2.0 * k, 0.0),
            SvfMode::Bell { gain_db } => {
                let a = gain_a(gain_db);
                k = 1.0 / (self.q * a);
                (1.0, k * (a * a - 1.0), 0.0)
            }
            SvfMode::LowShelf { gain_db } => {
                let a = gain_a(gain_db);
                g /= a.sqrt();
                (1.0, k * (a - 1.0), a * a - 1.0)
            }
            SvfMode::HighShelf { gain_db } => {
                let a = gain_a(gain_db);
                g *= a.sqrt();
                (a * a, k * (1.0 - a) * a, 1.0 - a * a)
            }
        };
        self.k = k;
        self.a1 = 1.0 / (1.0 + g * (g + k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
        self.m0 = m0;
        self.m1 = m1;
        self.m2 = m2;
    }

    #[inline]
    fn step(&mut self, v0: f64) -> (f64, f64) {
        let v3 = v0 - self.ic2eq;
        let v1 = self.a1 * self.ic1eq + self.a2 * v3;
        let v2 = self.ic2eq + self.a2 * self.ic1eq + self.a3 * v3;
        self.ic1eq = flush(2.0 * v1 - self.ic1eq);
        self.ic2eq = flush(2.0 * v2 - self.ic2eq);
        (v1, v2)
    }

    /// Filters one sample in the selected mode.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let v0 = input(x);
        let (v1, v2) = self.step(v0);
        (self.m0 * v0 + self.m1 * v1 + self.m2 * v2) as f32
    }

    /// Filters one sample and returns low, band (0 dB peak), high and notch at once. The
    /// selected mode does not matter except that shelf/bell modes change the internal `k`.
    #[inline]
    pub fn tick_all(&mut self, x: f32) -> SvfOutputs {
        let v0 = input(x);
        let (v1, v2) = self.step(v0);
        let k = self.k;
        SvfOutputs {
            low: v2 as f32,
            band: (k * v1) as f32,
            high: (v0 - k * v1 - v2) as f32,
            notch: (v0 - k * v1) as f32,
        }
    }

    /// Clears the state to silence.
    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }
}

/// Response of a [`OnePole`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OnePoleMode {
    #[default]
    LowPass,
    HighPass,
}

/// Trapezoidal one-pole filter, 6 dB/octave (-3 dB at the cutoff).
#[derive(Clone, Debug)]
pub struct OnePole {
    sr: f64,
    cutoff: f64,
    mode: OnePoleMode,
    gain: f64,
    s: f64,
}

impl OnePole {
    /// Low-pass at 1 kHz.
    pub fn new(sample_rate: f32) -> OnePole {
        let mut f = OnePole {
            sr: sr_or_default(sample_rate),
            cutoff: 1000.0,
            mode: OnePoleMode::LowPass,
            gain: 0.0,
            s: 0.0,
        };
        f.update();
        f
    }

    pub fn set_mode(&mut self, mode: OnePoleMode) {
        self.mode = mode;
    }

    /// Cutoff in Hz.
    pub fn set_cutoff(&mut self, hz: f32) {
        self.cutoff = clamp_cutoff(hz, self.sr);
        self.update();
    }

    fn update(&mut self) {
        let g = (PI * self.cutoff / self.sr).tan();
        self.gain = g / (1.0 + g);
    }

    /// Filters one sample and returns `(low, high)`.
    #[inline]
    pub fn tick_lp_hp(&mut self, x: f32) -> (f32, f32) {
        let x = input(x);
        let v = (x - self.s) * self.gain;
        let lp = v + self.s;
        self.s = flush(lp + v);
        (lp as f32, (x - lp) as f32)
    }

    /// Filters one sample in the selected mode.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let (lp, hp) = self.tick_lp_hp(x);
        match self.mode {
            OnePoleMode::LowPass => lp,
            OnePoleMode::HighPass => hp,
        }
    }

    pub fn reset(&mut self) {
        self.s = 0.0;
    }
}

/// Feedback at full resonance: slightly past the linear stability limit of 4, so resonance 1.0
/// self-oscillates into a sine at the cutoff whose level the saturation sets.
const LADDER_MAX_FEEDBACK: f64 = 4.2;

/// Four-pole (24 dB/octave) transistor-ladder style low-pass with `tanh` input saturation.
///
/// - Resonance `0..=1` maps to feedback `k = 0..=4.2`. Linear self-oscillation starts at
///   `k = 4` (resonance 0.952); at 1.0 the filter sings a sine at the cutoff, held in bounds by
///   the saturation.
/// - Resonance compensation: the input is scaled by `1 + feedback`, so the pass-band (DC) gain
///   stays at 0 dB at any resonance instead of thinning out.
/// - Drive (`0.1..=10`, default 1) scales the signal into the `tanh`; output is scaled back, so
///   small signals pass at unity gain and loud ones (or strong resonance) saturate smoothly.
///
/// The feedback loop is solved without a unit delay (zero-delay feedback): the linear response
/// is exactly `(1 + k) / ((1 + s)^4 + k)` after the bilinear transform, the cutoff stays where it
/// is set at any resonance, and per-sample modulation is safe.
#[derive(Clone, Debug)]
pub struct Ladder {
    sr: f64,
    cutoff: f64,
    resonance: f64,
    k: f64,
    drive: f64,
    gain: f64,
    beta: f64,
    s: [f64; 4],
}

impl Ladder {
    /// 1 kHz, no resonance, drive 1.
    pub fn new(sample_rate: f32) -> Ladder {
        let mut f = Ladder {
            sr: sr_or_default(sample_rate),
            cutoff: 1000.0,
            resonance: 0.0,
            k: 0.0,
            drive: 1.0,
            gain: 0.0,
            beta: 1.0,
            s: [0.0; 4],
        };
        f.update();
        f
    }

    /// Cutoff in Hz.
    pub fn set_cutoff(&mut self, hz: f32) {
        self.cutoff = clamp_cutoff(hz, self.sr);
        self.update();
    }

    /// Resonance `0..=1`. About 0.95 is the edge of self-oscillation; 1.0 self-oscillates.
    pub fn set_resonance(&mut self, r: f32) {
        self.resonance = if r.is_finite() {
            (r as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.k = LADDER_MAX_FEEDBACK * self.resonance;
    }

    /// Feedback amount `k` the current resonance maps to (`0..=4.2`).
    pub fn feedback(&self) -> f32 {
        self.k as f32
    }

    /// Drive into the saturation, `0.1..=10`.
    pub fn set_drive(&mut self, d: f32) {
        self.drive = if d.is_finite() {
            (d as f64).clamp(0.1, 10.0)
        } else {
            1.0
        };
    }

    fn update(&mut self) {
        let g = (PI * self.cutoff / self.sr).tan();
        self.gain = g / (1.0 + g);
        self.beta = 1.0 / (1.0 + g);
    }

    /// Filters one sample.
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let x = input(x);
        let gg = self.gain;
        let b = self.beta;
        let k = self.k;
        // Each trapezoidal stage: y = G * u + beta * s. Cascade: y4 = G^4 u + sigma.
        let g2 = gg * gg;
        let sigma = b * (gg * g2 * self.s[0] + g2 * self.s[1] + gg * self.s[2] + self.s[3]);
        let u_lin = ((1.0 + k) * x - k * sigma) / (1.0 + k * g2 * g2);
        let d = self.drive;
        let mut u = (d * u_lin).tanh() / d;
        for s in self.s.iter_mut() {
            let v = (u - *s) * gg;
            let y = v + *s;
            *s = flush(y + v);
            u = y;
        }
        u as f32
    }

    pub fn reset(&mut self) {
        self.s = [0.0; 4];
    }
}

/// DC blocker: `y = x - x[n-1] + R * y[n-1]`, a first-order high-pass with its corner at
/// `set_cutoff` Hz (default 10 Hz).
#[derive(Clone, Debug)]
pub struct DcBlocker {
    sr: f64,
    r: f64,
    x1: f64,
    y1: f64,
}

impl DcBlocker {
    pub fn new(sample_rate: f32) -> DcBlocker {
        let mut d = DcBlocker {
            sr: sr_or_default(sample_rate),
            r: 0.0,
            x1: 0.0,
            y1: 0.0,
        };
        d.set_cutoff(10.0);
        d
    }

    /// Corner frequency in Hz, `0.1..=500`.
    pub fn set_cutoff(&mut self, hz: f32) {
        let hz = if hz.is_finite() {
            (hz as f64).clamp(0.1, 500.0)
        } else {
            10.0
        };
        self.r = (-2.0 * PI * hz / self.sr).exp();
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let x = input(x);
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        self.y1 = flush(y);
        self.y1 as f32
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nan_input_never_poisons_state() {
        let mut s = Svf::new(48_000.0);
        let mut l = Ladder::new(48_000.0);
        let mut o = OnePole::new(48_000.0);
        let mut d = DcBlocker::new(48_000.0);
        for x in [f32::NAN, f32::INFINITY, 1.0, -1.0, f32::NEG_INFINITY] {
            for v in [s.tick(x), l.tick(x), o.tick(x), d.tick(x)] {
                assert!(v.is_finite());
            }
        }
    }

    #[test]
    fn dc_blocker_removes_dc() {
        let mut d = DcBlocker::new(48_000.0);
        let mut y = 0.0;
        for _ in 0..48_000 {
            y = d.tick(1.0);
        }
        assert!(y.abs() < 1e-3);
    }
}
