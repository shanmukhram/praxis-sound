//! State-variable filter effect (kind 1): a small Simper/Cytomic TPT SVF kept private to fx
//! (WP-B1's `filter::Svf` is developed in parallel and is not a dependency here).
//!
//! Parameters: 0 mode (enum: 0 lp, 1 hp, 2 bp, 3 notch), 1 cutoff_hz (Hz), 2 resonance
//! (0 = Q 0.5 .. 1 = Q 50, the self-oscillation edge), 3 mix (wet fraction).
//!
//! All responses come from one filter state, so a mode change crossfades the output taps over
//! 20 ms instead of switching state. The band-pass is normalised to unity gain at the centre.

use super::ramp::{RAMP_SECONDS, Ramp, radius_tail, ramp_len};
use crate::traits::Processor;
use crate::util::sanitize;

const KIND: usize = 1;

/// One channel of TPT SVF state.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SvfState {
    ic1: f32,
    ic2: f32,
}

/// Per-sample SVF coefficients.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SvfCoeffs {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
}

impl SvfCoeffs {
    pub(crate) fn new(cutoff_hz: f32, q: f32, sample_rate: f32) -> SvfCoeffs {
        let f = cutoff_hz.clamp(1.0, sample_rate * 0.49);
        let g = (std::f32::consts::PI * f / sample_rate).tan();
        let k = 1.0 / q.max(0.01);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        SvfCoeffs { k, a1, a2, a3 }
    }
}

impl SvfState {
    /// Returns (low, band (unity peak), high).
    #[inline]
    pub(crate) fn tick(&mut self, x: f32, c: &SvfCoeffs) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = sanitize(2.0 * v1 - self.ic1);
        self.ic2 = sanitize(2.0 * v2 - self.ic2);
        let low = v2;
        let band = v1;
        let high = x - c.k * v1 - v2;
        (low, c.k * band, high)
    }
}

/// -90 dB ring-out (samples) of a TPT SVF: its poles are the bilinear images of the analog
/// poles of `s^2 + k s + 1` with prewarp `g`.
pub(crate) fn svf_tail(cutoff_hz: f32, q: f32, sample_rate: f32) -> usize {
    let f = cutoff_hz.clamp(1.0, sample_rate * 0.49) as f64;
    let g = (std::f64::consts::PI * f / sample_rate as f64).tan();
    let k = 1.0 / (q.max(0.01) as f64);
    let disc = k * k - 4.0;
    let r = if disc < 0.0 {
        let im = (-disc).sqrt() * 0.5 * g;
        let re = k * 0.5 * g;
        (((1.0 - re).powi(2) + im * im) / ((1.0 + re).powi(2) + im * im)).sqrt()
    } else {
        let s = disc.sqrt();
        [(-k + s) * 0.5, (-k - s) * 0.5]
            .iter()
            .map(|p| ((1.0 + g * p) / (1.0 - g * p)).abs())
            .fold(0.0, f64::max)
    };
    radius_tail(r)
}

/// Resonance 0..1 to Q (0.5 .. 50, exponential).
pub(crate) fn resonance_to_q(r: f32) -> f32 {
    0.5 * 100f32.powf(r.clamp(0.0, 1.0))
}

fn mode_weights(mode: f32) -> [f32; 3] {
    match mode.round() as i32 {
        1 => [0.0, 0.0, 1.0],
        2 => [0.0, 1.0, 0.0],
        3 => [1.0, 0.0, 1.0],
        _ => [1.0, 0.0, 0.0],
    }
}

#[derive(Clone, Debug)]
pub struct FilterFx {
    sr: f32,
    params: [f32; 4],
    weights: [Ramp; 3],
    cutoff: Ramp, // log2 Hz
    reso: Ramp,
    mix: Ramp,
    c: SvfCoeffs,
    st: [SvfState; 2],
}

impl FilterFx {
    pub fn new(sample_rate: f32, max_block: usize) -> FilterFx {
        let params = super::defaults::<4>(KIND);
        let mut f = FilterFx {
            sr: sample_rate,
            params,
            weights: std::array::from_fn(|_| Ramp::new(0.0, 1)),
            cutoff: Ramp::new(0.0, 1),
            reso: Ramp::new(0.0, 1),
            mix: Ramp::new(0.0, 1),
            c: SvfCoeffs::new(1000.0, 0.5, sample_rate),
            st: [SvfState::default(); 2],
        };
        f.prepare(sample_rate, max_block);
        f
    }

    fn coeffs(&self) -> SvfCoeffs {
        SvfCoeffs::new(
            self.cutoff.value().exp2(),
            resonance_to_q(self.reso.value()),
            self.sr,
        )
    }
}

impl Processor for FilterFx {
    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sr = sample_rate;
        let len = ramp_len(RAMP_SECONDS, sample_rate);
        let w = mode_weights(self.params[0]);
        self.weights = std::array::from_fn(|i| Ramp::new(w[i], len));
        self.cutoff = Ramp::new(self.params[1].log2(), len);
        self.reso = Ramp::new(self.params[2], len);
        self.mix = Ramp::new(self.params[3], len);
        self.c = self.coeffs();
        self.st = [SvfState::default(); 2];
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            if self.cutoff.moving() || self.reso.moving() {
                self.cutoff.next();
                self.reso.next();
                self.c = self.coeffs();
            }
            let wl = self.weights[0].next();
            let wb = self.weights[1].next();
            let wh = self.weights[2].next();
            let mix = self.mix.next();
            for (x, st) in [&mut *l, &mut *r].into_iter().zip(self.st.iter_mut()) {
                *x = sanitize(*x);
                let (lo, bp, hi) = st.tick(*x, &self.c);
                let wet = lo * wl + bp * wb + hi * wh;
                *x = sanitize(*x + (wet - *x) * mix);
            }
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(v) = super::clamp_param(KIND, index, value) else {
            return;
        };
        self.params[index] = v;
        match index {
            0 => {
                let w = mode_weights(v);
                for (r, w) in self.weights.iter_mut().zip(w) {
                    r.set(w);
                }
            }
            1 => self.cutoff.set(v.log2()),
            2 => self.reso.set(v),
            3 => self.mix.set(v),
            _ => {}
        }
    }

    fn reset(&mut self) {
        for r in self.weights.iter_mut() {
            r.snap();
        }
        self.cutoff.snap();
        self.reso.snap();
        self.mix.snap();
        self.c = self.coeffs();
        self.st = [SvfState::default(); 2];
    }

    fn tail_samples(&self) -> usize {
        let a = svf_tail(
            self.cutoff.value().exp2(),
            resonance_to_q(self.reso.value()),
            self.sr,
        );
        let b = svf_tail(
            self.cutoff.target().exp2(),
            resonance_to_q(self.reso.target()),
            self.sr,
        );
        a.max(b)
    }
}
