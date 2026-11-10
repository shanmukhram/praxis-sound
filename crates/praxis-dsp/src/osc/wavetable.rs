//! Mip-mapped wavetable oscillator built from harmonic amplitudes.

use super::{sanitize_sr, wrap_phase};
use crate::interp::hermite4;
use realfft::num_complex::Complex;
use std::sync::Arc;

/// Samples per single-cycle table.
const SIZE: usize = 2048;
/// Stored samples per table: one guard sample before and two after for 4-point interpolation.
const STRIDE: usize = SIZE + 3;
/// Mip levels: level `l` holds harmonics `1..=MAX_HARMONIC >> l` (1023, 511, ... 1), one octave
/// apart.
const LEVELS: usize = 10;
/// Highest harmonic a table can hold (below the table's own Nyquist).
const MAX_HARMONIC: usize = SIZE / 2 - 1;

struct TableSet {
    frames: usize,
    /// `frames * LEVELS * STRIDE` samples.
    samples: Vec<f32>,
}

impl TableSet {
    fn build(frames: &[&[f32]]) -> TableSet {
        let frame_count = frames.len().max(1);
        let mut samples = vec![0.0f32; frame_count * LEVELS * STRIDE];
        let mut planner = realfft::RealFftPlanner::<f64>::new();
        let c2r = planner.plan_fft_inverse(SIZE);
        let mut spectrum = c2r.make_input_vec();
        let mut out = c2r.make_output_vec();
        for f in 0..frame_count {
            let amps: &[f32] = frames.get(f).copied().unwrap_or(&[1.0]);
            let mut scale = 1.0f64;
            for level in 0..LEVELS {
                let top = MAX_HARMONIC >> level;
                spectrum
                    .iter_mut()
                    .for_each(|c| *c = Complex::new(0.0, 0.0));
                for (h, &a) in amps.iter().enumerate().take(top) {
                    let a = if a.is_finite() { a as f64 } else { 0.0 };
                    // sin(k theta) with amplitude a  <=>  X[k] = -i a / 2 (unnormalised inverse).
                    spectrum[h + 1] = Complex::new(0.0, -0.5 * a);
                }
                if c2r.process(&mut spectrum, &mut out).is_err() {
                    out.iter_mut().for_each(|v| *v = 0.0);
                }
                if level == 0 {
                    // Normalise every level of a frame by the full-bandwidth peak so switching
                    // levels never changes loudness.
                    let peak = out.iter().fold(0.0f64, |m, v| m.max(v.abs()));
                    scale = if peak > 1e-9 { 1.0 / peak } else { 0.0 };
                }
                let base = (f * LEVELS + level) * STRIDE;
                let t = &mut samples[base..base + STRIDE];
                for i in 0..SIZE {
                    t[i + 1] = (out[i] * scale) as f32;
                }
                t[0] = t[SIZE];
                t[SIZE + 1] = t[1];
                t[SIZE + 2] = t[2];
            }
        }
        TableSet {
            frames: frame_count,
            samples,
        }
    }

    #[inline]
    fn read(&self, frame: usize, level: usize, i: usize, t: f32) -> f32 {
        let base = (frame * LEVELS + level) * STRIDE + i;
        let s = &self.samples[base..base + 4];
        hermite4(s[0], s[1], s[2], s[3], t)
    }
}

/// Single-cycle wavetable oscillator.
///
/// Built from harmonic amplitudes (`amps[0]` is the fundamental, `amps[k]` harmonic `k + 1`,
/// all sine phase; negative values flip a harmonic). Each table is stored at ten bandwidths an
/// octave apart, and playback picks the widest one whose top harmonic stays strictly below
/// Nyquist, so nothing ever aliases. Tables are normalised to a peak of 1 per frame.
///
/// Several frames can be given ([`Wavetable::from_frames`]); [`Wavetable::set_position`] then
/// morphs between neighbouring frames. Tables are shared (`Arc`) between clones, so cloning a
/// wavetable per voice does not copy or allocate table memory. Construction allocates and runs
/// an FFT: do it off the audio thread.
#[derive(Clone)]
pub struct Wavetable {
    tables: Arc<TableSet>,
    sr: f64,
    phase: f64,
    inc: f64,
    level: usize,
    position: f32,
}

impl std::fmt::Debug for Wavetable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wavetable")
            .field("frames", &self.tables.frames)
            .field("phase", &self.phase)
            .field("inc", &self.inc)
            .field("level", &self.level)
            .field("position", &self.position)
            .finish()
    }
}

impl Wavetable {
    /// Highest harmonic a frame can hold (harmonics beyond it are ignored).
    pub const MAX_HARMONIC: usize = MAX_HARMONIC;

    /// A sine table.
    pub fn new(sample_rate: f32) -> Wavetable {
        Wavetable::from_harmonics(sample_rate, &[1.0])
    }

    /// One frame from harmonic amplitudes (at most [`Wavetable::MAX_HARMONIC`] are used).
    pub fn from_harmonics(sample_rate: f32, amps: &[f32]) -> Wavetable {
        Wavetable::from_frames(sample_rate, &[amps])
    }

    /// Several frames, each a list of harmonic amplitudes, morphable with
    /// [`Wavetable::set_position`]. An empty list gives a single sine frame.
    pub fn from_frames<F: AsRef<[f32]>>(sample_rate: f32, frames: &[F]) -> Wavetable {
        let refs: Vec<&[f32]> = frames.iter().map(|f| f.as_ref()).collect();
        let tables = if refs.is_empty() {
            TableSet::build(&[&[1.0]])
        } else {
            TableSet::build(&refs)
        };
        Wavetable {
            tables: Arc::new(tables),
            sr: sanitize_sr(sample_rate),
            phase: 0.0,
            inc: 0.0,
            level: 0,
            position: 0.0,
        }
    }

    /// Number of frames.
    pub fn frames(&self) -> usize {
        self.tables.frames
    }

    /// Frequency in Hz, `0..Nyquist`. Also selects the mip level.
    pub fn set_freq(&mut self, hz: f32) {
        self.inc = if hz.is_finite() {
            (hz as f64 / self.sr).clamp(0.0, 0.499)
        } else {
            0.0
        };
        let mut level = 0;
        if self.inc > 0.0 {
            while level < LEVELS - 1 && (MAX_HARMONIC >> level) as f64 * self.inc >= 0.5 {
                level += 1;
            }
        }
        self.level = level;
    }

    /// Highest harmonic number the current mip level plays.
    pub fn top_harmonic(&self) -> usize {
        MAX_HARMONIC >> self.level
    }

    /// Morph position `0.0..=1.0` across the frames (linear crossfade between neighbours).
    pub fn set_position(&mut self, position: f32) {
        self.position = if position.is_finite() {
            position.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    /// Restarts at `phase` cycles.
    pub fn reset(&mut self, phase: f32) {
        self.phase = wrap_phase(phase);
    }

    /// The next sample.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let x = self.phase * SIZE as f64;
        let i = (x as usize).min(SIZE - 1);
        let t = (x - i as f64) as f32;
        let frames = self.tables.frames;
        let y = if frames <= 1 {
            self.tables.read(0, self.level, i, t)
        } else {
            let pos = self.position * (frames - 1) as f32;
            let f0 = (pos as usize).min(frames - 2);
            let m = pos - f0 as f32;
            let a = self.tables.read(f0, self.level, i, t);
            if m > 0.0 {
                let b = self.tables.read(f0 + 1, self.level, i, t);
                a + (b - a) * m
            } else {
                a
            }
        };
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_level_keeps_harmonics_below_nyquist() {
        let sr = 48_000.0;
        let mut w = Wavetable::from_harmonics(sr, &[1.0; 1200]);
        for hz in [
            10.0f32, 30.0, 100.0, 440.0, 1000.0, 5000.0, 12_000.0, 20_000.0,
        ] {
            w.set_freq(hz);
            assert!(
                (w.top_harmonic() as f32) * hz < sr / 2.0,
                "{hz} {}",
                w.top_harmonic()
            );
            // And not wastefully narrow: twice as many harmonics would alias (except at the
            // widest level).
            if w.top_harmonic() < MAX_HARMONIC {
                assert!((2 * w.top_harmonic() + 1) as f32 * hz >= sr / 2.0);
            }
        }
    }

    #[test]
    fn sine_table_is_a_sine() {
        let sr = 48_000.0;
        let mut w = Wavetable::new(sr);
        w.set_freq(1000.0);
        for n in 0..480 {
            let want = (std::f64::consts::TAU * 1000.0 * n as f64 / 48_000.0).sin() as f32;
            assert!((w.next() - want).abs() < 1e-5);
        }
    }
}
