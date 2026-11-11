//! Stereo width and left/right correlation.
//!
//! `width = E(side) / E(mid)` with `mid = (L + R) / 2`, `side = (L - R) / 2` (0 for mono, 1 for
//! uncorrelated channels of equal level, large for out-of-phase material), and the Pearson-style
//! correlation `sum(L R) / sqrt(sum(L^2) sum(R^2))` (1 for mono). Silence reads width 0,
//! correlation 1. Non-finite samples count as 0.

use super::finite_or_zero;
use crate::profile::StereoStats;

pub fn measure(left: &[f32], right: &[f32]) -> StereoStats {
    let (mut em, mut es, mut lr, mut ll, mut rr) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (l, r) in left.iter().zip(right.iter()) {
        let l = finite_or_zero(*l) as f64;
        let r = finite_or_zero(*r) as f64;
        let m = 0.5 * (l + r);
        let s = 0.5 * (l - r);
        em += m * m;
        es += s * s;
        lr += l * r;
        ll += l * l;
        rr += r * r;
    }
    let width = if em > 1e-12 {
        (es / em).min(100.0)
    } else if es > 1e-12 {
        100.0
    } else {
        0.0
    };
    let correlation = if ll > 1e-12 && rr > 1e-12 {
        (lr / (ll * rr).sqrt()).clamp(-1.0, 1.0)
    } else {
        1.0
    };
    StereoStats {
        width: width as f32,
        correlation: correlation as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_and_inverted() {
        let x: Vec<f32> = (0..1000).map(|i| ((i as f32) * 0.1).sin()).collect();
        let m = measure(&x, &x);
        assert_eq!(m.width, 0.0);
        assert!((m.correlation - 1.0).abs() < 1e-6);
        let inv: Vec<f32> = x.iter().map(|v| -v).collect();
        let o = measure(&x, &inv);
        assert!(o.width >= 99.0 && o.correlation < -0.99);
        let s = measure(&[0.0; 10], &[0.0; 10]);
        assert_eq!((s.width, s.correlation), (0.0, 1.0));
    }
}
