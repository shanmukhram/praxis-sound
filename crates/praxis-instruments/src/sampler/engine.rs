//! The sampler voice engine: [`SamplerInstrument`] implements `praxis_engine::Instrument`.
//!
//! - A fixed pool of voices is allocated in [`SamplerInstrument::new`]; `note_on`, `note_off`,
//!   `set_param` and `process` never allocate, lock or panic.
//! - Region choice by key and velocity, with an equal-power crossfade between adjacent velocity
//!   layers (or the SFZ `xfin_*`/`xfout_*` opcodes when present).
//! - Pitch from `pitch_keycenter`, `pitch_keytrack`, `transpose` and `tune`, resampled with
//!   4-point, 3rd-order Hermite interpolation (also across loop points).
//! - DAHDSR amplitude envelope with SFZ defaults, `amp_veltrack`, release samples with
//!   `rt_decay`, loops (`no_loop`, `one_shot`, `loop_continuous`, `loop_sustain`).
//! - Round robin (`seq_length`/`seq_position`) and `lorand`/`hirand` draw from the note's own
//!   seed, never from a running counter, so editing one bar cannot change another bar's notes.
//! - Click safety: a minimum attack ramp, a minimum release, a short fade at sample ends, and
//!   stolen voices fade out over a few milliseconds instead of being cut.

use super::program::Program;
use super::region::{FilterType, LoopMode, Trigger};
use super::sample::PAD_FRONT;
use praxis_dsp::biquad::{Biquad, BiquadShape, Coeffs};
use praxis_dsp::rng::{Pcg32, derive};
use praxis_dsp::util::{db_to_gain, pan_gains, sanitize};
use praxis_engine::{Instrument, NoteOn};
use std::sync::Arc;

/// Extra voice slots beyond the polyphony limit, used only by voices fading out after a steal.
const FADE_SLOTS: usize = 8;
/// Notes remembered for note-off (release samples, rt_decay).
const HELD_NOTES: usize = 128;
/// Voices one note may start (layers x stereo pairs).
const MAX_VOICES_PER_NOTE: usize = 8;
/// The envelope ends a voice once it falls below this level (-90 dB).
const SILENT: f32 = 3.0e-5;
/// Release time (-60 dB) to the end of the voice at [`SILENT`]: ln(SILENT) / ln(0.001), rounded up.
const RELEASE_TO_SILENT: f32 = 1.51;

/// Per-voice tone shaping: a gentle low-pass that tracks velocity, key and brightness, and a
/// static high shelf. All frequencies in Hz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tone {
    /// Low-pass cutoff at velocity 0.5, key 60, brightness 0. `f32::INFINITY` = no low-pass.
    pub lp_base_hz: f32,
    /// Octaves the cutoff moves from velocity 0 to velocity 1 (darker when soft).
    pub lp_vel_octaves: f32,
    /// Fraction of keyboard tracking (1 = cutoff follows pitch exactly).
    pub lp_key_track: f32,
    /// Octaves the cutoff opens at brightness 1 (bright accents).
    pub lp_bright_octaves: f32,
    pub lp_q: f32,
    pub shelf_hz: f32,
    /// Static shelf gain (negative tames hammer noise).
    pub shelf_db: f32,
    /// Shelf gain added at brightness 1.
    pub bright_shelf_db: f32,
}

impl Tone {
    /// No tone shaping except a gentle shelf lift on bright accents.
    pub const NATURAL: Tone = Tone {
        lp_base_hz: f32::INFINITY,
        lp_vel_octaves: 0.0,
        lp_key_track: 0.0,
        lp_bright_octaves: 0.0,
        lp_q: std::f32::consts::FRAC_1_SQRT_2,
        shelf_hz: 3000.0,
        shelf_db: 0.0,
        bright_shelf_db: 2.5,
    };
}

/// How a program is played: everything a preset adjusts on top of the SFZ/SF2 data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Voicing {
    /// Velocity used to pick layers = note velocity x this (below 1 favours soft layers).
    pub layer_velocity_scale: f32,
    /// Added to the layer velocity (0..1 scale) at brightness 1: bright accents reach the
    /// harder, brighter layer.
    pub bright_layer_boost: f32,
    /// Equal-power crossfade between adjacent velocity layers.
    pub auto_xfade: bool,
    /// Multiplies each region's `amp_veltrack` (below 1 = gentler dynamics).
    pub veltrack_scale: f32,
    /// Minimum attack ramp in seconds (click safety; larger values soften the hammer).
    pub min_attack_s: f32,
    /// Extra attack at velocity 0, shrinking to none at velocity 1 (soft notes bloom).
    pub soft_attack_s: f32,
    /// Replaces every region's `ampeg_release` when set.
    pub release_s: Option<f32>,
    /// Minimum release in seconds (click-free note-off).
    pub min_release_s: f32,
    pub tone: Tone,
    pub gain_db: f32,
    /// Per-note gain variation (+-dB), drawn from the note seed.
    pub gain_jitter_db: f32,
    /// Fade applied over the last part of a non-looping sample.
    pub sample_end_fade_s: f32,
    /// Fade-out of a stolen voice.
    pub steal_fade_s: f32,
    /// Start playback at each sample's measured onset ([`super::sample::measure_onset`]) so the
    /// audible attack lands on the note time; SFZ `offset` applies on top.
    pub align_onsets: bool,
}

impl Default for Voicing {
    fn default() -> Self {
        Voicing {
            layer_velocity_scale: 1.0,
            bright_layer_boost: 0.0,
            auto_xfade: true,
            veltrack_scale: 1.0,
            min_attack_s: 0.001,
            soft_attack_s: 0.0,
            release_s: None,
            min_release_s: 0.01,
            tone: Tone::NATURAL,
            gain_db: 0.0,
            gain_jitter_db: 0.0,
            sample_end_fade_s: 0.005,
            steal_fade_s: 0.005,
            align_onsets: true,
        }
    }
}

/// Maps preset parameter values (indexed like `PresetInfo::params`) to a voicing.
pub type VoicingFn = fn(&[f32]) -> Voicing;

/// Maximum preset parameters a sampler instrument stores.
pub const MAX_PARAMS: usize = 8;

/// Region data precomputed for the audio thread.
#[derive(Clone, Debug)]
struct Prepared {
    sample: usize,
    lokey: u8,
    hikey: u8,
    lovel: u8,
    hivel: u8,
    keycenter: f32,
    keytrack: f32,
    /// transpose + tune, in semitones.
    shift: f32,
    sample_rate: f32,
    volume_gain: f32,
    pan: f32,
    stereo: bool,
    width: f32,
    veltrack: f32,
    env: super::region::AmpEnvelope,
    offset: f64,
    /// `offset` plus the sample's measured pre-roll, clamped inside the sample.
    aligned_offset: f64,
    /// Exclusive end position in frames.
    end: f64,
    mode: LoopMode,
    /// Loop start and exclusive end (frames), when the region loops.
    loop_range: Option<(f64, f64)>,
    trigger: Trigger,
    rt_decay: f32,
    seq_length: u32,
    seq_position: u32,
    lorand: f32,
    hirand: f32,
    xf_explicit: bool,
    xf: (u8, u8, u8, u8),
    filter: Option<super::region::RegionFilter>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Done,
}

/// DAHDSR envelope: linear attack, exponential decay and release (-60 dB at the stated time).
#[derive(Clone, Copy, Debug)]
struct Env {
    stage: Stage,
    level: f32,
    n: u32,
    attack_n: u32,
    hold_n: u32,
    decay_n: u32,
    decay_coeff: f32,
    sustain: f32,
    release_coeff: f32,
}

impl Env {
    const IDLE: Env = Env {
        stage: Stage::Done,
        level: 0.0,
        n: 0,
        attack_n: 1,
        hold_n: 0,
        decay_n: 0,
        decay_coeff: 0.0,
        sustain: 1.0,
        release_coeff: 0.0,
    };

    fn exp_coeff(seconds: f32, sr: f32) -> f32 {
        let n = (seconds * sr).max(1.0);
        (0.001f32.ln() / n).exp()
    }

    fn start(
        delay: f32,
        attack: f32,
        hold: f32,
        decay: f32,
        sustain: f32,
        release: f32,
        sr: f32,
    ) -> Env {
        let delay_n = (delay * sr).round() as u32;
        let mut e = Env {
            stage: Stage::Delay,
            level: 0.0,
            n: delay_n,
            attack_n: ((attack * sr).round() as u32).max(1),
            hold_n: (hold * sr).round() as u32,
            decay_n: (decay * sr).round() as u32,
            decay_coeff: Env::exp_coeff(decay, sr),
            sustain: sustain.clamp(0.0, 1.0),
            release_coeff: Env::exp_coeff(release, sr),
        };
        if delay_n == 0 {
            e.stage = Stage::Attack;
            e.n = e.attack_n;
        }
        e
    }

    fn release(&mut self, release_s: f32, sr: f32) {
        self.release_coeff = Env::exp_coeff(release_s, sr);
        self.stage = if self.stage == Stage::Delay {
            Stage::Done
        } else {
            Stage::Release
        };
    }

    #[inline]
    fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Delay => {
                if self.n > 0 {
                    self.n -= 1;
                } else {
                    self.stage = Stage::Attack;
                    self.n = self.attack_n;
                }
                0.0
            }
            Stage::Attack => {
                self.level += 1.0 / self.attack_n as f32;
                self.n = self.n.saturating_sub(1);
                if self.n == 0 || self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Hold;
                    self.n = self.hold_n;
                }
                self.level
            }
            Stage::Hold => {
                if self.n > 0 {
                    self.n -= 1;
                } else {
                    self.stage = Stage::Decay;
                    self.n = self.decay_n;
                }
                self.level
            }
            Stage::Decay => {
                if self.n > 0 {
                    self.level = self.sustain + (self.level - self.sustain) * self.decay_coeff;
                    self.n -= 1;
                } else {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                }
                self.level
            }
            Stage::Sustain => {
                self.level = self.sustain;
                if self.level < SILENT {
                    self.stage = Stage::Done;
                    self.level = 0.0;
                }
                self.level
            }
            Stage::Release => {
                self.level *= self.release_coeff;
                if self.level < SILENT {
                    self.level = 0.0;
                    self.stage = Stage::Done;
                }
                self.level
            }
            Stage::Done => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VState {
    Free,
    Playing,
    Released,
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    state: VState,
    note_id: u64,
    /// Release-trigger voices ignore note-off.
    one_shot: bool,
    sample: usize,
    pos: f64,
    step: f64,
    end: f64,
    looping: bool,
    loop_start: f64,
    loop_end: f64,
    wrapped: bool,
    mode: LoopMode,
    env: Env,
    release_s: f32,
    gain_l: f32,
    gain_r: f32,
    width: f32,
    stereo: bool,
    fade: f32,
    fade_step: f32,
    end_fade: f64,
    lp_on: bool,
    lp: [Biquad; 2],
    shelf_on: bool,
    shelf: [Biquad; 2],
    rf_on: bool,
    rf: [Biquad; 2],
    started: u64,
}

impl Voice {
    fn free() -> Voice {
        Voice {
            state: VState::Free,
            note_id: 0,
            one_shot: false,
            sample: 0,
            pos: 0.0,
            step: 1.0,
            end: 0.0,
            looping: false,
            loop_start: 0.0,
            loop_end: 0.0,
            wrapped: false,
            mode: LoopMode::NoLoop,
            env: Env::IDLE,
            release_s: 0.01,
            gain_l: 0.0,
            gain_r: 0.0,
            width: 1.0,
            stereo: false,
            fade: 1.0,
            fade_step: 0.0,
            end_fade: 1.0,
            lp_on: false,
            lp: [idle_biquad(); 2],
            shelf_on: false,
            shelf: [idle_biquad(); 2],
            rf_on: false,
            rf: [idle_biquad(); 2],
            started: 0,
        }
    }

    fn loudness(&self) -> f32 {
        self.env.level * self.gain_l.max(self.gain_r) * self.fade
    }

    fn fading(&self) -> bool {
        self.fade_step > 0.0
    }

    /// One output sample of the voice: what to add, and whether the voice ends there.
    #[inline(always)]
    fn step(&mut self, sl: &[f32], sr: &[f32]) -> Step {
        let loop_len = self.loop_end - self.loop_start;
        let e = self.env.next();
        if self.env.stage == Stage::Done {
            return Step::Stop;
        }
        let mut g = e * self.fade;
        if self.fade_step > 0.0 {
            self.fade -= self.fade_step;
            if self.fade <= 0.0 {
                return Step::Stop;
            }
        }
        if !self.looping {
            let rem = self.end - self.pos;
            if rem < self.end_fade {
                g *= (rem / self.end_fade).max(0.0) as f32;
            }
        }
        let ip = self.pos as usize;
        let t = (self.pos - ip as f64) as f32;
        let (a, b, c, d) = if self.looping
            && (self.pos + 3.0 > self.loop_end
                || (self.wrapped && self.pos < self.loop_start + 1.0))
        {
            let ls = self.loop_start as usize;
            let le = self.loop_end as usize; // exclusive
            let len = le - ls;
            let wrap = |k: isize| -> usize {
                let mut k = k;
                if k >= le as isize {
                    k -= len as isize;
                } else if self.wrapped && k < ls as isize {
                    k += len as isize;
                }
                (k + PAD_FRONT as isize).max(0) as usize
            };
            let ip = ip as isize;
            (wrap(ip - 1), wrap(ip), wrap(ip + 1), wrap(ip + 2))
        } else {
            let base = ip + PAD_FRONT;
            (base - 1, base, base + 1, base + 2)
        };
        let mut xl = hermite(sl[a], sl[b], sl[c], sl[d], t);
        let mut xr = if self.stereo {
            hermite(sr[a], sr[b], sr[c], sr[d], t)
        } else {
            xl
        };
        if self.stereo && self.width != 1.0 {
            let m = 0.5 * (xl + xr);
            let s = 0.5 * (xl - xr) * self.width;
            xl = m + s;
            xr = m - s;
        }
        if self.rf_on {
            xl = self.rf[0].tick(xl);
            xr = self.rf[1].tick(xr);
        }
        if self.lp_on {
            xl = self.lp[0].tick(xl);
            xr = self.lp[1].tick(xr);
        }
        if self.shelf_on {
            xl = self.shelf[0].tick(xl);
            xr = self.shelf[1].tick(xr);
        }
        let out = (xl * g * self.gain_l, xr * g * self.gain_r);

        self.pos += self.step;
        if self.looping {
            if self.pos >= self.loop_end {
                self.pos -= loop_len;
                self.wrapped = true;
            }
        } else if self.pos >= self.end {
            return Step::Last(out.0, out.1);
        }
        Step::Add(out.0, out.1)
    }

    /// Renders and adds into `l`/`r`. Returns false when the voice has finished.
    fn render(&mut self, data: &super::sample::SampleData, l: &mut [f32], r: &mut [f32]) -> bool {
        self.render_from(data.left_padded(), data.right_padded(), l, r)
    }

    /// [`Voice::render`] from the sample's padded channels.
    #[inline]
    fn render_from(&mut self, sl: &[f32], sr: &[f32], l: &mut [f32], r: &mut [f32]) -> bool {
        for (a, b) in l.iter_mut().zip(r.iter_mut()) {
            match self.step(sl, sr) {
                Step::Add(x, y) => {
                    *a += x;
                    *b += y;
                }
                Step::Last(x, y) => {
                    *a += x;
                    *b += y;
                    return false;
                }
                Step::Stop => return false,
            }
        }
        true
    }

    /// [`Voice::render`] of two voices over the same block, interleaved sample by sample so the
    /// CPU overlaps their filter recursions. Each sample still gets `a`'s contribution before
    /// `b`'s, exactly as rendering `a` and then `b` would add them, so the output is
    /// bit-identical. Returns whether each voice is still playing.
    fn render_pair(
        a: &mut Voice,
        da: &super::sample::SampleData,
        b: &mut Voice,
        db: &super::sample::SampleData,
        l: &mut [f32],
        r: &mut [f32],
    ) -> (bool, bool) {
        let (al, ar) = (da.left_padded(), da.right_padded());
        let (bl, br) = (db.left_padded(), db.right_padded());
        let n = l.len().min(r.len());
        for i in 0..n {
            let a_ends = match a.step(al, ar) {
                Step::Add(x, y) => {
                    l[i] += x;
                    r[i] += y;
                    false
                }
                Step::Last(x, y) => {
                    l[i] += x;
                    r[i] += y;
                    true
                }
                Step::Stop => {
                    // `a` adds nothing from here: `b` carries on alone, from this sample.
                    return (false, b.render_from(bl, br, &mut l[i..n], &mut r[i..n]));
                }
            };
            let b_ends = match b.step(bl, br) {
                Step::Add(x, y) => {
                    l[i] += x;
                    r[i] += y;
                    false
                }
                Step::Last(x, y) => {
                    l[i] += x;
                    r[i] += y;
                    true
                }
                Step::Stop => true,
            };
            match (a_ends, b_ends) {
                (false, false) => {}
                (true, true) => return (false, false),
                (true, false) => {
                    return (
                        false,
                        b.render_from(bl, br, &mut l[i + 1..n], &mut r[i + 1..n]),
                    );
                }
                (false, true) => {
                    return (
                        a.render_from(al, ar, &mut l[i + 1..n], &mut r[i + 1..n]),
                        false,
                    );
                }
            }
        }
        (true, true)
    }

    fn enter_release(&mut self, sr: f32) {
        if self.state != VState::Playing || self.one_shot {
            return;
        }
        self.state = VState::Released;
        if self.mode == LoopMode::OneShot {
            return;
        }
        if self.mode == LoopMode::Sustain {
            self.looping = false;
        }
        self.env.release(self.release_s, sr);
    }
}

/// One sample of a voice ([`Voice::step`]).
enum Step {
    /// Add this and keep playing.
    Add(f32, f32),
    /// Add this; the voice ends after it.
    Last(f32, f32),
    /// The voice ended before this sample (nothing to add).
    Stop,
}

fn idle_biquad() -> Biquad {
    Biquad::new(Coeffs::IDENTITY)
}

/// 4-point, 3rd-order Hermite (Catmull-Rom) interpolation between `x0` and `x1`.
#[inline]
pub fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

#[derive(Clone, Copy, Debug)]
struct Held {
    active: bool,
    id: u64,
    pitch: u8,
    velocity: f32,
    seed: u64,
    started: u64,
}

const HELD_NONE: Held = Held {
    active: false,
    id: 0,
    pitch: 0,
    velocity: 0.0,
    seed: 0,
    started: 0,
};

/// A sampler instrument instance (one per layer).
pub struct SamplerInstrument {
    program: Arc<Program>,
    prepared: Vec<Prepared>,
    /// Attack-type regions (attack, first, legato) per key.
    by_key: Vec<Vec<u32>>,
    /// Release-trigger regions per key.
    release_by_key: Vec<Vec<u32>>,
    voices: Vec<Voice>,
    max_voices: usize,
    held: Vec<Held>,
    sample_rate: f32,
    clock: u64,
    params: [f32; MAX_PARAMS],
    n_params: usize,
    voicing_fn: Option<VoicingFn>,
    voicing: Voicing,
}

impl std::fmt::Debug for SamplerInstrument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SamplerInstrument")
            .field("source", &self.program.source)
            .field("regions", &self.prepared.len())
            .field("max_voices", &self.max_voices)
            .finish()
    }
}

fn filter_coeffs(kind: FilterType, hz: f32, res_db: f32, sr: f32) -> Option<Coeffs> {
    if !hz.is_finite() {
        return None;
    }
    let f = (hz as f64).clamp(10.0, sr as f64 * 0.49);
    let q = std::f64::consts::FRAC_1_SQRT_2 * 10f64.powf(res_db as f64 / 20.0);
    Some(match kind {
        FilterType::Lpf2p => Coeffs::design(BiquadShape::LowPass, f, q, sr as f64),
        FilterType::Hpf2p => Coeffs::design(BiquadShape::HighPass, f, q, sr as f64),
        FilterType::Bpf2p => Coeffs::design(BiquadShape::BandPass, f, q, sr as f64),
        FilterType::Lpf1p | FilterType::Hpf1p => {
            let p = (-2.0 * std::f64::consts::PI * f / sr as f64).exp();
            if kind == FilterType::Lpf1p {
                Coeffs {
                    b0: (1.0 - p) as f32,
                    b1: 0.0,
                    b2: 0.0,
                    a1: -p as f32,
                    a2: 0.0,
                }
            } else {
                Coeffs {
                    b0: ((1.0 + p) / 2.0) as f32,
                    b1: (-(1.0 + p) / 2.0) as f32,
                    b2: 0.0,
                    a1: -p as f32,
                    a2: 0.0,
                }
            }
        }
    })
}

impl SamplerInstrument {
    /// Builds an instrument with `max_voices` polyphony (plus a few fade-out slots), preallocating
    /// everything the audio thread uses. `voicing_fn` maps `params` (preset parameter values) to a
    /// voicing; without one, `voicing` is used as is and `set_param` only stores values.
    pub fn new(
        program: Arc<Program>,
        sample_rate: f32,
        max_voices: usize,
        voicing: Voicing,
        voicing_fn: Option<VoicingFn>,
        params: &[f32],
    ) -> SamplerInstrument {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate
        } else {
            48_000.0
        };
        let mut prepared = Vec::with_capacity(program.regions.len());
        for (i, r) in program.regions.iter().enumerate() {
            let s = &program.samples[program.sample_of[i]];
            let frames = s.frames() as u64;
            let last = frames.saturating_sub(1);
            let end_incl = r.end.unwrap_or(last).min(last);
            let offset = r.offset.min(end_incl);
            let mode = r.effective_loop_mode();
            let loop_range = match (mode, r.loop_start, r.loop_end) {
                (LoopMode::Continuous | LoopMode::Sustain, Some(ls), Some(le))
                    if le.min(end_incl) > ls =>
                {
                    Some((ls as f64, le.min(end_incl) as f64 + 1.0))
                }
                _ => None,
            };
            let mode = if loop_range.is_none()
                && matches!(mode, LoopMode::Continuous | LoopMode::Sustain)
            {
                LoopMode::NoLoop
            } else {
                mode
            };
            prepared.push(Prepared {
                sample: program.sample_of[i],
                lokey: r.lokey,
                hikey: r.hikey,
                lovel: r.lovel,
                hivel: r.hivel,
                keycenter: r.pitch_keycenter as f32,
                keytrack: r.pitch_keytrack / 100.0,
                shift: r.transpose as f32 + r.tune / 100.0,
                sample_rate: s.sample_rate,
                volume_gain: db_to_gain(r.volume),
                pan: r.pan / 100.0,
                stereo: s.is_stereo(),
                width: r.width / 100.0,
                veltrack: r.amp_veltrack / 100.0,
                env: r.ampeg,
                offset: offset as f64,
                aligned_offset: (offset + s.onset() as u64).min(end_incl) as f64,
                end: end_incl as f64 + 1.0,
                mode,
                loop_range,
                trigger: r.trigger,
                rt_decay: r.rt_decay,
                seq_length: r.seq_length.max(1),
                seq_position: r.seq_position.max(1),
                lorand: r.lorand,
                hirand: r.hirand,
                xf_explicit: r.has_explicit_xfade(),
                xf: (r.xfin_lovel, r.xfin_hivel, r.xfout_lovel, r.xfout_hivel),
                filter: r.filter,
            });
        }
        let mut by_key = vec![Vec::new(); 128];
        let mut release_by_key = vec![Vec::new(); 128];
        for (i, p) in prepared.iter().enumerate() {
            for k in p.lokey..=p.hikey.min(127) {
                if p.trigger == Trigger::Release {
                    release_by_key[k as usize].push(i as u32);
                } else {
                    by_key[k as usize].push(i as u32);
                }
            }
        }
        let max_voices = max_voices.max(1);
        let mut p = [0f32; MAX_PARAMS];
        let n_params = params.len().min(MAX_PARAMS);
        p[..n_params].copy_from_slice(&params[..n_params]);
        let voicing = match voicing_fn {
            Some(f) => f(&p[..n_params]),
            None => voicing,
        };
        SamplerInstrument {
            program,
            prepared,
            by_key,
            release_by_key,
            voices: vec![Voice::free(); max_voices + FADE_SLOTS],
            max_voices,
            held: vec![HELD_NONE; HELD_NOTES],
            sample_rate,
            clock: 0,
            params: p,
            n_params,
            voicing_fn,
            voicing,
        }
    }

    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }

    pub fn voicing(&self) -> &Voicing {
        &self.voicing
    }

    /// Voices currently sounding (playing, releasing or fading).
    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.state != VState::Free)
            .count()
    }

    fn release_time(&self, p: &Prepared) -> f32 {
        self.voicing
            .release_s
            .unwrap_or(p.env.release)
            .max(self.voicing.min_release_s)
    }

    /// Gain of region `ri` for layer velocity `v` (0..=127, fractional), or 0 if it does not
    /// sound. Adjacent layers on the same key crossfade with equal power.
    fn layer_gain(&self, ri: usize, list: &[u32], v: f32) -> f32 {
        let p = &self.prepared[ri];
        let vi = v.round().clamp(0.0, 127.0) as u8;
        if p.xf_explicit {
            if vi < p.lovel || vi > p.hivel {
                return 0.0;
            }
            let (in_lo, in_hi, out_lo, out_hi) = p.xf;
            let mut g = 1.0f32;
            if in_hi > in_lo {
                g *= ((v - in_lo as f32) / (in_hi - in_lo) as f32)
                    .clamp(0.0, 1.0)
                    .sqrt();
            } else if in_hi > 0 && vi < in_hi {
                g = 0.0;
            }
            if out_hi > out_lo {
                g *= (1.0 - (v - out_lo as f32) / (out_hi - out_lo) as f32)
                    .clamp(0.0, 1.0)
                    .sqrt();
            }
            return g;
        }
        let width = |q: &Prepared| (q.hivel as f32 - q.lovel as f32 + 1.0).max(1.0);
        let half = |q: &Prepared| {
            if self.voicing.auto_xfade {
                (0.25 * width(p).min(width(q))).min(8.0)
            } else {
                0.0
            }
        };
        let same_class =
            |q: &Prepared| (q.trigger == Trigger::Release) == (p.trigger == Trigger::Release);
        let mut g = 1.0f32;
        // Lower edge.
        let lo_edge = p.lovel as f32 - 0.5;
        let lower = (p.lovel > 0)
            .then(|| {
                list.iter()
                    .map(|&q| &self.prepared[q as usize])
                    .find(|q| q.hivel as u16 + 1 == p.lovel as u16 && same_class(q))
            })
            .flatten();
        match lower.map(half) {
            Some(w) if w > 0.0 => {
                let x = (v - (lo_edge - w)) / (2.0 * w);
                if x <= 0.0 {
                    return 0.0;
                }
                if x < 1.0 {
                    g *= (x * std::f32::consts::FRAC_PI_2).sin();
                }
            }
            _ => {
                if vi < p.lovel {
                    return 0.0;
                }
            }
        }
        let hi_edge = p.hivel as f32 + 0.5;
        let upper = (p.hivel < 127)
            .then(|| {
                list.iter()
                    .map(|&q| &self.prepared[q as usize])
                    .find(|q| q.lovel as u16 == p.hivel as u16 + 1 && same_class(q))
            })
            .flatten();
        match upper.map(half) {
            Some(w) if w > 0.0 => {
                let x = (v - (hi_edge - w)) / (2.0 * w);
                if x >= 1.0 {
                    return 0.0;
                }
                if x > 0.0 {
                    g *= (x * std::f32::consts::FRAC_PI_2).cos();
                }
            }
            _ => {
                if vi > p.hivel {
                    return 0.0;
                }
            }
        }
        g
    }

    fn alloc_voice(&mut self) -> usize {
        let busy = self
            .voices
            .iter()
            .filter(|v| v.state != VState::Free && !v.fading())
            .count();
        if busy >= self.max_voices {
            // Steal the quietest non-fading voice (oldest on ties) with a short fade.
            let mut best: Option<(usize, f32, u64)> = None;
            for (i, v) in self.voices.iter().enumerate() {
                if v.state == VState::Free || v.fading() {
                    continue;
                }
                let l = v.loudness();
                if best.is_none_or(|(_, bl, bs)| l < bl || (l == bl && v.started < bs)) {
                    best = Some((i, l, v.started));
                }
            }
            if let Some((i, _, _)) = best {
                let steps = (self.voicing.steal_fade_s * self.sample_rate).max(1.0);
                let v = &mut self.voices[i];
                v.fade_step = v.fade.max(1e-6) / steps;
            }
        }
        if let Some(i) = self.voices.iter().position(|v| v.state == VState::Free) {
            return i;
        }
        // Every fade slot is busy too: reuse the quietest fading voice.
        let mut best = 0;
        let mut best_l = f32::INFINITY;
        for (i, v) in self.voices.iter().enumerate() {
            let l = v.loudness();
            if l < best_l {
                best_l = l;
                best = i;
            }
        }
        best
    }

    /// Starts one voice of region `ri`.
    #[allow(clippy::too_many_arguments)]
    fn start_voice(
        &mut self,
        ri: usize,
        note_id: u64,
        pitch: u8,
        amp_vel: f32,
        brightness: f32,
        gain: f32,
        one_shot: bool,
    ) {
        let sr = self.sample_rate;
        let vc = self.voicing;
        let p = self.prepared[ri].clone();
        let semis = (pitch as f32 - p.keycenter) * p.keytrack + p.shift;
        let step = 2f64.powf(semis as f64 / 12.0) * (p.sample_rate as f64 / sr as f64);
        if !(step.is_finite() && step > 0.0) {
            return;
        }
        let vt = (p.veltrack * vc.veltrack_scale).clamp(-1.0, 1.0);
        let curve = amp_vel * amp_vel;
        let vel_gain = if vt >= 0.0 {
            1.0 - vt + vt * curve
        } else {
            1.0 + vt * curve
        };
        let amp = gain * vel_gain * p.volume_gain * db_to_gain(vc.gain_db);
        if amp < 1e-5 {
            return;
        }
        let (gl, gr) = if p.stereo {
            ((1.0 - p.pan).min(1.0), (1.0 + p.pan).min(1.0))
        } else {
            pan_gains(p.pan)
        };
        let attack = p
            .env
            .attack
            .max(vc.min_attack_s + vc.soft_attack_s * (1.0 - amp_vel));
        let release_s = self.release_time(&p);
        let env = Env::start(
            p.env.delay,
            attack,
            p.env.hold,
            p.env.decay,
            p.env.sustain,
            release_s,
            sr,
        );
        // Tone: low-pass tracking velocity, key and brightness; never below 3x the fundamental.
        let t = vc.tone;
        let f0 = 440.0 * 2f32.powf((pitch as f32 - 69.0) / 12.0);
        let mut lp_on = false;
        let mut lp = [idle_biquad(); 2];
        if t.lp_base_hz.is_finite() {
            let cutoff = t.lp_base_hz
                * 2f32.powf(t.lp_vel_octaves * (amp_vel - 0.5))
                * 2f32.powf(t.lp_key_track * (pitch as f32 - 60.0) / 12.0)
                * 2f32.powf(t.lp_bright_octaves * brightness);
            let cutoff = cutoff.max(3.0 * f0).max(150.0);
            if cutoff < 0.45 * sr {
                let c = Coeffs::design(
                    BiquadShape::LowPass,
                    cutoff as f64,
                    t.lp_q.max(0.1) as f64,
                    sr as f64,
                );
                lp = [Biquad::new(c); 2];
                lp_on = true;
            }
        }
        let shelf_db = t.shelf_db + t.bright_shelf_db * brightness;
        let mut shelf_on = false;
        let mut shelf = [idle_biquad(); 2];
        if shelf_db.abs() > 0.05 && t.shelf_hz < 0.45 * sr {
            let c = Coeffs::design(
                BiquadShape::HighShelf {
                    gain_db: shelf_db as f64,
                },
                t.shelf_hz as f64,
                std::f64::consts::FRAC_1_SQRT_2,
                sr as f64,
            );
            shelf = [Biquad::new(c); 2];
            shelf_on = true;
        }
        let (rf_on, rf) = match p
            .filter
            .and_then(|f| filter_coeffs(f.kind, f.cutoff_hz, f.resonance_db, sr))
        {
            Some(c) => (true, [Biquad::new(c); 2]),
            None => (false, [idle_biquad(); 2]),
        };
        let start = if vc.align_onsets {
            p.aligned_offset
        } else {
            p.offset
        };
        let (looping, ls, le) = match p.loop_range {
            Some((a, b)) if start < b => (true, a, b),
            _ => (false, 0.0, 0.0),
        };
        let slot = self.alloc_voice();
        self.voices[slot] = Voice {
            state: VState::Playing,
            note_id,
            one_shot,
            sample: p.sample,
            pos: start,
            step,
            end: p.end,
            looping,
            loop_start: ls,
            loop_end: le,
            wrapped: false,
            mode: p.mode,
            env,
            release_s,
            gain_l: amp * gl,
            gain_r: amp * gr,
            width: p.width,
            stereo: p.stereo,
            fade: 1.0,
            fade_step: 0.0,
            end_fade: (vc.sample_end_fade_s * p.sample_rate).max(1.0) as f64,
            lp_on,
            lp,
            shelf_on,
            shelf,
            rf_on,
            rf,
            started: self.clock,
        };
    }

    /// Starts the voices of every region in `list` that sounds for this note.
    #[allow(clippy::too_many_arguments)]
    fn trigger_regions(
        &mut self,
        release: bool,
        note_id: u64,
        pitch: u8,
        velocity: f32,
        brightness: f32,
        seed: u64,
        held_s: f32,
        held_others: usize,
    ) {
        let v_layer = (velocity * self.voicing.layer_velocity_scale
            + brightness * self.voicing.bright_layer_boost)
            .clamp(0.0, 1.0)
            * 127.0;
        let mut rng = Pcg32::new(derive(seed, if release { 0x5E1E } else { 0xA77C }));
        let rr = rng.next_u32();
        let rand = rng.next_f32();
        let jitter = rng.range(-1.0, 1.0) * self.voicing.gain_jitter_db;
        let base_gain = db_to_gain(jitter);
        let mut chosen = [(0usize, 0f32); MAX_VOICES_PER_NOTE];
        let mut n = 0;
        {
            let list = if release {
                &self.release_by_key[pitch as usize]
            } else {
                &self.by_key[pitch as usize]
            };
            for &ri in list.iter() {
                let p = &self.prepared[ri as usize];
                match p.trigger {
                    Trigger::First if held_others > 0 => continue,
                    Trigger::Legato if held_others == 0 => continue,
                    _ => {}
                }
                if p.seq_length > 1 && (rr % p.seq_length) + 1 != p.seq_position {
                    continue;
                }
                if !(p.lorand <= rand && (rand < p.hirand || p.hirand >= 1.0)) {
                    continue;
                }
                let g = self.layer_gain(ri as usize, list, v_layer);
                if g > 1e-3 && n < MAX_VOICES_PER_NOTE {
                    chosen[n] = (ri as usize, g);
                    n += 1;
                }
            }
        }
        for &(ri, g) in &chosen[..n] {
            // rt_decay: release samples lose this many dB per second the key was held.
            let extra = if release {
                db_to_gain(-self.prepared[ri].rt_decay * held_s)
            } else {
                1.0
            };
            self.start_voice(
                ri,
                note_id,
                pitch,
                velocity,
                brightness,
                base_gain * g * extra,
                release,
            );
        }
    }

    fn recompute_voicing(&mut self) {
        if let Some(f) = self.voicing_fn {
            self.voicing = f(&self.params[..self.n_params]);
        }
    }
}

impl Instrument for SamplerInstrument {
    fn note_on(&mut self, note: &NoteOn) {
        let velocity = if note.velocity.is_finite() {
            note.velocity.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let brightness = if note.brightness.is_finite() {
            note.brightness.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let pitch = note.pitch.min(127);
        let held_others = self.held.iter().filter(|h| h.active).count();
        // Remember the note for note-off (release samples): free slot, else the oldest.
        let mut slot = 0;
        let mut oldest = u64::MAX;
        for (i, h) in self.held.iter().enumerate() {
            if !h.active {
                slot = i;
                break;
            }
            if h.started < oldest {
                oldest = h.started;
                slot = i;
            }
        }
        self.held[slot] = Held {
            active: true,
            id: note.id,
            pitch,
            velocity,
            seed: note.seed,
            started: self.clock,
        };
        self.trigger_regions(
            false,
            note.id,
            pitch,
            velocity,
            brightness,
            note.seed,
            0.0,
            held_others,
        );
    }

    fn note_off(&mut self, id: u64) {
        let sr = self.sample_rate;
        for v in self.voices.iter_mut() {
            if v.note_id == id && v.state == VState::Playing {
                v.enter_release(sr);
            }
        }
        if let Some(i) = self.held.iter().position(|h| h.active && h.id == id) {
            let h = self.held[i];
            self.held[i].active = false;
            if !self.release_by_key[h.pitch as usize].is_empty() {
                let held_s = (self.clock - h.started) as f32 / sr;
                let others = self.held.iter().filter(|h| h.active).count();
                self.trigger_regions(
                    true,
                    u64::MAX,
                    h.pitch,
                    h.velocity,
                    0.0,
                    h.seed,
                    held_s,
                    others,
                );
            }
        }
    }

    fn all_notes_off(&mut self) {
        let sr = self.sample_rate;
        for v in self.voices.iter_mut() {
            v.enter_release(sr);
        }
        for h in self.held.iter_mut() {
            h.active = false;
        }
    }

    fn set_param(&mut self, index: usize, value: f32) {
        if index < self.n_params && value.is_finite() {
            self.params[index] = value;
            self.recompute_voicing();
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        left.fill(0.0);
        right.fill(0.0);
        let program = &self.program;
        // Voices in index order, two at a time (interleaved; same sums, see `render_pair`).
        let mut pending: Option<&mut Voice> = None;
        for v in self.voices.iter_mut() {
            if v.state == VState::Free {
                continue;
            }
            let Some(a) = pending.take() else {
                pending = Some(v);
                continue;
            };
            let (da, db) = (&program.samples[a.sample], &program.samples[v.sample]);
            let (a_on, b_on) = Voice::render_pair(a, da, v, db, &mut left[..n], &mut right[..n]);
            if !a_on {
                *a = Voice::free();
            }
            if !b_on {
                *v = Voice::free();
            }
        }
        if let Some(a) = pending
            && !a.render(&program.samples[a.sample], &mut left[..n], &mut right[..n])
        {
            *a = Voice::free();
        }
        for x in left.iter_mut().chain(right.iter_mut()) {
            *x = sanitize(*x);
        }
        self.clock += n as u64;
    }

    fn tail_samples(&self) -> usize {
        let sr = self.sample_rate;
        let mut tail = 0f32;
        for p in &self.prepared {
            // Exponential release reaches -60 dB at the release time and ends at -90 dB.
            let mut t = RELEASE_TO_SILENT * self.release_time(p);
            let length_s = (p.end - p.offset) as f32 / p.sample_rate;
            if p.trigger == Trigger::Release || p.mode == LoopMode::OneShot {
                // Plays to its end, at most an octave down.
                t = t.max(2.0 * length_s);
            } else if p.mode == LoopMode::Sustain {
                t += 2.0 * length_s;
            }
            tail = tail.max(t);
        }
        ((tail + self.voicing.steal_fade_s) * sr).ceil() as usize + 2
    }

    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            *v = Voice::free();
        }
        for h in self.held.iter_mut() {
            *h = HELD_NONE;
        }
        self.clock = 0;
    }
}
