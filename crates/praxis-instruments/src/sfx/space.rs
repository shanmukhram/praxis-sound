//! Physical-modelling helpers that make the sound effects read as recorded rather than
//! oscillator-made: a small per-voice [`Room`] (an 8-line feedback delay network with input
//! diffusion and frequency-dependent decay), a smooth 1/f random control signal ([`Wander`],
//! for gusts, flutter and drifting levels), a two-pole modal resonator ([`Mode`], one ringing
//! mode of a struck body) and a raised-cosine contact pulse ([`Contact`], the force of a strike).
//!
//! All of it is deterministic (seeded only through `praxis_dsp::rng`), allocates only in
//! `new` and keeps no hidden time base: output depends only on the samples fed in, so renders
//! do not depend on the block size.

// `next()` returning a bare sample is the crate-wide generator contract (as in `praxis_dsp`).
#![allow(clippy::should_implement_trait)]

use praxis_dsp::noise::White;
use std::f64::consts::TAU;

/// Feedback lines of a [`Room`].
const LINES: usize = 8;
/// Line lengths in ms at size 1: mutually incommensurate, 32..70 ms (a small, live room).
const LINE_MS: [f64; LINES] = [31.71, 37.03, 41.29, 45.97, 51.13, 56.83, 63.17, 70.31];
/// An all-pass inside each line (ms at size 1): echoes multiply on every pass, so the tail is
/// dense noise-like decay rather than a pattern of repeats.
const LINE_AP_MS: [f64; LINES] = [3.1, 4.3, 5.3, 6.1, 7.3, 8.1, 9.7, 10.9];
const LINE_AP_G: f64 = 0.5;
const LINE_AP_MAX_S: f64 = 0.0115;
/// Input diffusers (Schroeder all-passes), four per channel, ms.
const DIFFS: usize = 4;
const DIFF_MS: [[f64; DIFFS]; 2] = [[4.771, 3.595, 2.733, 1.793], [4.931, 3.713, 2.609, 1.871]];
const DIFF_G: f64 = 0.62;
/// Early reflections: taps per channel over the gap before the first loop arrivals (from
/// 1 ms to [`ER_SPAN_S`] at size 1), decaying with the room, so the wet sound starts right
/// away and builds without a gap or a ghost attack.
const ER_TAPS: usize = 16;
const ER_SPAN_S: f64 = 0.032;
const ER_LEVEL: f64 = 0.9;
/// Longest delay a room holds, seconds (the longest line at size 1, with headroom).
const ROOM_MAX_S: f64 = 0.0725;
const DIFF_MAX_S: f64 = 0.0055;
/// Default input low cut of a room (see [`Room::set_lowcut`]).
const ROOM_LOWCUT_HZ: f64 = 150.0;

#[inline]
fn flush(x: f64) -> f64 {
    if x.is_finite() && x.abs() > 1e-20 {
        x
    } else {
        0.0
    }
}

/// A small stereo room: diffused input into an 8-line Hadamard feedback delay network whose
/// lines lose `-60 dB` over `rt60` at low frequencies and faster above the damping corner
/// (air and wall absorption); its input is high-passed (150 Hz by default). It returns the wet signal only; the caller mixes it with the dry
/// sound and applies the voice envelope after it, so a sound still ends exactly when it should.
#[derive(Clone, Debug)]
pub struct Room {
    buf: Vec<f32>,
    cap: usize,
    pos: usize,
    len: [usize; LINES],
    gain: [f64; LINES],
    lp: [f64; LINES],
    damp: f64,
    ap: Vec<f32>,
    ap_cap: usize,
    ap_pos: usize,
    ap_len: [[usize; DIFFS]; 2],
    lap: Vec<f32>,
    lap_cap: usize,
    lap_pos: usize,
    lap_len: [usize; LINES],
    out_gain: f64,
    er: Vec<f32>,
    er_cap: usize,
    er_pos: usize,
    er_at: [[usize; ER_TAPS]; 2],
    er_gain: [[f64; ER_TAPS]; 2],
    er_shape: [[(f64, f64); ER_TAPS]; 2],
    lowcut: f64,
    lc: [[f64; 2]; 2],
}

impl Room {
    /// A silent room for `sample_rate` (allocates its delay memory).
    pub fn new(sample_rate: f64) -> Room {
        let sr = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            48_000.0
        };
        let cap = (ROOM_MAX_S * sr).ceil() as usize + 2;
        let ap_cap = (DIFF_MAX_S * sr).ceil() as usize + 2;
        let er_cap = (ER_SPAN_S * sr).ceil() as usize + 4;
        let lap_cap = (LINE_AP_MAX_S * sr).ceil() as usize + 2;
        // Fixed, irregular tap positions (fractions of the span) and signs, the same for
        // every room: a deterministic constant, not per-note randomness.
        let mut rng = praxis_dsp::rng::Pcg32::new(0x5EE_D0E1_2A11);
        let mut er_shape = [[(0.0, 1.0); ER_TAPS]; 2];
        for ch in er_shape.iter_mut() {
            for (k, t) in ch.iter_mut().enumerate() {
                let u = (k as f64 + 0.15 + 0.7 * rng.next_f32() as f64) / ER_TAPS as f64;
                let sign = if rng.next_u32() & 1 == 0 { 1.0 } else { -1.0 };
                *t = (u, sign);
            }
        }
        let mut r = Room {
            buf: vec![0.0; cap * LINES],
            cap,
            pos: 0,
            len: [1; LINES],
            gain: [0.0; LINES],
            lp: [0.0; LINES],
            damp: 0.0,
            ap: vec![0.0; ap_cap * 2 * DIFFS],
            ap_cap,
            ap_pos: 0,
            ap_len: [[1; DIFFS]; 2],
            lap: vec![0.0; lap_cap * LINES],
            lap_cap,
            lap_pos: 0,
            lap_len: [1; LINES],
            out_gain: 0.0,
            er: vec![0.0; er_cap * 2],
            er_cap,
            er_pos: 0,
            er_at: [[1; ER_TAPS]; 2],
            er_gain: [[0.0; ER_TAPS]; 2],
            er_shape,
            lowcut: 0.0,
            lc: [[0.0; 2]; 2],
        };
        r.set(0.3, 0.5, 4000.0, sr);
        r
    }

    /// Sets the decay time (RT60 at low frequencies, seconds), size (`0..1`, scales the line
    /// lengths 35..100%) and damping corner (Hz). Allocation-free; call [`Room::clear`] to start
    /// from silence.
    pub fn set(&mut self, rt60: f64, size: f64, damp_hz: f64, sr: f64) {
        let rt60 = if rt60.is_finite() {
            rt60.clamp(0.02, 30.0)
        } else {
            0.3
        };
        let scale = 0.35 + 0.65 * size.clamp(0.0, 1.0);
        let mut g2 = 0.0;
        for k in 0..LINES {
            let d = ((LINE_MS[k] * 1e-3 * sr * scale).round() as usize).clamp(2, self.cap - 1);
            self.len[k] = d;
            self.lap_len[k] =
                ((LINE_AP_MS[k] * 1e-3 * sr * scale).round() as usize).clamp(1, self.lap_cap - 1);
            self.gain[k] = 10f64.powf(-3.0 * d as f64 / (rt60 * sr));
            g2 += self.gain[k] * self.gain[k];
        }
        for c in 0..2 {
            for s in 0..DIFFS {
                self.ap_len[c][s] =
                    ((DIFF_MS[c][s] * 1e-3 * sr).round() as usize).clamp(1, self.ap_cap - 1);
            }
        }
        let hz = if damp_hz.is_finite() {
            damp_hz.clamp(200.0, 0.45 * sr)
        } else {
            4000.0
        };
        self.damp = (-TAU * hz / sr).exp();
        // Steady-state power gain of a loop with gain g is 1 / (1 - g^2): scale so a stationary
        // input gives a wet signal of about its own level, whatever the decay time.
        let gm2 = (g2 / LINES as f64).min(0.9999);
        self.out_gain = (1.0 - gm2).sqrt();
        let span = ER_SPAN_S * scale;
        let er_norm = ER_LEVEL * self.out_gain * (LINES as f64 / ER_TAPS as f64).sqrt();
        for c in 0..2 {
            for k in 0..ER_TAPS {
                let (u, sign) = self.er_shape[c][k];
                let t = 0.001 + u * (span - 0.001);
                self.er_at[c][k] = ((t * sr).round() as usize).clamp(1, self.er_cap - 1);
                // Rising slightly toward the loop's first arrivals, decaying at the RT60.
                let build = 0.6 + 0.4 * u;
                self.er_gain[c][k] = sign * er_norm * build * 10f64.powf(-3.0 * t / rt60);
            }
        }
        self.set_lowcut(ROOM_LOWCUT_HZ, sr);
    }

    /// The room hears only what is above `hz` (a 12 dB/octave high-pass on its input, 150 Hz after
    /// [`Room::set`]): low tones stay dry and in tune, as with a real room's low end and a
    /// microphone's low cut.
    pub fn set_lowcut(&mut self, hz: f64, sr: f64) {
        let hz = if hz.is_finite() {
            hz.clamp(5.0, 0.45 * sr)
        } else {
            ROOM_LOWCUT_HZ
        };
        self.lowcut = (-TAU * hz / sr).exp();
    }

    /// Clears every delay to silence.
    pub fn clear(&mut self) {
        self.buf.fill(0.0);
        self.ap.fill(0.0);
        self.er.fill(0.0);
        self.er_pos = 0;
        self.lap.fill(0.0);
        self.lap_pos = 0;
        self.lp = [0.0; LINES];
        self.lc = [[0.0; 2]; 2];
        self.pos = 0;
        self.ap_pos = 0;
    }

    /// Two cascaded one-pole high-passes (12 dB/octave below the low cut).
    #[inline]
    fn low_cut(&mut self, c: usize, x: f64) -> f64 {
        let mut v = x;
        for s in 0..2 {
            let lp = &mut self.lc[c][s];
            *lp = flush(v + self.lowcut * (*lp - v));
            v -= *lp;
        }
        v
    }

    #[inline]
    fn diffuse(&mut self, c: usize, x: f64) -> f64 {
        let mut v = x;
        for s in 0..DIFFS {
            let base = (DIFFS * c + s) * self.ap_cap;
            let mut r = self.ap_pos + self.ap_cap - self.ap_len[c][s];
            if r >= self.ap_cap {
                r -= self.ap_cap;
            }
            let wd = self.ap[base + r] as f64;
            let w = flush(v + DIFF_G * wd);
            self.ap[base + self.ap_pos] = w as f32;
            v = wd - DIFF_G * w;
        }
        v
    }

    /// One stereo sample in, the wet stereo sample out.
    #[inline]
    pub fn tick(&mut self, l: f64, r: f64) -> (f64, f64) {
        let l = self.low_cut(0, l);
        let r = self.low_cut(1, r);
        let il = self.diffuse(0, l);
        let ir = self.diffuse(1, r);
        self.ap_pos += 1;
        if self.ap_pos == self.ap_cap {
            self.ap_pos = 0;
        }
        let mut o = [0.0f64; LINES];
        for (k, ok) in o.iter_mut().enumerate() {
            let mut i = self.pos + self.cap - self.len[k];
            if i >= self.cap {
                i -= self.cap;
            }
            let v = self.buf[k * self.cap + i] as f64;
            // The line's all-pass.
            let base = k * self.lap_cap;
            let mut j = self.lap_pos + self.lap_cap - self.lap_len[k];
            if j >= self.lap_cap {
                j -= self.lap_cap;
            }
            let wd = self.lap[base + j] as f64;
            let w = flush(v + LINE_AP_G * wd);
            self.lap[base + self.lap_pos] = w as f32;
            let v = wd - LINE_AP_G * w;
            self.lp[k] = flush(v + self.damp * (self.lp[k] - v));
            *ok = self.lp[k];
        }
        // Fast Walsh-Hadamard transform, normalised (orthogonal mixing: lossless).
        let mut h = o;
        let mut span = 1;
        while span < LINES {
            for i in (0..LINES).step_by(2 * span) {
                for j in i..i + span {
                    let (a, b) = (h[j], h[j + span]);
                    h[j] = a + b;
                    h[j + span] = a - b;
                }
            }
            span *= 2;
        }
        let norm = 1.0 / (LINES as f64).sqrt();
        for k in 0..LINES {
            let input = if k % 2 == 0 { il } else { ir };
            let w = flush(self.gain[k] * h[k] * norm + 0.5 * input);
            self.buf[k * self.cap + self.pos] = w as f32;
        }
        self.pos += 1;
        if self.pos == self.cap {
            self.pos = 0;
        }
        self.lap_pos += 1;
        if self.lap_pos == self.lap_cap {
            self.lap_pos = 0;
        }
        // Early reflections from the diffused input.
        let mut er = [0.0; 2];
        for (c, (e, x)) in er.iter_mut().zip([il, ir]).enumerate() {
            let base = c * self.er_cap;
            self.er[base + self.er_pos] = x as f32;
            for k in 0..ER_TAPS {
                let mut i = self.er_pos + self.er_cap - self.er_at[c][k];
                if i >= self.er_cap {
                    i -= self.er_cap;
                }
                *e += self.er_gain[c][k] * self.er[base + i] as f64;
            }
        }
        self.er_pos += 1;
        if self.er_pos == self.er_cap {
            self.er_pos = 0;
        }
        let wl = o[0] - o[2] + o[4] - o[6];
        let wr = o[1] - o[3] + o[5] - o[7];
        (wl * self.out_gain + er[0], wr * self.out_gain + er[1])
    }
}

/// A smooth random control signal with a 1/f-like spectrum: three low-passed noises at
/// `rate`, `rate / 3` and `rate / 9` Hz (each two cascaded one-poles), the slower ones weighted
/// more. Unit standard deviation, zero mean, stationary from its first sample.
#[derive(Clone, Debug)]
pub struct Wander {
    noise: White,
    s1: [f64; 3],
    s2: [f64; 3],
    a: [f64; 3],
    w: [f64; 3],
}

impl Default for Wander {
    fn default() -> Self {
        Wander {
            noise: White::new(1),
            s1: [0.0; 3],
            s2: [0.0; 3],
            a: [0.0; 3],
            w: [0.0; 3],
        }
    }
}

impl Wander {
    /// Restarts from `seed` at `rate_hz` (the fastest of the three components).
    pub fn start(&mut self, seed: u64, rate_hz: f64, sr: f64) {
        self.noise.reseed(seed);
        let rate = if rate_hz.is_finite() {
            rate_hz.clamp(0.01, 200.0)
        } else {
            1.0
        };
        // White is uniform in -1..1: variance 1/3.
        let white_var = 1.0 / 3.0;
        let mut raw = [0.0; 3];
        let mut total = 0.0;
        for (i, r) in raw.iter_mut().enumerate() {
            let fr = (rate / 3f64.powi(i as i32)).max(0.01);
            *r = 0.8f64.powi(i as i32) / fr.sqrt();
            total += *r * *r;
        }
        let total = total.sqrt();
        for i in 0..3 {
            let fr = (rate / 3f64.powi(i as i32)).max(0.01);
            let a = 1.0 - (-TAU * fr / sr).exp();
            let b = 1.0 - a;
            // Variance of white noise through two cascaded one-poles a / (1 - b z^-1).
            let var = white_var * a.powi(4) * (1.0 + b * b) / (1.0 - b * b).powi(3);
            let sd = var.max(1e-30).sqrt();
            self.a[i] = a;
            self.w[i] = raw[i] / total / sd;
            // Start inside the stationary distribution (no slow settling from zero).
            let v = sd * 3f64.sqrt() * self.noise.next() as f64;
            self.s1[i] = v;
            self.s2[i] = v;
        }
    }

    /// The next control value (unit standard deviation).
    #[inline]
    pub fn next(&mut self) -> f64 {
        let x = self.noise.next() as f64;
        let mut out = 0.0;
        for i in 0..3 {
            self.s1[i] += self.a[i] * (x - self.s1[i]);
            self.s2[i] += self.a[i] * (self.s1[i] - self.s2[i]);
            out += self.w[i] * self.s2[i];
        }
        out
    }
}

/// One ringing mode of a struck body: a two-pole resonator whose impulse response is
/// `amp * e^(-t / tau) * sin(2 pi f t)` (unit-area force pulses in, a decaying sinusoid out).
#[derive(Clone, Copy, Debug, Default)]
pub struct Mode {
    c: f64,
    r2: f64,
    g: f64,
    y1: f64,
    y2: f64,
}

impl Mode {
    /// Tunes the mode: frequency (Hz), 60 dB decay time (seconds) and amplitude; clears it.
    pub fn set(&mut self, hz: f64, t60_s: f64, amp: f64, sr: f64) {
        let f = if hz.is_finite() {
            hz.clamp(1.0, 0.45 * sr)
        } else {
            1000.0
        };
        let w = TAU * f / sr;
        let r = if t60_s.is_finite() && t60_s > 0.0 {
            (-6.907_755_278_982_137 / (t60_s * sr)).exp()
        } else {
            0.0
        };
        self.c = 2.0 * r * w.cos();
        self.r2 = r * r;
        self.g = if amp.is_finite() { amp * w.sin() } else { 0.0 };
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    /// Drives the mode with the force `x` and returns its displacement.
    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        let y = flush(self.g * x + self.c * self.y1 - self.r2 * self.y2);
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// The force of a strike: a raised-cosine pulse of unit area over `len` samples (a short pulse is
/// a hard, bright contact; a long one a soft, dull one).
#[derive(Clone, Copy, Debug, Default)]
pub struct Contact {
    i: u64,
    len: u64,
}

impl Contact {
    /// Starts a pulse of `len` samples (at least 2).
    pub fn strike(&mut self, len: u64) {
        self.i = 0;
        self.len = len.max(2);
    }

    /// The next force sample (0 once the pulse is over).
    #[inline]
    pub fn next(&mut self) -> f64 {
        if self.i >= self.len {
            return 0.0;
        }
        let n = self.len as f64;
        let v = (1.0 - (TAU * (self.i as f64 + 0.5) / n).cos()) / n;
        self.i += 1;
        v
    }

    /// Whether the pulse is still pushing.
    pub fn active(&self) -> bool {
        self.i < self.len
    }
}

/// Soft saturation that keeps small signals linear (`tanh`, unity slope at 0).
#[inline]
pub fn soft_clip(x: f64, drive: f64) -> f64 {
    if drive <= 1e-6 {
        return x;
    }
    (x * drive).tanh() / drive
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_decays_by_its_rt60_and_is_finite() {
        let sr = 48_000.0;
        let mut room = Room::new(sr);
        room.set(0.5, 0.6, 6000.0, sr);
        room.clear();
        let mut e = [0.0f64; 4];
        for i in 0..(sr as usize) {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let (l, r) = room.tick(x, x);
            assert!(l.is_finite() && r.is_finite());
            let k = i * 4 / sr as usize;
            e[k] += l * l + r * r;
        }
        // Energy keeps falling quarter by quarter (about 30 dB per quarter at RT60 0.5 s).
        assert!(e[0] > 0.0);
        for w in e.windows(2) {
            assert!(w[1] < w[0] * 0.1, "{e:?}");
        }
    }

    #[test]
    fn wander_has_unit_spread_and_is_seeded() {
        let sr = 48_000.0;
        let mut a = Wander::default();
        a.start(7, 6.0, sr);
        let n = 20 * sr as usize;
        let xs: Vec<f64> = (0..n).map(|_| a.next()).collect();
        let mean = xs.iter().sum::<f64>() / n as f64;
        let sd = (xs.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n as f64).sqrt();
        assert!(mean.abs() < 0.5, "mean {mean}");
        assert!(sd > 0.5 && sd < 1.6, "sd {sd}");
        let mut b = Wander::default();
        b.start(7, 6.0, sr);
        assert_eq!(
            xs[..1000],
            (0..1000).map(|_| b.next()).collect::<Vec<_>>()[..]
        );
    }

    #[test]
    fn struck_mode_rings_at_its_frequency_and_amplitude() {
        let sr = 48_000.0;
        let mut m = Mode::default();
        m.set(440.0, 1.0, 0.5, sr);
        let mut c = Contact::default();
        c.strike(4);
        let y: Vec<f64> = (0..4800).map(|_| m.tick(c.next())).collect();
        let pk = y.iter().fold(0.0f64, |a, v| a.max(v.abs()));
        assert!((pk - 0.5).abs() < 0.05, "peak {pk}");
        // Zero crossings over 0.1 s: 2 * 44.
        let zc = y.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((43..=45).contains(&zc), "{zc}");
    }
}
