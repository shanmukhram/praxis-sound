//! `granular_texture`: a granular cloud over an internal wavetable grain source (class texture,
//! held notes).
//!
//! Each voice keeps a master phase running at the note frequency. Grains (up to [`GRAINS`] at
//! once) are Hann-windowed slices, 60..180 ms long, of a soft multi-frame wavetable read *at that
//! master phase* (or at twice it, for octave grains). Because every grain is a window on the
//! same continuous tone, the cloud is exactly in tune however the grains fall; what varies per
//! grain (seeded from the note) is its start time, length, wavetable frame (timbre), octave,
//! level and stereo place. `density` sets how often grains start; `tone`, velocity and bright
//! accents choose brighter frames and more octave grains.
//!
//! A slow attack and the release fade (applied last) keep the cloud click-free and let it end in
//! exact silence.
//!
//! Parameters: `density` 0.25..2 (1), `tone` 0..1 (0.4), `attack_s` 0.02..3 s (0.3),
//! `release_s` 0.1..6 s (1.4).

use super::common::{Params, Release, Swell, balance, param, samples, tone_amount, vel_gain};
use super::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::osc::Wavetable;
use praxis_dsp::rng::Pcg32;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, InstrumentKind};
use std::f64::consts::PI;

/// Grains one voice can play at once.
pub const GRAINS: usize = 12;
/// Mean time between grain starts at density 1.
const INTERVAL_S: f32 = 0.03;
const GRAIN_MIN_S: f32 = 0.06;
const GRAIN_MAX_S: f32 = 0.18;

/// The `granular_texture` preset.
#[derive(Clone, Debug)]
pub struct GranularTexture {
    density: f32,
    tone: f32,
    attack_s: f32,
    release_s: f32,
    sr: f64,
    table: Wavetable,
}

#[derive(Clone, Debug)]
struct Grain {
    active: bool,
    osc: Wavetable,
    n: u32,
    len: u32,
    mult: f64,
    amp: f32,
    pan: (f32, f32),
}

#[derive(Clone, Debug)]
pub struct GranularVoice {
    grains: [Grain; GRAINS],
    rng: Pcg32,
    phase: f64,
    inc: f64,
    hz: f64,
    spawn_in: u32,
    interval: f32,
    bright: f32,
    gain: f64,
    env: Swell,
    release: Release,
    done: bool,
}

fn frames() -> [Vec<f32>; 4] {
    // From bright to soft: a reedy tone, a warm vowel-like tone, a hollow tone, nearly a sine.
    let reed: Vec<f32> = (1..=16).map(|k| 0.8f32.powi(k - 1)).collect();
    let vowel = vec![1.0, 0.7, 0.5, 0.15, 0.3, 0.1, 0.05, 0.04];
    let hollow = vec![1.0, 0.05, 0.33, 0.02, 0.12, 0.0, 0.05];
    let soft = vec![1.0, 0.2, 0.04];
    [reed, vowel, hollow, soft]
}

impl Patch for GranularTexture {
    type Voice = GranularVoice;

    fn info() -> PresetInfo {
        PresetInfo {
            kind: InstrumentKind::Synth,
            name: "granular_texture".into(),
            class: GeneratorClass::Texture,
            pitched: true,
            sustaining: false,
            range: (36, 96),
            params: vec![
                param("density", 0.25, 2.0, 1.0, "x"),
                param("tone", 0.0, 1.0, 0.4, ""),
                param("attack_s", 0.02, 3.0, 0.3, "s"),
                param("release_s", 0.1, 6.0, 1.4, "s"),
            ],
            source: "praxis synth (procedural)".into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> GranularTexture {
        let sr = context_rate(ctx);
        GranularTexture {
            density: p[0],
            tone: p[1],
            attack_s: p[2],
            release_s: p[3],
            sr,
            table: Wavetable::from_frames(sr as f32, &frames()),
        }
    }

    fn polyphony(&self) -> usize {
        8
    }

    fn new_voice(&self) -> GranularVoice {
        GranularVoice {
            grains: std::array::from_fn(|_| Grain {
                active: false,
                osc: self.table.clone(),
                n: 0,
                len: 1,
                mult: 1.0,
                amp: 0.0,
                pan: (1.0, 1.0),
            }),
            rng: Pcg32::new(0),
            phase: 0.0,
            inc: 0.0,
            hz: 0.0,
            spawn_in: 0,
            interval: 0.0,
            bright: 0.0,
            gain: 0.0,
            env: Swell::default(),
            release: Release::default(),
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.density = value,
            1 => self.tone = value,
            2 => self.attack_s = value,
            3 => self.release_s = value,
            _ => {}
        }
    }

    fn tail_samples(&self) -> usize {
        samples(self.release_s, self.sr, 1) as usize + 1
    }
}

impl GranularVoice {
    fn spawn(&mut self, sr: f64) {
        let Some(g) = self.grains.iter_mut().find(|g| !g.active) else {
            return;
        };
        let r = &mut self.rng;
        let len = r.range(GRAIN_MIN_S, GRAIN_MAX_S);
        g.len = ((len as f64 * sr) as u32).max(16);
        g.n = 0;
        g.mult = if r.next_f32() < 0.15 + 0.3 * self.bright {
            2.0
        } else {
            1.0
        };
        g.osc.set_freq((self.hz * g.mult) as f32);
        g.osc
            .set_position(((1.0 - self.bright) * r.range(0.2, 1.0)).clamp(0.0, 1.0));
        g.amp = r.range(0.6, 1.0) / g.mult as f32;
        g.pan = balance(r.range(-0.6, 0.6));
        g.active = true;
    }
}

impl Voice<GranularTexture> for GranularVoice {
    fn start(&mut self, patch: &GranularTexture, note: &Note) {
        let sr = note.sample_rate;
        self.rng = note.rng(0x62A1);
        for g in self.grains.iter_mut() {
            g.active = false;
        }
        self.phase = 0.0;
        self.hz = note.hz;
        self.inc = note.hz / sr;
        self.bright = tone_amount(patch.tone, note.velocity, note.brightness);
        self.interval = INTERVAL_S / patch.density.max(0.05);
        self.spawn_in = 0;
        // Coherent grains add in phase: scale by the expected overlap of their windows.
        let overlap = 0.5 * (GRAIN_MIN_S + GRAIN_MAX_S) / self.interval;
        self.gain = 0.5 * vel_gain(note.velocity) as f64 / (0.5 * overlap as f64).max(1.0);
        self.env.start(patch.attack_s, 1.0, 1.0, sr);
        self.release.reset();
        self.done = false;
    }

    fn release(&mut self, patch: &GranularTexture) {
        self.release.start(patch.release_s, patch.sr);
    }

    #[inline]
    fn tick(&mut self, patch: &GranularTexture) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let sr = patch.sr;
        if self.spawn_in == 0 {
            self.spawn(sr);
            let gap = self.interval * self.rng.range(0.5, 1.5);
            self.spawn_in = ((gap as f64 * sr) as u32).max(1);
        }
        self.spawn_in -= 1;
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for g in self.grains.iter_mut() {
            if !g.active {
                continue;
            }
            let w = (PI * g.n as f64 / g.len as f64).sin();
            g.osc.reset((self.phase * g.mult).fract() as f32);
            let v = g.osc.next() * (w * w) as f32 * g.amp;
            l += v * g.pan.0;
            r += v * g.pan.1;
            g.n += 1;
            if g.n >= g.len {
                g.active = false;
            }
        }
        self.phase = (self.phase + self.inc).fract();
        let gain = (self.gain * self.env.next() as f64 * self.release.next() as f64) as f32;
        if self.release.is_done() {
            self.done = true;
        }
        (l * gain, r * gain)
    }

    fn level(&self) -> f32 {
        (self.gain * self.release.gain() as f64) as f32
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
