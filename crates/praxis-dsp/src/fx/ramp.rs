//! Small shared helpers for the effects: linear parameter ramps, one-pole filters, FIR design.

/// Default parameter ramp length in seconds (click-free parameter moves).
pub(crate) const RAMP_SECONDS: f32 = 0.02;

/// Ramp length in samples for `seconds` at `sample_rate`.
pub(crate) fn ramp_len(seconds: f32, sample_rate: f32) -> u32 {
    (seconds * sample_rate).round().max(1.0) as u32
}

/// A linear ramp from the current value to a target over a fixed number of samples.
/// Deterministic, allocation-free.
#[derive(Clone, Debug)]
pub(crate) struct Ramp {
    cur: f32,
    target: f32,
    step: f32,
    left: u32,
    len: u32,
}

impl Ramp {
    pub(crate) fn new(value: f32, len: u32) -> Ramp {
        Ramp {
            cur: value,
            target: value,
            step: 0.0,
            left: 0,
            len,
        }
    }

    /// Starts a ramp to `target`. Non-finite targets are ignored.
    pub(crate) fn set(&mut self, target: f32) {
        if !target.is_finite() || target == self.target {
            return;
        }
        self.target = target;
        if self.len == 0 {
            self.snap();
            return;
        }
        self.left = self.len;
        self.step = (target - self.cur) / self.len as f32;
    }

    /// Jumps to the target immediately.
    pub(crate) fn snap(&mut self) {
        self.cur = self.target;
        self.left = 0;
        self.step = 0.0;
    }

    /// Sets and jumps (used at construction and on prepare).
    pub(crate) fn set_now(&mut self, v: f32) {
        if v.is_finite() {
            self.target = v;
            self.snap();
        }
    }

    #[inline]
    pub(crate) fn next(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            if self.left == 0 {
                self.cur = self.target;
            } else {
                self.cur += self.step;
            }
        }
        self.cur
    }

    /// Advances `n` samples at once and returns the new value.
    #[inline]
    pub(crate) fn advance(&mut self, n: u32) -> f32 {
        if self.left > 0 {
            if n >= self.left {
                self.cur = self.target;
                self.left = 0;
            } else {
                self.left -= n;
                self.cur += self.step * n as f32;
            }
        }
        self.cur
    }

    #[inline]
    pub(crate) fn value(&self) -> f32 {
        self.cur
    }

    #[inline]
    pub(crate) fn target(&self) -> f32 {
        self.target
    }

    #[inline]
    pub(crate) fn moving(&self) -> bool {
        self.left > 0
    }
}

/// One-pole low-pass `y += (x - y) * (1 - a)`, unity gain at DC.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OnePole {
    pub(crate) y: f32,
}

impl OnePole {
    #[inline]
    pub(crate) fn lp(&mut self, x: f32, a: f32) -> f32 {
        self.y = crate::util::sanitize(x + (self.y - x) * a);
        self.y
    }
}

/// Pole coefficient of a one-pole low-pass with -3 dB-ish corner at `hz`.
pub(crate) fn one_pole_coeff(hz: f32, sample_rate: f32) -> f32 {
    let hz = hz.clamp(1.0, sample_rate * 0.49);
    (-2.0 * std::f32::consts::PI * hz / sample_rate).exp()
}

/// DC blocker `y = x - x1 + R y1`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DcBlock {
    x1: f32,
    y1: f32,
}

impl DcBlock {
    #[inline]
    pub(crate) fn tick(&mut self, x: f32, r: f32) -> f32 {
        let y = crate::util::sanitize(x - self.x1 + r * self.y1);
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// Samples for a decaying mode of pole radius `r` (< 1) to fall by 90 dB.
pub(crate) fn radius_tail(r: f64) -> usize {
    if r <= 1e-9 {
        return 2;
    }
    if r >= 1.0 {
        return usize::MAX / 4;
    }
    let n = (-90.0 / 20.0 * std::f64::consts::LN_10) / r.ln();
    n.ceil() as usize + 2
}

/// Largest pole radius of `1 + a1 z^-1 + a2 z^-2`.
pub(crate) fn pole_radius(a1: f64, a2: f64) -> f64 {
    let d = a1 * a1 - 4.0 * a2;
    if d < 0.0 {
        a2.abs().sqrt()
    } else {
        let s = d.sqrt();
        ((-a1 + s) * 0.5).abs().max(((-a1 - s) * 0.5).abs())
    }
}

/// -90 dB ring-out of a biquad (2 for the identity).
pub(crate) fn biquad_tail(c: &crate::biquad::Coeffs) -> usize {
    radius_tail(pole_radius(c.a1 as f64, c.a2 as f64))
}

/// Transposed direct form II biquad running the frozen [`crate::biquad::Coeffs`] designs with
/// f64 state: the same response, but without the f32 rounding noise that low cutoffs amplify.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Biquad64 {
    c: [f64; 5],
    z1: f64,
    z2: f64,
}

impl Biquad64 {
    pub(crate) fn new(c: &crate::biquad::Coeffs) -> Biquad64 {
        let mut b = Biquad64::default();
        b.set(c);
        b
    }

    pub(crate) fn set(&mut self, c: &crate::biquad::Coeffs) {
        self.c = [
            c.b0 as f64,
            c.b1 as f64,
            c.b2 as f64,
            c.a1 as f64,
            c.a2 as f64,
        ];
    }

    #[inline]
    pub(crate) fn tick(&mut self, x: f32) -> f32 {
        crate::util::sanitize(self.tick64(x as f64) as f32)
    }

    #[inline]
    pub(crate) fn tick64(&mut self, x: f64) -> f64 {
        let [b0, b1, b2, a1, a2] = self.c;
        let y = b0 * x + self.z1;
        self.z1 = b1 * x - a1 * y + self.z2;
        self.z2 = b2 * x - a2 * y;
        if self.z1.abs() < 1e-30 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-30 {
            self.z2 = 0.0;
        }
        y
    }

    pub(crate) fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    pub(crate) fn tail(&self) -> usize {
        radius_tail(pole_radius(self.c[3], self.c[4]))
    }
}

/// Zeroth-order modified Bessel function (power series), for Kaiser windows.
pub(crate) fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..64 {
        term *= q / (k as f64 * k as f64);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// Kaiser-windowed half-band low-pass of odd length `n` (cutoff at a quarter of the rate),
/// normalised to unity DC gain. Taps at even offsets from the centre (other than 0) are exactly 0.
pub(crate) fn halfband(n: usize, beta: f64) -> Vec<f32> {
    let c = (n - 1) as f64 / 2.0;
    let i0b = bessel_i0(beta);
    let mut h: Vec<f64> = (0..n)
        .map(|i| {
            let m = i as f64 - c;
            let r = m / c;
            let w = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0b;
            let mi = m.round() as i64;
            let s = if mi == 0 {
                0.5
            } else if mi % 2 == 0 {
                0.0
            } else {
                let x = std::f64::consts::PI * m * 0.5;
                0.5 * x.sin() / x
            };
            s * w
        })
        .collect();
    let sum: f64 = h.iter().sum();
    // Keep the centre tap at exactly 0.5 and scale the rest so the total is 1.
    let ci = (n - 1) / 2;
    let rest = sum - h[ci];
    if rest.abs() > 1e-12 {
        for (i, v) in h.iter_mut().enumerate() {
            if i != ci {
                *v *= 0.5 / rest;
            }
        }
    }
    h.into_iter().map(|v| v as f32).collect()
}

/// Delay-free read with linear blending below one sample, cubic Hermite above.
/// Call after `push`: `d = 0` returns the sample just pushed.
#[inline]
pub(crate) fn read_frac(line: &crate::delayline::DelayLine, d: f32) -> f32 {
    if d >= 1.0 {
        line.read(d)
    } else {
        let d = d.max(0.0);
        let a = line.read_int(0);
        let b = line.read_int(1);
        a + (b - a) * d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_reaches_target_exactly() {
        let mut r = Ramp::new(0.0, 10);
        r.set(1.0);
        let mut last = 0.0;
        for _ in 0..10 {
            last = r.next();
        }
        assert_eq!(last, 1.0);
        assert!(!r.moving());
        r.set(3.0);
        assert_eq!(r.advance(100), 3.0);
    }

    #[test]
    fn halfband_is_halfband() {
        let h = halfband(47, 7.0);
        assert_eq!(h[23], 0.5);
        assert_eq!(h[21], 0.0);
        assert_eq!(h[25], 0.0);
        let s: f32 = h.iter().sum();
        assert!((s - 1.0).abs() < 1e-6);
    }
}
