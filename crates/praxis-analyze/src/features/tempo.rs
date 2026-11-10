//! Tempo estimation, beat tracking and downbeat estimation from an onset envelope.
//!
//! **Tempo** (60-200 BPM by default):
//! 1. The envelope is smoothed (Gaussian, sigma one frame) and its mean removed.
//! 2. Its normalised autocorrelation `r(lag)` is read (linearly interpolated) at the lag of every
//!    candidate tempo on a 0.05 BPM grid and weighted by a log-normal tempo prior (centre 125 BPM,
//!    one octave standard deviation). The prior only breaks near-ties between octave levels.
//! 3. Octave handling: among the octave relatives of the winner (x0.5, x2, x1.5, x2/3 inside the
//!    range) the candidate whose own periodicity `r` (times prior) is strongest wins; within 3 % the
//!    faster level is preferred, because it explains every onset of the slower one.
//! 4. The tempo is refined by comb-filter scoring: for each tempo within +-3 % (0.01 BPM steps) a
//!    pulse train at the best phase is laid over the whole envelope and the mean envelope value
//!    under its teeth is the score. Long clips give sub-0.1 BPM precision.
//!
//! Confidence is the normalised autocorrelation at the final lag, clamped to 0..1.
//!
//! **Beats**: dynamic programming (Ellis 2007): `score[t] = env[t] + max over tau of
//! (score[tau] - tightness * ln((t - tau) / period)^2)`, backtracked from the best frame in the
//! last period, each beat snapped to a detected onset within a fifth of a period.
//!
//! **Downbeat**: of the `beats_per_bar` phases, the one whose beats have the largest mean energy
//! rise (accent).

use super::onset::{Onset, OnsetEnvelope, detect_onsets};
use crate::profile::TempoEstimate;

/// Tempo search settings.
#[derive(Clone, Debug, PartialEq)]
pub struct TempoConfig {
    pub min_bpm: f64,
    pub max_bpm: f64,
    /// Centre of the log-normal tempo prior, BPM.
    pub prior_centre_bpm: f64,
    /// Standard deviation of the prior, octaves.
    pub prior_octaves: f64,
    /// Coarse search step, BPM.
    pub step_bpm: f64,
    /// Fine (comb) search step, BPM.
    pub fine_step_bpm: f64,
    /// Fine search half-width as a fraction of the coarse tempo.
    pub fine_span: f64,
    /// Octave relatives within this fraction of the best score prefer the faster tempo.
    pub tie_ratio: f64,
}

impl Default for TempoConfig {
    fn default() -> Self {
        TempoConfig {
            min_bpm: 60.0,
            max_bpm: 200.0,
            prior_centre_bpm: 125.0,
            prior_octaves: 1.0,
            step_bpm: 0.05,
            fine_step_bpm: 0.01,
            fine_span: 0.03,
            tie_ratio: 0.97,
        }
    }
}

/// Estimated tempo.
#[derive(Clone, Debug, PartialEq)]
pub struct TempoResult {
    pub bpm: f64,
    /// 0..1, the normalised autocorrelation of the envelope at the beat period.
    pub confidence: f32,
}

/// Gaussian smoothing with sigma one frame (kernel radius 3).
fn smooth(values: &[f32]) -> Vec<f64> {
    let k: Vec<f64> = (-3i32..=3).map(|i| (-0.5 * (i * i) as f64).exp()).collect();
    let ks: f64 = k.iter().sum();
    let n = values.len() as isize;
    (0..n)
        .map(|i| {
            let mut s = 0.0;
            for (j, w) in k.iter().enumerate() {
                let idx = i + j as isize - 3;
                if idx >= 0 && idx < n {
                    s += w * values[idx as usize] as f64;
                }
            }
            s / ks
        })
        .collect()
}

fn interp(v: &[f64], x: f64) -> f64 {
    if x < 0.0 || v.is_empty() {
        return 0.0;
    }
    let i = x.floor() as usize;
    if i + 1 >= v.len() {
        return if i < v.len() { v[i] } else { 0.0 };
    }
    let f = x - i as f64;
    v[i] * (1.0 - f) + v[i + 1] * f
}

fn prior(cfg: &TempoConfig, bpm: f64) -> f64 {
    let o = (bpm / cfg.prior_centre_bpm).log2() / cfg.prior_octaves.max(1e-6);
    (-0.5 * o * o).exp()
}

/// Normalised autocorrelation of the mean-removed envelope up to `max_lag`.
fn autocorrelation(x: &[f64], max_lag: usize) -> Vec<f64> {
    let n = x.len();
    let mean = x.iter().sum::<f64>() / n.max(1) as f64;
    let y: Vec<f64> = x.iter().map(|v| v - mean).collect();
    let r0: f64 = y.iter().map(|v| v * v).sum();
    let max_lag = max_lag.min(n.saturating_sub(1));
    (0..=max_lag)
        .map(|lag| {
            if r0 <= 0.0 {
                return 0.0;
            }
            let s: f64 = y[..n - lag].iter().zip(&y[lag..]).map(|(a, b)| a * b).sum();
            s / r0
        })
        .collect()
}

/// Mean of `x` under a pulse train of period `period` frames at the best phase.
fn comb_score(x: &[f64], period: f64) -> f64 {
    let n = x.len() as f64;
    if period <= 0.0 || n < period {
        return 0.0;
    }
    let mut best = 0.0f64;
    let mut phase = 0.0;
    while phase < period {
        let mut s = 0.0;
        let mut c = 0usize;
        let mut t = phase;
        while t < n - 1.0 {
            s += interp(x, t);
            c += 1;
            t += period;
        }
        if c > 0 {
            best = best.max(s / c as f64);
        }
        phase += 0.25;
    }
    best
}

/// Estimates the tempo of an onset envelope. `None` when the envelope is too short (under two
/// periods of the slowest tempo) or has no periodic structure.
pub fn estimate_tempo(env: &OnsetEnvelope, cfg: &TempoConfig) -> Option<TempoResult> {
    let fps = env.frame_rate();
    if env.values.is_empty()
        || fps.is_nan()
        || fps <= 0.0
        || cfg.min_bpm <= 0.0
        || cfg.max_bpm <= cfg.min_bpm
    {
        return None;
    }
    let xs = smooth(&env.values);
    let lag_of = |bpm: f64| 60.0 * fps / bpm;
    let max_lag = (lag_of(cfg.min_bpm) * 1.6).ceil() as usize + 2;
    let r = autocorrelation(&xs, max_lag);
    let n = xs.len() as f64;
    let usable = |bpm: f64| lag_of(bpm) * 2.0 <= n;
    let strength = |bpm: f64| interp(&r, lag_of(bpm)).max(0.0) * prior(cfg, bpm);

    // Coarse: best prior-weighted autocorrelation peak.
    let steps = ((cfg.max_bpm - cfg.min_bpm) / cfg.step_bpm).floor() as usize;
    let mut best: Option<(f64, f64)> = None;
    for i in 0..=steps {
        let bpm = cfg.min_bpm + i as f64 * cfg.step_bpm;
        if !usable(bpm) {
            continue;
        }
        let s = strength(bpm);
        if best.is_none_or(|b| s > b.1) {
            best = Some((bpm, s));
        }
    }
    let (mut bpm0, mut s0) = best?;
    if s0 <= 0.0 {
        return None;
    }

    // Octave relatives: strongest periodicity wins; near-ties go to the faster level.
    let peak_near = |bpm: f64| -> Option<(f64, f64)> {
        let mut b: Option<(f64, f64)> = None;
        let span = bpm * 0.02;
        let k = (2.0 * span / cfg.step_bpm).ceil() as usize;
        for i in 0..=k {
            let t = bpm - span + i as f64 * cfg.step_bpm;
            if t < cfg.min_bpm || t > cfg.max_bpm || !usable(t) {
                continue;
            }
            let s = strength(t);
            if b.is_none_or(|x| s > x.1) {
                b = Some((t, s));
            }
        }
        b
    };
    for factor in [2.0, 0.5, 1.5, 2.0 / 3.0] {
        if let Some((t, s)) = peak_near(bpm0 * factor) {
            let faster = t > bpm0;
            let wins = if faster {
                s >= s0 * cfg.tie_ratio
            } else {
                s > s0 / cfg.tie_ratio
            };
            if wins {
                bpm0 = t;
                s0 = s;
            }
        }
    }

    // Fine: comb-filter scoring over the whole envelope.
    let lo = (bpm0 * (1.0 - cfg.fine_span)).max(cfg.min_bpm * 0.97);
    let hi = (bpm0 * (1.0 + cfg.fine_span)).min(cfg.max_bpm * 1.03);
    let k = ((hi - lo) / cfg.fine_step_bpm).floor() as usize;
    let mut fine = (bpm0, f64::NEG_INFINITY);
    for i in 0..=k {
        let bpm = lo + i as f64 * cfg.fine_step_bpm;
        let s = comb_score(&xs, lag_of(bpm));
        if s > fine.1 {
            fine = (bpm, s);
        }
    }
    let bpm = fine.0;
    let confidence = interp(&r, lag_of(bpm)).clamp(0.0, 1.0) as f32;
    Some(TempoResult { bpm, confidence })
}

/// Beat frames by dynamic programming for a known tempo. `tightness` 100 is a good default.
pub fn track_beats(env: &OnsetEnvelope, bpm: f64, tightness: f64) -> Vec<usize> {
    let fps = env.frame_rate();
    let n = env.values.len();
    if n == 0 || bpm.is_nan() || bpm <= 0.0 || fps.is_nan() || fps <= 0.0 {
        return Vec::new();
    }
    let period = 60.0 * fps / bpm;
    let xs = smooth(&env.values);
    let sd = {
        let m = xs.iter().sum::<f64>() / n as f64;
        (xs.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / n as f64).sqrt()
    };
    let local: Vec<f64> = xs
        .iter()
        .map(|v| if sd > 0.0 { v / sd } else { 0.0 })
        .collect();
    let min_back = (period / 2.0).round().max(1.0) as usize;
    let max_back = (period * 2.0).round().max(1.0) as usize;
    let mut score = vec![0.0f64; n];
    let mut back: Vec<Option<usize>> = vec![None; n];
    for t in 0..n {
        let mut best: Option<(usize, f64)> = None;
        if t >= min_back {
            let lo = t.saturating_sub(max_back);
            for tau in lo..=t - min_back {
                let d = ((t - tau) as f64 / period).ln();
                let s = score[tau] - tightness * d * d;
                if best.is_none_or(|b| s > b.1) {
                    best = Some((tau, s));
                }
            }
        }
        match best {
            Some((tau, s)) => {
                score[t] = local[t] + s;
                back[t] = Some(tau);
            }
            None => score[t] = local[t],
        }
    }
    let tail = n.saturating_sub(period.round().max(1.0) as usize);
    let mut end = tail;
    for t in tail..n {
        if score[t] > score[end] {
            end = t;
        }
    }
    let mut beats = vec![end];
    let mut cur = end;
    while let Some(p) = back[cur] {
        beats.push(p);
        cur = p;
    }
    beats.reverse();
    beats
}

/// Beat times in seconds, each snapped to an onset within a fifth of a period when one exists.
pub fn beat_times(env: &OnsetEnvelope, beats: &[usize], onsets: &[Onset], bpm: f64) -> Vec<f64> {
    let tol = if bpm > 0.0 { 0.2 * 60.0 / bpm } else { 0.0 };
    beats
        .iter()
        .map(|b| {
            let t = env.time_s(*b);
            onsets
                .iter()
                .map(|o| o.time_s)
                .filter(|o| (o - t).abs() <= tol)
                .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
                .unwrap_or(t)
        })
        .collect()
}

/// Index of the first downbeat in `beats` (the phase whose beats carry the most onset energy).
///
/// The accent of a beat is the energy rise across it: signal energy in the 60 ms after the beat
/// minus the 60 ms before (floored at 0), so louder attacks, not brighter ones, mark the bar.
pub fn downbeat_phase(
    signal: &[f32],
    sample_rate: u32,
    beats_s: &[f64],
    beats_per_bar: usize,
) -> usize {
    let bpb = beats_per_bar.max(1);
    let sr = sample_rate as f64;
    let w = ((0.06 * sr).round() as usize).max(1);
    let energy = |a: usize, b: usize| -> f64 {
        let b = b.min(signal.len());
        let a = a.min(b);
        signal[a..b].iter().map(|x| (*x as f64).powi(2)).sum()
    };
    let accent = |t: f64| -> f64 {
        let c = (t.max(0.0) * sr).round() as usize;
        (energy(c, c + w) - energy(c.saturating_sub(w), c)).max(0.0)
    };
    let mut best = (0usize, f64::NEG_INFINITY);
    for phase in 0..bpb.min(beats_s.len().max(1)) {
        let vals: Vec<f64> = beats_s
            .iter()
            .skip(phase)
            .step_by(bpb)
            .map(|t| accent(*t))
            .collect();
        if vals.is_empty() {
            continue;
        }
        let m = vals.iter().sum::<f64>() / vals.len() as f64;
        if m > best.1 {
            best = (phase, m);
        }
    }
    best.0
}

/// Full tempo analysis of a mono signal: tempo, beats, downbeat. `None` without a usable tempo.
pub fn analyze(signal: &[f32], sample_rate: u32, beats_per_bar: u8) -> Option<TempoEstimate> {
    let (env, onsets) = detect_onsets(signal, sample_rate);
    let t = estimate_tempo(&env, &TempoConfig::default())?;
    let frames = track_beats(&env, t.bpm, 100.0);
    let beats_s = beat_times(&env, &frames, &onsets, t.bpm);
    let phase = downbeat_phase(signal, sample_rate, &beats_s, beats_per_bar as usize);
    let downbeat_s = beats_s.get(phase).copied().unwrap_or(0.0);
    Some(TempoEstimate {
        bpm: t.bpm,
        confidence: t.confidence,
        beats_s,
        downbeat_s,
        beats_per_bar: beats_per_bar.max(1),
    })
}
