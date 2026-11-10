//! Saturation (kind 3): 2x oversampled asymmetric tanh with a post tilt tone.
//!
//! Parameters: 0 drive_db (dB), 1 tone (-1 dark .. +1 bright), 2 mix (wet fraction),
//! 3 output_db (dB, trims the mixed output).
//!
//! The shaper is `tanh(d x + b) - tanh(b)` (normalised to unity slope at zero) with a small
//! fixed bias `b` (even harmonics), run at twice the sample rate between two linear-phase
//! half-band FIRs (47 taps, Kaiser), followed by a 10 Hz DC blocker and a +-6 dB tilt around a
//! 700 Hz one-pole split. Latency is exactly [`LATENCY`] samples; the dry path is delayed by the
//! same amount so the mix stays phase-aligned.

use super::ramp::{
    DcBlock, OnePole, RAMP_SECONDS, Ramp, halfband, one_pole_coeff, radius_tail, ramp_len,
};
use crate::traits::Processor;
use crate::util::{db_to_gain, sanitize};

const KIND: usize = 3;
const TAPS: usize = 47;
/// Latency in samples at the base rate: (TAPS - 1) oversampled samples = (TAPS - 1) / 2.
pub const LATENCY: usize = (TAPS - 1) / 2;
const BIAS: f32 = 0.15;
const TILT_HZ: f32 = 700.0;
const TILT_DB: f32 = 6.0;

#[derive(Clone, Debug)]
struct Channel {
    /// Base-rate history for the upsampler (doubled ring).
    up: [f32; 2 * TAPS],
    up_pos: usize,
    /// Oversampled history for the downsampler (doubled ring).
    down: [f32; 2 * TAPS],
    down_pos: usize,
    dry: [f32; LATENCY + 1],
    dry_pos: usize,
    dc: DcBlock,
    tilt: OnePole,
}

impl Channel {
    fn new() -> Channel {
        Channel {
            up: [0.0; 2 * TAPS],
            up_pos: 0,
            down: [0.0; 2 * TAPS],
            down_pos: 0,
            dry: [0.0; LATENCY + 1],
            dry_pos: 0,
            dc: DcBlock::default(),
            tilt: OnePole::default(),
        }
    }
}

#[inline]
fn push(ring: &mut [f32; 2 * TAPS], pos: &mut usize, x: f32) {
    *pos = if *pos == 0 { TAPS - 1 } else { *pos - 1 };
    ring[*pos] = x;
    ring[*pos + TAPS] = x;
}

/// `(tanh(x + b) - tanh(b)) / (1 - tanh(b)^2)`: unity slope at 0, bounded by about 1.18.
#[inline]
fn shape(x: f32, tb: f32) -> f32 {
    ((x + BIAS).tanh() - tb) / (1.0 - tb * tb)
}

#[derive(Clone, Debug)]
pub struct Saturation {
    sr: f32,
    params: [f32; 4],
    drive: Ramp, // linear
    tone: Ramp,
    mix: Ramp,
    out: Ramp, // linear
    h: Vec<f32>,
    /// Even-index taps (upsampler phase 0), times 2.
    h_even: Vec<f32>,
    ch: [Channel; 2],
    dc_r: f32,
    tilt_a: f32,
    tanh_b: f32,
}

impl Saturation {
    pub fn new(sample_rate: f32, max_block: usize) -> Saturation {
        let params = super::defaults::<4>(KIND);
        let h = halfband(TAPS, 7.0);
        let h_even = h.iter().step_by(2).map(|v| v * 2.0).collect();
        let mut s = Saturation {
            sr: sample_rate,
            params,
            drive: Ramp::new(1.0, 1),
            tone: Ramp::new(0.0, 1),
            mix: Ramp::new(1.0, 1),
            out: Ramp::new(1.0, 1),
            h,
            h_even,
            ch: [Channel::new(), Channel::new()],
            dc_r: 0.0,
            tilt_a: 0.0,
            tanh_b: BIAS.tanh(),
        };
        s.prepare(sample_rate, max_block);
        s
    }

    #[inline]
    fn tick(&mut self, c: usize, x: f32, drive: f32, gl: f32, gh: f32, mix: f32, out: f32) -> f32 {
        let tb = self.tanh_b;
        let ch = &mut self.ch[c];
        // dry delay
        let dry = ch.dry[ch.dry_pos];
        ch.dry[ch.dry_pos] = x;
        ch.dry_pos = (ch.dry_pos + 1) % (LATENCY + 1);
        // upsample: phase 0 uses the even taps, phase 1 is the centre tap (pure delay)
        push(&mut ch.up, &mut ch.up_pos, x * drive);
        let hist = &ch.up[ch.up_pos..ch.up_pos + TAPS];
        let mut u0 = 0.0;
        for (k, hk) in self.h_even.iter().enumerate() {
            u0 += hk * hist[k];
        }
        let u1 = hist[(TAPS - 1) / 4];
        // shape at 2x and decimate
        push(&mut ch.down, &mut ch.down_pos, shape(u0, tb));
        let d = &ch.down[ch.down_pos..ch.down_pos + TAPS];
        let mut y = 0.0;
        for (hj, s) in self.h.iter().zip(d.iter()) {
            y += hj * s;
        }
        push(&mut ch.down, &mut ch.down_pos, shape(u1, tb));
        let y = ch.dc.tick(y, self.dc_r);
        let lo = ch.tilt.lp(y, self.tilt_a);
        let wet = lo * gl + (y - lo) * gh;
        sanitize((dry + (wet - dry) * mix) * out)
    }
}

impl Processor for Saturation {
    fn prepare(&mut self, sample_rate: f32, _max_block: usize) {
        self.sr = sample_rate;
        let len = ramp_len(RAMP_SECONDS, sample_rate);
        let p = self.params;
        self.drive = Ramp::new(db_to_gain(p[0]), len);
        self.tone = Ramp::new(p[1], len);
        self.mix = Ramp::new(p[2], len);
        self.out = Ramp::new(db_to_gain(p[3]), len);
        self.dc_r = 1.0 - 2.0 * std::f32::consts::PI * 10.0 / sample_rate;
        self.tilt_a = one_pole_coeff(TILT_HZ, sample_rate);
        self.ch = [Channel::new(), Channel::new()];
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let drive = self.drive.next();
            let tone = self.tone.next();
            let mix = self.mix.next();
            let out = self.out.next();
            let gh = db_to_gain(tone * TILT_DB);
            let gl = 1.0 / gh;
            *l = self.tick(0, sanitize(*l), drive, gl, gh, mix, out);
            *r = self.tick(1, sanitize(*r), drive, gl, gh, mix, out);
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        let Some(v) = super::clamp_param(KIND, index, value) else {
            return;
        };
        self.params[index] = v;
        match index {
            0 => self.drive.set(db_to_gain(v)),
            1 => self.tone.set(v),
            2 => self.mix.set(v),
            3 => self.out.set(db_to_gain(v)),
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.drive.snap();
        self.tone.snap();
        self.mix.snap();
        self.out.snap();
        self.ch = [Channel::new(), Channel::new()];
    }

    fn latency_samples(&self) -> usize {
        LATENCY
    }

    /// The oversampling filters plus the ring-out of the DC blocker and tilt filter.
    fn tail_samples(&self) -> usize {
        TAPS + radius_tail(self.dc_r as f64) + radius_tail(self.tilt_a as f64)
    }
}
