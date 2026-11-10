//! RBJ cookbook biquad (transposed direct form II): f32 state, f64 coefficient design.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BiquadShape {
    LowPass,
    HighPass,
    BandPass,
    Notch,
    Peak { gain_db: f64 },
    LowShelf { gain_db: f64 },
    HighShelf { gain_db: f64 },
    AllPass,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl Coeffs {
    pub const IDENTITY: Coeffs = Coeffs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    pub fn design(shape: BiquadShape, freq_hz: f64, q: f64, sample_rate: f64) -> Coeffs {
        let f = freq_hz.clamp(1.0, sample_rate * 0.49);
        let q = q.max(1e-3);
        let w0 = 2.0 * std::f64::consts::PI * f / sample_rate;
        let (sn, cs) = w0.sin_cos();
        let alpha = sn / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match shape {
            BiquadShape::LowPass => (
                (1.0 - cs) / 2.0,
                1.0 - cs,
                (1.0 - cs) / 2.0,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            BiquadShape::HighPass => (
                (1.0 + cs) / 2.0,
                -(1.0 + cs),
                (1.0 + cs) / 2.0,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            BiquadShape::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            BiquadShape::Notch => (1.0, -2.0 * cs, 1.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            BiquadShape::AllPass => (
                1.0 - alpha,
                -2.0 * cs,
                1.0 + alpha,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            BiquadShape::Peak { gain_db } => {
                let a = 10f64.powf(gain_db / 40.0);
                (
                    1.0 + alpha * a,
                    -2.0 * cs,
                    1.0 - alpha * a,
                    1.0 + alpha / a,
                    -2.0 * cs,
                    1.0 - alpha / a,
                )
            }
            BiquadShape::LowShelf { gain_db } => {
                let a = 10f64.powf(gain_db / 40.0);
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cs + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                    a * ((a + 1.0) - (a - 1.0) * cs - s),
                    (a + 1.0) + (a - 1.0) * cs + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                    (a + 1.0) + (a - 1.0) * cs - s,
                )
            }
            BiquadShape::HighShelf { gain_db } => {
                let a = 10f64.powf(gain_db / 40.0);
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cs + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                    a * ((a + 1.0) + (a - 1.0) * cs - s),
                    (a + 1.0) - (a - 1.0) * cs + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cs),
                    (a + 1.0) - (a - 1.0) * cs - s,
                )
            }
        };
        Coeffs {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
        }
    }

    /// Magnitude response at `freq_hz`, for tests and EQ displays.
    pub fn magnitude(&self, freq_hz: f64, sample_rate: f64) -> f64 {
        let w = 2.0 * std::f64::consts::PI * freq_hz / sample_rate;
        let (c1, s1) = (w.cos(), -w.sin());
        let (c2, s2) = ((2.0 * w).cos(), -(2.0 * w).sin());
        let nr = self.b0 as f64 + self.b1 as f64 * c1 + self.b2 as f64 * c2;
        let ni = self.b1 as f64 * s1 + self.b2 as f64 * s2;
        let dr = 1.0 + self.a1 as f64 * c1 + self.a2 as f64 * c2;
        let di = self.a1 as f64 * s1 + self.a2 as f64 * s2;
        ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
    }
}

/// One channel of biquad state.
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    pub c: Coeffs,
    z1: f32,
    z2: f32,
}

impl Biquad {
    pub fn new(c: Coeffs) -> Biquad {
        Biquad {
            c,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.c.b0 * x + self.z1;
        self.z1 = crate::util::sanitize(self.c.b1 * x - self.c.a1 * y + self.z2);
        self.z2 = crate::util::sanitize(self.c.b2 * x - self.c.a2 * y);
        y
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses() {
        let sr = 48000.0;
        let c = Coeffs::design(
            BiquadShape::LowPass,
            1000.0,
            std::f64::consts::FRAC_1_SQRT_2,
            sr,
        );
        assert!((c.magnitude(10.0, sr) - 1.0).abs() < 0.01);
        assert!((c.magnitude(1000.0, sr) - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.01);
        assert!(c.magnitude(10000.0, sr) < 0.02);
        let p = Coeffs::design(BiquadShape::Peak { gain_db: 6.0 }, 1000.0, 1.0, sr);
        assert!((p.magnitude(1000.0, sr) - 10f64.powf(6.0 / 20.0)).abs() < 0.01);
    }
}
