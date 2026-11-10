//! WP-B2 dsp-effects: loudness (BS.1770), true peak, RMS. See docs/contracts/dsp.md.

pub mod loudness;
pub mod true_peak;

pub use loudness::{LoudnessMeter, integrated_lufs};
pub use true_peak::{TruePeakDetector, true_peak_dbtp};

/// Sample peak of a stereo signal in dBFS (`-inf` for silence).
pub fn sample_peak_dbfs(left: &[f32], right: &[f32]) -> f64 {
    let p = left
        .iter()
        .chain(right.iter())
        .fold(0.0f32, |m, v| if v.abs() > m { v.abs() } else { m }) as f64;
    if p > 0.0 {
        20.0 * p.log10()
    } else {
        f64::NEG_INFINITY
    }
}

/// RMS of a stereo signal in dBFS over both channels: `sqrt(mean(l^2 + r^2) / 2)`.
/// A full-scale sine on both channels reads -3.01 dBFS. `-inf` for silence or no samples.
pub fn rms_dbfs(left: &[f32], right: &[f32]) -> f64 {
    let n = left.len() + right.len();
    if n == 0 {
        return f64::NEG_INFINITY;
    }
    let s: f64 = left
        .iter()
        .chain(right.iter())
        .map(|&v| v as f64 * v as f64)
        .sum();
    let ms = s / n as f64;
    if ms > 0.0 {
        10.0 * ms.log10()
    } else {
        f64::NEG_INFINITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_and_rms() {
        let s: Vec<f32> = (0..48000)
            .map(|n| (2.0 * std::f32::consts::PI * 1000.0 * n as f32 / 48000.0).sin() * 0.5)
            .collect();
        assert!((sample_peak_dbfs(&s, &s) - 20.0 * 0.5f64.log10()).abs() < 0.01);
        assert!((rms_dbfs(&s, &s) - (20.0 * 0.5f64.log10() - 3.0103)).abs() < 0.01);
        assert_eq!(rms_dbfs(&[0.0], &[0.0]), f64::NEG_INFINITY);
    }
}
