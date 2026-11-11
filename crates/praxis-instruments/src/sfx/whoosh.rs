//! `whoosh`: something passing by (class whoosh, pitched one-shot).
//!
//! Modelled as moving air heard from one spot. The source's level follows its distance
//! (`(1 + (t / w)^2)^-0.75` around the pass point, a narrower approach than wake), and the air
//! it pushes is turbulence: pink noise through three band-passes moving together (1, 1.6 and
//! 2.5 times a centre), under a low-pass, plus a high hiss band. The centre rises as the source
//! comes closer (nearby air is brighter: distant highs are absorbed) and bends like a Doppler
//! shift. Gusts (a smooth 1/f random control at ~7 Hz) move the level and the centre, and a
//! faster rustle roughens the hiss. A quiet tonal core (a soft additive tone plus a narrow noise
//! whistle on the note) carries the pitch: above the note while approaching, through it at the
//! pass, below it after (`70 / sqrt(seconds)` cents each way). The sound pans across the field
//! (direction from the note seed) with a decorrelated air band between the ears, and plays in
//! a room (1 s decay, heard almost as loud as the dry sound, as outdoors off nearby walls). The pass is at 60% of the length; the end of `length` fades to exact silence.
//!
//! Macros:
//! - `length`: total duration, 0.25..4 s (exponential; 0.5 = 1 s). The sound ends exactly then;
//!   note-offs do not cut it.
//! - `brightness`: the air's top centre, 900..7000 Hz (exponential), the hiss band, the room's
//!   damping and the core's harmonics.
//! - `weight`: a low air band (40..110 Hz) that swells with the pass and a sub-bass swell on the
//!   note's pitch class (41..78 Hz); 0 removes both.
//! - `pitch`: the core moves by whole scale steps of the key (-7..+7, 0.5 = the note); the air
//!   shifts continuously, one octave down at 0 to one up at 1.

use super::common::{
    Macros, NOISE_CEILING_HZ, Phasor, Tone, UPDATE, colour_shift, dc_blocker, expmap, fall, info,
    low_layer, midi_hz, pan_unity, pitch_steps, rise, step_in_key, to_samples,
};
use super::space::{Room, Wander};
use crate::synth::common::{Params, vel_gain};
use crate::synth::poly::{Note, Patch, Voice, context_rate};
use praxis_dsp::filter::{DcBlocker, Svf, SvfMode};
use praxis_dsp::noise::{Pink, White};
use praxis_dsp::rng::derive;
use praxis_engine::{InstrumentContext, PresetInfo};
use praxis_score::{GeneratorClass, Key};
use std::f64::consts::TAU;

/// Shortest and longest whoosh, seconds.
pub const LENGTH_S: (f64, f64) = (0.25, 4.0);
/// Top air centre at brightness 0 and 1, Hz.
const CENTRE_HZ: (f64, f64) = (900.0, 7000.0);
/// The centre rises this many octaves from far away to the pass.
const CENTRE_RISE_OCT: f64 = 2.0;
/// The pass point (closest, loudest, on pitch) as a fraction of the length.
pub const PASS: f64 = 0.6;
/// Distance-envelope widths (fractions of the length) before and after the pass.
const APPROACH_W: f64 = 0.2;
const WAKE_W: f64 = 0.24;
/// Start and end windows (fractions of the length) that bring the sound in from and out to
/// exact silence.
const START_WIN: f64 = 0.3;
const END_FROM: f64 = 0.75;
/// Doppler bend in cents at a 1-second whoosh (scales with 1/sqrt(seconds)).
const BEND_CENTS_1S: f64 = 70.0;
/// Doppler transition time as a fraction of the length.
const BEND_TAU: f64 = 0.15;
/// Band ratios and gains of the air body.
const BANDS: [(f64, f64); 3] = [(1.0, 1.0), (1.6, 0.55), (2.5, 0.3)];
const GUST_HZ: f64 = 7.0;
const GUST_DEPTH: f64 = 0.35;
const RUSTLE_HZ: f64 = 19.0;
const RUSTLE_DEPTH: f64 = 0.3;
const LEVEL: f64 = 0.45;
const AIR_LEVEL: f64 = 2.4;
const HISS_LEVEL: f64 = 0.3;
const SIDE_LEVEL: f64 = 0.35;
const CORE_LEVEL: f64 = 0.16;
const WHISTLE_LEVEL: f64 = 0.5;
const RUMBLE_LEVEL: f64 = 2.4;
const SUB_LEVEL: f64 = 0.35;
const ROOM_WET: f64 = 0.9;

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
    src: [Pink; 2],
    hiss_noise: White,
    side_noise: White,
    whistle_noise: White,
    rumble_noise: White,
    bands: [Svf; 3],
    air_lp: Svf,
    hiss_hp: [Svf; 2],
    hiss_lp: Svf,
    side_bp: Svf,
    whistle: Svf,
    rumble: Svf,
    rumble_hp: Svf,
    low_dc: DcBlocker,
    gust: Wander,
    rustle: Wander,
    room: Room,
    centre_lo: f64,
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
            src: [Pink::new(sr, 1), Pink::new(sr, 2)],
            hiss_noise: White::new(3),
            side_noise: White::new(4),
            whistle_noise: White::new(5),
            rumble_noise: White::new(6),
            bands: [
                filter(SvfMode::BandPass),
                filter(SvfMode::BandPass),
                filter(SvfMode::BandPass),
            ],
            air_lp: filter(SvfMode::LowPass),
            hiss_hp: [filter(SvfMode::HighPass), filter(SvfMode::HighPass)],
            hiss_lp: filter(SvfMode::LowPass),
            side_bp: filter(SvfMode::BandPass),
            whistle: filter(SvfMode::BandPass),
            rumble: filter(SvfMode::LowPass),
            rumble_hp: filter(SvfMode::HighPass),
            low_dc: dc_blocker(sr),
            gust: Wander::default(),
            rustle: Wander::default(),
            room: Room::new(self.sr),
            centre_lo: 1000.0,
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
    /// Position relative to the pass, in transition times (Doppler).
    #[inline]
    fn doppler(&self) -> f64 {
        ((self.i as f64 - self.pass as f64) / self.tau).tanh()
    }

    /// Closeness `0..1` (1 at the pass): the inverse-distance level of a passing source.
    #[inline]
    fn closeness(&self) -> f64 {
        let dt = (self.i as f64 - self.pass as f64) / self.n as f64;
        let w = if dt < 0.0 { APPROACH_W } else { WAKE_W };
        let u = dt / w;
        (1.0 + u * u).powf(-0.75)
    }

    fn update(&mut self, m: f64, d: f64, gust: f64) {
        let ratio = 2f64.powf(-self.bend * d / 1200.0);
        let centre = self.centre_lo * 2f64.powf(CENTRE_RISE_OCT * m.sqrt() + 0.25 * gust) * ratio;
        for (f, (r, _)) in self.bands.iter_mut().zip(BANDS) {
            f.set_cutoff_q((centre * r).min(NOISE_CEILING_HZ) as f32, 0.9);
        }
        self.air_lp
            .set_cutoff_q((centre * 3.0).min(NOISE_CEILING_HZ) as f32, 0.7);
        let h = (centre * 2.2).min(0.7 * NOISE_CEILING_HZ);
        self.hiss_hp[0].set_cutoff_q(h as f32, 0.54);
        self.hiss_hp[1].set_cutoff_q(h as f32, 1.31);
        self.hiss_lp
            .set_cutoff_q((h * 2.5).min(NOISE_CEILING_HZ) as f32, 0.6);
        self.side_bp
            .set_cutoff_q((centre * 2.0).min(NOISE_CEILING_HZ) as f32, 0.8);
        self.whistle.set_cutoff_q(
            (self.core_hz * 2f64.powf(-self.bend * d / 1200.0)) as f32,
            14.0,
        );
        self.pan = pan_unity(0.7 * self.dir * d);
    }
}

impl Voice<Whoosh> for WhooshVoice {
    fn start(&mut self, patch: &Whoosh, note: &Note) {
        let sr = note.sample_rate;
        let m = patch.m;
        let bright = m.note_brightness(note.brightness);
        let mut rng = note.rng(0x5F_0001);
        self.src[0].reseed(derive(note.seed, 0x5F_0002));
        self.src[1].reseed(derive(note.seed, 0x5F_0003));
        self.hiss_noise.reseed(derive(note.seed, 0x5F_0004));
        self.side_noise.reseed(derive(note.seed, 0x5F_0005));
        self.whistle_noise.reseed(derive(note.seed, 0x5F_0006));
        self.rumble_noise.reseed(derive(note.seed, 0x5F_0007));
        self.gust.start(derive(note.seed, 0x5F_0008), GUST_HZ, sr);
        self.rustle
            .start(derive(note.seed, 0x5F_0009), RUSTLE_HZ, sr);
        self.sr = sr;
        self.n = patch.length_samples();
        let seconds = self.n as f64 / sr;
        self.pass = ((self.n as f64 * PASS).round() as u64).clamp(1, self.n - 1);
        self.tau = (BEND_TAU * self.n as f64).max(1.0);
        self.bend = BEND_CENTS_1S / seconds.sqrt();
        self.dir = if rng.next_u32() & 1 == 0 { 1.0 } else { -1.0 };

        let top = expmap(bright, CENTRE_HZ.0, CENTRE_HZ.1) * colour_shift(m.pitch);
        self.centre_lo = top / 2f64.powf(CENTRE_RISE_OCT);
        for f in self.bands.iter_mut().chain(self.hiss_hp.iter_mut()).chain([
            &mut self.air_lp,
            &mut self.hiss_lp,
            &mut self.side_bp,
            &mut self.whistle,
        ]) {
            f.reset();
        }
        self.rumble.reset();
        self.rumble.set_cutoff_q(110.0, 0.7);
        self.rumble_hp.reset();
        self.rumble_hp.set_cutoff_q(40.0, 0.7);
        self.low_dc.reset();
        self.room.set(1.0, 0.7, expmap(bright, 2500.0, 9000.0), sr);
        self.room.clear();

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
        let d = self.doppler();
        self.update(self.closeness(), d, 0.0);
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
        let x = self.i as f64 / self.n as f64;
        let m = self.closeness();
        let window = rise(x / START_WIN);
        let src_env = m * window;
        let end = fall((x - END_FROM) / (1.0 - END_FROM));
        let d = self.doppler();
        let gust = self.gust.next();
        let flutter = (GUST_DEPTH * gust - 0.5 * GUST_DEPTH * GUST_DEPTH).exp();
        let rustle = (RUSTLE_DEPTH * self.rustle.next()).exp();
        self.count += 1;
        if self.count >= UPDATE {
            self.count = 0;
            self.update(m, d, gust);
        }

        // Air body: three moving bands of turbulence, a soft top above them.
        let a = self.src[0].next();
        let b = self.src[1].next();
        let mut air = 0.0;
        for (k, (f, (_, g))) in self.bands.iter_mut().zip(BANDS).enumerate() {
            let s = if k == 0 { a } else { b };
            air += g * f.tick(s) as f64;
        }
        let air = AIR_LEVEL * self.air_lp.tick(air as f32) as f64 * flutter;
        let h = self.hiss_hp[0].tick(self.hiss_noise.next());
        let h = self.hiss_hp[1].tick(h);
        let hiss = HISS_LEVEL * self.hiss_lp.tick(h) as f64 * rustle * flutter;
        let side = SIDE_LEVEL * self.side_bp.tick(self.side_noise.next()) as f64 * flutter;

        // Tonal core: the pitch carrier, a soft tone and a narrow whistle on it.
        let hz = self.core_hz * 2f64.powf(-self.bend * d / 1200.0);
        let tone = self.core.next(hz, self.sr);
        let whistle = WHISTLE_LEVEL * self.whistle.tick(self.whistle_noise.next()) as f64;
        let core = CORE_LEVEL * (tone + whistle) * m.sqrt();

        let mid = (air + hiss + core) * src_env;
        let rumble = self
            .rumble_hp
            .tick(self.rumble.tick(self.rumble_noise.next())) as f64;
        let rumble = self.rumble_level * rumble * flutter * m.powf(0.6) * window;
        let sub = self.sub_level * (TAU * self.sub.phase()).sin() * m * window;
        self.sub.advance();
        let low = self.low_dc.tick((rumble + sub) as f32) as f64;

        let (pl, pr) = self.pan;
        let dl = mid * pl + side * src_env;
        let dr = mid * pr - side * src_env;
        let (wl, wr) = self.room.tick(dl, dr);
        let g = self.gain * end;
        self.i += 1;
        (
            ((dl + ROOM_WET * wl + low) * g) as f32,
            ((dr + ROOM_WET * wr + low) * g) as f32,
        )
    }

    fn level(&self) -> f32 {
        if self.done { 0.0 } else { self.gain as f32 }
    }

    fn is_done(&self) -> bool {
        self.done
    }
}
