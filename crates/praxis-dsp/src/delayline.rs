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
        self.buf[(self.write + n - 1 - d) % n]
    }

    /// Fractional read with cubic Hermite interpolation; `delay` is clamped to >= 1.
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, (self.capacity() - 2) as f32);
        let i = d.floor() as usize;
        let t = d - i as f32;
        let xm1 = self.read_int(i - 1);
        let x0 = self.read_int(i);
        let x1 = self.read_int(i + 1);
        let x2 = self.read_int(i + 2);
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + x0
    }

    pub fn reset(&mut self) {
        self.buf.iter_mut().for_each(|v| *v = 0.0);
        self.write = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
