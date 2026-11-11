//! Profile to Score for music (FR-14).
//!
//! The reference's character becomes a [`ScorePlan`] that the planner's own validation, repairs
//! and deterministic arranger expand (so every owner rule holds by construction: rhythm first,
//! notes connect, layering, restraint, in key), followed by a few settings the plan cannot
//! carry: the instruments' tone, the room, an air filter, width, swing and the loudness target.
//!
//! | Reference | Becomes |
//! | --- | --- |
//! | tempo, key | the plan's tempo and key (or the ones the user asked for) |
//! | groove (onsets folded on the grid) | the spoken rhythm of every motif; swing as micro-timing |
//! | spectral balance, attack | the lead instrument (chosen by analysis by synthesis) and its tone; a bass layer when the low end carries weight |
//! | percussive onsets, flatness | a soft kick and closed hat on the groove |
//! | reverb decay | the room's decay and mix |
//! | density (onsets per bar) | how many grid steps carry hits |
//! | sections | an intro with the lead alone, then the full layering |
//! | loudness | the master's loudness target |
//!
//! The melody is never taken from the reference: degrees come from a fixed set of contours
//! chosen by the seed (and changed when the originality guard rejects a result).

use crate::character::{Character, GrooveShape, exp_map};
use crate::error::MatchError;
use praxis_planner::Catalogue;
use praxis_planner::plan::{
    Dynamics, Motif, PlanChord, PlanLayer, PlanPlayback, PlanSection, PresetChoice, Register,
    ScorePlan, diatonic_symbol,
};
use praxis_score::{
    BarRange, Effect, EffectKind, GeneratorClass, Grid, InstrumentKind, Key, Mode, Score,
    SectionRole, TimeSignature,
};
use std::collections::BTreeMap;

/// Lead instruments the matcher tries, as (preset, kind).
pub const LEADS: &[(&str, InstrumentKind)] = &[
    ("felt_piano", InstrumentKind::Sampler),
    ("upright_piano", InstrumentKind::Sampler),
    ("soft_keys", InstrumentKind::Synth),
    ("mallet", InstrumentKind::Synth),
    ("pluck", InstrumentKind::Synth),
    ("bell", InstrumentKind::Synth),
    ("glass", InstrumentKind::Synth),
    ("fm_keys", InstrumentKind::Synth),
];

/// Bass instruments the matcher tries.
pub const BASSES: &[&str] = &["sub_bass", "warm_bass"];

/// Melodic contours (scale degrees cycled over the motif's hits). The guard moves to the next
/// one when a result is too close to the reference's melody.
pub const CONTOURS: &[&[i32]] = &[
    &[0, 2, 4, 2, 0, 4, 3, 1],
    &[4, 3, 2, 0, 2, 1, 0, -1],
    &[0, 4, 5, 4, 2, 3, 2, 0],
    &[7, 4, 5, 2, 4, 0, 1, -3],
    &[0, 1, 2, 4, 2, 1, 0, 2],
    &[2, 0, 4, 2, 5, 4, 2, 1],
    &[0, -1, 0, 2, 4, 2, 1, -1],
    &[4, 2, 0, 1, 2, 4, 5, 7],
];

/// The continuous parameters the match loop tunes, each `0..1`.
pub const DIMS: &[&str] = &[
    "tone",
    "air",
    "reverb_mix",
    "reverb_decay",
    "sustain",
    "density",
    "bass_db",
    "drums_db",
    "width",
];

/// Index of each entry of [`DIMS`].
pub mod dim {
    pub const TONE: usize = 0;
    pub const AIR: usize = 1;
    pub const REVERB_MIX: usize = 2;
    pub const REVERB_DECAY: usize = 3;
    pub const SUSTAIN: usize = 4;
    pub const DENSITY: usize = 5;
    pub const BASS_DB: usize = 6;
    pub const DRUMS_DB: usize = 7;
    pub const WIDTH: usize = 8;
}

/// Which layers a candidate has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Lead preset; `None` for a drums-only groove.
    pub lead: Option<(&'static str, InstrumentKind)>,
    pub bass: Option<&'static str>,
    pub drums: bool,
}

impl Palette {
    /// A short description, e.g. "felt piano + sub bass + kick and hat".
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some((l, _)) = self.lead {
            parts.push(l.replace('_', " "));
        }
        if let Some(b) = self.bass {
            parts.push(b.replace('_', " "));
        }
        if self.drums {
            parts.push("kick and hat".to_string());
        }
        parts.join(" + ")
    }

    /// The dimensions that do something for this palette.
    pub fn active_dims(&self) -> Vec<usize> {
        let mut d = vec![
            dim::TONE,
            dim::AIR,
            dim::REVERB_MIX,
            dim::REVERB_DECAY,
            dim::DENSITY,
            dim::WIDTH,
        ];
        if self.lead.is_some() {
            d.push(dim::SUSTAIN);
        }
        if self.bass.is_some() {
            d.push(dim::BASS_DB);
        }
        if self.drums && self.lead.is_some() {
            d.push(dim::DRUMS_DB);
        }
        d.sort_unstable();
        d
    }
}

/// Everything fixed for one music match: what the user asked for plus the reference's timing.
#[derive(Clone, Debug, PartialEq)]
pub struct Setup {
    pub key: Key,
    pub bpm: f64,
    pub ts: TimeSignature,
    pub groove: GrooveShape,
    pub emotion: String,
    /// Octave of the lead's degree 0.
    pub octave: i32,
    /// An intro with the lead alone before the full layering.
    pub intro: bool,
    pub target_lufs: f64,
    pub brief: Option<String>,
    /// Profile names written to `meta.reference_profiles`.
    pub references: Vec<String>,
    pub seed: u64,
}

/// The palette the character suggests before any listening.
pub fn first_palette(c: &Character) -> Palette {
    let percussive = c.tonality < 0.35 && c.flatness > 0.2;
    Palette {
        lead: if c.tonality < 0.15 && c.median_midi.is_none() {
            None
        } else {
            Some(lead_by_brightness(c))
        },
        bass: (c.low_share > 0.2).then_some("sub_bass"),
        drums: percussive || c.tonality < 0.15,
    }
}

/// Leads ordered by how close their nominal brightness is to the reference's.
pub fn leads_by_distance(c: &Character) -> Vec<(&'static str, InstrumentKind)> {
    let b = c.brightness();
    let mut v: Vec<_> = LEADS
        .iter()
        .map(|l| (l, (nominal_brightness(l.0) - b).abs()))
        .collect();
    v.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.0.cmp(b.0.0)));
    v.into_iter().map(|(l, _)| *l).collect()
}

fn lead_by_brightness(c: &Character) -> (&'static str, InstrumentKind) {
    leads_by_distance(c)[0]
}

/// Rough brightness (0..1 on the log centroid scale of [`Character::brightness`]) of each lead
/// at its default tone. Only an ordering hint: the match loop listens to every candidate.
fn nominal_brightness(preset: &str) -> f64 {
    match preset {
        "felt_piano" => 0.42,
        "soft_keys" => 0.45,
        "upright_piano" => 0.52,
        "fm_keys" => 0.55,
        "mallet" => 0.56,
        "pluck" => 0.6,
        "bell" => 0.68,
        "glass" => 0.72,
        _ => 0.5,
    }
}

/// The starting point of the match loop, read from the character.
pub fn first_params(c: &Character) -> Vec<f64> {
    let mut x = vec![0.5; DIMS.len()];
    x[dim::TONE] = c.brightness();
    x[dim::AIR] = 0.75;
    let rt = c.rt60_s.unwrap_or(1.2) as f64;
    x[dim::REVERB_DECAY] = crate::character::log_unit(rt, 0.3, 8.0);
    x[dim::REVERB_MIX] = if c.rt60_s.is_some() {
        (rt / 8.0).sqrt().clamp(0.1, 0.8)
    } else {
        0.25
    };
    x[dim::SUSTAIN] = crate::character::log_unit(
        (c.release_s as f64 * 4.0).max(0.2) + c.attack_s as f64 * 2.0,
        0.2,
        6.0,
    );
    // Density: a lower threshold keeps more steps. Aim for the reference's onsets per bar.
    let steps = c.groove.step_weights.len().max(1) as f64;
    let want = (c.groove.onsets_per_bar as f64 / steps).clamp(0.05, 1.0);
    x[dim::DENSITY] = (1.0 - want).clamp(0.0, 1.0);
    x[dim::WIDTH] = (c.width as f64 / 0.6).sqrt().clamp(0.0, 1.0);
    x
}

/// Threshold on the groove's step weights for a density setting.
fn threshold(x: f64) -> f32 {
    (0.12 + 0.76 * x.clamp(0.0, 1.0)) as f32
}

/// Spoken rhythm of one bar: steps whose weight reaches `thr` are hits, accented by weight.
pub fn spoken_rhythm(weights: &[f32], thr: f32) -> String {
    let mut tokens: Vec<&str> = weights
        .iter()
        .map(|w| {
            if *w < thr {
                "-"
            } else if *w >= 0.85 {
                "DON"
            } else if *w >= 0.6 {
                "don"
            } else {
                "din"
            }
        })
        .collect();
    if tokens.iter().all(|t| *t == "-") {
        // Rhythm first: never an empty motif; keep the strongest step.
        let i = weights
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1).then(b.0.cmp(&a.0)))
            .map(|(i, _)| i)
            .unwrap_or(0);
        if let Some(t) = tokens.get_mut(i) {
            *t = "DON";
        } else {
            tokens.push("DON");
        }
    }
    tokens.join(" ")
}

/// Only the strongest steps (for bass and kick).
fn strong_rhythm(weights: &[f32], thr: f32) -> String {
    spoken_rhythm(weights, thr.max(0.7))
}

/// Builds the plan for a palette and parameter point.
pub fn plan(
    setup: &Setup,
    palette: &Palette,
    x: &[f64],
    bars: u32,
    contour: usize,
) -> Result<ScorePlan, MatchError> {
    if bars == 0 {
        return Err(MatchError::Invalid("bars must be at least 1".into()));
    }
    let g = &setup.groove;
    let thr = threshold(x[dim::DENSITY]);
    let motif_bars = bars.min(2);
    let sustain = exp_map(x[dim::SUSTAIN], 1.1, 6.0) as f32;
    let mut layers = Vec::new();
    let mut lead_id = None;
    if let Some((preset, kind)) = palette.lead {
        let degrees = CONTOURS[contour % CONTOURS.len()].to_vec();
        layers.push(PlanLayer {
            id: "lead".into(),
            name: preset.replace('_', " "),
            class: GeneratorClass::Motif,
            preset: PresetChoice {
                kind,
                name: preset.into(),
            },
            register: Register::Mid,
            dynamics: Dynamics::Normal,
            gain_db: None,
            motif: Motif {
                grid: g.grid,
                length_bars: motif_bars,
                spoken: spoken_rhythm(&g.step_weights, thr),
                degrees,
                octave: Some(setup.octave),
                sustain_steps: Some(sustain),
                follow_chords: false,
                notes: vec![],
            },
        });
        lead_id = Some("lead");
    }
    if let Some(bass) = palette.bass {
        layers.push(PlanLayer {
            id: "bass".into(),
            name: bass.replace('_', " "),
            class: GeneratorClass::Bass,
            preset: PresetChoice {
                kind: InstrumentKind::Synth,
                name: bass.into(),
            },
            register: Register::Low,
            dynamics: Dynamics::Normal,
            gain_db: Some(exp_db(x[dim::BASS_DB])),
            motif: Motif {
                grid: g.grid,
                length_bars: 1,
                spoken: strong_rhythm(&g.step_weights, thr),
                degrees: vec![0, 0, 4, 0],
                octave: Some(2),
                sustain_steps: Some(1.8),
                follow_chords: true,
                notes: vec![],
            },
        });
    }
    if palette.drums {
        let drums_db = if palette.lead.is_some() {
            Some(exp_db(x[dim::DRUMS_DB]))
        } else {
            None
        };
        layers.push(drum_layer(
            "kick",
            "soft_kick",
            g.grid,
            strong_rhythm(&g.step_weights, thr),
            drums_db,
        ));
        layers.push(drum_layer(
            "hat",
            "hat_closed",
            g.grid,
            spoken_rhythm(&g.step_weights, thr * 0.8),
            drums_db.map(|d| d - 6.0),
        ));
    }
    if layers.is_empty() {
        return Err(MatchError::Invalid("a palette needs at least one layer".into()));
    }
    let all: Vec<String> = layers.iter().map(|l| l.id.clone()).collect();
    let sections = match (setup.intro && bars >= 8, lead_id) {
        (true, Some(lead)) if all.len() > 1 => {
            let intro_end = (bars / 4).max(1);
            vec![
                PlanSection {
                    name: "Intro".into(),
                    bars: BarRange {
                        start: 1,
                        end: intro_end,
                    },
                    role: SectionRole::Intro,
                    intensity: 0.4,
                    layers: vec![lead.to_string()],
                },
                PlanSection {
                    name: "Main".into(),
                    bars: BarRange {
                        start: intro_end + 1,
                        end: bars,
                    },
                    role: SectionRole::Main,
                    intensity: 0.6,
                    layers: all.clone(),
                },
            ]
        }
        _ => vec![PlanSection {
            name: "Main".into(),
            bars: BarRange {
                start: 1,
                end: bars,
            },
            role: SectionRole::Main,
            intensity: 0.5,
            layers: all.clone(),
        }],
    };
    Ok(ScorePlan {
        title: "Made like the reference".into(),
        emotion: setup.emotion.clone(),
        key: setup.key,
        time_signature: setup.ts,
        tempo_bpm: Some(setup.bpm),
        duration_s: None,
        bars: Some(bars),
        playback: PlanPlayback::OneShot,
        avoid: vec![],
        sections,
        layers,
        chords: progression(&setup.key, bars, setup.bpm),
        notes: vec![],
        requested: vec![],
    })
}

fn drum_layer(
    id: &str,
    preset: &str,
    grid: Grid,
    spoken: String,
    gain_db: Option<f32>,
) -> PlanLayer {
    PlanLayer {
        id: id.into(),
        name: preset.replace('_', " "),
        class: GeneratorClass::Drums,
        preset: PresetChoice {
            kind: InstrumentKind::Drum,
            name: preset.into(),
        },
        register: Register::Mid,
        dynamics: Dynamics::Soft,
        gain_db,
        motif: Motif {
            grid,
            length_bars: 1,
            spoken,
            degrees: vec![],
            octave: None,
            sustain_steps: None,
            follow_chords: false,
            notes: vec![],
        },
    }
}

/// `0..1` to a layer gain of -18..+6 dB (0.75 = 0 dB).
fn exp_db(x: f64) -> f32 {
    (-18.0 + 24.0 * x.clamp(0.0, 1.0)) as f32
}

/// A plain diatonic progression (i VI III VII in minor keys, I V vi IV in major ones), one chord
/// per bar, or per two bars below 90 BPM. None for pentatonic and chromatic keys.
pub fn progression(key: &Key, bars: u32, bpm: f64) -> Vec<PlanChord> {
    let degrees: &[i32] = match key.mode {
        Mode::Minor
        | Mode::HarmonicMinor
        | Mode::MelodicMinor
        | Mode::Dorian
        | Mode::Phrygian
        | Mode::Locrian => &[0, 5, 2, 6],
        Mode::Major | Mode::Lydian | Mode::Mixolydian => &[0, 4, 5, 3],
        Mode::MajorPentatonic | Mode::MinorPentatonic | Mode::Chromatic => return vec![],
    };
    let per = if bpm < 90.0 { 2 } else { 1 };
    let mut out = Vec::new();
    let mut start = 1;
    let mut i = 0;
    while start <= bars {
        let end = (start + per - 1).min(bars);
        out.push(PlanChord {
            bars: BarRange { start, end },
            symbol: diatonic_symbol(key, degrees[i % degrees.len()]),
        });
        start = end + 1;
        i += 1;
    }
    out
}

/// Composes the plan into a Score with the planner's validation, repairs and arranger, then
/// applies the settings a plan cannot carry.
pub fn score(
    setup: &Setup,
    palette: &Palette,
    x: &[f64],
    bars: u32,
    contour: usize,
    catalogue: &Catalogue,
) -> Result<Score, MatchError> {
    let p = plan(setup, palette, x, bars, contour)?;
    let value = serde_json::to_value(&p).map_err(|e| MatchError::Plan(e.to_string()))?;
    let outcome = praxis_planner::compose_plan(&value, catalogue, setup.seed)
        .map_err(|e| MatchError::Plan(e.to_string()))?;
    let mut s = outcome.score;
    finish(&mut s, setup, palette, x, catalogue);
    Ok(s)
}

/// Tone, room, air, width, swing, loudness and provenance.
fn finish(s: &mut Score, setup: &Setup, palette: &Palette, x: &[f64], catalogue: &Catalogue) {
    let tone = x[dim::TONE].clamp(0.0, 1.0);
    let decay = exp_map(x[dim::REVERB_DECAY], 0.3, 8.0);
    let mix = 0.6 * x[dim::REVERB_MIX].clamp(0.0, 1.0);
    for layer in &mut s.layers {
        let spec = catalogue.get_kind(layer.instrument.kind, &layer.instrument.preset);
        if let Some(t) = spec.and_then(|p| p.tone) {
            let is_lead = layer.id == "lead";
            // The lead follows the tone setting; the others stay near their defaults.
            let v = if is_lead {
                tone
            } else {
                0.5 * t.tone_default + 0.5 * tone
            };
            layer.instrument.params.insert("tone".into(), v);
            if t.felt_default.is_some() {
                layer
                    .instrument
                    .params
                    .insert("felt".into(), (0.95 - 0.8 * tone).clamp(0.0, 1.0));
            }
        }
        // The room is on the lead (the bass and drums stay dry, so the low end stays clean).
        if layer.id == "lead" {
            set_room(&mut layer.effects, decay, mix);
        }
    }
    // Air: a gentle low-pass on the master; width: side gain.
    s.master.effects.retain(|e| e.id != "air" && e.id != "width");
    s.master.effects.push(effect(
        "air",
        EffectKind::Eq,
        &[("lp_hz", exp_map(x[dim::AIR], 1500.0, 22000.0))],
    ));
    s.master.effects.push(effect(
        "width",
        EffectKind::Width,
        &[("width", (2.0 * x[dim::WIDTH]).clamp(0.0, 2.0))],
    ));
    apply_swing(s, setup.groove.swing);
    s.master.loudness.target_lufs = setup.target_lufs.clamp(-30.0, -9.0);
    s.master.loudness.true_peak_dbtp = -1.0;
    s.master.loudness.normalize = true;
    s.title = "Made like the reference".into();
    s.meta.brief = setup.brief.clone();
    s.meta.reference_profiles = setup.references.clone();
    s.meta.tags.push("matched".into());
    s.meta
        .provenance
        .insert("matcher".into(), crate::MATCHER_VERSION.into());
    s.meta
        .provenance
        .insert("palette".into(), palette.describe());
}

/// Sets (or adds) the layer's room reverb.
fn set_room(effects: &mut Vec<Effect>, decay_s: f64, mix: f64) {
    let size = (0.15 + 0.1 * decay_s).clamp(0.1, 1.0);
    if let Some(e) = effects.iter_mut().find(|e| e.kind == EffectKind::Reverb) {
        e.params.insert("decay_s".into(), decay_s);
        e.params.insert("mix".into(), mix);
        e.params.insert("size".into(), size);
    } else {
        effects.push(effect(
            "room",
            EffectKind::Reverb,
            &[
                ("decay_s", decay_s),
                ("predelay_ms", 12.0),
                ("size", size),
                ("mix", mix),
            ],
        ));
    }
}

pub(crate) fn effect(id: &str, kind: EffectKind, params: &[(&str, f64)]) -> Effect {
    Effect {
        id: id.into(),
        kind,
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect::<BTreeMap<_, _>>(),
        bypass: false,
        sidechain: None,
        impulse: None,
    }
}

/// Swing: off-beat steps of straight eighth and sixteenth grids are delayed by up to a third of
/// a step (full triplet swing).
fn apply_swing(s: &mut Score, swing: f32) {
    let off = (swing.clamp(0.0, 1.0) / 3.0).min(0.33);
    if off < 0.02 {
        return;
    }
    for p in &mut s.patterns {
        if !matches!(p.grid, Grid::Eighth | Grid::Sixteenth) {
            continue;
        }
        for n in &mut p.notes {
            if n.step % 2 == 1 {
                n.offset = off;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::PitchClass;

    fn setup() -> Setup {
        let ts = TimeSignature {
            numerator: 4,
            denominator: 4,
        };
        Setup {
            key: Key {
                tonic: PitchClass::D,
                mode: Mode::Minor,
            },
            bpm: 96.0,
            ts,
            groove: GrooveShape::plain(ts),
            emotion: "calm".into(),
            octave: 4,
            intro: true,
            target_lufs: -16.0,
            brief: None,
            references: vec!["ref".into()],
            seed: 3,
        }
    }

    #[test]
    fn spoken_rhythm_follows_weights() {
        let w = [1.0, 0.1, 0.65, 0.3, 0.9, 0.0, 0.5, 0.2];
        assert_eq!(spoken_rhythm(&w, 0.45), "DON - don - DON - din -");
        assert_eq!(spoken_rhythm(&w, 0.95), "DON - - - - - - -");
        assert_eq!(spoken_rhythm(&[0.0, 0.0], 0.5), "DON -");
    }

    #[test]
    fn progression_covers_every_bar() {
        let key = Key {
            tonic: PitchClass::A,
            mode: Mode::Minor,
        };
        let c = progression(&key, 7, 120.0);
        assert_eq!(c.len(), 7);
        assert_eq!(c[0].symbol, diatonic_symbol(&key, 0));
        let slow = progression(&key, 7, 70.0);
        assert_eq!(slow.len(), 4);
        assert_eq!(slow[3].bars, BarRange { start: 7, end: 7 });
    }

    #[test]
    fn every_palette_composes_into_a_valid_score() {
        let cat = Catalogue::standard();
        let s = setup();
        let x = vec![0.5; DIMS.len()];
        let palettes = [
            Palette {
                lead: Some(("felt_piano", InstrumentKind::Sampler)),
                bass: Some("sub_bass"),
                drums: true,
            },
            Palette {
                lead: Some(("bell", InstrumentKind::Synth)),
                bass: None,
                drums: false,
            },
            Palette {
                lead: None,
                bass: Some("warm_bass"),
                drums: true,
            },
        ];
        for p in &palettes {
            for contour in 0..CONTOURS.len() {
                let score = score(&s, p, &x, 8, contour, &cat).expect("composes");
                let report = praxis_score::validate::validate(&score);
                assert!(report.is_valid(), "{:?}: {:?}", p, report.errors);
                assert_eq!(score.bars, 8);
                assert_eq!(score.key, s.key);
                assert!(score.meta.reference_profiles == vec!["ref".to_string()]);
            }
        }
    }

    #[test]
    fn intro_has_the_lead_alone() {
        let p = Palette {
            lead: Some(("mallet", InstrumentKind::Synth)),
            bass: Some("sub_bass"),
            drums: false,
        };
        let plan = plan(&setup(), &p, &[0.5; DIMS.len()], 8, 0).unwrap();
        assert_eq!(plan.sections.len(), 2);
        assert_eq!(plan.sections[0].layers, vec!["lead".to_string()]);
        assert_eq!(plan.sections[1].layers.len(), 2);
    }

    #[test]
    fn contours_change_only_the_degrees() {
        let p = Palette {
            lead: Some(("mallet", InstrumentKind::Synth)),
            bass: None,
            drums: false,
        };
        let x = vec![0.5; DIMS.len()];
        let a = plan(&setup(), &p, &x, 4, 0).unwrap();
        let b = plan(&setup(), &p, &x, 4, 1).unwrap();
        assert_ne!(a.layers[0].motif.degrees, b.layers[0].motif.degrees);
        assert_eq!(a.layers[0].motif.spoken, b.layers[0].motif.spoken);
    }
}
