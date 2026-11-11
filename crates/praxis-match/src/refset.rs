//! The reference test set for the Phase 6 exit (no downloads: the owner buys nothing).
//!
//! Ten references rendered by the engine from Scores written here by hand, with settings the
//! matcher never sees: their own melodies and voicings, tones, delays, choruses, saturation,
//! filters, pans and rooms. The matcher only receives each render's Sound Profile. Five are music
//! in different styles, four are sound effects (whoosh, impact, riser, drone) and one is an
//! ambience bed.

use praxis_analyze::profile::ClipKind;
use praxis_score::{
    BarRange, Clip, Effect, EffectKind, GeneratorClass, Grid, InstrumentKind, InstrumentRef, Key,
    Layer, Mode, Note, Pattern, PitchClass, Playback, Score, Tempo, TimeSignature,
};
use std::collections::BTreeMap;

/// One reference of the set.
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    /// File-name stem, e.g. `lullaby_felt`.
    pub name: &'static str,
    /// What the clip is (the label the classification is checked against).
    pub kind: ClipKind,
    /// One line for the listening review.
    pub description: &'static str,
    pub score: Score,
}

/// The whole set, in a fixed order.
pub fn references() -> Vec<Reference> {
    vec![
        lullaby_felt(),
        night_groove(),
        bell_arp(),
        mallet_bounce(),
        dark_keys(),
        whoosh_by(),
        heavy_impact(),
        bright_riser(),
        low_drone(),
        rain_bed(),
    ]
}

// ---------------------------------------------------------------------------------------------
// Builders

fn key(tonic: PitchClass, mode: Mode) -> Key {
    Key { tonic, mode }
}

fn params(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn fx(id: &str, kind: EffectKind, p: &[(&str, f64)]) -> Effect {
    Effect {
        id: id.into(),
        kind,
        params: params(p),
        bypass: false,
        sidechain: None,
        impulse: None,
    }
}

fn reverb(decay_s: f64, mix: f64) -> Effect {
    fx(
        "verb",
        EffectKind::Reverb,
        &[
            ("decay_s", decay_s),
            ("predelay_ms", 20.0),
            ("size", (0.2 + 0.1 * decay_s).min(1.0)),
            ("mix", mix),
        ],
    )
}

/// A note: (step, pitch, velocity, length).
fn n(step: u32, pitch: u8, velocity: f32, length: f32) -> Note {
    Note {
        step,
        pitch,
        velocity,
        accent: Default::default(),
        length,
        offset: 0.0,
    }
}

struct Part {
    id: &'static str,
    class: GeneratorClass,
    kind: InstrumentKind,
    preset: &'static str,
    params: &'static [(&'static str, f64)],
    grid: Grid,
    length_steps: u32,
    notes: Vec<Note>,
    gain_db: f32,
    pan: f32,
    effects: Vec<Effect>,
    /// First and last bar the part plays.
    bars: (u32, u32),
}

#[allow(clippy::too_many_arguments)]
fn score(
    title: &str,
    seed: u64,
    bpm: f64,
    key: Key,
    numerator: u8,
    bars: u32,
    playback: Playback,
    parts: Vec<Part>,
    master: Vec<Effect>,
    lufs: f64,
) -> Score {
    let mut s = Score::empty(bars, bpm);
    s.title = title.into();
    s.seed = seed;
    s.tempo = Tempo { bpm };
    s.key = key;
    s.time_signature = TimeSignature {
        numerator,
        denominator: 4,
    };
    s.playback = playback;
    s.meta.tags = vec!["reference-set".into()];
    for p in parts {
        s.patterns.push(Pattern {
            id: p.id.into(),
            grid: p.grid,
            length_steps: p.length_steps,
            notes: p.notes,
            spoken: None,
        });
        s.layers.push(Layer {
            id: p.id.into(),
            name: p.id.into(),
            class: p.class,
            instrument: InstrumentRef {
                kind: p.kind,
                preset: p.preset.into(),
                params: params(p.params),
            },
            clips: vec![Clip {
                bars: BarRange {
                    start: p.bars.0,
                    end: p.bars.1,
                },
                pattern: p.id.into(),
                pattern_offset_steps: 0,
                transpose: 0,
                velocity_scale: 1.0,
            }],
            locked: false,
            muted: false,
            solo: false,
            gain_db: p.gain_db,
            pan: p.pan,
            effects: p.effects,
            automation: vec![],
            seed_offset: 0,
        });
    }
    s.master.effects = master;
    s.master.loudness.target_lufs = lufs;
    s
}

fn one_shot(tail: f64) -> Playback {
    Playback::OneShot { tail_seconds: tail }
}

// ---------------------------------------------------------------------------------------------
// Music

/// A slow felt-piano lullaby in D major with a long, soft room.
fn lullaby_felt() -> Reference {
    // D major, two bars of eighths: D5 F#5 A5 F#5 E5 | D5 B4 A4 (a falling answer).
    let melody = vec![
        n(0, 74, 0.7, 2.5),
        n(2, 78, 0.55, 2.0),
        n(4, 81, 0.65, 1.5),
        n(5, 78, 0.5, 1.5),
        n(6, 76, 0.55, 2.0),
        n(8, 74, 0.7, 3.0),
        n(11, 71, 0.5, 1.5),
        n(12, 69, 0.6, 4.0),
    ];
    let left = vec![n(0, 50, 0.5, 6.0), n(8, 43, 0.5, 6.0)];
    Reference {
        name: "lullaby_felt",
        kind: ClipKind::Music,
        description: "a slow felt-piano lullaby, D major, 72 BPM, soft and roomy",
        score: score(
            "Reference: felt lullaby",
            11,
            72.0,
            key(PitchClass::D, Mode::Major),
            4,
            8,
            one_shot(3.0),
            vec![
                Part {
                    id: "melody",
                    class: GeneratorClass::Melody,
                    kind: InstrumentKind::Sampler,
                    preset: "felt_piano",
                    params: &[("tone", 0.35), ("felt", 0.85)],
                    grid: Grid::Eighth,
                    length_steps: 16,
                    notes: melody,
                    gain_db: 0.0,
                    pan: -0.1,
                    effects: vec![reverb(3.5, 0.35)],
                    bars: (1, 8),
                },
                Part {
                    id: "left",
                    class: GeneratorClass::Harmony,
                    kind: InstrumentKind::Sampler,
                    preset: "felt_piano",
                    params: &[("tone", 0.3), ("felt", 0.9)],
                    grid: Grid::Eighth,
                    length_steps: 16,
                    notes: left,
                    gain_db: -5.0,
                    pan: 0.15,
                    effects: vec![reverb(3.5, 0.35)],
                    bars: (1, 8),
                },
            ],
            vec![],
            -20.0,
        ),
    }
}

/// A dry, syncopated bass-and-drums groove in E minor.
fn night_groove() -> Reference {
    let bass = vec![
        n(0, 40, 0.85, 2.5),
        n(3, 40, 0.6, 1.0),
        n(6, 43, 0.75, 1.5),
        n(10, 47, 0.7, 1.0),
        n(11, 45, 0.6, 1.5),
        n(14, 43, 0.65, 1.0),
    ];
    let kick = vec![n(0, 36, 0.9, 1.0), n(6, 36, 0.7, 1.0), n(10, 36, 0.8, 1.0)];
    let hat: Vec<Note> = (0..8)
        .map(|i| n(i * 2 + 1, 42, if i % 2 == 0 { 0.5 } else { 0.35 }, 1.0))
        .collect();
    Reference {
        name: "night_groove",
        kind: ClipKind::Music,
        description: "a dry, syncopated bass, kick and hat groove, E minor, 118 BPM",
        score: score(
            "Reference: night groove",
            12,
            118.0,
            key(PitchClass::E, Mode::Minor),
            4,
            8,
            one_shot(1.0),
            vec![
                Part {
                    id: "bass",
                    class: GeneratorClass::Bass,
                    kind: InstrumentKind::Synth,
                    preset: "warm_bass",
                    params: &[("tone", 0.55)],
                    grid: Grid::Sixteenth,
                    length_steps: 16,
                    notes: bass,
                    gain_db: 0.0,
                    pan: 0.0,
                    effects: vec![fx(
                        "drive",
                        EffectKind::Saturation,
                        &[("drive_db", 8.0), ("mix", 0.6)],
                    )],
                    bars: (1, 8),
                },
                Part {
                    id: "kick",
                    class: GeneratorClass::Drums,
                    kind: InstrumentKind::Drum,
                    preset: "kick",
                    params: &[],
                    grid: Grid::Sixteenth,
                    length_steps: 16,
                    notes: kick,
                    gain_db: -2.0,
                    pan: 0.0,
                    effects: vec![],
                    bars: (1, 8),
                },
                Part {
                    id: "hat",
                    class: GeneratorClass::Drums,
                    kind: InstrumentKind::Drum,
                    preset: "hat_closed",
                    params: &[("tone", 0.7)],
                    grid: Grid::Sixteenth,
                    length_steps: 16,
                    notes: hat,
                    gain_db: -8.0,
                    pan: 0.25,
                    effects: vec![],
                    bars: (1, 8),
                },
            ],
            vec![],
            -14.0,
        ),
    }
}

/// A bright bell arpeggio over a sub, F# minor, with a dotted delay.
fn bell_arp() -> Reference {
    // F#5 A5 C#6 E6 C#6 A5 ... over one bar of sixteenths, then a variation.
    let arp_pitches = [
        78u8, 81, 85, 88, 85, 81, 78, 81, 76, 80, 83, 88, 83, 80, 76, 73,
    ];
    let arp: Vec<Note> = arp_pitches
        .iter()
        .enumerate()
        .map(|(i, p)| n(i as u32, *p, if i % 4 == 0 { 0.75 } else { 0.5 }, 1.5))
        .collect();
    let sub = vec![n(0, 30, 0.8, 14.0)];
    Reference {
        name: "bell_arp",
        kind: ClipKind::Music,
        description: "a bright bell arpeggio over a sub bass, F# minor, 96 BPM, with a dotted delay",
        score: score(
            "Reference: bell arpeggio",
            13,
            96.0,
            key(PitchClass::Fs, Mode::Minor),
            4,
            8,
            one_shot(3.0),
            vec![
                Part {
                    id: "arp",
                    class: GeneratorClass::Arp,
                    kind: InstrumentKind::Synth,
                    preset: "bell",
                    params: &[("tone", 0.7)],
                    grid: Grid::Sixteenth,
                    length_steps: 16,
                    notes: arp,
                    gain_db: 0.0,
                    pan: 0.2,
                    effects: vec![
                        fx(
                            "echo",
                            EffectKind::Delay,
                            &[("time_beats", 0.75), ("feedback", 0.4), ("mix", 0.25)],
                        ),
                        reverb(2.5, 0.3),
                    ],
                    bars: (1, 8),
                },
                Part {
                    id: "sub",
                    class: GeneratorClass::Bass,
                    kind: InstrumentKind::Synth,
                    preset: "sub_bass",
                    params: &[],
                    grid: Grid::Sixteenth,
                    length_steps: 16,
                    notes: sub,
                    gain_db: -14.0,
                    pan: 0.0,
                    effects: vec![],
                    bars: (1, 8),
                },
            ],
            vec![],
            -16.0,
        ),
    }
}

/// A bouncy, swung mallet tune with a soft kick, Bb major.
fn mallet_bounce() -> Reference {
    let mut tune = vec![
        n(0, 70, 0.8, 1.2),
        n(1, 74, 0.5, 1.2),
        n(2, 77, 0.65, 1.2),
        n(3, 74, 0.5, 1.2),
        n(4, 75, 0.7, 1.2),
        n(6, 74, 0.6, 1.2),
        n(7, 72, 0.5, 1.2),
        n(8, 70, 0.8, 1.2),
        n(10, 67, 0.6, 1.2),
        n(11, 69, 0.5, 1.2),
        n(12, 70, 0.7, 2.5),
    ];
    for note in &mut tune {
        if note.step % 2 == 1 {
            note.offset = 0.17;
        }
    }
    let kick: Vec<Note> = (0..4).map(|i| n(i * 4, 36, 0.7, 1.0)).collect();
    Reference {
        name: "mallet_bounce",
        kind: ClipKind::Music,
        description: "a bouncy, swung mallet tune over a soft kick, Bb major, 104 BPM",
        score: score(
            "Reference: mallet bounce",
            14,
            104.0,
            key(PitchClass::Bb, Mode::Major),
            4,
            8,
            one_shot(2.0),
            vec![
                Part {
                    id: "tune",
                    class: GeneratorClass::Motif,
                    kind: InstrumentKind::Synth,
                    preset: "mallet",
                    params: &[("tone", 0.55)],
                    grid: Grid::Eighth,
                    length_steps: 16,
                    notes: tune,
                    gain_db: 0.0,
                    pan: -0.2,
                    effects: vec![
                        fx(
                            "chorus",
                            EffectKind::Chorus,
                            &[("rate_hz", 0.6), ("depth_ms", 2.0), ("mix", 0.25)],
                        ),
                        reverb(1.2, 0.18),
                    ],
                    bars: (1, 8),
                },
                Part {
                    id: "kick",
                    class: GeneratorClass::Drums,
                    kind: InstrumentKind::Drum,
                    preset: "soft_kick",
                    params: &[],
                    grid: Grid::Sixteenth,
                    length_steps: 16,
                    notes: kick,
                    gain_db: -3.0,
                    pan: 0.0,
                    effects: vec![],
                    bars: (1, 8),
                },
            ],
            vec![],
            -15.0,
        ),
    }
}

/// Slow, dark soft-keys chords in C minor, filtered, in a large room.
fn dark_keys() -> Reference {
    // Cm | Ab | Eb | Bb over half notes, low voicings.
    let chords: [[u8; 3]; 4] = [[48, 51, 55], [44, 48, 51], [46, 51, 55], [46, 50, 53]];
    let mut notes = Vec::new();
    for (bar, c) in chords.iter().enumerate() {
        for half in 0..2 {
            for p in c {
                let step = bar as u32 * 8 + half * 4;
                notes.push(n(step, *p, if half == 0 { 0.6 } else { 0.45 }, 4.5));
            }
        }
    }
    let sub: Vec<Note> = [36u8, 32, 39, 34]
        .iter()
        .enumerate()
        .map(|(i, p)| n(i as u32 * 8, *p, 0.6, 7.5))
        .collect();
    Reference {
        name: "dark_keys",
        kind: ClipKind::Music,
        description: "slow, dark soft-keys chords and a sub, C minor, 64 BPM, filtered, big room",
        score: score(
            "Reference: dark keys",
            15,
            64.0,
            key(PitchClass::C, Mode::Minor),
            4,
            8,
            one_shot(5.0),
            vec![
                Part {
                    id: "keys",
                    class: GeneratorClass::Chords,
                    kind: InstrumentKind::Synth,
                    preset: "soft_keys",
                    params: &[("tone", 0.2)],
                    grid: Grid::Quarter,
                    length_steps: 32,
                    notes,
                    gain_db: 0.0,
                    pan: 0.0,
                    effects: vec![
                        fx("dark", EffectKind::Eq, &[("lp_hz", 2500.0)]),
                        reverb(6.0, 0.45),
                    ],
                    bars: (1, 8),
                },
                Part {
                    id: "sub",
                    class: GeneratorClass::Bass,
                    kind: InstrumentKind::Synth,
                    preset: "sub_bass",
                    params: &[],
                    grid: Grid::Quarter,
                    length_steps: 32,
                    notes: sub,
                    gain_db: -6.0,
                    pan: 0.0,
                    effects: vec![],
                    bars: (1, 8),
                },
            ],
            vec![],
            -19.0,
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// Sound effects and ambience

#[allow(clippy::too_many_arguments)]
fn effect_ref(
    name: &'static str,
    kind: ClipKind,
    description: &'static str,
    preset: &'static str,
    class: GeneratorClass,
    macros: &'static [(&'static str, f64)],
    pitch: u8,
    bpm: f64,
    bars: u32,
    grid: Grid,
    note_len: f32,
    effects: Vec<Effect>,
    tail: f64,
    lufs: f64,
) -> Reference {
    let ts = TimeSignature {
        numerator: 4,
        denominator: 4,
    };
    let steps = ts.steps_per_bar(grid).unwrap_or(8) * bars;
    Reference {
        name,
        kind,
        description,
        score: score(
            name,
            21,
            bpm,
            key(PitchClass::A, Mode::Minor),
            4,
            bars,
            one_shot(tail),
            vec![Part {
                id: "fx",
                class,
                kind: if preset == "noise_bed" {
                    InstrumentKind::Synth
                } else {
                    InstrumentKind::Sfx
                },
                preset,
                params: macros,
                grid,
                length_steps: steps,
                notes: vec![n(0, pitch, 0.8, note_len)],
                gain_db: 0.0,
                pan: 0.1,
                effects,
                bars: (1, bars),
            }],
            vec![],
            lufs,
        ),
    }
}

fn whoosh_by() -> Reference {
    effect_ref(
        "whoosh_by",
        ClipKind::Sfx,
        "a long, dark, heavy whoosh passing by (2 s)",
        "whoosh",
        GeneratorClass::Whoosh,
        &[
            ("length", 0.75),
            ("brightness", 0.3),
            ("weight", 0.7),
            ("pitch", 0.35),
        ],
        57,
        120.0,
        2,
        Grid::Eighth,
        1.0,
        vec![reverb(1.0, 0.2)],
        1.0,
        -18.0,
    )
}

fn heavy_impact() -> Reference {
    effect_ref(
        "heavy_impact",
        ClipKind::Sfx,
        "a bright, heavy hit on A with a roomy tail",
        "impact",
        GeneratorClass::Impact,
        &[
            ("length", 0.35),
            ("brightness", 0.7),
            ("weight", 0.9),
            ("pitch", 0.5),
        ],
        45,
        120.0,
        2,
        Grid::Eighth,
        1.0,
        vec![reverb(2.5, 0.3)],
        1.5,
        -16.0,
    )
}

fn bright_riser() -> Reference {
    // 6 beats at 100 BPM: length = log(6)/log(16).
    effect_ref(
        "bright_riser",
        ClipKind::Sfx,
        "a bright riser building over six beats into nothing",
        "riser",
        GeneratorClass::Riser,
        &[
            ("length", 0.646),
            ("brightness", 0.8),
            ("weight", 0.2),
            ("pitch", 0.5),
        ],
        57,
        100.0,
        2,
        Grid::Eighth,
        12.0,
        vec![],
        0.5,
        -18.0,
    )
}

fn low_drone() -> Reference {
    effect_ref(
        "low_drone",
        ClipKind::Sfx,
        "a dark, low drone on D, ten seconds",
        "drone",
        GeneratorClass::Drone,
        &[
            ("length", 0.4),
            ("brightness", 0.25),
            ("weight", 0.6),
            ("pitch", 0.5),
        ],
        50,
        120.0,
        5,
        Grid::Quarter,
        18.0,
        vec![reverb(3.0, 0.25)],
        2.0,
        -20.0,
    )
}

fn rain_bed() -> Reference {
    effect_ref(
        "rain_bed",
        ClipKind::Ambience,
        "a soft, airy noise bed like distant rain, eight seconds",
        "noise_bed",
        GeneratorClass::Texture,
        &[("tone", 0.6)],
        57,
        120.0,
        4,
        Grid::Quarter,
        16.0,
        vec![
            fx(
                "soft",
                EffectKind::Eq,
                &[("lp_hz", 6000.0), ("hp_hz", 150.0)],
            ),
            reverb(2.0, 0.3),
        ],
        2.0,
        -24.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reference_is_a_valid_score_with_a_unique_name() {
        let refs = references();
        assert_eq!(refs.len(), 10);
        let mut names: Vec<_> = refs.iter().map(|r| r.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 10);
        for r in &refs {
            let report = praxis_score::validate::validate(&r.score);
            assert!(report.is_valid(), "{}: {:?}", r.name, report.errors);
        }
    }
}
