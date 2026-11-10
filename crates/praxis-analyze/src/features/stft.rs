//! Short-time Fourier transform with a periodic Hann window, built on `realfft`.
//!
//! Magnitudes are amplitude-normalised: a sine of amplitude `A` centred on a bin reads `A` in that
//! bin, whatever the frame length. Frames are centred: frame `i` covers samples
//! `i * hop - frame_len / 2 .. i * hop + frame_len / 2`, with zeros outside the signal, so frame `i`
//! describes time `i * hop / sample_rate`.

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

/// Periodic Hann window of length `n` (`sin^2(pi i / n)`), the STFT-friendly variant.
pub fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let x = std::f64::consts::PI * i as f64 / n as f64;
            (x.sin() * x.sin()) as f32
        })
        .collect()
}

/// Smallest power of two `>= n` (at least 2).
pub fn next_pow2(n: usize) -> usize {
    n.max(2).next_power_of_two()
}

/// Frame length (power of two) closest above `seconds` at `sample_rate`.
pub fn frame_len_for(seconds: f64, sample_rate: u32) -> usize {
    next_pow2((seconds * sample_rate as f64).round().max(2.0) as usize)
}

/// Reusable FFT of one frame: plan, window and scratch buffers are allocated once.
pub struct FrameAnalyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    norm: f32,
}

impl std::fmt::Debug for FrameAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameAnalyzer")
            .field("frame_len", &self.window.len())
            .finish()
    }
}

impl FrameAnalyzer {
    /// Plans a real FFT of `frame_len` samples (any length >= 2; powers of two are fastest).
    pub fn new(frame_len: usize) -> FrameAnalyzer {
        let frame_len = frame_len.max(2);
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(frame_len);
        let window = hann(frame_len);
        let sum: f32 = window.iter().sum();
        FrameAnalyzer {
            input: fft.make_input_vec(),
            spectrum: fft.make_output_vec(),
            scratch: fft.make_scratch_vec(),
            fft,
            window,
            norm: if sum > 0.0 { 2.0 / sum } else { 0.0 },
        }
    }

    pub fn frame_len(&self) -> usize {
        self.window.len()
    }

    /// Number of magnitude bins, `frame_len / 2 + 1`.
    pub fn bins(&self) -> usize {
        self.window.len() / 2 + 1
    }

    /// Windows the samples produced by `sample(i)` for `i in 0..frame_len` and writes the
    /// amplitude-normalised magnitude spectrum into `out` (length [`bins`](Self::bins)).
    pub fn magnitudes_with(&mut self, mut sample: impl FnMut(usize) -> f32, out: &mut [f32]) {
        for (i, (x, w)) in self.input.iter_mut().zip(self.window.iter()).enumerate() {
            *x = super::finite_or_zero(sample(i)) * *w;
        }
        let ok = self
            .fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .is_ok();
        for (o, c) in out.iter_mut().zip(self.spectrum.iter()) {
            *o = if ok { c.norm() * self.norm } else { 0.0 };
        }
    }

    /// Like [`magnitudes_with`](Self::magnitudes_with) but writes the amplitude-normalised complex
    /// spectrum (phase included).
    pub fn complex_with(&mut self, mut sample: impl FnMut(usize) -> f32, out: &mut [Complex<f32>]) {
        for (i, (x, w)) in self.input.iter_mut().zip(self.window.iter()).enumerate() {
            *x = super::finite_or_zero(sample(i)) * *w;
        }
        let ok = self
            .fft
            .process_with_scratch(&mut self.input, &mut self.spectrum, &mut self.scratch)
            .is_ok();
        for (o, c) in out.iter_mut().zip(self.spectrum.iter()) {
            *o = if ok {
                *c * self.norm
            } else {
                Complex::new(0.0, 0.0)
            };
        }
    }

    /// Magnitude spectrum of `frame` (missing samples count as zero).
    pub fn magnitudes(&mut self, frame: &[f32], out: &mut [f32]) {
        self.magnitudes_with(|i| frame.get(i).copied().unwrap_or(0.0), out);
    }
}

/// Magnitude spectrogram, frames stored row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Spectrogram {
    pub sample_rate: u32,
    pub frame_len: usize,
    pub hop: usize,
    pub bins: usize,
    /// `frames() * bins` magnitudes.
    pub mags: Vec<f32>,
}

impl Spectrogram {
    pub fn frames(&self) -> usize {
        self.mags.len().checked_div(self.bins).unwrap_or(0)
    }

    /// Magnitudes of frame `i`.
    pub fn frame(&self, i: usize) -> &[f32] {
        &self.mags[i * self.bins..(i + 1) * self.bins]
    }

    /// Centre frequency of bin `k`, Hz.
    pub fn bin_hz(&self, k: usize) -> f64 {
        k as f64 * self.sample_rate as f64 / self.frame_len as f64
    }

    /// Time of frame `i` (its centre), seconds.
    pub fn frame_time_s(&self, i: usize) -> f64 {
        (i * self.hop) as f64 / self.sample_rate.max(1) as f64
    }
}

/// Centred STFT of `signal`: frames at `0, hop, 2 hop, ...` up to and including the last sample.
pub fn stft(signal: &[f32], sample_rate: u32, frame_len: usize, hop: usize) -> Spectrogram {
    let hop = hop.max(1);
    let mut fa = FrameAnalyzer::new(frame_len);
    let frame_len = fa.frame_len();
    let bins = fa.bins();
    let frames = if signal.is_empty() {
        0
    } else {
        (signal.len() - 1) / hop + 1
    };
    let half = frame_len / 2;
    let mut mags = vec![0.0f32; frames * bins];
    for i in 0..frames {
        let start = (i * hop) as isize - half as isize;
        let out = &mut mags[i * bins..(i + 1) * bins];
        fa.magnitudes_with(
            |j| {
                let n = start + j as isize;
                if n >= 0 {
                    signal.get(n as usize).copied().unwrap_or(0.0)
                } else {
                    0.0
                }
            },
            out,
        );
    }
    Spectrogram {
        sample_rate,
        frame_len,
        hop,
        bins,
        mags,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_amplitude_reads_back() {
        let sr = 48_000;
        let n = 2048;
        // Bin-centred sine: bin 64.
        let f = 64.0 * sr as f64 / n as f64;
        let sig: Vec<f32> = (0..8192)
            .map(|i| (0.5 * (2.0 * std::f64::consts::PI * f * i as f64 / sr as f64).sin()) as f32)
            .collect();
        let s = stft(&sig, sr, n, 512);
        let fr = s.frame(8);
        let (k, m) = fr
            .iter()
            .enumerate()
            .fold((0, 0.0f32), |b, (k, m)| if *m > b.1 { (k, *m) } else { b });
        assert_eq!(k, 64);
        assert!((m - 0.5).abs() < 1e-3, "{m}");
    }
}
