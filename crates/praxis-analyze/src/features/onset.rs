//! Onset strength envelope and onset detection.
//!
//! 1. **Envelope**: log-magnitude spectral flux. Each STFT frame (about 21 ms, Hann) is compressed
//!    to `ln(1 + compression * |X|)`, and the envelope value of frame `i` is the mean over bins of
//!    the positive increase from frame `i - 1` (frame `-1` is silence). Hop 10 ms. Frames whose
//!    window runs past the last sample read 0 (the abrupt end would look like an onset).
//! 2. **Peak picking**: a frame is an onset candidate when it is the local maximum within
//!    `max_radius` frames and exceeds an adaptive threshold: the local mean plus `delta` times the
//!    largest value within `context_s`. Candidates closer than `min_gap_s` keep the stronger one.
//! 3. **Refinement**: with log compression the flux peak leads the true attack by up to half a
//!    frame (the new energy is noticed as soon as it enters the window edge), too coarse for a
//!    10 ms grid check. Each onset is re-placed at the maximum of a rectified complex-domain
//!    onset function (Duxbury/Bello) computed with ~20 ms frames at a 0.5 ms hop, searched from
//!    one hop before to half a frame after the coarse peak. Each bin is predicted from the two
//!    previous frames (same magnitude, same phase advance), so ringing partials, including a
//!    still-sounding note of the same pitch, predict themselves and only new energy counts. The
//!    deviation of a sustained attack is largest when the attack crosses the window centre, so
//!    the refined time sits on the attack (within ~2 ms for tones, ~2.5 ms early for bare clicks).

use super::stft::{FrameAnalyzer, frame_len_for};
use realfft::num_complex::Complex;

/// Onset analysis settings. [`Default`] gives the documented values.
#[derive(Clone, Debug, PartialEq)]
pub struct OnsetConfig {
    /// Envelope hop, seconds (10 ms).
    pub hop_s: f64,
    /// STFT frame length, seconds (rounded up to a power of two: 1024 samples at 44.1/48 kHz).
    pub frame_s: f64,
    /// Log compression `ln(1 + compression * |X|)`.
    pub compression: f32,
    /// A peak must be the maximum within this many frames on each side.
    pub max_radius: usize,
    /// Frames before/after the peak used for the local mean.
    pub mean_radius: usize,
    /// Threshold above the local mean, as a fraction of the context maximum.
    pub delta: f32,
    /// Half-width of the context window for the adaptive threshold, seconds.
    pub context_s: f64,
    /// Absolute floor as a fraction of the global envelope maximum.
    pub floor: f32,
    /// Minimum time between two onsets, seconds.
    pub min_gap_s: f64,
    /// Refine onset times in the time domain.
    pub refine: bool,
}

impl Default for OnsetConfig {
    fn default() -> Self {
        OnsetConfig {
            hop_s: 0.01,
            frame_s: 0.02,
            compression: 100.0,
            max_radius: 3,
            mean_radius: 8,
            delta: 0.1,
            context_s: 1.5,
            floor: 0.02,
            min_gap_s: 0.04,
            refine: true,
        }
    }
}

impl OnsetConfig {
    /// Hop in samples at `sample_rate` (441 at 44.1 kHz, 480 at 48 kHz).
    pub fn hop(&self, sample_rate: u32) -> usize {
        ((self.hop_s * sample_rate as f64).round() as usize).max(1)
    }
}

/// Onset strength envelope: one value per hop, frame `i` centred on sample `i * hop`.
#[derive(Clone, Debug, PartialEq)]
pub struct OnsetEnvelope {
    pub sample_rate: u32,
    pub hop: usize,
    pub frame_len: usize,
    pub values: Vec<f32>,
}

impl OnsetEnvelope {
    /// Envelope frames per second.
    pub fn frame_rate(&self) -> f64 {
        self.sample_rate as f64 / self.hop.max(1) as f64
    }

    /// Time of frame `i`, seconds.
    pub fn time_s(&self, i: usize) -> f64 {
        (i * self.hop) as f64 / self.sample_rate.max(1) as f64
    }
}

/// One detected onset.
#[derive(Clone, Debug, PartialEq)]
pub struct Onset {
    /// Refined onset time, seconds from the first sample.
    pub time_s: f64,
    /// Refined onset position, samples.
    pub sample: usize,
    /// Envelope frame of the spectral-flux peak.
    pub frame: usize,
    /// Envelope value at the peak.
    pub strength: f32,
}

/// Log-magnitude spectral-flux onset envelope of `signal`.
pub fn onset_envelope(signal: &[f32], sample_rate: u32, cfg: &OnsetConfig) -> OnsetEnvelope {
    let hop = cfg.hop(sample_rate);
    let mut fa = FrameAnalyzer::new(frame_len_for(cfg.frame_s, sample_rate.max(1)));
    let frame_len = fa.frame_len();
    let bins = fa.bins();
    let frames = if signal.is_empty() {
        0
    } else {
        (signal.len() - 1) / hop + 1
    };
    let half = frame_len as isize / 2;
    let mut prev = vec![0.0f32; bins];
    let mut cur = vec![0.0f32; bins];
    let mut values = Vec::with_capacity(frames);
    for i in 0..frames {
        let start = (i * hop) as isize - half;
        // A window running past the last sample sees the signal stop abruptly, which reads as
        // broadband flux. Those last frames carry no onset information; report 0.
        if i > 0 && start + frame_len as isize > signal.len() as isize {
            values.push(0.0);
            continue;
        }
        fa.magnitudes_with(
            |j| {
                let n = start + j as isize;
                if n >= 0 {
                    signal.get(n as usize).copied().unwrap_or(0.0)
                } else {
                    0.0
                }
            },
            &mut cur,
        );
        let mut flux = 0.0f64;
        for (c, p) in cur.iter_mut().zip(prev.iter()) {
            *c = (cfg.compression * *c).ln_1p();
            let d = *c - *p;
            if d > 0.0 {
                flux += d as f64;
            }
        }
        values.push((flux / bins as f64) as f32);
        std::mem::swap(&mut prev, &mut cur);
    }
    OnsetEnvelope {
        sample_rate,
        hop,
        frame_len,
        values,
    }
}

/// Peak-picks the envelope with the adaptive threshold described in the module docs.
/// Returns frame indices in increasing order.
pub fn pick_peaks(env: &OnsetEnvelope, cfg: &OnsetConfig) -> Vec<usize> {
    let v = &env.values;
    let n = v.len();
    let gmax = v.iter().fold(0.0f32, |m, x| m.max(*x));
    if n == 0 || gmax <= 0.0 {
        return Vec::new();
    }
    let ctx = ((cfg.context_s * env.frame_rate()).round() as usize).max(1);
    let min_gap = (cfg.min_gap_s * env.frame_rate()).round() as usize;
    let r = cfg.max_radius;
    let mut out: Vec<usize> = Vec::new();
    for i in 0..n {
        let x = v[i];
        if x <= cfg.floor * gmax {
            continue;
        }
        let lo = i.saturating_sub(r);
        let hi = (i + r).min(n - 1);
        // Plateaus resolve to their first frame.
        let is_max = (lo..i).all(|j| v[j] < x) && (i + 1..=hi).all(|j| v[j] <= x);
        if !is_max {
            continue;
        }
        let mlo = i.saturating_sub(cfg.mean_radius);
        let mhi = (i + cfg.mean_radius).min(n - 1);
        let mean = v[mlo..=mhi].iter().map(|x| *x as f64).sum::<f64>() / (mhi - mlo + 1) as f64;
        let clo = i.saturating_sub(ctx);
        let chi = (i + ctx).min(n - 1);
        let cmax = v[clo..=chi].iter().fold(0.0f32, |m, x| m.max(*x));
        if (x as f64) < mean + (cfg.delta * cmax) as f64 {
            continue;
        }
        match out.last_mut() {
            Some(last) if i - *last < min_gap => {
                if v[*last] < x {
                    *last = i;
                }
            }
            _ => out.push(i),
        }
    }
    out
}

/// Fine-hop complex-domain onset function, used to place onsets precisely.
pub struct Refiner<'a> {
    signal: &'a [f32],
    fa: FrameAnalyzer,
    hop: usize,
    /// Spectra at `c - 2 hop`, `c - hop`, `c`.
    older: Vec<Complex<f32>>,
    prev: Vec<Complex<f32>>,
    cur: Vec<Complex<f32>>,
}

impl std::fmt::Debug for Refiner<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Refiner")
            .field("frame_len", &self.fa.frame_len())
            .field("hop", &self.hop)
            .finish()
    }
}

impl<'a> Refiner<'a> {
    /// Refinement frames of about 20 ms (1024 samples at 44.1/48 kHz), hop 0.5 ms.
    pub fn new(signal: &'a [f32], sample_rate: u32) -> Refiner<'a> {
        let fa = FrameAnalyzer::new(frame_len_for(0.02, sample_rate.max(1)));
        let bins = fa.bins();
        let zero = Complex::new(0.0, 0.0);
        Refiner {
            signal,
            fa,
            hop: ((0.0005 * sample_rate as f64).round() as usize).max(1),
            older: vec![zero; bins],
            prev: vec![zero; bins],
            cur: vec![zero; bins],
        }
    }

    /// Refinement frame length, samples.
    pub fn frame_len(&self) -> usize {
        self.fa.frame_len()
    }

    /// Computes the spectrum of the frame centred on `centre` into `cur`.
    fn spectrum_at(&mut self, centre: isize) {
        let start = centre - self.fa.frame_len() as isize / 2;
        let signal = self.signal;
        self.fa.complex_with(
            |j| {
                let n = start + j as isize;
                if n >= 0 {
                    signal.get(n as usize).copied().unwrap_or(0.0)
                } else {
                    0.0
                }
            },
            &mut self.cur,
        );
    }

    /// Rectified complex-domain deviation of `cur` from the prediction made from `prev` and
    /// `older` (same magnitude as `prev`, phase advanced by the last phase step).
    fn deviation(&self) -> f64 {
        let eps = 1e-9f32;
        let mut s = 0.0f64;
        for ((c, p), o) in self.cur.iter().zip(&self.prev).zip(&self.older) {
            let (mc, mp, mo) = (c.norm(), p.norm(), o.norm());
            if mc < mp {
                continue;
            }
            let predicted = if mp < eps {
                Complex::new(0.0, 0.0)
            } else if mo < eps {
                *p
            } else {
                let up = *p / mp;
                let uo = *o / mo;
                *p * (up * uo.conj())
            };
            s += (*c - predicted).norm() as f64;
        }
        s
    }

    /// Position in `lo..=hi` (samples, stepped by the fine hop) of the largest rectified
    /// complex-domain deviation, or `None` when nothing new appears in the range.
    pub fn refine(&mut self, lo: usize, hi: usize) -> Option<usize> {
        let hop = self.hop as isize;
        let mut c = lo as isize;
        self.spectrum_at(c - 2 * hop);
        std::mem::swap(&mut self.older, &mut self.cur);
        self.spectrum_at(c - hop);
        std::mem::swap(&mut self.prev, &mut self.cur);
        let mut devs: Vec<(isize, f64)> = Vec::new();
        while c <= hi as isize {
            self.spectrum_at(c);
            // The change between frames at c - hop and c describes their midpoint.
            devs.push((c - hop / 2, self.deviation()));
            // Rotate: older <- prev <- cur.
            std::mem::swap(&mut self.older, &mut self.prev);
            std::mem::swap(&mut self.prev, &mut self.cur);
            c += hop;
        }
        // An attack is a bump about half a frame wide; partials beating inside one bin give
        // one-hop spikes. A triangular smoother over +-1.5 ms keeps the bump and drops spikes.
        let r = 3isize;
        let mut best: Option<(usize, f64)> = None;
        for i in 0..devs.len() as isize {
            let (mut s, mut w) = (0.0, 0.0);
            for j in -r..=r {
                if let Some((_, d)) = devs.get((i + j) as usize).filter(|_| i + j >= 0) {
                    let wt = (r + 1 - j.abs()) as f64;
                    s += wt * d;
                    w += wt;
                }
            }
            let v = s / w;
            if v > 0.0 && best.is_none_or(|b| v > b.1) {
                best = Some((devs[i as usize].0.max(0) as usize, v));
            }
        }
        best.map(|b| b.0)
    }
}

/// Detects onsets with the given settings. Also returns the envelope (reused for tempo).
pub fn detect_onsets_with(
    signal: &[f32],
    sample_rate: u32,
    cfg: &OnsetConfig,
) -> (OnsetEnvelope, Vec<Onset>) {
    let env = onset_envelope(signal, sample_rate, cfg);
    if sample_rate == 0 {
        return (env, Vec::new());
    }
    let peaks = pick_peaks(&env, cfg);
    let mut refiner = cfg.refine.then(|| Refiner::new(signal, sample_rate));
    let sr = sample_rate as f64;
    let min_gap = (cfg.min_gap_s * sr * 0.5) as usize;
    let mut out: Vec<Onset> = Vec::with_capacity(peaks.len());
    for f in peaks {
        let centre = f * env.hop;
        let sample = match refiner.as_mut() {
            Some(r) => {
                let lo = centre.saturating_sub(env.hop);
                let last = signal.len().saturating_sub(r.frame_len() / 2).max(lo);
                let hi = (centre + env.frame_len / 2 + env.hop / 2).min(last);
                r.refine(lo, hi).unwrap_or(centre)
            }
            None => centre,
        };
        let onset = Onset {
            time_s: sample as f64 / sr,
            sample,
            frame: f,
            strength: env.values[f],
        };
        // Two flux peaks can refine to the same attack; keep the stronger.
        match out.last_mut() {
            Some(last) if sample <= last.sample + min_gap => {
                if onset.strength > last.strength {
                    *last = onset;
                }
            }
            _ => out.push(onset),
        }
    }
    (env, out)
}

/// Detects onsets with [`OnsetConfig::default`].
pub fn detect_onsets(signal: &[f32], sample_rate: u32) -> (OnsetEnvelope, Vec<Onset>) {
    detect_onsets_with(signal, sample_rate, &OnsetConfig::default())
}
