//! `pluck`: a Karplus-Strong plucked string (class melody, sustaining).
//!
//! One period of seeded noise, low-passed by the tone and shaped by a pick-position comb, is
//! fed into a delay loop. The loop holds an integer delay, a linear-phase damping filter
//! `[a, 1 - 2a, a]` (exactly one sample of delay at every frequency, so it darkens the string
//! without detuning it) and a first-order Thiran all-pass for the fractional part of the period,
//! so the loop delay is `sample_rate / f` and the string is in tune. The loop gain sets the ring:
//! 60 dB in `decay_s x sqrt(261.6 Hz / f)`. Brighter tone, higher velocity and bright accents
//! open the excitation filter and lighten the damping.
//!
//! A 1.5 ms ramp softens the pluck onset; the release fade (after the loop) ends notes in exact
//! silence. Loop and excitation buffers are preallocated per voice for the lowest note at the
//! session sample rate.
//!
//! Parameters: `tone` 0..1 (0.45), `decay_s` 0.3..8 s (2.5), `release_s` 0.03..2 s (0.25).

use super::common::{Params, Ramp, Release, key_decay, param, samples, tone_amount, vel_gain};
use super::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::OnePole;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, InstrumentKind};

const LOWEST: u8 = 36;
const FREE_LEVEL: f64 = 1e-5;

/// The `pluck` preset.
#[derive(Clone, Debug)]
pub struct Pluck {
    tone: f32,
    decay_s: f32,
    release_s: f32,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct PluckVoice {
    buf: Vec<f32>,
    exc: Vec<f32>,
    mask: usize,
    w: usize,
    n: usize,
    exc_len: usize,
    delay: usize,
    a: f32,
    eta: f32,
    ap_x1: f32,
    ap_y1: f32,
    g: f32,
    amp: f64,
    amp_coeff: f64,
    gain: f64,
    ramp: Ramp,
    release: Release,
    level: f64,
    done: bool,
}

impl Patch for Pluck {
    type Voice = PluckVoice;

    fn info() -> PresetInfo {
        PresetInfo {
            kind: InstrumentKind::Synth,
            name: "pluck".into(),
            class: GeneratorClass::Melody,
            pitched: true,
            sustaining: true,
            range: (LOWEST, 96),
            params: vec![
                param("tone", 0.0, 1.0, 0.45, ""),
                param("decay_s", 0.3, 8.0, 2.5, "s"),
                param("release_s", 0.03, 2.0, 0.25, "s"),
            ],
            source: "praxis synth (procedural)".into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> Pluck {
        Pluck {
            tone: p[0],
            decay_s: p[1],
            release_s: p[2],
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        16
    }

    fn new_voice(&self) -> PluckVoice {
        let longest = (self.sr / super::common::midi_hz(LOWEST)).ceil() as usize + 8;
        let size = longest.next_power_of_two();
        PluckVoice {
            buf: vec![0.0; size],
            exc: vec![0.0; size],
            mask: size - 1,
            w: 0,
            n: 0,
            exc_len: 0,
            delay: 1,
            a: 0.0,
            eta: 0.0,
            ap_x1: 0.0,
            ap_y1: 0.0,
            g: 0.0,
            amp: 0.0,
            amp_coeff: 0.0,
            gain: 0.0,
            ramp: Ramp::default(),
            release: Release::default(),
            level: 0.0,
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.tone = value,
            1 => self.decay_s = value,
            2 => self.release_s = value,
            _ => {}
        }
    }

    fn tail_samples(&self) -> usize {
        samples(self.release_s, self.sr, 1) as usize + 1
    }
}

impl Voice<Pluck> for PluckVoice {
    fn start(&mut self, patch: &Pluck, note: &Note) {
        let sr = note.sample_rate;
        let t = tone_amount(patch.tone, note.velocity, note.brightness);
        let mut rng = note.rng(0x9C4C);
        // Loop delay: integer part, one sample in the damping filter, Thiran fraction in
        // 0.5..1.5 (well-behaved all-pass).
        let period = (sr / note.hz).clamp(4.0, (self.mask - 4) as f64);
        let n = (period - 1.5).floor().max(1.0);
        let d = period - 1.0 - n;
        self.delay = n as usize;
        self.eta = ((1.0 - d) / (1.0 + d)) as f32;
        self.a = 0.22 - 0.17 * t;
        let t60 = patch.decay_s as f64 * key_decay(note.hz);
        self.g = 10f64.powf(-3.0 / (note.hz * t60)) as f32;
        self.amp = 1.0;
        self.amp_coeff = super::common::t60_coeff(t60, sr);

        // Excitation: one period of noise, low-passed, with a pick-position comb.
        let len = (period.round() as usize).clamp(2, self.exc.len());
        let mut lp = OnePole::new(sr as f32);
        lp.set_cutoff((note.hz * (2.0 + 22.0 * t as f64)).min(0.45 * sr) as f32);
        for v in self.exc[..len].iter_mut() {
            *v = lp.tick(rng.next_bipolar());
        }
        let pick = ((len as f32 * rng.range(0.12, 0.2)) as usize).max(1);
        for i in (pick..len).rev() {
            self.exc[i] -= self.exc[i - pick];
        }
        let mean = self.exc[..len].iter().sum::<f32>() / len as f32;
        let mut peak = 0.0f32;
        for v in self.exc[..len].iter_mut() {
            *v -= mean;
            peak = peak.max(v.abs());
        }
        let norm = 1.0 / peak.max(1e-6);
        for v in self.exc[..len].iter_mut() {
            *v *= norm;
        }
        self.exc_len = len;
        self.buf.fill(0.0);
        self.w = 0;
        self.n = 0;
        self.ap_x1 = 0.0;
        self.ap_y1 = 0.0;
        self.gain = 0.5 * vel_gain(note.velocity) as f64;
        self.ramp.start(0.0015, sr);
        self.release.reset();
        self.level = self.gain;
        self.done = false;
    }

    fn release(&mut self, patch: &Pluck) {
        self.release.start(patch.release_s, patch.sr);
    }

    #[inline]
    fn tick(&mut self, _patch: &Pluck) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let m = self.mask;
        let r = self.w.wrapping_sub(self.delay);
        let r0 = self.buf[r & m];
        let r1 = self.buf[r.wrapping_sub(1) & m];
        let r2 = self.buf[r.wrapping_sub(2) & m];
        let h = self.a * r0 + (1.0 - 2.0 * self.a) * r1 + self.a * r2;
        let ap = self.eta * h + self.ap_x1 - self.eta * self.ap_y1;
        self.ap_x1 = h;
        self.ap_y1 = flush(ap);
        let input = if self.n < self.exc_len {
            self.exc[self.n]
        } else {
            0.0
        };
        let y = flush(input + self.g * ap);
        self.buf[self.w & m] = y;
        self.w = self.w.wrapping_add(1);
        self.n = self.n.saturating_add(1);
        self.amp *= self.amp_coeff;
        let g = self.gain * self.ramp.next() as f64 * self.release.next() as f64;
        self.level = self.gain * self.amp * self.release.gain() as f64;
        if self.release.is_done() || (self.n > self.exc_len && self.level < FREE_LEVEL) {
            self.done = true;
        }
        let out = (y as f64 * g) as f32;
        (out, out)
    }

    fn level(&self) -> f32 {
        self.level as f32
    }

    fn is_done(&self) -> bool {
        self.done
    }
}

#[inline]
fn flush(x: f32) -> f32 {
    if x.is_finite() && x.abs() > 1e-20 {
        x
    } else {
        0.0
    }
}
