//! The drum voice that plays any [`DrumSpec`], and the [`Drum`] patch behind every drum preset
//! (one drum per preset, or the whole GM-mapped `kit`).
//!
//! Drums are one-shots: a note-off does not cut them, the hit always plays to its end, and
//! `tail_samples` is the full length of the longest hit the preset can play (at the current
//! `decay`). Every envelope ends in exact silence, followed by [`POST_SAMPLES`] samples that let
//! the filters settle, so a hit never ends with a step.

use super::spec::{DRUMS, DrumSpec, KIT, METAL_HZ, kit_drum};
use crate::synth::common::{OneShot, Params, balance, param, samples, tone_amount, vel_gain};
use crate::synth::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{Svf, SvfMode};
use praxis_dsp::noise::White;
use praxis_dsp::osc::PolyBlepSquare;
use praxis_dsp::rng::derive;
use praxis_engine::{InstrumentContext, NoteOn, PresetInfo};
use praxis_score::{GeneratorClass, InstrumentKind};
use std::f64::consts::TAU;

/// Samples a voice keeps running after its envelopes end, so the filters ring out.
pub const POST_SAMPLES: u64 = 256;
/// Largest seeded decay variation (+/-).
const DECAY_VARIATION: f32 = 0.02;
/// A clap burst decays over this many gaps, so bursts overlap a little.
const BURST_DECAY_GAPS: f32 = 1.6;
const KIT_POLYPHONY: usize = 16;
const SINGLE_POLYPHONY: usize = 8;

/// Settings shared by every drum preset (`tune`, `decay`, `tone`).
#[derive(Clone, Debug)]
pub struct DrumSettings {
    /// Semitones.
    pub tune: f32,
    /// Decay scale.
    pub decay: f32,
    /// 0..1, opens the output low-pass and the attack parts.
    pub tone: f32,
    pub sr: f64,
}

/// A drum preset: one drum (`D` = its index in [`DRUMS`]) or the GM-mapped kit (`D` = [`KIT`]).
#[derive(Clone, Debug)]
pub struct Drum<const D: usize> {
    pub settings: DrumSettings,
}

/// Total length in seconds of one hit of `spec` at decay scale 1, before seeded variation.
fn hit_seconds(spec: &DrumSpec) -> f32 {
    let mut t = 0.0f32;
    if spec.body_level > 0.0 {
        t = t.max(spec.body_attack_s + spec.body_decay_s);
        if spec.mode2_level > 0.0 {
            t = t.max(spec.body_attack_s + spec.mode2_decay_s);
        }
    }
    if spec.noise_level > 0.0 {
        let bursts = spec.bursts.max(1) as f32 - 1.0;
        t = t.max(bursts * spec.burst_gap_s + spec.noise_attack_s + spec.noise_decay_s);
    }
    if spec.metal_level > 0.0 {
        t = t.max(spec.metal_attack_s + spec.metal_decay_s);
    }
    if spec.click_level > 0.0 {
        t = t.max(CLICK_ATTACK_S + spec.click_decay_s);
    }
    t
}

const CLICK_ATTACK_S: f32 = 0.0006;

impl<const D: usize> Drum<D> {
    fn drums() -> &'static [DrumSpec] {
        if D == KIT { &DRUMS } else { &DRUMS[D..D + 1] }
    }

    fn spec_for(pitch: u8) -> &'static DrumSpec {
        if D == KIT {
            &DRUMS[kit_drum(pitch).unwrap_or(0)]
        } else {
            &DRUMS[D]
        }
    }
}

const _: () = assert!(KIT == DRUMS.len());

impl<const D: usize> Patch for Drum<D> {
    type Voice = DrumVoice;

    fn info() -> PresetInfo {
        let (name, class, range) = if D == KIT {
            ("kit", GeneratorClass::Drums, (35, 81))
        } else {
            (DRUMS[D].name, DRUMS[D].class, (0, 127))
        };
        PresetInfo {
            kind: InstrumentKind::Drum,
            name: name.into(),
            class,
            pitched: false,
            sustaining: false,
            range,
            params: vec![
                param("tune", -12.0, 12.0, 0.0, "st"),
                param("decay", 0.5, 2.0, 1.0, "x"),
                param("tone", 0.0, 1.0, 0.5, ""),
            ],
            source: "praxis drums (procedural)".into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> Drum<D> {
        Drum {
            settings: DrumSettings {
                tune: p[0],
                decay: p[1],
                tone: p[2],
                sr: context_rate(ctx),
            },
        }
    }

    fn polyphony(&self) -> usize {
        if D == KIT {
            KIT_POLYPHONY
        } else {
            SINGLE_POLYPHONY
        }
    }

    fn new_voice(&self) -> DrumVoice {
        DrumVoice::new(self.settings.sr)
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.settings.tune = value,
            1 => self.settings.decay = value,
            2 => self.settings.tone = value,
            _ => {}
        }
    }

    fn tail_samples(&self) -> usize {
        let longest = Self::drums().iter().map(hit_seconds).fold(0.0f32, f32::max);
        let s = longest * self.settings.decay * (1.0 + DECAY_VARIATION);
        // +2: rounding of each envelope segment to whole samples.
        (samples(s, self.settings.sr, 1) + POST_SAMPLES + 2) as usize
    }

    fn accepts(&self, note: &NoteOn) -> bool {
        D != KIT || kit_drum(note.pitch).is_some()
    }

    fn choke(&self, note: &NoteOn) -> (u8, u32) {
        let s = Self::spec_for(note.pitch);
        (s.group, s.chokes)
    }
}

impl<const D: usize> Voice<Drum<D>> for DrumVoice {
    fn start(&mut self, patch: &Drum<D>, note: &Note) {
        self.begin(Drum::<D>::spec_for(note.pitch), &patch.settings, note);
    }

    fn release(&mut self, _patch: &Drum<D>) {
        // One-shot: the hit plays to its end.
    }

    #[inline]
    fn tick(&mut self, _patch: &Drum<D>) -> (f32, f32) {
        DrumVoice::tick(self)
    }

    fn level(&self) -> f32 {
        self.level
    }

    fn is_done(&self) -> bool {
        self.done
    }
}

/// One drum hit in progress.
#[derive(Clone, Debug)]
pub struct DrumVoice {
    sr: f64,
    gain: f32,
    pan: (f32, f32),

    body_on: bool,
    phase1: f64,
    phase2: f64,
    hz: f64,
    sweep_amt: f64,
    sweep_env: f64,
    sweep_coeff: f64,
    ratio2: f64,
    body_env: OneShot,
    mode2_env: OneShot,
    body_level: f32,
    mode2_level: f32,

    noise_on: bool,
    noise: White,
    noise_hp: Svf,
    noise_lp: Svf,
    noise_env: OneShot,
    noise_level: f32,
    noise_attack_s: f32,
    noise_decay_s: f32,
    burst_short_s: f32,
    bursts_left: u8,
    burst_timer: u64,
    burst_gap: u64,

    metal_on: bool,
    metal: [PolyBlepSquare; 6],
    metal_hp: [Svf; 2],
    metal_env: OneShot,
    metal_level: f32,

    click_on: bool,
    click_noise: White,
    click_lp: Svf,
    click_env: OneShot,
    click_level: f32,

    out_lp: Svf,
    drive: f32,
    drive_norm: f32,

    post: u64,
    level: f32,
    done: bool,
}

fn lowpass(sr: f64) -> Svf {
    let mut f = Svf::new(sr as f32);
    f.set_mode(SvfMode::LowPass);
    f
}

fn highpass(sr: f64) -> Svf {
    let mut f = Svf::new(sr as f32);
    f.set_mode(SvfMode::HighPass);
    f
}

impl DrumVoice {
    /// A silent voice with every filter and oscillator preallocated.
    pub fn new(sr: f64) -> DrumVoice {
        let fsr = sr as f32;
        DrumVoice {
            sr,
            gain: 0.0,
            pan: (1.0, 1.0),
            body_on: false,
            phase1: 0.0,
            phase2: 0.0,
            hz: 0.0,
            sweep_amt: 0.0,
            sweep_env: 0.0,
            sweep_coeff: 0.0,
            ratio2: 1.0,
            body_env: OneShot::default(),
            mode2_env: OneShot::default(),
            body_level: 0.0,
            mode2_level: 0.0,
            noise_on: false,
            noise: White::new(0),
            noise_hp: highpass(sr),
            noise_lp: lowpass(sr),
            noise_env: OneShot::default(),
            noise_level: 0.0,
            noise_attack_s: 0.0,
            noise_decay_s: 0.0,
            burst_short_s: 0.0,
            bursts_left: 0,
            burst_timer: 0,
            burst_gap: 0,
            metal_on: false,
            metal: std::array::from_fn(|_| PolyBlepSquare::new(fsr)),
            metal_hp: [highpass(sr), highpass(sr)],
            metal_env: OneShot::default(),
            metal_level: 0.0,
            click_on: false,
            click_noise: White::new(0),
            click_lp: lowpass(sr),
            click_env: OneShot::default(),
            click_level: 0.0,
            out_lp: lowpass(sr),
            drive: 0.0,
            drive_norm: 1.0,
            post: 0,
            level: 0.0,
            done: true,
        }
    }

    /// Starts a hit of `spec`.
    pub fn begin(&mut self, spec: &DrumSpec, set: &DrumSettings, note: &Note) {
        let sr = note.sample_rate;
        self.sr = sr;
        let mut rng = note.rng(0xD5A1);
        let detune = 2f64.powf((set.tune as f64 + 0.03 * rng.next_bipolar() as f64) / 12.0);
        let decay = set.decay * (1.0 + DECAY_VARIATION * rng.next_bipolar());
        let level_var = 1.0 + 0.035 * rng.next_bipolar();
        let t = tone_amount(set.tone, note.velocity, note.brightness);
        let attack_boost = 0.4 + 1.2 * t;

        self.gain = spec.gain * vel_gain(note.velocity) * level_var;
        self.pan = balance(spec.pan);

        // Body.
        self.body_on = spec.body_level > 0.0;
        self.phase1 = 0.0;
        self.phase2 = 0.0;
        self.hz = spec.body_hz as f64 * detune;
        self.sweep_amt = (spec.sweep as f64 - 1.0).max(0.0) * (0.75 + 0.5 * t as f64);
        self.sweep_env = 1.0;
        self.sweep_coeff = (-1.0 / (spec.sweep_s.max(1e-4) as f64 * sr)).exp();
        self.ratio2 = spec.mode2_ratio as f64;
        self.body_level = spec.body_level;
        self.mode2_level = if self.body_on {
            spec.mode2_level * (0.6 + 0.8 * t)
        } else {
            0.0
        };
        self.body_env.clear();
        self.mode2_env.clear();
        if self.body_on {
            self.body_env
                .trigger(spec.body_attack_s, spec.body_decay_s * decay, sr);
            if self.mode2_level > 0.0 {
                self.mode2_env
                    .trigger(spec.body_attack_s, spec.mode2_decay_s * decay, sr);
            }
        }

        // Noise.
        self.noise_on = spec.noise_level > 0.0;
        self.noise.reseed(derive(note.seed, 0x4E01));
        self.noise_level = spec.noise_level * (0.8 + 0.4 * t);
        self.noise_hp.reset();
        self.noise_hp.set_cutoff(spec.noise_hp.max(5.0));
        self.noise_lp.reset();
        self.noise_lp.set_cutoff(if spec.noise_lp > 0.0 {
            spec.noise_lp
        } else {
            20_000.0
        });
        self.noise_attack_s = spec.noise_attack_s;
        self.noise_decay_s = spec.noise_decay_s * decay;
        self.burst_gap = samples(spec.burst_gap_s, sr, 1);
        self.burst_short_s = spec.burst_gap_s * BURST_DECAY_GAPS;
        self.bursts_left = spec.bursts.max(1) - 1;
        self.burst_timer = self.burst_gap;
        self.noise_env.clear();
        if self.noise_on {
            let first = if self.bursts_left > 0 {
                self.burst_short_s
            } else {
                self.noise_decay_s
            };
            self.noise_env.trigger(self.noise_attack_s, first, sr);
        } else {
            self.bursts_left = 0;
        }

        // Metal.
        self.metal_on = spec.metal_level > 0.0;
        self.metal_level = spec.metal_level * (0.8 + 0.4 * t);
        self.metal_env.clear();
        if self.metal_on {
            for (osc, hz) in self.metal.iter_mut().zip(METAL_HZ) {
                osc.set_freq((hz as f64 * detune) as f32);
                osc.reset(rng.next_f32());
            }
            for f in self.metal_hp.iter_mut() {
                f.reset();
                f.set_cutoff(spec.metal_hp);
            }
            self.metal_env
                .trigger(spec.metal_attack_s, spec.metal_decay_s * decay, sr);
        }

        // Click.
        self.click_on = spec.click_level > 0.0;
        self.click_level = spec.click_level * attack_boost;
        self.click_noise.reseed(derive(note.seed, 0xC11C));
        self.click_lp.reset();
        self.click_lp.set_cutoff(spec.click_lp);
        self.click_env.clear();
        if self.click_on {
            self.click_env
                .trigger(CLICK_ATTACK_S, spec.click_decay_s * decay, sr);
        }

        // Output.
        self.out_lp.reset();
        let cutoff = spec.lp_hz * 2f32.powf(2.4 * (t - 0.5));
        self.out_lp.set_cutoff(cutoff.min(20_000.0));
        self.drive = spec.drive;
        self.drive_norm = if spec.drive > 0.0 {
            1.0 / spec.drive.tanh()
        } else {
            1.0
        };
        self.post = POST_SAMPLES;
        self.level = self.gain;
        self.done = false;
    }

    /// The next stereo sample.
    #[inline]
    pub fn tick(&mut self) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let mut x = 0.0f32;
        let mut env_sum = 0.0f32;
        if self.body_on {
            let hz = self.hz * (1.0 + self.sweep_amt * self.sweep_env);
            self.sweep_env *= self.sweep_coeff;
            let e = self.body_env.next();
            x += (self.phase1 * TAU).sin() as f32 * e * self.body_level;
            env_sum += e * self.body_level;
            self.phase1 = (self.phase1 + hz / self.sr).fract();
            if self.mode2_level > 0.0 {
                let e2 = self.mode2_env.next();
                x += (self.phase2 * TAU).sin() as f32 * e2 * self.mode2_level;
                env_sum += e2 * self.mode2_level;
                self.phase2 = (self.phase2 + hz * self.ratio2 / self.sr).fract();
            }
        }
        if self.noise_on {
            if self.bursts_left > 0 {
                self.burst_timer -= 1;
                if self.burst_timer == 0 {
                    self.bursts_left -= 1;
                    self.burst_timer = self.burst_gap;
                    let d = if self.bursts_left == 0 {
                        self.noise_decay_s
                    } else {
                        self.burst_short_s
                    };
                    self.noise_env.trigger(self.noise_attack_s, d, self.sr);
                }
            }
            let n = self.noise_lp.tick(self.noise_hp.tick(self.noise.next()));
            let e = self.noise_env.next();
            x += n * e * self.noise_level;
            env_sum += e * self.noise_level;
        }
        if self.metal_on {
            let mut m = 0.0;
            for osc in self.metal.iter_mut() {
                m += osc.next();
            }
            m *= 1.0 / 6.0;
            for f in self.metal_hp.iter_mut() {
                m = f.tick(m);
            }
            let e = self.metal_env.next();
            x += m * e * self.metal_level;
            env_sum += e * self.metal_level;
        }
        if self.click_on {
            let c = self.click_lp.tick(self.click_noise.next());
            let e = self.click_env.next();
            x += c * e * self.click_level;
            env_sum += e * self.click_level;
        }
        let mut y = self.out_lp.tick(x);
        if self.drive > 0.0 {
            y = (self.drive * y).tanh() * self.drive_norm;
        }
        y *= self.gain;
        self.level = env_sum * self.gain;

        let running = self.body_env.is_active()
            || self.mode2_env.is_active()
            || self.noise_env.is_active()
            || self.bursts_left > 0
            || self.metal_env.is_active()
            || self.click_env.is_active();
        if !running {
            self.post = self.post.saturating_sub(1);
            if self.post == 0 {
                self.done = true;
            }
        }
        (y * self.pan.0, y * self.pan.1)
    }
}
