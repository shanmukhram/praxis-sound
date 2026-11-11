//! Profile to patch for sound effects and ambience (FR-14, FR-16).
//!
//! A sound effect becomes one layer of the matching procedural family (`whoosh`, `impact`,
//! `riser`, `drone`, `glitch`, `tick`) with its four macros set from the profile:
//!
//! | Macro | From |
//! | --- | --- |
//! | `length` | the sounding duration (the riser's in beats of the tempo; the drone's attack) |
//! | `brightness` | the spectral centroid |
//! | `weight` | the share of energy at or below 125 Hz |
//! | `pitch` | 0.5: the note itself carries the reference's median pitch, snapped to the key |
//!
//! plus a room (reverb decay and mix). Ticks and glitches repeat on the reference's onsets, so a
//! clock stays a clock. Ambience becomes a held bed (`noise_bed`, `granular_texture` or `drone`)
//! with tone, air, room and width. Which family is right is decided by listening: every family is
//! rendered and measured, and the closest is refined (analysis by synthesis).

use crate::character::{Character, exp_map, log_unit};
use crate::error::MatchError;
use crate::music::effect;
use praxis_instruments::sfx::{drone, glitch, impact, riser, tick, whoosh};
use praxis_score::{
    BarRange, Clip, EffectKind, GeneratorClass, Grid, InstrumentKind, InstrumentRef, Key, Layer,
    Mode, Note, Pattern, Playback, Score, Tempo, TimeSignature,
};
use std::collections::BTreeMap;

/// The procedural families, in the order they are tried.
pub const FAMILIES: &[&str] = &["whoosh", "impact", "riser", "drone", "glitch", "tick"];

/// Beds for ambience: (preset, kind, class).
pub const BEDS: &[(&str, InstrumentKind, GeneratorClass)] = &[
    ("noise_bed", InstrumentKind::Synth, GeneratorClass::Texture),
    ("granular_texture", InstrumentKind::Synth, GeneratorClass::Pad),
    ("drone", InstrumentKind::Sfx, GeneratorClass::Drone),
];

/// Continuous parameters of a sound effect, each `0..1`.
pub const SFX_DIMS: &[&str] = &[
    "length",
    "brightness",
    "weight",
    "pitch",
    "reverb_mix",
    "reverb_decay",
];

/// Continuous parameters of an ambience bed, each `0..1`.
pub const BED_DIMS: &[&str] = &["tone", "air", "reverb_mix", "reverb_decay", "width"];

/// Fixed facts of one effect or ambience match.
#[derive(Clone, Debug, PartialEq)]
pub struct SfxSetup {
    pub key: Key,
    pub bpm: f64,
    /// MIDI note the family plays.
    pub note: u8,
    /// Onset times to repeat on (ticks and glitches), seconds from the start.
    pub hits_s: Vec<f64>,
    /// Sounding length of the reference, seconds.
    pub active_s: f64,
    pub target_lufs: f64,
    pub brief: Option<String>,
    pub references: Vec<String>,
    pub seed: u64,
}

impl SfxSetup {
    pub fn from_character(
        c: &Character,
        key: Option<Key>,
        bpm: Option<f64>,
        references: Vec<String>,
        brief: Option<String>,
        seed: u64,
    ) -> SfxSetup {
        let key = key.unwrap_or_else(|| c.key_or_default());
        let bpm = bpm.or(c.bpm).unwrap_or(120.0).clamp(40.0, 220.0);
        let note = match c.median_midi {
            Some(m) => key.snap(m.round().clamp(33.0, 84.0) as u8),
            None => {
                // The key's tonic in octave 3 (A3 for A minor).
                let t = key.tonic.semitone() as i32;
                (48 + t + if t > 9 { -12 } else { 0 }) as u8
            }
        };
        SfxSetup {
            key,
            bpm,
            note,
            hits_s: c.onsets_s.iter().copied().filter(|t| *t >= 0.0).collect(),
            active_s: c.active_s.max(0.02),
            target_lufs: c.lufs,
            brief,
            references,
            seed,
        }
    }

    fn beat_s(&self) -> f64 {
        60.0 / self.bpm
    }
}

/// The starting macros for a family, from the character.
pub fn first_params(family: &str, c: &Character, setup: &SfxSetup) -> Vec<f64> {
    let mut x = vec![0.5; SFX_DIMS.len()];
    let d = setup.active_s;
    x[0] = match family {
        "whoosh" => inv_exp(d, whoosh::LENGTH_S),
        "impact" => inv_exp(d, impact::LENGTH_S),
        "glitch" => inv_exp(d.min(2.0), glitch::LENGTH_S),
        "tick" => inv_exp(single_tick_s(setup), tick::LENGTH_S),
        "riser" => inv_exp(d / setup.beat_s(), riser::LENGTH_BEATS),
        "drone" => inv_exp((c.attack_s as f64).max(0.25), drone::LENGTH_S),
        _ => 0.5,
    };
    x[1] = c.brightness();
    x[2] = (c.low_share as f64 * 2.5).clamp(0.0, 1.0);
    x[3] = 0.5;
    let rt = c.rt60_s.unwrap_or(0.6) as f64;
    x[4] = if c.rt60_s.is_some() {
        (rt / 6.0).sqrt().clamp(0.05, 0.8)
    } else {
        0.1
    };
    x[5] = log_unit(rt, 0.3, 6.0);
    x
}

/// The starting point for an ambience bed.
pub fn first_bed_params(c: &Character) -> Vec<f64> {
    let rt = c.rt60_s.unwrap_or(1.5) as f64;
    vec![
        c.brightness(),
        0.8,
        (rt / 8.0).sqrt().clamp(0.1, 0.8),
        log_unit(rt, 0.3, 8.0),
        (c.width as f64 / 0.6).sqrt().clamp(0.0, 1.0),
    ]
}

fn inv_exp(v: f64, range: (f64, f64)) -> f64 {
    log_unit(v.max(range.0), range.0, range.1)
}

/// The length of one tick: the reference's shortest gap between onsets, or its whole length.
fn single_tick_s(setup: &SfxSetup) -> f64 {
    let gaps: Vec<f64> = setup
        .hits_s
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|g| *g > 0.005)
        .collect();
    let gap = gaps.iter().copied().fold(f64::INFINITY, f64::min);
    if gap.is_finite() {
        (gap * 0.6).min(setup.active_s)
    } else {
        setup.active_s
    }
}

/// Builds the Score of one family at a parameter point.
pub fn score(family: &str, setup: &SfxSetup, x: &[f64]) -> Result<Score, MatchError> {
    if !FAMILIES.contains(&family) {
        return Err(MatchError::Invalid(format!("unknown family `{family}`")));
    }
    let ts = TimeSignature {
        numerator: 4,
        denominator: 4,
    };
    let beat = setup.beat_s();
    let bar_s = 4.0 * beat;
    let mut params = BTreeMap::new();
    for (i, name) in ["length", "brightness", "weight", "pitch"].iter().enumerate() {
        params.insert(name.to_string(), x[i].clamp(0.0, 1.0));
    }
    let velocity = 0.8;
    let mut notes = Vec::new();
    let mut tail_s = 1.0;
    let (grid, sounding_s) = match family {
        "riser" => {
            let beats = exp_map(x[0], riser::LENGTH_BEATS.0, riser::LENGTH_BEATS.1);
            let steps = (beats * 2.0).round().clamp(1.0, 64.0) as f32;
            notes.push(note(0, setup.note, velocity, steps));
            tail_s = 0.5;
            (Grid::Eighth, beats * beat)
        }
        "drone" => {
            // Held for the reference's length minus its release, on a quarter-note grid.
            let release = exp_map(x[0], drone::LENGTH_S.0, drone::LENGTH_S.1);
            let held = (setup.active_s - release).max(setup.active_s * 0.5).max(beat);
            let steps = (held / beat).round().clamp(1.0, 64.0) as f32;
            notes.push(note(0, setup.note, velocity, steps));
            tail_s = release + 0.5;
            (Grid::Quarter, steps as f64 * beat)
        }
        "tick" | "glitch" if setup.hits_s.len() >= 2 => {
            // Repeat on the reference's onsets (sixteenth grid), first onset at step 0.
            let step_s = beat / 4.0;
            let t0 = setup.hits_s[0];
            let mut last = None;
            for t in setup.hits_s.iter().take(64) {
                let s = ((t - t0) / step_s).round().max(0.0) as u32;
                if last == Some(s) {
                    continue;
                }
                last = Some(s);
                notes.push(note(s, setup.note, velocity, 1.0));
            }
            let span = last.unwrap_or(0) as f64 * step_s;
            (Grid::Sixteenth, span + one_shot_s(family, x[0]))
        }
        _ => {
            notes.push(note(0, setup.note, velocity, 1.0));
            (Grid::Eighth, one_shot_s(family, x[0]))
        }
    };
    let bars = ((sounding_s / bar_s).ceil() as u32).clamp(1, 64);
    let steps_per_bar = ts.steps_per_bar(grid).unwrap_or(8);
    let length_steps = steps_per_bar * bars;
    notes.retain(|n| n.step < length_steps);
    let class = match family {
        "whoosh" => GeneratorClass::Whoosh,
        "impact" => GeneratorClass::Impact,
        "riser" => GeneratorClass::Riser,
        "drone" => GeneratorClass::Drone,
        "glitch" => GeneratorClass::Glitch,
        _ => GeneratorClass::Tick,
    };
    let layer = Layer {
        id: family.to_string(),
        name: family.to_string(),
        class,
        instrument: InstrumentRef {
            kind: InstrumentKind::Sfx,
            preset: family.to_string(),
            params,
        },
        clips: vec![Clip {
            bars: BarRange {
                start: 1,
                end: bars,
            },
            pattern: "hit".into(),
            pattern_offset_steps: 0,
            transpose: 0,
            velocity_scale: 1.0,
        }],
        locked: false,
        muted: false,
        solo: false,
        gain_db: 0.0,
        pan: 0.0,
        effects: vec![room(x[4], x[5], 6.0)],
        automation: vec![],
        seed_offset: 0,
    };
    let pattern = Pattern {
        id: "hit".into(),
        grid,
        length_steps,
        notes,
        spoken: None,
    };
    Ok(finish_score(
        setup,
        bars,
        ts,
        pattern,
        layer,
        Playback::OneShot {
            tail_seconds: tail_s,
        },
        family,
    ))
}

/// Seconds a one-shot family sounds at a `length` macro value.
fn one_shot_s(family: &str, length: f64) -> f64 {
    let r = match family {
        "whoosh" => whoosh::LENGTH_S,
        "impact" => impact::LENGTH_S,
        "glitch" => glitch::LENGTH_S,
        "tick" => tick::LENGTH_S,
        _ => (0.5, 2.0),
    };
    exp_map(length, r.0, r.1)
}

/// Builds an ambience bed: one note held over the whole length.
pub fn bed_score(bed: usize, setup: &SfxSetup, x: &[f64]) -> Result<Score, MatchError> {
    let (preset, kind, class) = *BEDS
        .get(bed)
        .ok_or_else(|| MatchError::Invalid(format!("unknown bed {bed}")))?;
    let ts = TimeSignature {
        numerator: 4,
        denominator: 4,
    };
    let beat = setup.beat_s();
    let bars = ((setup.active_s / (4.0 * beat)).ceil() as u32).clamp(1, 16);
    let steps = (bars * 4).min(64);
    let mut params = BTreeMap::new();
    let tone = x[0].clamp(0.0, 1.0);
    match preset {
        "drone" => {
            params.insert("brightness".into(), tone);
            params.insert("length".into(), 0.45);
        }
        _ => {
            params.insert("tone".into(), tone);
        }
    }
    let layer = Layer {
        id: "bed".into(),
        name: preset.replace('_', " "),
        class,
        instrument: InstrumentRef {
            kind,
            preset: preset.into(),
            params,
        },
        clips: vec![Clip {
            bars: BarRange {
                start: 1,
                end: bars,
            },
            pattern: "held".into(),
            pattern_offset_steps: 0,
            transpose: 0,
            velocity_scale: 1.0,
        }],
        locked: false,
        muted: false,
        solo: false,
        gain_db: 0.0,
        pan: 0.0,
        effects: vec![
            effect(
                "air",
                EffectKind::Eq,
                &[("lp_hz", exp_map(x[1], 1500.0, 22000.0))],
            ),
            room(x[2], x[3], 8.0),
            effect(
                "width",
                EffectKind::Width,
                &[("width", (2.0 * x[4]).clamp(0.0, 2.0))],
            ),
        ],
        automation: vec![],
        seed_offset: 0,
    };
    let pattern = Pattern {
        id: "held".into(),
        grid: Grid::Quarter,
        length_steps: steps,
        notes: vec![note(0, setup.note, 0.7, steps as f32)],
        spoken: None,
    };
    Ok(finish_score(
        setup,
        bars,
        ts,
        pattern,
        layer,
        Playback::OneShot { tail_seconds: 2.0 },
        preset,
    ))
}

fn note(step: u32, pitch: u8, velocity: f32, length: f32) -> Note {
    Note {
        step,
        pitch,
        velocity,
        accent: Default::default(),
        length,
        offset: 0.0,
    }
}

fn room(mix: f64, decay: f64, max_decay: f64) -> praxis_score::Effect {
    let decay_s = exp_map(decay, 0.3, max_decay);
    effect(
        "room",
        EffectKind::Reverb,
        &[
            ("decay_s", decay_s),
            ("predelay_ms", 8.0),
            ("size", (0.15 + 0.1 * decay_s).clamp(0.1, 1.0)),
            ("mix", 0.6 * mix.clamp(0.0, 1.0)),
        ],
    )
}

fn finish_score(
    setup: &SfxSetup,
    bars: u32,
    ts: TimeSignature,
    pattern: Pattern,
    layer: Layer,
    playback: Playback,
    what: &str,
) -> Score {
    let mut s = Score::empty(bars, setup.bpm);
    s.tempo = Tempo { bpm: setup.bpm };
    s.time_signature = ts;
    s.key = if setup.key.mode == Mode::Chromatic {
        Key {
            tonic: setup.key.tonic,
            mode: Mode::Minor,
        }
    } else {
        setup.key
    };
    s.seed = setup.seed;
    s.title = format!("A {} made like the reference", what.replace('_', " "));
    s.playback = playback;
    s.patterns = vec![pattern];
    s.layers = vec![layer];
    s.master.loudness.target_lufs = setup.target_lufs.clamp(-30.0, -9.0);
    s.master.loudness.true_peak_dbtp = -1.0;
    s.master.loudness.normalize = true;
    s.meta.brief = setup.brief.clone();
    s.meta.reference_profiles = setup.references.clone();
    s.meta.tags.push("matched".into());
    s.meta
        .provenance
        .insert("matcher".into(), crate::MATCHER_VERSION.into());
    s.meta.provenance.insert("family".into(), what.to_string());
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::PitchClass;

    fn setup(hits: Vec<f64>) -> SfxSetup {
        SfxSetup {
            key: Key {
                tonic: PitchClass::A,
                mode: Mode::Minor,
            },
            bpm: 120.0,
            note: 57,
            hits_s: hits,
            active_s: 1.5,
            target_lufs: -18.0,
            brief: None,
            references: vec!["r".into()],
            seed: 1,
        }
    }

    #[test]
    fn every_family_and_bed_builds_a_valid_score() {
        let s = setup(vec![0.0, 0.5, 1.0]);
        for f in FAMILIES {
            let score = score(f, &s, &[0.5; 6]).unwrap();
            let r = praxis_score::validate::validate(&score);
            assert!(r.is_valid(), "{f}: {:?}", r.errors);
        }
        for b in 0..BEDS.len() {
            let score = bed_score(b, &s, &[0.5; 5]).unwrap();
            let r = praxis_score::validate::validate(&score);
            assert!(r.is_valid(), "{b}: {:?}", r.errors);
        }
    }

    #[test]
    fn ticks_repeat_on_the_onsets() {
        let s = setup(vec![0.25, 0.75, 1.25, 1.75]);
        let score = score("tick", &s, &[0.5; 6]).unwrap();
        let steps: Vec<u32> = score.patterns[0].notes.iter().map(|n| n.step).collect();
        // 0.5 s apart at 120 BPM = 4 sixteenths.
        assert_eq!(steps, vec![0, 4, 8, 12]);
    }

    #[test]
    fn riser_length_follows_the_macro() {
        let s = setup(vec![]);
        let short = score("riser", &s, &[0.0, 0.5, 0.5, 0.5, 0.1, 0.5]).unwrap();
        let long = score("riser", &s, &[1.0, 0.5, 0.5, 0.5, 0.1, 0.5]).unwrap();
        assert_eq!(short.patterns[0].notes[0].length, 2.0);
        assert_eq!(long.patterns[0].notes[0].length, 32.0);
        assert_eq!(long.bars, 4);
    }

    #[test]
    fn length_inverse_maps_back() {
        for d in [0.3, 1.0, 3.0] {
            let x = inv_exp(d, whoosh::LENGTH_S);
            assert!((one_shot_s("whoosh", x) - d).abs() < 1e-9);
        }
    }
}
