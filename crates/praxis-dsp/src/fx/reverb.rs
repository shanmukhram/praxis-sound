//! Algorithmic reverb (kind 5): 8-line feedback delay network.
//!
//! Parameters: 0 decay_s (RT60, s), 1 predelay_ms (ms), 2 size (0..1), 3 damp_hz (Hz),
//! 4 width (stereo width of the wet signal), 5 mix (wet fraction).
//!
//! Signal flow: stereo predelay -> four allpass diffusers per channel -> early reflections and
//! 8 delay lines mixed by a normalised Hadamard matrix. Each line has a gain
//! `10^(-3 d / (RT60 fs))` for its delay `d`, so low frequencies decay at exactly RT60, and a
//! one-pole absorption filter (unity at DC) designed per line so that the decay time at
//! `damp_hz` is half the RT60 and shorter above it, independent of size and sample rate.
//!
//! Early reflections: the network's first arrivals come only after its shortest line (16..53 ms
//! with `size`), so on its own the wet signal of a click would be silent for that long and then
//! arrive all at once, heard as a ghost attack. A tapped delay of the diffused input fills the
//! gap: [`ER_TAPS`] jittered taps per channel from the predelay to the longest line, starting at
//! [`ER_START`] of full level and rising smoothly, decaying at the RT60 and cross-fading out while
//! the network's arrivals come in. The wet envelope of a click therefore starts right after the
//! predelay and builds up without gaps or steps (tested in `fx_time.rs`).
//!
//! Line lengths are mutually incommensurate and scale with `size`. The read positions are not
//! modulated ([`MOD_MS`] is 0): even a 0.2 ms sweep (1.5 cents) makes the tail time-varying, which
//! log-spectral-flux onset detectors read as extra onsets inside the tail (the felt-piano
//! acceptance render lost its grid check that way), and a time-invariant reverb is exactly
//! periodic in loops. Fractional reads use a first-order allpass, so interpolation never
//! low-passes the loop. Deterministic. RT60 is the mid-band (500 Hz / 1 kHz) decay time; highs
//! decay faster by design.

use super::ramp::{OnePole, RAMP_SECONDS, Ramp, ramp_len, read_frac};
use crate::delayline::DelayLine;
use crate::rng::Pcg32;
use crate::traits::Processor;
use crate::util::sanitize;

const KIND: usize = 5;
const LINES: usize = 8;
/// Line lengths in ms at size = 1.
const BASE_MS: [f32; LINES] = [53.13, 59.71, 67.33, 73.97, 81.43, 89.29, 97.61, 107.11];
/// Allpass diffuser lengths in ms, per channel.
const DIFF_MS: [[f32; 4]; 2] = [[4.771, 3.595, 2.733, 1.793], [4.931, 3.713, 2.609, 1.871]];
const DIFF_G: f32 = 0.62;
/// Read-position modulation depth (ms). Off: see the module docs.
const MOD_MS: f32 = 0.0;
const MAX_PREDELAY_MS: f32 = 200.0;
const CHUNK: usize = 32;
const SEED: u64 = 0x5EED_FD4E_2E7E_2B01;
const SIZE_GLIDE_SECONDS: f32 = 0.2;
const MAX_TAIL_SECONDS: f64 = 30.0;
/// Early-reflection taps per channel.
const ER_TAPS: usize = 24;
/// Overall early-reflection level relative to the late field (calibrated so the wet envelope
/// of a click is continuous where the network's first arrivals take over).
const ER_LEVEL: f32 = 0.35;
/// Early-reflection level right after the predelay, relative to its full level (the build-up
/// starts here and rises smoothly).
const ER_START: f32 = 0.6;

fn size_scale(size: f32) -> f32 {
    0.3 + 0.7 * size.clamp(0.0, 1.0)
}

#[derive(Clone, Debug)]
struct Allpass {
    line: DelayLine,
    len: usize,
}

impl Allpass {
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let d = self.line.read_int(self.len - 1);
        let w = sanitize(x - DIFF_G * d);
        self.line.push(w);
        d + DIFF_G * w
    }
}

#[derive(Clone, Debug)]
pub struct FdnReverb {
    sr: f32,
    params: [f32; 6],
    decay: Ramp,
    predelay: Ramp, // samples
    size: Ramp,
    damp: Ramp, // log2 Hz
    width: Ramp,
    mix: Ramp,
    pre: [DelayLine; 2],
    diff: [[Allpass; 4]; 2],
    lines: [DelayLine; LINES],
    lp: [OnePole; LINES],
    ap: [f32; LINES],
    len: [f32; LINES],
    gain: [f32; LINES],
    damp_p: [f32; LINES],
    mod_phase: [f32; LINES],
    mod_inc: [f32; LINES],
    mod_phase0: [f32; LINES],
    mod_depth: f32,
    /// Early reflections: a tapped delay of the diffused input per channel.
    er: [DelayLine; 2],
    /// Tap positions as a fraction of the longest line (fixed, from the seed).
    er_u: [[f32; ER_TAPS]; 2],
    er_sign: [[f32; ER_TAPS]; 2],
    er_pos: [[f32; ER_TAPS]; 2],
    er_gain: [[f32; ER_TAPS]; 2],
}

fn hadamard8(v: &mut [f32; LINES]) {
    let mut h = 1;
    while h < LINES {
        let mut i = 0;
        while i < LINES {
            for j in i..i + h {
                let a = v[j];
                let b = v[j + h];
                v[j] = a + b;
                v[j + h] = a - b;
            }
            i += 2 * h;
        }
        h *= 2;
    }
    let s = 1.0 / (LINES as f32).sqrt();
    for x in v.iter_mut() {
        *x *= s;
    }
}

impl FdnReverb {
    pub fn new(sample_rate: f32, max_block: usize) -> FdnReverb {
        let params = super::defaults::<6>(KIND);
        let mut r = FdnReverb {
            sr: sample_rate,
            params,
            decay: Ramp::new(1.0, 1),
            predelay: Ramp::new(0.0, 1),
            size: Ramp::new(0.0, 1),
            damp: Ramp::new(0.0, 1),
            width: Ramp::new(1.0, 1),
            mix: Ramp::new(0.0, 1),
            pre: [DelayLine::new(4), DelayLine::new(4)],
            diff: std::array::from_fn(|_| {
                std::array::from_fn(|_| Allpass {
                    line: DelayLine::new(4),
                    len: 2,
                })
            }),
            lines: std::array::from_fn(|_| DelayLine::new(4)),
            lp: [OnePole::default(); LINES],
            ap: [0.0; LINES],
            len: [1.0; LINES],
            gain: [0.0; LINES],
            damp_p: [0.0; LINES],
            mod_phase: [0.0; LINES],
            mod_inc: [0.0; LINES],
            mod_phase0: [0.0; LINES],
            mod_depth: 0.0,
            er: [DelayLine::new(4), DelayLine::new(4)],
            er_u: [[0.0; ER_TAPS]; 2],
            er_sign: [[1.0; ER_TAPS]; 2],
            er_pos: [[1.0; ER_TAPS]; 2],
            er_gain: [[0.0; ER_TAPS]; 2],
        };
        r.prepare(sample_rate, max_block);
        r
    }

    fn update(&mut self) {
        let scale = size_scale(self.size.value());
        let rt60 = self.decay.value().max(0.05);
        let ms = self.sr * 0.001;
        let w = (2.0 * std::f64::consts::PI * self.damp.value().exp2() as f64 / self.sr as f64)
            .min(0.98 * std::f64::consts::PI);
        let c = w.cos();
        for i in 0..LINES {
            self.len[i] = BASE_MS[i] * scale * ms;
            let g = 10f64.powf(-3.0 * self.len[i] as f64 / (rt60 as f64 * self.sr as f64));
            self.gain[i] = g as f32;
            // One-pole absorption (Jot): unity at DC, and at damp_hz an extra factor g per pass,
            // i.e. half the RT60 there. Solves (1-p)^2 = g^2 (1 - 2 p cos w + p^2).
            let r2 = g * g;
            let a = 1.0 - r2;
            let b = 1.0 - r2 * c;
            self.damp_p[i] = if a < 1e-12 {
                0.0
            } else {
                ((b - (b * b - a * a).max(0.0).sqrt()) / a) as f32
            };
        }
        // Early reflections span from the predelay to the longest line, where the network's
        // first arrivals have all come in. Their level rises from ER_START to full over the
        // first half of the shortest line, decays at the RT60 and fades out (raised cosine)
        // between the shortest and the longest line as the network takes over.
        let lmin = self.len[0];
        let lmax = self.len[LINES - 1];
        let k = ER_LEVEL / (ER_TAPS as f32).sqrt();
        for c in 0..2 {
            for j in 0..ER_TAPS {
                let t = (self.er_u[c][j] * lmax).max(1.0);
                let rise = {
                    let x = (t / (0.5 * lmin)).min(1.0);
                    ER_START + (1.0 - ER_START) * x * x * (3.0 - 2.0 * x)
                };
                let fade = if t <= lmin {
                    1.0
                } else {
                    let x = ((t - lmin) / (lmax - lmin).max(1.0)).min(1.0);
                    0.5 + 0.5 * (std::f32::consts::PI * x).cos()
                };
                let decay = 10f32.powf(-3.0 * t / (rt60 * self.sr));
                self.er_pos[c][j] = t;
                self.er_gain[c][j] = k * rise * fade * decay * self.er_sign[c][j];
            }
        }
    }

    fn reset_state(&mut self) {
        for l in self.pre.iter_mut() {
            l.reset();
        }
        for ch in self.diff.iter_mut() {
            for a in ch.iter_mut() {
                a.line.reset();
            }
        }
        for l in self.lines.iter_mut() {
            l.reset();
        }
        for l in self.er.iter_mut() {
            l.reset();
        }
        self.lp = [OnePole::default(); LINES];
        self.ap = [0.0; LINES];
        self.mod_phase = self.mod_phase0;
    }
}

impl Processor for FdnReverb {
    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sr = sample_rate;
        let ms = sample_rate * 0.001;
        let len = ramp_len(RAMP_SECONDS, sample_rate);
        let p = self.params;
        self.decay = Ramp::new(p[0], len);
        self.predelay = Ramp::new(p[1] * ms, len);
        self.size = Ramp::new(p[2], ramp_len(SIZE_GLIDE_SECONDS, sample_rate));
        self.damp = Ramp::new(p[3].log2(), len);
        self.width = Ramp::new(p[4], len);
        self.mix = Ramp::new(p[5], len);
        let pre_cap = (MAX_PREDELAY_MS * ms).ceil() as usize + 8;
        self.pre = [DelayLine::new(pre_cap), DelayLine::new(pre_cap)];
        for (c, ch) in self.diff.iter_mut().enumerate() {
            for (k, a) in ch.iter_mut().enumerate() {
                let n = (DIFF_MS[c][k] * ms).round().max(2.0) as usize;
                *a = Allpass {
                    line: DelayLine::new(n + 2),
                    len: n,
                };
            }
        }
        self.mod_depth = MOD_MS * ms;
        let mut rng = Pcg32::new(SEED);
        for i in 0..LINES {
            let cap = (BASE_MS[i] * ms).ceil() as usize + self.mod_depth.ceil() as usize + 8;
            self.lines[i] = DelayLine::new(cap);
            let rate = 0.12 + 0.5 * rng.next_f32();
            self.mod_inc[i] = rate / sample_rate;
            self.mod_phase0[i] = rng.next_f32();
        }
        let er_cap = (BASE_MS[LINES - 1] * ms).ceil() as usize + 8;
        self.er = [DelayLine::new(er_cap), DelayLine::new(er_cap)];
        for c in 0..2 {
            for j in 0..ER_TAPS {
                // Evenly spread with jitter, decorrelated between the channels.
                let jit = rng.next_f32() - 0.5;
                self.er_u[c][j] = (j as f32 + 0.5 + 0.8 * jit) / ER_TAPS as f32;
                self.er_sign[c][j] = if rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
            }
        }
        self.update();
        self.reset_state();
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let tau = std::f32::consts::TAU;
        let mut start = 0;
        while start < n {
            let len = CHUNK.min(n - start);
            if self.decay.moving() || self.size.moving() || self.damp.moving() {
                self.decay.advance(len as u32);
                self.size.advance(len as u32);
                self.damp.advance(len as u32);
                self.update();
            }
            for i in start..start + len {
                let pd = self.predelay.next();
                let width = self.width.next();
                let mix = self.mix.next();
                let (xl, xr) = (sanitize(left[i]), sanitize(right[i]));
                self.pre[0].push(xl);
                self.pre[1].push(xr);
                let mut il = read_frac(&self.pre[0], pd);
                let mut ir = read_frac(&self.pre[1], pd);
                for a in self.diff[0].iter_mut() {
                    il = a.tick(il);
                }
                for a in self.diff[1].iter_mut() {
                    ir = a.tick(ir);
                }
                self.er[0].push(il);
                self.er[1].push(ir);
                // read the lines
                let mut v = [0.0f32; LINES];
                for k in 0..LINES {
                    let ph = self.mod_phase[k];
                    let m = (ph * tau).sin() * self.mod_depth;
                    let mut np = ph + self.mod_inc[k];
                    if np >= 1.0 {
                        np -= 1.0;
                    }
                    self.mod_phase[k] = np;
                    // The line is read before this sample's write: delay d reads d - 1. The
                    // fractional part goes through a first-order allpass (unity magnitude, so
                    // interpolation does not low-pass the loop); kept in [0.1, 1.1).
                    let d = self.len[k] + m - 1.0;
                    let di = (d - 0.1).floor().max(0.0);
                    let f = d - di;
                    let eta = (1.0 - f) / (1.0 + f);
                    let i0 = di as usize;
                    let x0 = self.lines[k].read_int(i0);
                    let x1 = self.lines[k].read_int(i0 + 1);
                    let y = sanitize(eta * x0 + x1 - eta * self.ap[k]);
                    self.ap[k] = y;
                    v[k] = self.lp[k].lp(y, self.damp_p[k]) * self.gain[k];
                }
                let mut wl = 0.0;
                let mut wr = 0.0;
                for k in 0..LINES / 2 {
                    let s = if k % 2 == 0 { 1.0 } else { -1.0 };
                    wl += v[2 * k] * s;
                    wr += v[2 * k + 1] * s;
                }
                hadamard8(&mut v);
                for k in 0..LINES {
                    let s = if (k / 2) % 2 == 0 { 1.0 } else { -1.0 };
                    let inj = if k % 2 == 0 { il } else { ir };
                    self.lines[k].push(sanitize(v[k] + inj * s * 0.5));
                }
                wl *= 0.5;
                wr *= 0.5;
                for j in 0..ER_TAPS {
                    wl += read_frac(&self.er[0], self.er_pos[0][j]) * self.er_gain[0][j];
                    wr += read_frac(&self.er[1], self.er_pos[1][j]) * self.er_gain[1][j];
                }
                let m = 0.5 * (wl + wr);
                let s = 0.5 * (wl - wr) * width;
                let (wl, wr) = (m + s, m - s);
                left[i] = sanitize(xl + (wl - xl) * mix);
                right[i] = sanitize(xr + (wr - xr) * mix);
            }
            start += len;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(v) = super::clamp_param(KIND, index, value) else {
            return;
        };
        self.params[index] = v;
        match index {
            0 => self.decay.set(v),
            1 => self.predelay.set(v * self.sr * 0.001),
            2 => self.size.set(v),
            3 => self.damp.set(v.log2()),
            4 => self.width.set(v),
            5 => self.mix.set(v),
            _ => {}
        }
        if !(self.decay.moving() || self.size.moving() || self.damp.moving()) {
            self.update();
        }
    }

    fn reset(&mut self) {
        self.decay.snap();
        self.predelay.snap();
        self.size.snap();
        self.damp.snap();
        self.width.snap();
        self.mix.snap();
        self.update();
        self.reset_state();
    }

    /// Predelay + diffusion + RT60, capped at 30 s.
    fn tail_samples(&self) -> usize {
        let rt60 = self.decay.value().max(self.decay.target()) as f64;
        let pd = self.predelay.value().max(self.predelay.target()) as f64;
        let diff: f32 = DIFF_MS[0]
            .iter()
            .chain(DIFF_MS[1].iter())
            .fold(0.0, |m, v| m.max(*v));
        let diff = 4.0 * diff as f64 * self.sr as f64 * 0.001;
        let sr = self.sr as f64;
        (pd + diff + rt60 * sr).min(MAX_TAIL_SECONDS * sr).ceil() as usize
    }
}
