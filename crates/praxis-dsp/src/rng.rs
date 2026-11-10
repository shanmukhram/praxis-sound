//! Deterministic randomness: the only source of random numbers in Praxis Sound.
//! SplitMix64 for seeding and hashing, PCG32 (XSH-RR) for streams.

/// SplitMix64 finaliser: a good 64-bit mixer for deriving child seeds.
pub fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Derives a child seed from a parent seed and a salt (a note key, a voice index...).
pub fn derive(seed: u64, salt: u64) -> u64 {
    mix64(seed ^ mix64(salt))
}

#[derive(Clone, Debug)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    pub fn new(seed: u64) -> Pcg32 {
        let mut r = Pcg32 {
            state: 0,
            inc: (mix64(seed ^ 0xDA3E_39CB_94B9_5BDB) << 1) | 1,
        };
        r.next_u32();
        r.state = r.state.wrapping_add(mix64(seed));
        r.next_u32();
        r
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// Uniform in [-1, 1).
    pub fn next_bipolar(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }

    /// Uniform in [lo, hi).
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reproducible_and_spread() {
        let mut a = Pcg32::new(42);
        let mut b = Pcg32::new(42);
        let mut c = Pcg32::new(43);
        let va: Vec<u32> = (0..8).map(|_| a.next_u32()).collect();
        let vb: Vec<u32> = (0..8).map(|_| b.next_u32()).collect();
        let vc: Vec<u32> = (0..8).map(|_| c.next_u32()).collect();
        assert_eq!(va, vb);
        assert_ne!(va, vc);
        let mut r = Pcg32::new(7);
        let mean: f32 = (0..10_000).map(|_| r.next_f32()).sum::<f32>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02);
    }
}
