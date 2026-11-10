//! Synthetic test signals for the analyzer tests. Independent of the engine: tones are additive
//! sines with a short attack and exponential decay, clicks are short noise bursts, all
//! deterministic (PCG32 noise).

#![allow(dead_code)]

use praxis_analyze::features::loudness;
use praxis_dsp::StereoBuffer;
use praxis_dsp::rng::Pcg32;
use praxis_score::timeline::expand_notes;
use praxis_score::{
    Clip, GeneratorClass, Grid, InstrumentKind, InstrumentRef, Key, Layer, Mode, Note, Pattern,
    PitchClass, Playback, Score,
};
use std::f64::consts::PI;

pub const SR: u32 = 48_000;

/// Additive tone with harmonics `1/k^1.5`, 2 ms attack, exponential decay with time constant
/// `decay_s`, ended by a 30 ms cosine release at `len_s`. Writes circularly when `wrap`.
pub fn add_tone(
    buf: &mut [f32],
    sr: u32,
    start_s: f64,
    hz: f64,
    amp: f32,
    decay_s: f64,
    len_s: f64,
    wrap: bool,
) {
    let n = buf.len();
    if n == 0 {
        return;
    }
    let sr_f = sr as f64;
    let start = (start_s * sr_f).round() as i64;
    let attack = 0.002 * sr_f;
    let release = 0.03 * sr_f;
    let total = ((len_s * sr_f) + release).round() as i64;
    let harmonics: Vec<(f64, f64)> = (1..=6)
        .map(|k| (k as f64, 1.0 / (k as f64).powf(1.5)))
        .filter(|(k, _)| hz * k < sr_f * 0.45)
        .collect();
    let norm: f64 = harmonics.iter().map(|(_, a)| a).sum();
    for i in 0..total {
        let idx = start + i;
        let idx = if wrap {
            idx.rem_euclid(n as i64) as usize
        } else if idx < 0 || idx >= n as i64 {
            continue;
        } else {
            idx as usize
        };
        let t = i as f64 / sr_f;
        let mut env = (-t / decay_s).exp();
        if (i as f64) < attack {
            env *= i as f64 / attack;
        }
        let rel_start = len_s * sr_f;
        if i as f64 > rel_start {
            let r = ((i as f64 - rel_start) / release).min(1.0);
            env *= 0.5 * (1.0 + (PI * r).cos());
        }
        let mut s = 0.0;
        for (k, a) in &harmonics {
            s += a * (2.0 * PI * hz * k * t).sin();
        }
        buf[idx] += (amp as f64 * env * s / norm) as f32;
    }
}

/// Short click: a 4 ms burst of noise with a 1 ms exponential decay (first sample is the peak).
pub fn add_click(buf: &mut [f32], sr: u32, start_s: f64, amp: f32, seed: u64) {
    let mut rng = Pcg32::new(seed);
    let start = (start_s * sr as f64).round() as usize;
    let len = (0.004 * sr as f64) as usize;
    for i in 0..len {
        if let Some(x) = buf.get_mut(start + i) {
            let env = (-(i as f64) / (0.001 * sr as f64)).exp();
            *x += amp * env as f32 * rng.next_bipolar();
        }
    }
}

/// Reverb-like tail: low-passed noise with an exponential decay (time constant `decay_s`)
/// lasting `len_s`, starting at `start_s`. With `wrap` the part past the end continues at the
/// start (a loop); without it the tail is cut at the end of the buffer.
pub fn add_noise_tail(
    buf: &mut [f32],
    sr: u32,
    start_s: f64,
    amp: f32,
    decay_s: f64,
    len_s: f64,
    wrap: bool,
    seed: u64,
) {
    let n = buf.len();
    let mut rng = Pcg32::new(seed);
    let start = (start_s * sr as f64).round() as usize;
    let len = (len_s * sr as f64) as usize;
    let attack = (0.005 * sr as f64) as usize;
    let mut lp = 0.0f32;
    for i in 0..len {
        let idx = start + i;
        let idx = if wrap {
            idx % n
        } else if idx < n {
            idx
        } else {
            break;
        };
        lp += 0.3 * (rng.next_bipolar() - lp);
        let mut env = (-(i as f64) / (decay_s * sr as f64)).exp() as f32;
        if i < attack {
            env *= i as f32 / attack as f32;
        }
        buf[idx] += amp * env * lp;
    }
}

/// Prints every check of a verification report.
pub fn print_report(name: &str, r: &praxis_analyze::verify::VerifyReport) {
    println!("== {name}: passed {}", r.passed());
    for c in &r.checks {
        println!(
            "   {:<9} {:<5} measured: {} | expected: {}",
            c.name, c.passed, c.measured, c.expected
        );
    }
}

/// The check named `name`.
pub fn check<'a>(
    r: &'a praxis_analyze::verify::VerifyReport,
    name: &str,
) -> &'a praxis_analyze::verify::Check {
    r.checks
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no check {name}"))
}

pub fn midi_hz(p: f64) -> f64 {
    440.0 * 2f64.powf((p - 69.0) / 12.0)
}

pub fn stereo(mono: Vec<f32>) -> StereoBuffer {
    StereoBuffer {
        left: mono.clone(),
        right: mono,
    }
}

/// Scales `buf` so its integrated loudness is `target` LUFS. Returns the gain applied, dB.
pub fn normalize_lufs(buf: &mut StereoBuffer, sr: u32, target: f64) -> f64 {
    let l = loudness::measure(&buf.left, &buf.right, sr).expect("meter");
    let g_db = target - l.integrated_lufs;
    let g = 10f64.powf(g_db / 20.0) as f32;
    for x in buf.left.iter_mut().chain(buf.right.iter_mut()) {
        *x *= g;
    }
    g_db
}

pub fn key(tonic: PitchClass, mode: Mode) -> Key {
    Key { tonic, mode }
}

pub fn note(step: u32, pitch: u8, velocity: f32, length: f32) -> Note {
    Note {
        step,
        pitch,
        velocity,
        accent: Default::default(),
        length,
        offset: 0.0,
    }
}

/// One layer playing `pattern` over all bars.
pub fn layer(id: &str, class: GeneratorClass, pattern: &str, bars: u32) -> Layer {
    Layer {
        id: id.to_string(),
        name: String::new(),
        class,
        instrument: InstrumentRef {
            kind: InstrumentKind::Synth,
            preset: "test_tone".to_string(),
            params: Default::default(),
        },
        clips: vec![Clip {
            bars: praxis_score::BarRange {
                start: 1,
                end: bars,
            },
            pattern: pattern.to_string(),
            pattern_offset_steps: 0,
            transpose: 0,
            velocity_scale: 1.0,
        }],
        locked: false,
        muted: false,
        solo: false,
        gain_db: 0.0,
        pan: 0.0,
        effects: vec![],
        automation: vec![],
        seed_offset: 0,
    }
}

/// A 4/4 score with one pattern (`grid`, `length_steps`) on one layer of `class`.
pub fn score(
    bpm: f64,
    bars: u32,
    key: Key,
    grid: Grid,
    length_steps: u32,
    notes: Vec<Note>,
    class: GeneratorClass,
    playback: Playback,
) -> Score {
    let mut s = Score::empty(bars, bpm);
    s.key = key;
    s.playback = playback;
    s.patterns.push(Pattern {
        id: "p".to_string(),
        grid,
        length_steps,
        notes,
        spoken: None,
    });
    s.layers.push(layer("l", class, "p", bars));
    s
}

/// Eighth-note motif of one bar with accents: downbeat 1.0, other beats 0.75, off-beats 0.45.
/// Pitches cycle through `pitches`.
pub fn accented_eighths(pitches: &[u8]) -> Vec<Note> {
    (0..8)
        .map(|s| {
            let v = if s == 0 {
                1.0
            } else if s % 2 == 0 {
                0.75
            } else {
                0.45
            };
            note(s, pitches[s as usize % pitches.len()], v, 1.5)
        })
        .collect()
}

/// Options for [`render`].
#[derive(Clone, Debug)]
pub struct RenderOpts {
    /// Tone decay time constant, seconds.
    pub decay_s: f64,
    /// Seconds added to every note start (timing error).
    pub shift_s: f64,
    /// Extra per-note timing error, alternating sign by note index, seconds.
    pub jitter_s: f64,
    /// Decay of drum and percussion layers, seconds.
    pub drum_decay_s: f64,
    /// Drum and percussion layers are rendered as clicks instead of tones.
    pub clicks: bool,
    /// Amplitude of a noise "reverb" tail after every note (0 = none), relative to the note.
    pub reverb: f32,
    /// Loop: tails wrap from the end into the start. One-shot: `tail_s` of tail after the bars.
    pub wrap: bool,
    pub tail_s: f64,
    pub gain: f32,
}

impl Default for RenderOpts {
    fn default() -> Self {
        RenderOpts {
            decay_s: 0.25,
            shift_s: 0.0,
            jitter_s: 0.0,
            drum_decay_s: 0.12,
            clicks: false,
            reverb: 0.0,
            wrap: true,
            tail_s: 0.0,
            gain: 0.3,
        }
    }
}

/// Mini renderer for tests: every note event of `score` becomes an additive tone (or a click
/// when `opts.clicks`), velocity sets the amplitude, the note length (+ ring) sets the duration.
pub fn render(score: &Score, sr: u32, opts: &RenderOpts) -> StereoBuffer {
    let bars_len = praxis_score::timeline::score_length_samples(score, sr) as usize;
    let len = bars_len + (opts.tail_s * sr as f64) as usize;
    let mut buf = vec![0.0f32; len];
    for (i, e) in expand_notes(score, sr).iter().enumerate() {
        let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
        let t = e.start_s + opts.shift_s + sign * opts.jitter_s;
        let drum = matches!(
            score.layers[e.layer].class,
            GeneratorClass::Drums | GeneratorClass::Percussion
        );
        let decay = if drum {
            opts.drum_decay_s
        } else {
            opts.decay_s
        };
        if opts.clicks && drum {
            add_click(&mut buf, sr, t, opts.gain * e.velocity, e.key);
        } else {
            add_tone(
                &mut buf,
                sr,
                t,
                midi_hz(e.pitch as f64),
                opts.gain * e.velocity,
                decay,
                (e.length_s * 1.5).max(0.05),
                opts.wrap,
            );
        }
        if opts.reverb > 0.0 && !drum {
            add_noise_tail(
                &mut buf,
                sr,
                t,
                opts.reverb * opts.gain * e.velocity,
                0.35,
                1.5,
                opts.wrap,
                e.key,
            );
        }
    }
    stereo(buf)
}
