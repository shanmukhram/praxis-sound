//! Four-band EQ plus high/low-pass cuts (kind 0), built from the frozen RBJ biquads.
//!
//! Parameters: 0 hp_hz, 1 lp_hz, 2 low_hz, 3 low_db, 4 mid1_hz, 5 mid1_db, 6 mid1_q,
//! 7 mid2_hz, 8 mid2_db, 9 mid2_q, 10 high_hz, 11 high_db.
//!
//! Stage order: high-pass (Q 0.7071, 12 dB/oct; off at 10 Hz), low-pass (Q 0.7071; off at
//! 22 kHz), low shelf (Q 0.7071), peak 1, peak 2, high shelf (Q 0.7071). The cuts run all the
//! time and are crossfaded with the dry signal over 20 ms when switched on or off, so "off" is
//! exactly flat and switching never clicks. [`Eq4::stage_coeffs`]
//! exposes the designs so tests and displays can compute the exact expected response.

use super::ramp::{Biquad64, RAMP_SECONDS, Ramp, biquad_tail, ramp_len};
use crate::biquad::{BiquadShape, Coeffs};
use crate::traits::Processor;

const KIND: usize = 0;
const N: usize = 12;
/// Coefficients are recomputed at most every this many samples while a parameter moves.
const CHUNK: usize = 16;
const SHELF_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// Parameters whose ramps run in log2(Hz).
const FREQ: [bool; N] = [
    true, true, true, false, true, false, false, true, false, false, true, false,
];

#[derive(Clone, Debug)]
pub struct Eq4 {
    sr: f32,
    params: [f32; N],
    ramps: [Ramp; N],
    /// Crossfade amounts of the high-pass and low-pass cuts (0 = off).
    cut: [Ramp; 2],
    coeffs: [Coeffs; 6],
    st: [[Biquad64; 6]; 2],
}

fn cut_on(i: usize, v: f32) -> bool {
    if i == 0 { v > 10.0 } else { v < 22000.0 }
}

fn to_ramp(i: usize, v: f32) -> f32 {
    if FREQ[i] { v.max(1.0).log2() } else { v }
}

fn from_ramp(i: usize, v: f32) -> f32 {
    if FREQ[i] { v.exp2() } else { v }
}

impl Eq4 {
    pub fn new(sample_rate: f32, max_block: usize) -> Eq4 {
        let params = super::defaults::<N>(KIND);
        let mut e = Eq4 {
            sr: sample_rate,
            params,
            ramps: std::array::from_fn(|i| Ramp::new(to_ramp(i, params[i]), 1)),
            cut: [Ramp::new(0.0, 1), Ramp::new(0.0, 1)],
            coeffs: [Coeffs::IDENTITY; 6],
            st: [[Biquad64::new(&Coeffs::IDENTITY); 6]; 2],
        };
        e.prepare(sample_rate, max_block);
        e
    }

    /// The six stage designs for a parameter vector (index order of the spec table).
    pub fn stage_coeffs(p: &[f32; N], sample_rate: f32) -> [Coeffs; 6] {
        let sr = sample_rate as f64;
        let q = std::f64::consts::FRAC_1_SQRT_2;
        let hp = if p[0] <= 10.0 {
            Coeffs::IDENTITY
        } else {
            Coeffs::design(BiquadShape::HighPass, p[0] as f64, q, sr)
        };
        let lp = if p[1] >= 22000.0 {
            Coeffs::IDENTITY
        } else {
            Coeffs::design(BiquadShape::LowPass, p[1] as f64, q, sr)
        };
        let shelf = |shape: BiquadShape, hz: f32, db: f32| {
            if db == 0.0 {
                Coeffs::IDENTITY
            } else {
                Coeffs::design(shape, hz as f64, SHELF_Q, sr)
            }
        };
        let low = shelf(
            BiquadShape::LowShelf {
                gain_db: p[3] as f64,
            },
            p[2],
            p[3],
        );
        let high = shelf(
            BiquadShape::HighShelf {
                gain_db: p[11] as f64,
            },
            p[10],
            p[11],
        );
        let peak = |hz: f32, db: f32, q: f32| {
            if db == 0.0 {
                Coeffs::IDENTITY
            } else {
                Coeffs::design(
                    BiquadShape::Peak { gain_db: db as f64 },
                    hz as f64,
                    q as f64,
                    sr,
                )
            }
        };
        [
            hp,
            lp,
            low,
            peak(p[4], p[5], p[6]),
            peak(p[7], p[8], p[9]),
            high,
        ]
    }

    /// Expected magnitude (linear) at `hz` for a parameter vector.
    pub fn magnitude(p: &[f32; N], hz: f64, sample_rate: f32) -> f64 {
        Self::stage_coeffs(p, sample_rate)
            .iter()
            .map(|c| c.magnitude(hz, sample_rate as f64))
            .product()
    }

    fn current(&self) -> [f32; N] {
        std::array::from_fn(|i| from_ramp(i, self.ramps[i].value()))
    }

    fn update_coeffs(&mut self) {
        let mut p = self.current();
        // Snap the "off" ends exactly when the ramp has settled there.
        for i in 0..N {
            if !self.ramps[i].moving() {
                p[i] = self.params[i];
            }
        }
        let mut c = Self::stage_coeffs(&p, self.sr);
        // The cuts always run (warm state); "off" is a crossfade to the dry signal, so turning
        // one on or off never switches a filter in cold.
        let sr = self.sr as f64;
        let q = std::f64::consts::FRAC_1_SQRT_2;
        c[0] = Coeffs::design(BiquadShape::HighPass, p[0].max(10.0) as f64, q, sr);
        c[1] = Coeffs::design(BiquadShape::LowPass, (p[1] as f64).min(0.45 * sr), q, sr);
        self.coeffs = c;
        for ch in self.st.iter_mut() {
            for (b, c) in ch.iter_mut().zip(self.coeffs.iter()) {
                b.set(c);
            }
        }
    }
}

impl Processor for Eq4 {
    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sr = sample_rate;
        let len = ramp_len(RAMP_SECONDS, sample_rate);
        for i in 0..N {
            self.ramps[i] = Ramp::new(to_ramp(i, self.params[i]), len);
        }
        for i in 0..2 {
            let on = cut_on(i, self.params[i]);
            self.cut[i] = Ramp::new(if on { 1.0 } else { 0.0 }, len);
        }
        self.update_coeffs();
        self.reset();
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let mut start = 0;
        while start < n {
            let len = CHUNK.min(n - start);
            if self.ramps.iter().any(|r| r.moving()) {
                for r in self.ramps.iter_mut() {
                    r.advance(len as u32);
                }
                self.update_coeffs();
            }
            let a0 = [self.cut[0].value() as f64, self.cut[1].value() as f64];
            let a1 = [
                self.cut[0].advance(len as u32) as f64,
                self.cut[1].advance(len as u32) as f64,
            ];
            for (ch, buf) in [&mut *left, &mut *right].into_iter().enumerate() {
                let st = &mut self.st[ch];
                for (k, x) in buf[start..start + len].iter_mut().enumerate() {
                    let t = (k + 1) as f64 / len as f64;
                    let mut y = crate::util::sanitize(*x) as f64;
                    for (s, b) in st.iter_mut().enumerate() {
                        let f = b.tick64(y);
                        y = if s < 2 {
                            let a = a0[s] + (a1[s] - a0[s]) * t;
                            y + (f - y) * a
                        } else {
                            f
                        };
                    }
                    *x = crate::util::sanitize(y as f32);
                }
            }
            start += len;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if let Some(v) = super::clamp_param(KIND, index, value) {
            self.params[index] = v;
            if index < 2 {
                let on = cut_on(index, v);
                self.cut[index].set(if on { 1.0 } else { 0.0 });
                if !on {
                    // fade out through the filter as it was
                    return;
                }
            }
            self.ramps[index].set(to_ramp(index, v));
            if !self.ramps[index].moving() {
                self.update_coeffs();
            }
        }
    }

    fn reset(&mut self) {
        for r in self.ramps.iter_mut() {
            r.snap();
        }
        for r in self.cut.iter_mut() {
            r.snap();
        }
        self.update_coeffs();
        for ch in self.st.iter_mut() {
            for b in ch.iter_mut() {
                b.reset();
            }
        }
    }

    /// Ring-out of the slowest stage to -90 dB.
    fn tail_samples(&self) -> usize {
        let cut_off = |i: usize| self.cut[i].value() == 0.0 && self.cut[i].target() == 0.0;
        self.coeffs
            .iter()
            .enumerate()
            .filter(|(i, _)| *i >= 2 || !cut_off(*i))
            .map(|(_, c)| biquad_tail(c))
            .max()
            .unwrap_or(0)
    }
}
