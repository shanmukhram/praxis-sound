//! Tuning-robust 12-bin chroma from a log-frequency mapping of the STFT.
//!
//! Each frame (about 170 ms, hop a quarter frame) is reduced to its spectral peaks between
//! `min_hz` and `max_hz` (local maxima within 60 dB of the frame maximum, frequency and amplitude
//! refined by parabolic interpolation of the log magnitude). Peak frequencies are mapped to
//! fractional MIDI pitch. The tuning offset is the amplitude-weighted circular mean of the peaks'
//! deviation from equal temperament (A4 = 440 Hz), so a render tuned 30 cents sharp still maps
//! to the right pitch classes. Each peak adds `amplitude * cos^2(pi * d)` to its nearest pitch
//! class, where `d` is its distance in semitones (at most 0.5) after tuning correction.

use super::stft::{FrameAnalyzer, frame_len_for};

/// Chroma settings.
#[derive(Clone, Debug, PartialEq)]
pub struct ChromaConfig {
    /// Frame length, seconds (8192 samples at 44.1/48 kHz).
    pub frame_s: f64,
    /// Hop as a fraction of the frame.
    pub hop_fraction: f64,
    pub min_hz: f64,
    pub max_hz: f64,
    /// Peaks more than this many dB below the frame maximum are ignored.
    pub peak_range_db: f32,
}

impl Default for ChromaConfig {
    fn default() -> Self {
        ChromaConfig {
            frame_s: 0.17,
            hop_fraction: 0.25,
            min_hz: 55.0,
            max_hz: 4200.0,
            peak_range_db: 60.0,
        }
    }
}

/// Chroma per frame, C first.
#[derive(Clone, Debug, PartialEq)]
pub struct Chromagram {
    pub frames: Vec<[f32; 12]>,
    pub hop_s: f64,
    /// Estimated tuning offset from A4 = 440 Hz, semitones (-0.5..0.5).
    pub tuning_semitones: f32,
}

impl Chromagram {
    /// Sum of all frames, normalised to a maximum of 1 (all zeros for silence).
    pub fn mean(&self) -> [f32; 12] {
        let mut acc = [0.0f64; 12];
        for f in &self.frames {
            for (a, v) in acc.iter_mut().zip(f.iter()) {
                *a += *v as f64;
            }
        }
        let max = acc.iter().fold(0.0f64, |m, v| m.max(*v));
        let mut out = [0.0f32; 12];
        if max > 0.0 {
            for (o, a) in out.iter_mut().zip(acc.iter()) {
                *o = (*a / max) as f32;
            }
        }
        out
    }
}

/// A spectral peak: fractional MIDI pitch and amplitude.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Peak {
    pub midi: f64,
    pub amplitude: f64,
}

/// Spectral peaks of every frame of `signal`.
pub fn spectral_peaks(signal: &[f32], sample_rate: u32, cfg: &ChromaConfig) -> Vec<Vec<Peak>> {
    if signal.is_empty() || sample_rate == 0 {
        return Vec::new();
    }
    let mut fa = FrameAnalyzer::new(frame_len_for(cfg.frame_s, sample_rate));
    let n = fa.frame_len();
    let hop = ((n as f64 * cfg.hop_fraction).round() as usize).max(1);
    let bins = fa.bins();
    let mut mags = vec![0.0f32; bins];
    let bin_hz = sample_rate as f64 / n as f64;
    let kmin = ((cfg.min_hz / bin_hz).floor() as usize).max(1);
    let kmax = ((cfg.max_hz / bin_hz).ceil() as usize).min(bins - 2);
    let range = 10f32.powf(-cfg.peak_range_db / 20.0);
    let mut out = Vec::new();
    let mut start = 0usize;
    // Frames start at 0 and step by `hop` until the frame start passes the end.
    while start < signal.len() {
        fa.magnitudes(&signal[start..signal.len().min(start + n)], &mut mags);
        let fmax = mags.iter().fold(0.0f32, |m, v| m.max(*v));
        let mut peaks = Vec::new();
        if fmax > 1e-7 && kmin < kmax {
            let thr = fmax * range;
            for k in kmin..=kmax {
                let b = mags[k];
                if b <= thr || b <= mags[k - 1] || b < mags[k + 1] {
                    continue;
                }
                let la = (mags[k - 1].max(1e-12) as f64).ln();
                let lb = (b as f64).ln();
                let lc = (mags[k + 1].max(1e-12) as f64).ln();
                let den = la - 2.0 * lb + lc;
                let d = if den.abs() > 1e-12 {
                    (0.5 * (la - lc) / den).clamp(-0.5, 0.5)
                } else {
                    0.0
                };
                let hz = (k as f64 + d) * bin_hz;
                let amp = (lb - 0.25 * (la - lc) * d).exp();
                if hz > 0.0 {
                    peaks.push(Peak {
                        midi: 69.0 + 12.0 * (hz / 440.0).log2(),
                        amplitude: amp,
                    });
                }
            }
        }
        out.push(peaks);
        start += hop;
    }
    out
}

/// Amplitude-weighted circular mean deviation of the peaks from equal temperament, semitones.
pub fn estimate_tuning(frames: &[Vec<Peak>]) -> f32 {
    let (mut s, mut c) = (0.0f64, 0.0f64);
    for p in frames.iter().flatten() {
        let a = 2.0 * std::f64::consts::PI * p.midi;
        s += p.amplitude * a.sin();
        c += p.amplitude * a.cos();
    }
    if s == 0.0 && c == 0.0 {
        return 0.0;
    }
    (s.atan2(c) / (2.0 * std::f64::consts::PI)) as f32
}

/// Chromagram of `signal` with `cfg`.
pub fn chromagram_with(signal: &[f32], sample_rate: u32, cfg: &ChromaConfig) -> Chromagram {
    let peaks = spectral_peaks(signal, sample_rate, cfg);
    let tuning = estimate_tuning(&peaks);
    let n = frame_len_for(cfg.frame_s, sample_rate.max(1));
    let hop = ((n as f64 * cfg.hop_fraction).round() as usize).max(1);
    let frames = peaks
        .iter()
        .map(|ps| {
            let mut ch = [0.0f32; 12];
            for p in ps {
                let m = p.midi - tuning as f64;
                let nearest = m.round();
                let d = (m - nearest).abs();
                let w = (std::f64::consts::PI * d).cos();
                let pc = (nearest as i64).rem_euclid(12) as usize;
                ch[pc] += (p.amplitude * w * w) as f32;
            }
            ch
        })
        .collect();
    Chromagram {
        frames,
        hop_s: hop as f64 / sample_rate.max(1) as f64,
        tuning_semitones: tuning,
    }
}

/// Chromagram of `signal` with [`ChromaConfig::default`].
pub fn chromagram(signal: &[f32], sample_rate: u32) -> Chromagram {
    chromagram_with(signal, sample_rate, &ChromaConfig::default())
}
