//! New musical material for edits and variations, built by the same deterministic arranger as a
//! whole Score: a one-layer ScorePlan in the Score's key, metre, tempo and chords is validated,
//! repaired and arranged, and its pattern and layer settings are taken over. The owner's taste
//! rules therefore hold for edit material exactly as for planned material.
//!
//! Patterns built here have step 0 on bar 1 (the arranger's global phase); [`rotate`] moves
//! them so a clip that starts at another bar plays the same notes it would in the full Score.

use crate::arrange::{ArrangeContext, arrange, register_octave};
use crate::catalogue::{Catalogue, PresetSpec};
use crate::motifs::{CHORD_RELATIVE_MOTIF, RhythmRole, default_spoken, fitting_grid, rhythm_role};
use crate::plan::{
    Dynamics, MAX_BARS, MAX_BPM, MIN_BPM, Motif, PlanChord, PlanContext, PlanLayer, PlanPlayback,
    PlanSection, PresetChoice, Register, ScorePlan, TRAILER_CLASSES, issues_text, validate_plan,
};
use crate::repair::{Repair, fix_plan};
use praxis_score::rhythm::{SpokenStep, VELOCITY_STRONG, parse_spoken};
use praxis_score::timeline::stable_hash;
use praxis_score::validate::unpitched_class;
use praxis_score::{
    Accent, BarRange, GeneratorClass, Grid, InstrumentKind, Layer, Pattern, Playback, Score,
    SectionRole,
};
use std::collections::BTreeSet;

/// Deterministic draw `0..n` from a seed and a salt (no running stream: the same inputs always
/// give the same draw).
pub fn pick(seed: u64, salt: &str, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (stable_hash(&[&seed.to_le_bytes(), salt.as_bytes()]) % n as u64) as usize
}

/// A derived seed for item `i` of a seeded set (variations, retries).
pub fn derive_seed(seed: u64, salt: &str, i: u64) -> u64 {
    stable_hash(&[&seed.to_le_bytes(), salt.as_bytes(), &i.to_le_bytes()])
}

/// What new material should be: the layer's instrument, role and motif.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialSpec {
    pub id: String,
    pub name: String,
    pub class: GeneratorClass,
    pub preset: PresetSpec,
    pub register: Register,
    pub dynamics: Dynamics,
    pub gain_db: Option<f32>,
    pub motif: Motif,
}

/// Arranged material: a pattern (step 0 = bar 1) and a layer template without clips.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub pattern: Pattern,
    pub layer: Layer,
    /// Deterministic repairs the arranger's validation made to the material's motif.
    pub repairs: Vec<Repair>,
}

/// The Score's chords as plan chords (bars past the plan limit are dropped).
fn plan_chords(score: &Score, bars: u32) -> Vec<PlanChord> {
    score
        .chords
        .iter()
        .filter(|c| c.bars.start <= bars)
        .map(|c| PlanChord {
            bars: BarRange {
                start: c.bars.start,
                end: c.bars.end.min(bars),
            },
            symbol: c.symbol.clone(),
        })
        .collect()
}

/// Builds material with the arranger in the Score's musical context.
pub fn build(
    score: &Score,
    spec: &MaterialSpec,
    catalogue: &Catalogue,
) -> Result<Material, String> {
    let bars = score.bars.clamp(1, MAX_BARS);
    let requested: BTreeSet<GeneratorClass> = TRAILER_CLASSES
        .iter()
        .copied()
        .filter(|c| !score.constraints.avoid.contains(c))
        .collect();
    let pl = PlanLayer {
        id: spec.id.clone(),
        name: spec.name.clone(),
        class: spec.class,
        preset: PresetChoice {
            kind: spec.preset.kind,
            name: spec.preset.name.clone(),
        },
        register: spec.register,
        dynamics: spec.dynamics,
        gain_db: spec.gain_db,
        motif: spec.motif.clone(),
    };
    let plan = ScorePlan {
        title: String::new(),
        emotion: score
            .meta
            .emotion
            .as_deref()
            .and_then(|e| e.split_whitespace().next())
            .unwrap_or("calm")
            .to_string(),
        key: score.key,
        time_signature: score.time_signature,
        tempo_bpm: Some(score.tempo.bpm.clamp(MIN_BPM, MAX_BPM)),
        duration_s: None,
        bars: Some(bars),
        playback: match score.playback {
            Playback::Loop => PlanPlayback::Loop,
            Playback::OneShot { .. } => PlanPlayback::OneShot,
        },
        avoid: vec![],
        sections: vec![PlanSection {
            name: "edit".into(),
            bars: BarRange {
                start: 1,
                end: bars,
            },
            role: SectionRole::Main,
            intensity: 0.5,
            layers: vec![spec.id.clone()],
        }],
        layers: vec![pl],
        chords: plan_chords(score, bars),
        notes: vec![],
        requested: requested.iter().copied().collect(),
    };
    let ctx = PlanContext {
        catalogue,
        requested: &requested,
    };
    let issues = validate_plan(&plan, &ctx);
    let (plan, repairs) = if issues.is_empty() {
        (plan, vec![])
    } else {
        let (fixed, repairs) = fix_plan(&plan, &ctx);
        let remaining = validate_plan(&fixed, &ctx);
        if !remaining.is_empty() {
            return Err(issues_text(&remaining));
        }
        (fixed, repairs)
    };
    let actx = ArrangeContext {
        catalogue,
        requested: &requested,
        seed: score.seed,
        brief: None,
    };
    let arranged = arrange(&plan, &actx).map_err(|e| e.to_string())?;
    let (Some(layer), Some(pattern)) = (arranged.layers.first(), arranged.patterns.first()) else {
        return Err(format!(
            "no `{}` material could be built (its class is on the avoid-list)",
            spec.preset.name
        ));
    };
    let mut layer = layer.clone();
    layer.clips.clear();
    Ok(Material {
        pattern: pattern.clone(),
        layer,
        repairs,
    })
}

/// Steps per bar of a grid in the Score's metre.
pub fn steps_per_bar(score: &Score, grid: Grid) -> Option<u32> {
    score.time_signature.steps_per_bar(grid)
}

/// The pattern step that plays at the first step of bar `bar` when step 0 sits on bar 1.
pub fn global_phase(score: &Score, pattern: &Pattern, bar: u32) -> u32 {
    let spb = steps_per_bar(score, pattern.grid).unwrap_or(1) as u64;
    let len = pattern.length_steps.max(1) as u64;
    ((bar.saturating_sub(1) as u64 * spb) % len) as u32
}

/// The pattern rotated so that its step 0 is the old step `phase`.
pub fn rotate(pattern: &Pattern, phase: u32) -> Pattern {
    let len = pattern.length_steps.max(1);
    let phase = phase % len;
    let mut p = pattern.clone();
    for n in &mut p.notes {
        n.step = (n.step + len - phase) % len;
    }
    p.notes.sort_by_key(|n| (n.step, n.pitch));
    p
}

/// The catalogue preset a layer plays (by kind and name, then by name).
pub fn layer_preset<'a>(catalogue: &'a Catalogue, layer: &Layer) -> Option<&'a PresetSpec> {
    catalogue
        .get_kind(layer.instrument.kind, &layer.instrument.preset)
        .or_else(|| catalogue.get(&layer.instrument.preset))
}

/// True for layers whose notes are drum hits (no melody to rewrite).
pub fn is_drum(layer: &Layer) -> bool {
    layer.instrument.kind == InstrumentKind::Drum || unpitched_class(layer.class)
}

/// Octave of degree 0 that keeps new material near the pitches a pattern plays.
pub fn octave_of(score: &Score, pattern: &Pattern) -> Option<i32> {
    let mut pitches: Vec<u8> = pattern.notes.iter().map(|n| n.pitch).collect();
    if pitches.is_empty() {
        return None;
    }
    pitches.sort_unstable();
    let mid = pitches[pitches.len() / 2] as i32;
    let tonic = score.key.tonic.semitone() as i32;
    // The octave whose tonic is at or just below the median note.
    Some(((mid - tonic).div_euclid(12) - 1).clamp(0, 8))
}

/// Dynamics a pattern was arranged with, read from its strongest accent's velocity.
pub fn dynamics_of(pattern: &Pattern) -> Dynamics {
    let strong = pattern
        .notes
        .iter()
        .filter(|n| n.accent == Accent::Strong)
        .map(|n| n.velocity)
        .fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v))));
    match strong {
        Some(v) if v < VELOCITY_STRONG * 0.93 => Dynamics::Soft,
        Some(v) if v > VELOCITY_STRONG * 1.05 => Dynamics::Strong,
        _ => Dynamics::Normal,
    }
}

/// Register of a layer relative to its class default.
pub fn register_of(score: &Score, layer: &Layer, pattern: &Pattern) -> Register {
    match octave_of(score, pattern) {
        Some(o) if o < register_octave(layer.class, Register::Mid) => Register::Low,
        Some(o) if o > register_octave(layer.class, Register::Mid) => Register::High,
        _ => Register::Mid,
    }
}

/// Spoken-rhythm melody templates for an eighth grid in 4/4 (two bars, ties so notes connect)
/// with chord-relative degrees cycled over their hits.
const MELODY_TEMPLATES: [(&str, &[i32]); 4] = [
    (
        "DON ~ ~ din don ~ din ~ TIN ~ ~ din don ~ ~ -",
        &[4, 2, 0, 2, 7, 4, 2],
    ),
    (
        "DON ~ din ~ don ~ ~ - TIN ~ din ~ don ~ ~ -",
        &[7, 4, 2, 4, 2, 0],
    ),
    (
        "DON ~ ~ ~ don ~ din ~ TIN ~ ~ ~ din ~ ~ -",
        &[0, 2, 4, 7, 4],
    ),
    (
        "DON ~ ~ din TIN ~ ~ din don ~ ~ din TIN ~ ~ -",
        &[4, 2, 4, 7, 4, 2, 0],
    ),
];

/// Chord-relative degree templates for rewritten pitched motifs.
const DEGREE_TEMPLATES: [&[i32]; 6] = [
    &CHORD_RELATIVE_MOTIF,
    &[0, 2, 4, 7, 4, 2],
    &[7, 4, 2, 0],
    &[0, 4, 2, 4, 7, 4, 2, 4],
    &[4, 2, 0, 2, 4, 7],
    &[0, 7, 4, 2],
];

/// A melody motif: the named template, or one chosen by seed.
pub fn melody_motif(score: &Score, seed: u64, grid: Option<Grid>) -> Motif {
    let ts = score.time_signature;
    let four_four = ts.numerator == 4 && ts.denominator == 4;
    let grid = grid.unwrap_or(if four_four {
        Grid::Eighth
    } else {
        fitting_grid(ts)
    });
    if four_four && grid == Grid::Eighth {
        let (spoken, degrees) = MELODY_TEMPLATES[pick(seed, "melody", MELODY_TEMPLATES.len())];
        return Motif {
            grid,
            length_bars: 2,
            spoken: spoken.to_string(),
            degrees: degrees.to_vec(),
            octave: None,
            sustain_steps: None,
            follow_chords: true,
            notes: vec![],
        };
    }
    let spoken = default_spoken(RhythmRole::Sparse, ts, grid)
        .or_else(|| default_spoken(RhythmRole::Sparse, ts, fitting_grid(ts)))
        .unwrap_or_else(|| "DON".to_string());
    Motif {
        grid,
        length_bars: 1,
        spoken,
        degrees: vec![4, 2, 0, 2],
        octave: None,
        sustain_steps: None,
        follow_chords: true,
        notes: vec![],
    }
}

/// Degrees for rewritten pitched material, chosen and rotated by seed.
pub fn degrees_for(seed: u64) -> Vec<i32> {
    let t = DEGREE_TEMPLATES[pick(seed, "degrees", DEGREE_TEMPLATES.len())];
    let rot = pick(seed, "rotate", t.len());
    let mut d = t.to_vec();
    d.rotate_left(rot);
    d
}

fn token_text(s: &SpokenStep) -> &'static str {
    match s {
        SpokenStep::Hit {
            accent: Accent::Strong,
            ..
        } => "DON",
        SpokenStep::Hit {
            accent: Accent::Bright,
            ..
        } => "TIN",
        SpokenStep::Hit {
            accent: Accent::Soft,
            ..
        } => "din",
        SpokenStep::Hit { .. } => "don",
        SpokenStep::Rest => "-",
        SpokenStep::Tie => "~",
    }
}

/// A rhythm variation of `base` (one token per grid step): the downbeat of every bar keeps its
/// hit; one to three other steps change (a rest becomes a soft hit, a soft hit rests or ties,
/// a soft and a normal hit swap). Grid and length stay, so the motif still repeats.
pub fn vary_spoken(base: &str, spb: u32, seed: u64, sustaining: bool) -> String {
    let Ok(steps) = parse_spoken(base) else {
        return base.to_string();
    };
    let mut toks: Vec<&'static str> = steps.iter().map(token_text).collect();
    let n = toks.len();
    if n < 2 {
        return base.to_string();
    }
    let changes = 1 + pick(seed, "changes", 3);
    for k in 0..changes {
        let salt = format!("pos{k}");
        let mut p = pick(seed, &salt, n);
        // Never touch a bar's downbeat.
        if spb > 0 && (p as u32).is_multiple_of(spb) {
            p = (p + 1) % n;
            if (p as u32).is_multiple_of(spb) {
                continue;
            }
        }
        let prev_sounds = p > 0 && toks[p - 1] != "-";
        toks[p] = match toks[p] {
            "-" => "din",
            "~" => "din",
            "din" if sustaining && prev_sounds => "~",
            "din" => "-",
            "don" => "din",
            other => other,
        };
    }
    // A tie needs something to extend.
    for i in 0..n {
        if toks[i] == "~" && (i == 0 || toks[i - 1] == "-") {
            toks[i] = "din";
        }
    }
    toks.join(" ")
}

/// A one-bar base rhythm for a layer when its pattern has no usable spoken source.
pub fn base_spoken(
    score: &Score,
    layer: &Layer,
    pattern: &Pattern,
    preset: Option<&PresetSpec>,
) -> Option<String> {
    if let Some(s) = &pattern.spoken
        && parse_spoken(s).is_ok_and(|v| v.len() as u32 <= pattern.length_steps)
    {
        return Some(s.clone());
    }
    let role = match preset {
        Some(p) => rhythm_role(p, layer.class),
        None if is_drum(layer) => RhythmRole::Kick,
        None => RhythmRole::Motif,
    };
    default_spoken(role, score.time_signature, pattern.grid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::Note;

    fn pat(steps: &[u32], len: u32) -> Pattern {
        Pattern {
            id: "p".into(),
            grid: Grid::Eighth,
            length_steps: len,
            notes: steps
                .iter()
                .map(|s| Note {
                    step: *s,
                    pitch: 60,
                    velocity: 0.5,
                    accent: Accent::Normal,
                    length: 1.0,
                    offset: 0.0,
                })
                .collect(),
            spoken: None,
        }
    }

    #[test]
    fn rotation_moves_steps() {
        let p = rotate(&pat(&[0, 3, 9], 16), 8);
        let steps: Vec<u32> = p.notes.iter().map(|n| n.step).collect();
        assert_eq!(steps, [1, 8, 11]);
        assert_eq!(rotate(&pat(&[0, 3], 8), 0).notes, pat(&[0, 3], 8).notes);
    }

    #[test]
    fn spoken_variations_keep_downbeats_and_length() {
        let base = "DON din don din TIN din don din";
        let mut seen = BTreeSet::new();
        for seed in 0..32 {
            let v = vary_spoken(base, 8, seed, true);
            let steps = parse_spoken(&v).expect("valid spoken");
            assert_eq!(steps.len(), 8);
            assert!(v.starts_with("DON"));
            seen.insert(v);
        }
        assert!(seen.len() > 4, "{seen:?}");
    }

    #[test]
    fn melody_templates_parse() {
        for (s, d) in MELODY_TEMPLATES {
            let steps = parse_spoken(s).expect("template");
            assert_eq!(steps.len(), 16);
            assert!(!d.is_empty());
        }
    }
}
