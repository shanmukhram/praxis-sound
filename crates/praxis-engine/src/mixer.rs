//! Mixing primitives: equal-power gain/pan with per-block linear ramps, ordered summing, master
//! gain. All allocation-free and deterministic (fixed summation order).

use praxis_dsp::util::{db_to_gain, pan_gains, sanitize};

/// Left/right gains of a layer for `gain_db` and `pan`, using the shared equal-power law
/// (`praxis_dsp::util::pan_gains`, -3 dB per side at centre).
pub fn layer_gains(gain_db: f32, pan: f32) -> (f32, f32) {
    let g = db_to_gain(gain_db);
    let (l, r) = pan_gains(pan);
    (g * l, g * r)
}

/// Multiplies a block by gains ramping linearly from `start` (at sample 0) towards `end` (reached
/// at the sample after the block), and flushes NaN/denormals. With `start == end` this is a plain
/// multiply, so constant settings give results independent of the block size.
pub fn apply_gain_ramp(left: &mut [f32], right: &mut [f32], start: (f32, f32), end: (f32, f32)) {
    let n = left.len().min(right.len());
    if start == end {
        let (gl, gr) = start;
        for i in 0..n {
            left[i] = sanitize(left[i] * gl);
            right[i] = sanitize(right[i] * gr);
        }
        return;
    }
    let inv = 1.0 / n.max(1) as f32;
    let (dl, dr) = (end.0 - start.0, end.1 - start.1);
    for i in 0..n {
        let f = i as f32 * inv;
        left[i] = sanitize(left[i] * (start.0 + dl * f));
        right[i] = sanitize(right[i] * (start.1 + dr * f));
    }
}

/// `dst += src`, sample by sample.
pub fn sum_into(dst_l: &mut [f32], dst_r: &mut [f32], src_l: &[f32], src_r: &[f32]) {
    let n = dst_l.len().min(src_l.len());
    for i in 0..n {
        dst_l[i] += src_l[i];
    }
    let n = dst_r.len().min(src_r.len());
    for i in 0..n {
        dst_r[i] += src_r[i];
    }
}

/// Multiplies a block by a constant gain (skipped when the gain is exactly 1).
pub fn apply_gain(left: &mut [f32], right: &mut [f32], gain: f32) {
    if gain == 1.0 {
        return;
    }
    for v in left.iter_mut() {
        *v = sanitize(*v * gain);
    }
    for v in right.iter_mut() {
        *v = sanitize(*v * gain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_power_centre_and_ramp() {
        let (l, r) = layer_gains(0.0, 0.0);
        assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((l - r).abs() < 1e-7);
        let (l, r) = layer_gains(0.0, 1.0);
        assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        let mut a = vec![1.0f32; 4];
        let mut b = vec![1.0f32; 4];
        apply_gain_ramp(&mut a, &mut b, (0.0, 1.0), (1.0, 1.0));
        assert_eq!(a, vec![0.0, 0.25, 0.5, 0.75]);
        assert_eq!(b, vec![1.0; 4]);
    }
}
