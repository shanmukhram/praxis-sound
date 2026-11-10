//! `warm_pad` (class pad): three band-limited saws (-7, 0, +7 cents, seeded phases, the outer two
//! placed left and right) through a 12 dB low-pass that opens with the slow attack and settles
//! a little lower. The class is `pad`, so the planner's avoid-list blocks it unless a brief asks
//! for a pad (audio-quality rule 4, restraint).
//!
//! Parameters: `tone` 0..1 (0.35), `attack_s` 0.05..4 s (0.9), `release_s` 0.1..6 s (1.8).

use super::common::{Params, Release, Swell, cents, param, samples, tone_amount, vel_gain};
use super::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{Svf, SvfMode};
use praxis_dsp::osc::PolyBlepSaw;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, InstrumentKind};

const DETUNE_CENTS: [f64; 3] = [-7.0, 0.0, 7.0];

/// The `warm_pad` preset.
#[derive(Clone, Debug)]
pub struct WarmPad {
    tone: f32,
    attack_s: f32,
    release_s: f32,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct WarmPadVoice {
    saws: [PolyBlepSaw; 3],
    filters: [Svf; 2],
    base_cut: f32,
    open_cut: f32,
    gain: f64,
    env: Swell,
    release: Release,
    done: bool,
}

impl Patch for WarmPad {
    type Voice = WarmPadVoice;

    fn info() -> PresetInfo {
        PresetInfo {
            kind: InstrumentKind::Synth,
            name: "warm_pad".into(),
            class: GeneratorClass::Pad,
            pitched: true,
            sustaining: false,
            range: (36, 96),
            params: vec![
                param("tone", 0.0, 1.0, 0.35, ""),
                param("attack_s", 0.05, 4.0, 0.9, "s"),
                param("release_s", 0.1, 6.0, 1.8, "s"),
            ],
            source: "praxis synth (procedural)".into(),
        }
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> WarmPad {
        WarmPad {
            tone: p[0],
            attack_s: p[1],
            release_s: p[2],
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        12
    }

    fn new_voice(&self) -> WarmPadVoice {
        let sr = self.sr as f32;
        let lp = || {
            let mut f = Svf::new(sr);
            f.set_mode(SvfMode::LowPass);
            f
        };
        WarmPadVoice {
            saws: std::array::from_fn(|_| PolyBlepSaw::new(sr)),
            filters: [lp(), lp()],
            base_cut: 0.0,
            open_cut: 0.0,
            gain: 0.0,
            env: Swell::default(),
            release: Release::default(),
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.tone = value,
            1 => self.attack_s = value,
            2 => self.release_s = value,
            _ => {}
        }
    }

    fn tail_samples(&self) -> usize {
        samples(self.release_s, self.sr, 1) as usize + 1
    }
}

impl Voice<WarmPad> for WarmPadVoice {
    fn start(&mut self, patch: &WarmPad, note: &Note) {
        let sr = note.sample_rate;
        let t = tone_amount(patch.tone, note.velocity, note.brightness);
        let mut rng = note.rng(0xBAD0);
        for (saw, c) in self.saws.iter_mut().zip(DETUNE_CENTS) {
            saw.set_freq((note.hz * cents(c)) as f32);
            saw.reset(rng.next_f32());
        }
        for f in self.filters.iter_mut() {
            f.reset();
        }
        let hz = note.hz as f32;
        self.base_cut = (hz * (1.5 + 3.0 * t)).max(150.0);
        self.open_cut = hz * (1.0 + 5.0 * t);
        self.gain = vel_gain(note.velocity) as f64;
        self.env.start(patch.attack_s, 2.0, 0.85, sr);
        self.release.reset();
        self.done = false;
    }

    fn release(&mut self, patch: &WarmPad) {
        self.release.start(patch.release_s, patch.sr);
    }

    #[inline]
    fn tick(&mut self, _patch: &WarmPad) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let e = self.env.next();
        let cut = (self.base_cut + self.open_cut * e).min(16_000.0);
        for f in self.filters.iter_mut() {
            f.set_cutoff(cut);
        }
        let [a, b, c] = &mut self.saws;
        let (a, b, c) = (a.next(), b.next(), c.next());
        let l = self.filters[0].tick(a + 0.7 * b + 0.3 * c);
        let r = self.filters[1].tick(c + 0.7 * b + 0.3 * a);
        let g = (self.gain * e as f64 * self.release.next() as f64) as f32;
        if self.release.is_done() {
            self.done = true;
        }
        (l * g * 0.6, r * g * 0.6)
    }

    fn level(&self) -> f32 {
        (self.gain * self.release.gain() as f64) as f32
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
