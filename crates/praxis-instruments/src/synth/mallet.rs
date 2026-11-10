//! `mallet`: a modal struck bar (class motif, sustaining).
//!
//! Three bar modes as decaying sine partials. `material` moves the mode ratios and ring from a
//! marimba (1 : 3.93 : 9.2, short ring, upper modes die fast) to a vibraphone (1 : 4 : 10, long
//! ring). The fundamental is tuned exactly to the note. A soft felt-mallet knock (low-passed
//! seeded noise, a few ms) sits on the attack. `tone`, velocity and bright accents raise the
//! upper modes and the knock. Notes ring 60 dB in `ring x sqrt(261.6 Hz / f)`.
//!
//! Parameters: `material` 0..1 (0.3), `tone` 0..1 (0.45), `release_s` 0.05..4 s (0.6).

use super::common::{
    OneShot, Params, Partial, Ramp, Release, key_decay, param, samples, tone_amount, vel_gain,
};
use super::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{Svf, SvfMode};
use praxis_dsp::noise::White;
use praxis_dsp::rng::derive;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, InstrumentKind};

const PEAK: f64 = 0.6;
const FREE_LEVEL: f64 = 1e-5;
/// (marimba ratio, vibraphone ratio, level, marimba ring factor, vibraphone ring factor).
const MODES: [(f64, f64, f64, f64, f64); 3] = [
    (1.0, 1.0, 1.0, 1.0, 1.0),
    (3.93, 4.0, 0.42, 0.33, 0.5),
    (9.2, 10.0, 0.16, 0.14, 0.25),
];
const MARIMBA_RING_S: f64 = 1.4;
const VIBE_RING_S: f64 = 4.5;

/// The `mallet` preset.
#[derive(Clone, Debug)]
pub struct Mallet {
    material: f32,
    tone: f32,
    release_s: f32,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct MalletVoice {
    parts: [Partial; 3],
    knock_noise: White,
    knock_lp: Svf,
    knock_env: OneShot,
    knock_level: f32,
    gain: f64,
    ramp: Ramp,
    release: Release,
    level: f64,
    done: bool,
}

impl Patch for Mallet {
    type Voice = MalletVoice;

    fn info() -> PresetInfo {
        PresetInfo {
            kind: InstrumentKind::Synth,
            name: "mallet".into(),
            class: GeneratorClass::Motif,
            pitched: true,
            sustaining: true,
            range: (36, 96),
            params: vec![
                param("material", 0.0, 1.0, 0.3, ""),
                param("tone", 0.0, 1.0, 0.45, ""),
                param("release_s", 0.05, 4.0, 0.6, "s"),
            ],
            source: "praxis synth (procedural)".into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> Mallet {
        Mallet {
            material: p[0],
            tone: p[1],
            release_s: p[2],
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        24
    }

    fn new_voice(&self) -> MalletVoice {
        let mut lp = Svf::new(self.sr as f32);
        lp.set_mode(SvfMode::LowPass);
        MalletVoice {
            parts: [Partial::default(); 3],
            knock_noise: White::new(0),
            knock_lp: lp,
            knock_env: OneShot::default(),
            knock_level: 0.0,
            gain: 0.0,
            ramp: Ramp::default(),
            release: Release::default(),
            level: 0.0,
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.material = value,
            1 => self.tone = value,
            2 => self.release_s = value,
            _ => {}
        }
    }

    fn tail_samples(&self) -> usize {
        samples(self.release_s, self.sr, 1) as usize + 1
    }
}

impl Voice<Mallet> for MalletVoice {
    fn start(&mut self, patch: &Mallet, note: &Note) {
        let sr = note.sample_rate;
        let f0 = note.hz;
        let m = patch.material as f64;
        let t = tone_amount(patch.tone, note.velocity, note.brightness) as f64;
        let upper = 0.3 + 1.4 * t;
        let ring = (MARIMBA_RING_S + (VIBE_RING_S - MARIMBA_RING_S) * m) * key_decay(f0);
        let mut rng = note.rng(0x3A11);
        let mut total = 0.0;
        for (k, &(rm, rv, lvl, fm, fv)) in MODES.iter().enumerate() {
            let ratio = rm + (rv - rm) * m;
            let ring_factor = fm + (fv - fm) * m;
            let amp = if k == 0 { lvl } else { lvl * upper };
            let hz = f0 * ratio;
            let amp = amp * ((16_000.0 - hz) / 4_000.0).clamp(0.0, 1.0);
            // Fundamental starts at phase 0 (silent onset); upper modes at a seeded phase.
            let phase = if k == 0 { 0.0 } else { rng.next_f32() as f64 };
            self.parts[k].start(hz, sr, phase, amp, ring * ring_factor);
            total += self.parts[k].amp();
        }
        self.gain = PEAK * vel_gain(note.velocity) as f64 / total.max(1e-9);
        self.knock_noise.reseed(derive(note.seed, 0x4B0C));
        self.knock_lp.reset();
        self.knock_lp.set_cutoff((1500.0 + 2500.0 * t) as f32);
        self.knock_env.clear();
        self.knock_env.trigger(0.0004, 0.008, sr);
        self.knock_level = (0.25 * (0.5 + t)) as f32;
        self.ramp.start(0.0008, sr);
        self.release.reset();
        self.level = PEAK;
        self.done = false;
    }

    fn release(&mut self, patch: &Mallet) {
        self.release.start(patch.release_s, patch.sr);
    }

    #[inline]
    fn tick(&mut self, _patch: &Mallet) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let mut s = 0.0;
        let mut lvl = 0.0;
        for p in self.parts.iter_mut() {
            s += p.next();
            lvl += p.amp();
        }
        let knock = self.knock_lp.tick(self.knock_noise.next()) * self.knock_env.next();
        s += (knock * self.knock_level) as f64;
        let g = self.gain * self.ramp.next() as f64 * self.release.next() as f64;
        self.level = lvl * self.gain * self.release.gain() as f64;
        if self.release.is_done()
            || (self.ramp.is_done() && !self.knock_env.is_active() && self.level < FREE_LEVEL)
        {
            self.done = true;
        }
        let y = (s * g) as f32;
        (y, y)
    }

    fn level(&self) -> f32 {
        self.level as f32
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
