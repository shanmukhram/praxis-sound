//! Interpolation helpers and [`Resampler`], a variable-rate reader of sample slices (used by the
//! sampler for pitched playback with loops).
//!
//! All helpers interpolate between `x0` (at `t = 0`) and `x1` (at `t = 1`) given their
//! neighbours `xm1` (at -1) and `x2` (at 2); `t` is the fractional position in samples.

// `next()` returning a bare sample is the crate-wide reader contract.
#![allow(clippy::should_implement_trait)]

use std::sync::OnceLock;

/// Linear interpolation.
#[inline]
pub fn linear(x0: f32, x1: f32, t: f32) -> f32 {
    x0 + (x1 - x0) * t
}

/// 4-point, 3rd-order Hermite (Catmull-Rom) interpolation. Continuous first derivative;
/// a good default for audio.
#[inline]
pub fn hermite4(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

/// 4-point, 3rd-order Lagrange interpolation (the cubic through all four points).
#[inline]
pub fn lagrange4(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let tp1 = t + 1.0;
    let tm1 = t - 1.0;
    let tm2 = t - 2.0;
    let a = -xm1 * (t * tm1 * tm2) * (1.0 / 6.0);
    let b = x0 * (tp1 * tm1 * tm2) * 0.5;
    let c = -x1 * (tp1 * t * tm2) * 0.5;
    let d = x2 * (tp1 * t * tm1) * (1.0 / 6.0);
    a + b + c + d
}

/// Lagrange interpolation of any order: evaluates the polynomial through
/// `(0, points[0]), (1, points[1]), ...` at `x`. O(n^2); empty input gives 0.
pub fn lagrange(points: &[f32], x: f32) -> f32 {
    let x = x as f64;
    let mut sum = 0.0f64;
    for (i, &yi) in points.iter().enumerate() {
        let mut w = 1.0f64;
        for j in 0..points.len() {
            if j != i {
                w *= (x - j as f64) / (i as f64 - j as f64);
            }
        }
        sum += w * yi as f64;
    }
    sum as f32
}

// ---- windowed-sinc kernel for band-limited resampling ----

/// Zero crossings of the sinc kernel either side of the read position.
const SINC_HALF: usize = 8;
/// Kernel table points per unit.
const SINC_RES: usize = 512;
const SINC_POINTS: usize = SINC_HALF * SINC_RES + 2;
/// Passband edge as a fraction of the source sample rate (0.45 = 90 % of Nyquist).
const SINC_CUTOFF: f64 = 0.45;
const SINC_BETA: f64 = 8.0;
/// Highest rate the sinc reader widens its kernel for (beyond it, some aliasing returns).
const SINC_MAX_STRETCH: f64 = 4.0;

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let half = x * 0.5;
    for k in 1..64 {
        let f = half / k as f64;
        term *= f * f;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

fn sinc_table() -> &'static [f32; SINC_POINTS] {
    static TABLE: OnceLock<[f32; SINC_POINTS]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0.0f32; SINC_POINTS];
        let norm = bessel_i0(SINC_BETA);
        for (p, v) in t.iter_mut().enumerate() {
            let x = p as f64 / SINC_RES as f64;
            let r = x / SINC_HALF as f64;
            if r >= 1.0 {
                continue;
            }
            let w = bessel_i0(SINC_BETA * (1.0 - r * r).sqrt()) / norm;
            let a = 2.0 * std::f64::consts::PI * SINC_CUTOFF * x;
            let s = if a < 1e-12 { 1.0 } else { a.sin() / a };
            *v = (2.0 * SINC_CUTOFF * s * w) as f32;
        }
        t
    })
}

#[inline]
fn sinc_weight(table: &[f32; SINC_POINTS], x: f64) -> f32 {
    let p = x.abs() * SINC_RES as f64;
    let i = p as usize;
    if i + 1 >= SINC_POINTS {
        return 0.0;
    }
    let fr = (p - i as f64) as f32;
    table[i] + (table[i + 1] - table[i]) * fr
}

/// Interpolation used by [`Resampler`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Quality {
    /// 2-point linear. Cheapest; audible high-frequency loss and imaging.
    Linear,
    /// 4-point Hermite. Good for playback near the original pitch.
    #[default]
    Hermite,
    /// 16-tap Kaiser-windowed sinc whose cutoff follows the playback rate, so pitching up (up to
    /// two octaves) does not alias. Up to 64 taps per sample at high rates.
    Sinc,
}

/// Reads a sample slice at a fractional, variable rate, with an optional sustain loop.
///
/// The reader holds no reference to the audio; every read takes the slice, so one reader can
/// play several channels in lockstep ([`Resampler::next_stereo`]). Positions are in source
/// samples (`f64`, so long samples stay sample-accurate); `rate` is source samples per output
/// sample (`2^(semitones / 12) * source_rate / output_rate`).
///
/// Loops: `set_loop(Some((start, end)))` plays `start..end` (end exclusive) repeatedly once the
/// play position reaches `end`. Interpolation reads across the loop seam as if the loop were
/// continuous, so a loop cut at matching points plays without a click. `set_loop(None)` (for
/// example on note release) lets playback continue to the end of the sample. Indices outside
/// the sample read as silence.
#[derive(Clone, Debug)]
pub struct Resampler {
    quality: Quality,
    pos: f64,
    rate: f64,
    end: Option<usize>,
    loop_region: Option<(usize, usize)>,
    looped: bool,
    finished: bool,
}

impl Default for Resampler {
    fn default() -> Resampler {
        Resampler::new(Quality::default())
    }
}

impl Resampler {
    pub fn new(quality: Quality) -> Resampler {
        if quality == Quality::Sinc {
            // Build the shared kernel table now, not on the first audio-thread read.
            let _ = sinc_table();
        }
        Resampler {
            quality,
            pos: 0.0,
            rate: 1.0,
            end: None,
            loop_region: None,
            looped: false,
            finished: false,
        }
    }

    pub fn set_quality(&mut self, quality: Quality) {
        if quality == Quality::Sinc {
            let _ = sinc_table();
        }
        self.quality = quality;
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    /// Starts (or restarts) playback at `pos` source samples.
    pub fn start(&mut self, pos: f64) {
        self.pos = if pos.is_finite() { pos.max(0.0) } else { 0.0 };
        self.looped = false;
        self.finished = false;
    }

    /// Playback rate in source samples per output sample, clamped to `0.0..=64.0`.
    pub fn set_rate(&mut self, rate: f64) {
        self.rate = if rate.is_finite() {
            rate.clamp(0.0, 64.0)
        } else {
            0.0
        };
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Playback stops at `end` (exclusive) instead of the end of the slice.
    pub fn set_end(&mut self, end: Option<usize>) {
        self.end = end;
    }

    /// Loop region `start..end` (exclusive end) or `None`. Regions shorter than 2 samples are
    /// ignored.
    pub fn set_loop(&mut self, region: Option<(usize, usize)>) {
        self.loop_region = region.filter(|&(s, e)| e > s + 1);
        if self.loop_region.is_none() {
            self.looped = false;
        }
    }

    /// Current read position in source samples.
    pub fn position(&self) -> f64 {
        self.pos
    }

    /// True once a non-looping playback has passed the end.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    #[inline]
    fn end_of(&self, len: usize) -> usize {
        self.end.map_or(len, |e| e.min(len))
    }

    #[inline]
    fn fetch(&self, src: &[f32], end: usize, i: i64) -> f32 {
        let mut i = i;
        if let Some((ls, le)) = self.loop_region {
            let (ls, le) = (ls as i64, le.min(end) as i64);
            let len = le - ls;
            if len > 1 {
                if i >= le {
                    i = ls + (i - le) % len;
                } else if self.looped && i < ls {
                    i = le - 1 - (ls - 1 - i) % len;
                }
            }
        }
        if i < 0 || i >= end as i64 {
            0.0
        } else {
            src[i as usize]
        }
    }

    /// The interpolated value at the current position (does not advance).
    #[inline]
    pub fn read(&self, src: &[f32]) -> f32 {
        if self.finished {
            return 0.0;
        }
        let end = self.end_of(src.len());
        let fl = self.pos.floor();
        let i = fl as i64;
        let t = self.pos - fl;
        match self.quality {
            Quality::Linear => linear(
                self.fetch(src, end, i),
                self.fetch(src, end, i + 1),
                t as f32,
            ),
            Quality::Hermite => hermite4(
                self.fetch(src, end, i - 1),
                self.fetch(src, end, i),
                self.fetch(src, end, i + 1),
                self.fetch(src, end, i + 2),
                t as f32,
            ),
            Quality::Sinc => {
                let table = sinc_table();
                let stretch = self.rate.clamp(1.0, SINC_MAX_STRETCH);
                let half = (SINC_HALF as f64 * stretch).ceil() as i64;
                let inv = 1.0 / stretch;
                let mut acc = 0.0f32;
                let mut wsum = 0.0f32;
                for k in (i - half + 1)..=(i + half) {
                    let w = sinc_weight(table, (k as f64 - self.pos) * inv);
                    acc += w * self.fetch(src, end, k);
                    wsum += w;
                }
                if wsum > 1e-6 { acc / wsum } else { acc }
            }
        }
    }

    /// Advances the position by one output sample for a source of `len` samples.
    #[inline]
    pub fn advance(&mut self, len: usize) {
        if self.finished {
            return;
        }
        self.pos += self.rate;
        let end = self.end_of(len);
        if let Some((ls, le)) = self.loop_region {
            let le = le.min(end);
            if le > ls + 1 && self.pos >= le as f64 {
                let span = (le - ls) as f64;
                self.pos = ls as f64 + (self.pos - le as f64) % span;
                self.looped = true;
                return;
            }
        }
        if self.pos >= end as f64 {
            self.finished = true;
        }
    }

    /// Reads the current value from `src` and advances.
    #[inline]
    pub fn next(&mut self, src: &[f32]) -> f32 {
        let v = self.read(src);
        self.advance(src.len());
        v
    }

    /// Reads `left` and `right` at the same position and advances once.
    #[inline]
    pub fn next_stereo(&mut self, left: &[f32], right: &[f32]) -> (f32, f32) {
        let l = self.read(left);
        let r = self.read(right);
        self.advance(left.len().min(right.len()));
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolators_pass_through_points() {
        let (a, b, c, d) = (0.3, -0.7, 0.9, 0.1);
        assert_eq!(hermite4(a, b, c, d, 0.0), b);
        assert!((hermite4(a, b, c, d, 1.0) - c).abs() < 1e-6);
        assert!((lagrange4(a, b, c, d, 0.0) - b).abs() < 1e-6);
        assert!((lagrange4(a, b, c, d, 1.0) - c).abs() < 1e-6);
        assert!((lagrange4(a, b, c, d, -1.0) - a).abs() < 1e-6);
        assert!((lagrange4(a, b, c, d, 2.0) - d).abs() < 1e-6);
        // General Lagrange agrees with the 4-point one (shifted by one node).
        let g = lagrange(&[a, b, c, d], 1.37);
        assert!((g - lagrange4(a, b, c, d, 0.37)).abs() < 1e-5);
        assert_eq!(lagrange(&[], 0.5), 0.0);
    }

    #[test]
    fn non_looping_playback_finishes() {
        let src = [1.0f32; 100];
        let mut r = Resampler::new(Quality::Hermite);
        r.set_rate(2.0);
        r.start(0.0);
        let mut n = 0;
        while !r.is_finished() && n < 1000 {
            r.next(&src);
            n += 1;
        }
        assert_eq!(n, 50);
        assert_eq!(r.next(&src), 0.0);
    }
}
