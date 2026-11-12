//! Circular delay line with fractional (cubic Hermite) reads. Allocates only in `new`.

#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Vec<f32>,
    write: usize,
}

impl DelayLine {
    /// A delay line able to hold `max_samples` of delay.
    pub fn new(max_samples: usize) -> DelayLine {
        DelayLine {
            buf: vec![0.0; max_samples.max(4) + 4],
            write: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.buf.len() - 4
    }

    #[inline]
    pub fn push(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write += 1;
        if self.write == self.buf.len() {
            self.write = 0;
        }
    }

    /// The sample pushed `delay` pushes ago (0 = the most recent push).
    #[inline]
    pub fn read_int(&self, delay: usize) -> f32 {
        let n = self.buf.len();
        let d = delay.min(n - 1);
        // `write < n` and `d <= n - 1`, so the index is below 2n: one compare instead of a
        // division (hot in every delay-based effect; same sample as `% n`).
        let i = self.write + n - 1 - d;
        debug_assert!(i < 2 * n);
        self.buf[if i >= n { i - n } else { i }]
    }

    /// Fractional read with cubic Hermite interpolation; `delay` is clamped to >= 1.
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, (self.capacity() - 2) as f32);
        let i = d.floor() as usize;
        let t = d - i as f32;
        self.hermite_at(i, t)
    }

    /// [`DelayLine::read`] at a delay already split into `i` (1 ..= capacity - 2) and `t` in
    /// [0, 1): callers with fixed taps split once instead of every sample. Bit-identical to
    /// `read(i + t)`.
    #[inline]
    pub(crate) fn hermite_at(&self, i: usize, t: f32) -> f32 {
        let n = self.buf.len();
        debug_assert!(i >= 1 && i + 2 < n);
        // Delays i + 2, i + 1, i, i - 1 are four consecutive slots starting here (mod n).
        let b = self.write + n - 1 - (i + 2);
        debug_assert!(b < 2 * n);
        let b = if b >= n { b - n } else { b };
        let (x2, x1, x0, xm1) = if b + 3 < n {
            (
                self.buf[b],
                self.buf[b + 1],
                self.buf[b + 2],
                self.buf[b + 3],
            )
        } else {
            let at = |k: usize| self.buf[if b + k >= n { b + k - n } else { b + k }];
            (at(0), at(1), at(2), at(3))
        };
        hermite(xm1, x0, x1, x2, t)
    }

    /// Adds `gain * hermite_at(i + (len - 1 - k), t)` to `acc[k]` for `len = acc.len()`: the
    /// read as it was when the `k`-th of the last `len` pushes was the newest. One tap over a
    /// block of samples, from contiguous memory; bit-identical to calling `hermite_at` per sample.
    #[inline]
    pub(crate) fn add_hermite_run(&self, i: usize, t: f32, gain: f32, acc: &mut [f32]) {
        let len = acc.len();
        if len == 0 {
            return;
        }
        let n = self.buf.len();
        let d0 = i + len - 1;
        debug_assert!(i >= 1 && d0 + 2 < n);
        let b = self.write + n - 1 - (d0 + 2);
        debug_assert!(b < 2 * n);
        let b = if b >= n { b - n } else { b };
        if b + len + 3 <= n {
            // Four aligned views of the window (x2, x1, x0, xm1 for every k), so the loop has no
            // index arithmetic and vectorises; each lane does exactly the scalar operations.
            let (x2, x1) = (&self.buf[b..b + len], &self.buf[b + 1..b + 1 + len]);
            let (x0, xm1) = (&self.buf[b + 2..b + 2 + len], &self.buf[b + 3..b + 3 + len]);
            for ((((a, &x2), &x1), &x0), &xm1) in acc.iter_mut().zip(x2).zip(x1).zip(x0).zip(xm1) {
                *a += hermite(xm1, x0, x1, x2, t) * gain;
            }
        } else {
            for (k, a) in acc.iter_mut().enumerate() {
                *a += self.hermite_at(i + (len - 1 - k), t) * gain;
            }
        }
    }

    pub fn reset(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0.0);
        self.write = 0;
    }
}

/// Cubic Hermite through `xm1, x0, x1, x2` at `t` (0..1) from `x0`.
#[inline(always)]
fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The old `% n` indexing, as the reference the faster reads must match bit for bit.
    fn reference(d: &DelayLine, delay: usize) -> f32 {
        let n = d.buf.len();
        d.buf[(d.write + n - 1 - delay.min(n - 1)) % n]
    }

    fn reference_hermite(d: &DelayLine, delay: f32) -> f32 {
        let dd = delay.clamp(1.0, (d.capacity() - 2) as f32);
        let i = dd.floor() as usize;
        let t = dd - i as f32;
        let (xm1, x0, x1, x2) = (
            reference(d, i - 1),
            reference(d, i),
            reference(d, i + 1),
            reference(d, i + 2),
        );
        // The formula as `read` had it inline before the faster reads.
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + x0
    }

    #[test]
    fn a_block_run_matches_per_sample_reads() {
        let mut d = DelayLine::new(53);
        let n = d.buf.len();
        for k in 0..(3 * n) {
            d.push((k as f32 * 0.21).cos() * (1.0 + k as f32 * 1e-3));
            if k < 8 {
                continue;
            }
            // Every tap position a block can take, including runs that wrap round the buffer.
            for len in [1, 7, 32] {
                for i in [1, 2, 9, d.capacity() - 2 - len] {
                    let t = 0.375;
                    let mut acc = vec![0.25f32; len];
                    d.add_hermite_run(i, t, 0.5, &mut acc);
                    for (kk, a) in acc.iter().enumerate() {
                        let want = 0.25 + d.hermite_at(i + (len - 1 - kk), t) * 0.5;
                        assert_eq!(a.to_bits(), want.to_bits(), "i {i} len {len} k {kk}");
                    }
                }
            }
        }
    }

    #[test]
    fn reads_match_modulo_indexing_at_every_wrap_position() {
        let mut d = DelayLine::new(37);
        let n = d.buf.len();
        // Walk the write position all the way round twice, so every read wraps somewhere.
        for k in 0..(2 * n + 3) {
            d.push((k as f32 * 0.37).sin() + k as f32 * 1e-3);
            for delay in [0, 1, 2, n / 2, n - 2, n - 1, n, n + 5] {
                assert_eq!(d.read_int(delay).to_bits(), reference(&d, delay).to_bits());
            }
            let cap = d.capacity() as f32;
            for delay in [
                0.0,
                1.0,
                1.25,
                2.5,
                cap * 0.5 + 0.3,
                cap - 2.5,
                cap - 2.001,
                cap - 2.0,
                cap,
            ] {
                assert_eq!(
                    d.read(delay).to_bits(),
                    reference_hermite(&d, delay).to_bits(),
                    "delay {delay} at write {}",
                    d.write
                );
            }
        }
    }

    #[test]
    fn integer_and_fractional_reads() {
        let mut d = DelayLine::new(64);
        for i in 0..32 {
            d.push(i as f32);
        }
        assert_eq!(d.read_int(0), 31.0);
        assert_eq!(d.read_int(5), 26.0);
        assert!((d.read(5.5) - 25.5).abs() < 1e-4);
    }
}
