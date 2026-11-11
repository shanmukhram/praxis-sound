//! `whoosh`: something passing by (class whoosh, pitched one-shot).
//!
//! White noise through two band-passes whose centre rises as the source approaches and falls
//! after it passes, plus a quiet tonal core (a soft additive tone on the note) that carries the
//! pitch. Both bend like a Doppler shift: above the note while approaching, through the note at
//! the pass, below it after. A shorter whoosh is a faster pass, so its bend is wider
//! (`70 / sqrt(seconds)` cents each way) and quicker. The sound pans across the stereo field
//! (direction from the note seed), and its level swells to the pass point (60% of the length)
//! and falls away to exact silence at the end.
//!
//! Macros:
//! - `length`: total duration, 0.25..4 s (exponential; 0.5 = 1 s). The sound ends exactly then;
//!   note-offs do not cut it.
//! - `brightness`: the band's top centre, 900..7000 Hz (exponential), and the core's harmonics.
//! - `weight`: low-passed rumble (below ~110 Hz) and a sub-bass swell at the pass on the note's
//!   pitch class (41..78 Hz); 0 removes both.
//! - `pitch`: the core moves by whole scale steps of the key (-7..+7, 0.5 = the note); the noise
//!   bands shift continuously, one octave down at 0 to one up at 1.

use super::common::{
    Macros, Phasor, Tone, UPDATE, dc_blocker, expmap, fall, info, low_layer, midi_hz, pan_unity,
    pitch_steps, rise, step_in_key, to_samples,
};
use crate::synth::common::{Params, vel_gain};
use crate::synth::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{DcBlocker, Svf, SvfMode};
use praxis_dsp::noise::White;
use praxis_dsp::rng::derive;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, Key};
use std::f64::consts::TAU;

/// Shortest and longest whoosh, seconds.
pub const LENGTH_S: (f64, f64) = (0.25, 4.0);
/// Top band centre at brightness 0 and 1, Hz.
const CENTRE_HZ: (f64, f64) = (900.0, 7000.0);
/// The band centre rises this many octaves from its start to the pass.
const CENTRE_RISE_OCT: f64 = 2.0;
/// The pass point (loudest, on pitch) as a fraction of the length.
pub const PASS: f64 = 0.6;
/// Doppler bend in cents at a 1-second whoosh (scales with 1/sqrt(seconds)).
const BEND_CENTS_1S: f64 = 70.0;
/// Doppler transition time as a fraction of the length.
const BEND_TAU: f64 = 0.15;
const LEVEL: f64 = 0.55;
const CORE_LEVEL: f64 = 0.22;
const RUMBLE_LEVEL: f64 = 2.4;
const SUB_LEVEL: f64 = 0.35;

/// The `whoosh` preset.
#[derive(Clone, Debug)]
pub struct Whoosh {
    m: Macros,
    key: Key,
    sr: f64,
}

#[derive(Clone, Debug)]
pub struct WhooshVoice {
    i: u64,
    n: u64,
    pass: u64,
    tau: f64,
    bend: f64,
    noise: White,
    rumble_noise: White,
    bands: [Svf; 2],
    air_lp: Svf,
    rumble: Svf,
    low_dc: DcBlocker,
    centre_lo: f64,
    rise_oct: f64,
    core: Tone,
    core_hz: f64,
    sub: Phasor,
    sub_level: f64,
    rumble_level: f64,
    dir: f64,
    pan: (f64, f64),
    gain: f64,
    count: u32,
    sr: f64,
    done: bool,
}

impl Whoosh {
    fn length_samples(&self) -> u64 {
        to_samples(expmap(self.m.length, LENGTH_S.0, LENGTH_S.1), self.sr, 2)
    }
}

impl Patch for Whoosh {
    type Voice = WhooshVoice;

    fn info() -> PresetInfo {
        info("whoosh", GeneratorClass::Whoosh, true, false, (24, 96))
    }

    fn new(p: &Params, ctx: &InstrumentContext) -> Whoosh {
        Whoosh {
            m: Macros::from_params(p),
            key: ctx.key,
            sr: context_rate(ctx),
        }
    }

    fn polyphony(&self) -> usize {
        4
    }

    fn new_voice(&self) -> WhooshVoice {
        let sr = self.sr as f32;
        let filter = |mode| {
            let mut f = Svf::new(sr);
            f.set_mode(mode);
            f
        };
        WhooshVoice {
            i: 0,
            n: 0,
            pass: 0,
            tau: 1.0,
            bend: 0.0,
            noise: White::new(1),
            rumble_noise: White::new(2),
            bands: [filter(SvfMode::BandPass), filter(SvfMode::BandPass)],
            air_lp: filter(SvfMode::LowPass),
            rumble: filter(SvfMode::LowPass),
            low_dc: dc_blocker(sr),
            centre_lo: 1000.0,
            rise_oct: CENTRE_RISE_OCT,
            core: Tone::default(),
            core_hz: 0.0,
            sub: Phasor::default(),
            sub_level: 0.0,
            rumble_level: 0.0,
            dir: 1.0,
            pan: (1.0, 1.0),
            gain: 0.0,
            count: 0,
            sr: self.sr,
            done: true,
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        self.m.set(index, value);
    }

    fn tail_samples(&self) -> usize {
        self.length_samples() as usize + 1
    }
}

impl WhooshVoice {
    /// Position relative to the pass, in transition times.
    #[inline]
    fn doppler(&self) -> f64 {
        ((self.i as f64 - self.pass as f64) / self.tau).tanh()
    }

    fn update(&mut self, env: f64, d: f64) {
        let ratio = 2f64.powf(-self.bend * d / 1200.0);
        let centre = self.centre_lo * 2f64.powf(self.rise_oct * env) * ratio;
        self.bands[0].set_cutoff_q(centre.min(9_000.0) as f32, 1.3);
        self.bands[1].set_cutoff_q((centre * 2.2).min(11_000.0) as f32, 2.0);
        self.pan = pan_unity(0.7 * self.dir * d);
    }
}

impl Voice<Whoosh> for WhooshVoice {
    fn start(&mut self, patch: &Whoosh, note: &Note) {
        let sr = note.sample_rate;
        let m = patch.m;
        let bright = m.note_brightness(note.brightness);
        let mut rng = note.rng(0x5F_0001);
        self.noise.reseed(derive(note.seed, 0x5F_0002));
        self.rumble_noise.reseed(derive(note.seed, 0x5F_0003));
        self.sr = sr;
        self.n = patch.length_samples();
        let seconds = self.n as f64 / sr;
        self.pass = ((self.n as f64 * PASS).round() as u64).clamp(1, self.n - 1);
        self.tau = (BEND_TAU * self.n as f64).max(1.0);
        self.bend = BEND_CENTS_1S / seconds.sqrt();
        self.dir = if rng.next_u32() & 1 == 0 { 1.0 } else { -1.0 };

        let top = expmap(bright, CENTRE_HZ.0, CENTRE_HZ.1) * super::common::colour_shift(m.pitch);
        self.centre_lo = top / 2f64.powf(CENTRE_RISE_OCT);
        self.rise_oct = CENTRE_RISE_OCT;
        for f in self.bands.iter_mut() {
            f.reset();
        }
        self.air_lp.reset();
        self.air_lp
            .set_cutoff_q((top * 2.5).min(super::common::NOISE_CEILING_HZ) as f32, 0.7);
        self.rumble.reset();
        self.rumble.set_cutoff_q(110.0, 0.7);
        self.low_dc.reset();

        let pitch = step_in_key(&patch.key, note.pitch, pitch_steps(m.pitch, 7));
        self.core_hz = midi_hz(pitch as f64);
        let max_hz = self.core_hz * 2f64.powf(self.bend / 1200.0);
        self.core.start(2.2 - 1.4 * bright as f64, max_hz, sr, 6);

        let sub_hz = midi_hz(low_layer(pitch as i32 % 12) as f64);
        self.sub = Phasor::new(sub_hz, sr, 0, 0.0);
        let w = m.weight as f64;
        self.sub_level = SUB_LEVEL * w;
        self.rumble_level = RUMBLE_LEVEL * w;

        self.gain = LEVEL * vel_gain(note.velocity) as f64;
        self.i = 0;
        self.count = 0;
        self.update(0.0, self.doppler());
        self.done = false;
    }

    fn release(&mut self, _patch: &Whoosh) {
        // One-shot: the pass always completes.
    }

    #[inline]
    fn tick(&mut self, _patch: &Whoosh) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        if self.i >= self.n {
            self.done = true;
            return (0.0, 0.0);
        }
        let env = if self.i < self.pass {
            rise(self.i as f64 / self.pass as f64)
        } else {
            fall((self.i - self.pass) as f64 / (self.n - self.pass) as f64)
        };
        let d = self.doppler();
        self.count += 1;
        if self.count >= UPDATE {
            self.count = 0;
            self.update(env, d);
        }
        let wn = self.noise.next();
        let air = self.bands[0].tick(wn) as f64 + 0.45 * self.bands[1].tick(wn) as f64;
        let air = self.air_lp.tick(air as f32) as f64;
        let hz = self.core_hz * 2f64.powf(-self.bend * d / 1200.0);
        let core = CORE_LEVEL * self.core.next(hz, self.sr) * env;
        let mid = air + core;
        let rumble = self.rumble_level * self.rumble.tick(self.rumble_noise.next()) as f64;
        let sub = self.sub_level * (TAU * self.sub.phase()).sin() * env;
        self.sub.advance();
        let low = self.low_dc.tick((rumble + sub) as f32) as f64;
        let g = self.gain * env;
        let (pl, pr) = self.pan;
        self.i += 1;
        (((mid * pl + low) * g) as f32, ((mid * pr + low) * g) as f32)
    }

    fn level(&self) -> f32 {
        if self.done { 0.0 } else { self.gain as f32 }
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
