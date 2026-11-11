//! Attack-start timing and swell rejection by linear prediction.
//!
//! The spectral-flux envelope finds *where* something new happens to within a frame or so. This
//! module says *when the attack starts* and *whether there is an attack at all*:
//!
//! 1. The region around the candidate is low-passed and decimated to about 12 kHz (musical
//!    attacks live well inside 0..5 kHz; at 12 kHz a 64-tap predictor spans 5.3 ms, long enough
//!    to resolve and predict the partials of ringing notes).
//! 2. A linear predictor (order 64, autocorrelation method, Hann-windowed, -40 dB white-noise
//!    correction) is fitted to the 80 ms that end 10 ms before the search region. Whatever is
//!    already sounding (ringing partials, a decaying note, a reverb tail) is predicted from its
//!    own past; a new note is not.
//! 3. The one-step prediction error from there on is the *new* part of the signal. Its mean
//!    square over the 10 ms right before the search region (measured out of sample) is the
//!    baseline.
//! 4. **Attack start**: the squared error from the baseline segment to the end of the search
//!    region is split into two segments of constant mean square at the most likely point (change
//!    in variance, maximum likelihood). The predictor is causal (64 taps at 12 kHz), so the error
//!    cannot rise before the attack does: a soft or slowly rising attack is timed by its first new
//!    samples, not by where it is loudest. Measured on felt-piano renders against the true note
//!    starts (each note's audio isolated by subtracting a render without it), the error is under
//!    1 ms.
//! 5. **Rise**: the error's mean square over `rise_s` after the start, in dB over the baseline.
//!    A struck note puts in new, unpredictable signal (felt-piano notes under ringing notes rise
//!    2 dB or more, most of them 5-25 dB). The second swell of a note already sounding (beating
//!    unison strings) and reverb build-up only change the level of what is already there; they
//!    stay predictable and do not rise (about -2 dB measured). The caller rejects candidates
//!    below its `min_rise_db`.
//!
//! Pure functions of the input: no state, no randomness, no allocation beyond the analysed region.

/// Attack analysis settings. [`Default`] gives the documented values.
#[derive(Clone, Debug, PartialEq)]
pub struct AttackConfig {
    /// Linear-prediction order at the analysis rate; 0 turns the analysis off.
    pub order: usize,
    /// Length of the segment the predictor is fitted to, seconds.
    pub fit_s: f64,
    /// Length of the baseline segment between the fit segment and the search region, seconds.
    pub baseline_s: f64,
    /// The search region starts this long before the coarse onset position, seconds.
    pub before_s: f64,
    /// The search region ends this long after the coarse onset position, seconds.
    pub after_s: f64,
    /// Window after the attack start in which the rise is measured, seconds.
    pub rise_s: f64,
}

impl Default for AttackConfig {
    fn default() -> Self {
        AttackConfig {
            order: 64,
            fit_s: 0.08,
            baseline_s: 0.01,
            before_s: 0.02,
            after_s: 0.015,
            rise_s: 0.006,
        }
    }
}

/// One analysed attack.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attack {
    /// First sample of the attack.
    pub sample: usize,
    /// Mean-square prediction error over `rise_s` after the start, dB over the baseline.
    pub rise_db: f64,
}

/// Rate the prediction runs at, Hz.
pub const ANALYSIS_RATE: f64 = 12_000.0;

/// Mean-square level treated as digital silence (-120 dB).
const SILENCE_MS: f64 = 1e-12;

/// Integer decimation factor that brings `sample_rate` close to [`ANALYSIS_RATE`] (at least 1).
pub fn decimation(sample_rate: u32) -> usize {
    ((sample_rate as f64 / ANALYSIS_RATE).round() as usize).max(1)
}

/// Windowed-sinc low-pass for decimation by `dec` (Hann, `16 dec + 1` taps, cut-off 0.45 of the
/// decimated rate, unity gain at DC).
fn decimation_filter(dec: usize) -> Vec<f64> {
    let taps = 16 * dec + 1;
    let fc = 0.45 / dec as f64;
    let mid = (taps / 2) as f64;
    let pi = std::f64::consts::PI;
    let h: Vec<f64> = (0..taps)
        .map(|i| {
            let m = i as f64 - mid;
            let sinc = if m == 0.0 {
                2.0 * fc
            } else {
                (2.0 * pi * fc * m).sin() / (pi * m)
            };
            let w = 0.5 - 0.5 * (2.0 * pi * i as f64 / (taps - 1) as f64).cos();
            sinc * w
        })
        .collect();
    let sum: f64 = h.iter().sum();
    h.into_iter().map(|v| v / sum).collect()
}

/// Prediction-error filter `A(z) = 1 + a1 z^-1 + ... + ap z^-p` of `x` (Hann-windowed
/// autocorrelation method, Levinson-Durbin, -40 dB white-noise correction). Returns
/// `[1, a1, .., ap]`; all zeros after the 1 when `x` is silent or shorter than the order.
pub fn lpc(x: &[f64], order: usize) -> Vec<f64> {
    let n = x.len();
    let mut a = vec![0.0; order + 1];
    a[0] = 1.0;
    if n <= order {
        return a;
    }
    let w: Vec<f64> = (0..n)
        .map(|i| {
            let s = (std::f64::consts::PI * (i as f64 + 0.5) / n as f64).sin();
            x[i] * s * s
        })
        .collect();
    let mut r = vec![0.0; order + 1];
    for (k, rk) in r.iter_mut().enumerate() {
        *rk = (k..n).map(|i| w[i] * w[i - k]).sum();
    }
    if r[0].is_nan() || r[0] <= SILENCE_MS * n as f64 {
        return a;
    }
    // White-noise correction keeps the filter well conditioned on pure tones.
    r[0] *= 1.0 + 1e-4;
    let mut err = r[0];
    let mut prev = a.clone();
    for i in 1..=order {
        let mut acc = r[i];
        for j in 1..i {
            acc += prev[j] * r[i - j];
        }
        let k = -acc / err;
        a[i] = k;
        for j in 1..i {
            a[j] = prev[j] + k * prev[i - j];
        }
        err *= 1.0 - k * k;
        if err.is_nan() || err <= 0.0 || !err.is_finite() {
            return prev;
        }
        prev.copy_from_slice(&a);
    }
    a
}

/// Analyses the attack near `centre` (a sample position from the coarse onset envelope). Samples
/// outside the signal read as silence. `None` when the analysis is off (`order` 0), the sample
/// rate is 0 or `centre` lies past the end of the signal.
pub fn find_attack(
    signal: &[f32],
    sample_rate: u32,
    centre: usize,
    cfg: &AttackConfig,
) -> Option<Attack> {
    if sample_rate == 0 || centre >= signal.len() || cfg.order == 0 {
        return None;
    }
    let dec = decimation(sample_rate);
    let rate = sample_rate as f64 / dec as f64;
    // Lengths in decimated samples.
    let d = |s: f64| ((s * rate).round() as usize).max(1);
    let p = cfg.order;
    let (fit, base) = (d(cfg.fit_s).max(p + 1), d(cfg.baseline_s));
    let (before, after, rise) = (d(cfg.before_s), d(cfg.after_s), d(cfg.rise_s));
    // Decimated timeline: index k is original sample `origin + k dec`. Layout: p samples of
    // history, the fit segment, the baseline, the search region, the rise window.
    let total = p + fit + base + before + after + rise;
    let origin = centre as isize - ((p + fit + base + before) * dec) as isize;
    let h = decimation_filter(dec);
    let half = (h.len() / 2) as isize;
    let at = |n: isize| -> f64 {
        if n >= 0 {
            signal.get(n as usize).map(|v| *v as f64).unwrap_or(0.0)
        } else {
            0.0
        }
    };
    let y: Vec<f64> = (0..total)
        .map(|k| {
            let n0 = origin + (k * dec) as isize - half;
            let v: f64 = h
                .iter()
                .enumerate()
                .map(|(i, hi)| hi * at(n0 + i as isize))
                .sum();
            if v.is_finite() { v } else { 0.0 }
        })
        .collect();
    let base0 = p + fit;
    let a = lpc(&y[p..base0], p);
    // Prefix sums of the squared one-step prediction error from the baseline on.
    let mut pre = Vec::with_capacity(total - base0 + 1);
    pre.push(0.0f64);
    for n in base0..total {
        let e: f64 = a.iter().enumerate().map(|(k, ak)| ak * y[n - k]).sum();
        let last = pre.last().copied().unwrap_or(0.0);
        pre.push(last + e * e);
    }
    let len = pre.len() - 1;
    let mean = |lo: usize, hi: usize| ((pre[hi] - pre[lo]) / (hi - lo) as f64).max(SILENCE_MS);
    let lambda0 = mean(0, base);
    // Change in variance: split [0, len) at tau into two segments of constant mean square; the
    // most likely split is the attack start.
    let (lo, hi) = (base, (base + before + after).min(len - 1));
    let (mut best, mut start) = (f64::INFINITY, lo);
    for tau in lo..=hi {
        let cost = tau as f64 * mean(0, tau).ln() + (len - tau) as f64 * mean(tau, len).ln();
        if cost < best {
            best = cost;
            start = tau;
        }
    }
    let rise_db = 10.0 * (mean(start, (start + rise).min(len)) / lambda0).log10();
    let sample = origin + ((base0 + start) * dec) as isize;
    Some(Attack {
        sample: (sample.max(0) as usize).min(signal.len() - 1),
        rise_db,
    })
}
