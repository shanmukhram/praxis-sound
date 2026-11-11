//! Amplitude envelope and its attack / decay / sustain / release fit.
//!
//! **Frames**: RMS of the mono signal over consecutive 10 ms blocks ([`HOP_S`]). The profile
//! keeps at most [`MAX_FRAMES`] of them: longer clips are decimated by an integer factor `k`
//! (power-averaged), and `hop_s` becomes `k * 10 ms`.
//!
//! **ADSR fit** (on the 10 ms amplitude envelope smoothed over 50 ms):
//! - *active span*: from the first to the last frame at or above 10 % (-20 dB) of the peak;
//! - *sustain*: the median amplitude of the active span relative to the peak (0..1);
//! - *attack*: from the start of the active span to the first frame reaching
//!   `max(sustain, 0.5) * peak` (a percussive hit reaches it at once, a swell or riser late);
//! - *decay*: from the loudest frame within 0.5 s after the attack to the first frame that falls
//!   to `1.05 * max(sustain, 0.1) * peak` (0 when it never falls that far);
//! - *release*: from the last frame at or above the sustain level to the end of the active span.

use super::{FLOOR_DB, amp_to_db, median};

/// Envelope hop, seconds.
pub const HOP_S: f64 = 0.01;
/// Most envelope frames kept in a profile.
pub const MAX_FRAMES: usize = 2000;

/// RMS amplitude of each consecutive `hop`-sample block of `signal`.
pub fn rms_frames(signal: &[f32], hop: usize) -> Vec<f64> {
    let hop = hop.max(1);
    signal
        .chunks(hop)
        .map(|c| {
            let s: f64 = c.iter().map(|x| (*x as f64) * (*x as f64)).sum();
            (s / c.len().max(1) as f64).sqrt()
        })
        .collect()
}

/// Hop in samples for [`HOP_S`].
pub fn hop_samples(sample_rate: u32) -> usize {
    ((HOP_S * sample_rate as f64).round() as usize).max(1)
}

/// Decimation factor that keeps `n` frames at or under [`MAX_FRAMES`].
pub fn decimation(n: usize) -> usize {
    n.div_ceil(MAX_FRAMES).max(1)
}

/// Power-averages consecutive groups of `k` amplitude frames.
pub fn decimate_rms(frames: &[f64], k: usize) -> Vec<f64> {
    let k = k.max(1);
    frames
        .chunks(k)
        .map(|c| (c.iter().map(|a| a * a).sum::<f64>() / c.len() as f64).sqrt())
        .collect()
}

/// Mean of groups of `k` values (for tracks that share the envelope's hop).
pub fn decimate_mean(values: &[f64], k: usize) -> Vec<f64> {
    let k = k.max(1);
    values
        .chunks(k)
        .map(|c| c.iter().sum::<f64>() / c.len() as f64)
        .collect()
}

/// Centred moving average over `2 * radius + 1` frames.
pub fn smooth(values: &[f64], radius: usize) -> Vec<f64> {
    let n = values.len();
    let mut prefix = vec![0.0f64; n + 1];
    for i in 0..n {
        prefix[i + 1] = prefix[i] + values[i];
    }
    (0..n)
        .map(|i| {
            let a = i.saturating_sub(radius);
            let b = (i + radius + 1).min(n);
            (prefix[b] - prefix[a]) / (b - a).max(1) as f64
        })
        .collect()
}

/// Fitted envelope shape, seconds and 0..1.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Adsr {
    pub attack_s: f64,
    pub decay_s: f64,
    pub sustain: f64,
    pub release_s: f64,
    /// Active span (first and last frame at or above -20 dB of the peak), seconds.
    pub start_s: f64,
    pub end_s: f64,
}

/// ADSR fit of a 10 ms amplitude envelope (see the module docs).
pub fn fit_adsr(frames: &[f64], hop_s: f64) -> Adsr {
    let a = smooth(frames, 2);
    let peak = a.iter().fold(0.0f64, |m, v| m.max(*v));
    if a.is_empty() || peak <= 0.0 || !peak.is_finite() {
        return Adsr::default();
    }
    let lo = 0.1 * peak;
    let start = a.iter().position(|v| *v >= lo).unwrap_or(0);
    let end = a.iter().rposition(|v| *v >= lo).unwrap_or(start);
    let span = &a[start..=end];
    let sustain = (median(span) / peak).clamp(0.0, 1.0);
    let target = sustain.max(0.5) * peak;
    let attack_end = (start..=end).find(|i| a[*i] >= target).unwrap_or(start);
    let look = ((0.5 / hop_s).round() as usize).max(1);
    let local_end = (attack_end + look).min(end);
    let mut local_peak = attack_end;
    for i in attack_end..=local_end {
        if a[i] > a[local_peak] {
            local_peak = i;
        }
    }
    let floor = 1.05 * sustain.max(0.1) * peak;
    let decay_end = (local_peak..=end).find(|i| a[*i] <= floor);
    let decay_s = decay_end.map_or(0.0, |d| (d - local_peak) as f64 * hop_s);
    let sustain_abs = sustain * peak;
    let from = decay_end.unwrap_or(local_peak);
    let rel_start = (from..=end)
        .rev()
        .find(|i| a[*i] >= sustain_abs)
        .unwrap_or(from);
    Adsr {
        attack_s: (attack_end - start) as f64 * hop_s,
        decay_s,
        sustain,
        release_s: (end - rel_start) as f64 * hop_s,
        start_s: start as f64 * hop_s,
        end_s: (end + 1) as f64 * hop_s,
    }
}

/// Amplitudes to dBFS (floored), as `f32`.
pub fn to_db(frames: &[f64]) -> Vec<f32> {
    frames
        .iter()
        .map(|a| amp_to_db(*a).max(FLOOR_DB) as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(f: impl Fn(f64) -> f64, secs: f64) -> Vec<f64> {
        (0..(secs / HOP_S) as usize)
            .map(|i| f(i as f64 * HOP_S))
            .collect()
    }

    #[test]
    fn percussive_hit_has_short_attack_and_low_sustain() {
        let e = env(
            |t| {
                if t < 0.005 {
                    t / 0.005
                } else {
                    (-(t - 0.005) * 8.0).exp()
                }
            },
            2.0,
        );
        let f = fit_adsr(&e, HOP_S);
        assert!(f.attack_s <= 0.03, "{f:?}");
        assert!(f.sustain < 0.5, "{f:?}");
        assert!(f.decay_s > 0.05, "{f:?}");
    }

    #[test]
    fn swell_has_long_attack() {
        let e = env(|t| (t / 3.0).min(1.0), 4.0);
        let f = fit_adsr(&e, HOP_S);
        assert!(f.attack_s > 1.0, "{f:?}");
    }

    #[test]
    fn decimation_keeps_at_most_max_frames() {
        let n = 18_001;
        let k = decimation(n);
        assert!(n.div_ceil(k) <= MAX_FRAMES);
        assert_eq!(decimation(10), 1);
    }
}
