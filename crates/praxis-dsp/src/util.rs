//! Small conversions shared everywhere.

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn gain_to_db(g: f32) -> f32 {
    20.0 * g.abs().max(1e-12).log10()
}

pub fn db_to_gain_f64(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Flushes denormals and NaN to zero.
#[inline]
pub fn sanitize(x: f32) -> f32 {
    // Keeps x exactly when it is finite and |x| > 1e-30, as one unsigned range check on the
    // magnitude's bits (positive floats order like their bits; infinity and NaN lie above
    // 0x7f80_0000): no branches in the hot loops that call this per sample.
    const LO: u32 = 0x0da2_4260; // 1e-30f32.to_bits()
    const INF: u32 = 0x7f80_0000;
    let m = x.to_bits() & 0x7fff_ffff;
    if m.wrapping_sub(LO + 1) < INF - LO - 1 {
        x
    } else {
        0.0
    }
}

/// Equal-power pan law: pan -1..1 -> (left gain, right gain), both 1/sqrt(2) at centre.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * std::f32::consts::PI;
    (p.cos(), p.sin())
}

/// One-pole smoothing coefficient for a time constant in seconds.
pub fn time_coeff(seconds: f32, sample_rate: f32) -> f32 {
    if seconds <= 0.0 {
        0.0
    } else {
        (-1.0 / (seconds * sample_rate)).exp()
    }
}

#[cfg(test)]
mod sanitize_tests {
    use super::sanitize;

    /// The old definition, which the bit-range check must match for every input.
    fn reference(x: f32) -> f32 {
        if x.is_finite() && x.abs() > 1e-30 {
            x
        } else {
            0.0
        }
    }

    #[test]
    fn the_bit_range_check_matches_the_float_comparison() {
        assert_eq!(1e-30f32.to_bits(), 0x0da2_4260);
        let edges = [
            0.0,
            -0.0,
            1e-30,
            -1e-30,
            f32::from_bits(0x0da2_4261),
            f32::from_bits(0x0da2_425f),
            f32::MIN_POSITIVE,
            1e-45,
            f32::MAX,
            f32::MIN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            -f32::NAN,
            1.0,
            -0.5,
        ];
        for x in edges {
            assert_eq!(sanitize(x).to_bits(), reference(x).to_bits(), "{x:e}");
        }
        // Every 997th bit pattern over the whole f32 range.
        let mut b: u32 = 0;
        loop {
            let x = f32::from_bits(b);
            assert_eq!(sanitize(x).to_bits(), reference(x).to_bits(), "{b:#x}");
            match b.checked_add(997) {
                Some(n) => b = n,
                None => break,
            }
        }
    }
}
