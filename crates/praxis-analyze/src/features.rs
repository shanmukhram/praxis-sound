//! Feature extractors (WP-D). Every function here is a pure function of its input: no state, no
//! randomness, no clocks, so the same audio always gives the same numbers.
//!
//! The extractors are shared by render verification ([`crate::verify`]) and, later, by Sound
//! Profiles of reference clips (FR-12). See docs/contracts/sound-profile.md for the methods.
//!
//! | Module | What it measures |
//! | --- | --- |
//! | [`stft`] | Hann-windowed short-time Fourier transform (realfft) |
//! | [`onset`] | log-magnitude spectral-flux onset envelope (10 ms hop) and peak-picked onsets |
//! | [`tempo`] | tempo (60-200 BPM), beat tracking by dynamic programming, downbeat |
//! | [`chroma`] | tuning-robust 12-bin chroma from a log-frequency mapping of the STFT |
//! | [`key`] | key by Krumhansl-Schmuckler and Temperley profiles over 24 keys |
//! | [`loudness`] | BS.1770 integrated loudness, LRA, true peak (`ebur128`), sample peak, RMS |
//! | [`seam`] | loop-seam continuity (sample jump, first-difference energy, spectral flux) |

pub mod chroma;
pub mod key;
pub mod loudness;
pub mod onset;
pub mod seam;
pub mod stft;
pub mod tempo;

/// Errors from feature extraction.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FeatureError {
    #[error("sample rate must be greater than zero")]
    SampleRate,
    #[error("left and right channels differ in length ({left} vs {right})")]
    ChannelLength { left: usize, right: usize },
    #[error("loudness meter failed: {0}")]
    Meter(String),
}

/// Level reported for silence instead of minus infinity, dB (keeps JSON finite).
pub const FLOOR_DB: f64 = -144.0;

/// Mono mix `(L + R) / 2`. Non-finite samples become 0. The shorter channel sets the length.
pub fn mono(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter()
        .zip(right.iter())
        .map(|(l, r)| finite_or_zero(0.5 * (*l + *r)))
        .collect()
}

/// `x`, or 0 when `x` is NaN or infinite.
pub fn finite_or_zero(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}

/// Amplitude ratio to dB, floored at [`FLOOR_DB`].
pub fn amp_to_db(a: f64) -> f64 {
    if a > 0.0 && a.is_finite() {
        (20.0 * a.log10()).max(FLOOR_DB)
    } else {
        FLOOR_DB
    }
}

/// Value at quantile `q` (0..=1) of `values` using linear interpolation between order
/// statistics. Returns 0 for an empty slice. NaNs sort last (total order) and are not expected.
pub fn quantile(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let pos = q.clamp(0.0, 1.0) * (v.len() - 1) as f64;
    let i = pos.floor() as usize;
    let j = (i + 1).min(v.len() - 1);
    let f = pos - i as f64;
    v[i] * (1.0 - f) + v[j] * f
}

/// Median of `values` (0 for an empty slice).
pub fn median(values: &[f64]) -> f64 {
    quantile(values, 0.5)
}
