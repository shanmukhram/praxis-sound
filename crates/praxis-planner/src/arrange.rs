//! Deterministic arranger: expands a validated [`ScorePlan`] into a [`Score`] (ADR-0006).
//!
//! The owner's taste rules hold by construction:
//! - rhythm first: every layer is one motif on a step grid, built from a spoken rhythm, with
//!   accents, and it repeats;
//! - notes connect: sustaining instruments get note lengths above 1.0 step;
//! - layering: each layer has exactly one pattern, so a later section that keeps a layer plays
//!   the identical pattern (same id, same phase); sections only add or remove whole layers;
//! - restraint: layers whose class or preset class is on the avoid-list are never generated, and
//!   the avoid-list always holds the trailer classes the brief did not ask for;
//! - always in key: pitches come from scale degrees (or are snapped to the key).

use crate::catalogue::{Catalogue, PresetSpec};
use crate::plan::{
    Dynamics, PlanLayer, PlanPlayback, Register, ScorePlan, diatonic_symbol, effective_avoid,
    effective_class, parse_chord, resolve_timing, scale_degree,
};
use praxis_score::rhythm::{
    SpokenStep, VELOCITY_BRIGHT, VELOCITY_NORMAL, VELOCITY_SOFT, VELOCITY_STRONG, parse_spoken,
};
use praxis_score::validate::unpitched_class;
use praxis_score::{
    Accent, BarRange, ChordSpan, Clip, Constraints, Effect, EffectKind, GeneratorClass,
    InstrumentKind, InstrumentRef, Key, Layer, Master, Meta, Mode, Note, Pattern, Playback, Score,
    Section, Tempo,
};
use std::collections::{BTreeMap, BTreeSet};

/// Default note length (in steps) for sustaining instruments: rings into the next step.
pub const DEFAULT_SUSTAIN: f32 = 1.6;

/// What the arranger needs besides the plan.
#[derive(Clone, Copy, Debug)]
pub struct ArrangeContext<'a> {
    pub catalogue: &'a Catalogue,
    /// Trailer classes the brief explicitly asked for.
    pub requested: &'a BTreeSet<GeneratorClass>,
    pub seed: u64,
    /// The brief, recorded in `meta.brief`.
    pub brief: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ArrangeError {
    #[error("plan cannot be arranged: {0}")]
    Plan(String),
}

/// Expands a plan (validate and repair it first) into a Score.
pub fn arrange(plan: &ScorePlan, ctx: &ArrangeContext) -> Result<Score, ArrangeError> {
    let timing = resolve_timing(plan).map_err(ArrangeError::Plan)?;
    let bars = timing.bars;
    let ts = plan.time_signature;
    let key = plan.key;
    let avoid = effective_avoid(plan, ctx.requested);

    let mut score = Score::empty(bars, timing.bpm);
    score.tempo = Tempo { bpm: timing.bpm };
    score.time_signature = ts;
    score.key = key;
    score.seed = ctx.seed;
    score.title = if plan.title.trim().is_empty() {
        "Untitled".to_string()
    } else {
        plan.title.trim().to_string()
    };
    score.playback = match plan.playback {
        PlanPlayback::Loop => Playback::Loop,
        PlanPlayback::OneShot => Playback::OneShot { tail_seconds: 4.0 },
    };
    score.constraints = Constraints {
        avoid: avoid.clone(),
    };
    score.master = Master::default();

    // Harmony: one chord root (scale degree) per bar.
    let chord_bars = chord_per_bar(plan, &key, bars);
    score.chords = chord_spans(&chord_bars);

    // Sections.
    let mut sections: Vec<_> = plan.sections.iter().collect();
    sections.sort_by_key(|s| s.bars.start);
    let mut section_ids = BTreeSet::new();
    for (i, s) in sections.iter().enumerate() {
        let id = unique_id(
            &slug(&s.name, &format!("section-{}", i + 1)),
            &mut section_ids,
        );
        score.sections.push(Section {
            id,
            name: s.name.clone(),
            bars: s.bars,
            role: s.role,
            intensity: s.intensity.clamp(0.0, 1.0),
        });
    }

    // Layers.
    let mut layer_ids = BTreeSet::new();
    let mut pattern_ids = BTreeSet::new();
    for pl in &plan.layers {
        let Some(preset) = ctx
            .catalogue
            .get_kind(pl.preset.kind, &pl.preset.name)
            .or_else(|| ctx.catalogue.get(&pl.preset.name))
        else {
            return Err(ArrangeError::Plan(format!(
                "unknown preset `{}`",
                pl.preset.name
            )));
        };
        let class = effective_class(pl, Some(preset));
        if avoid.contains(&class) || avoid.contains(&preset.class) {
            // Banned layers are never generated.
            continue;
        }
        let active: BTreeSet<u32> = sections
            .iter()
            .filter(|s| s.layers.iter().any(|l| l == &pl.id))
            .flat_map(|s| s.bars.start.max(1)..=s.bars.end.min(bars))
            .collect();
        if active.is_empty() {
            continue;
        }
        let layer_id = unique_id(&slug(&pl.id, "layer"), &mut layer_ids);
        let pattern_id = unique_id(&format!("{layer_id}_motif"), &mut pattern_ids);
        let pattern = build_pattern(pl, preset, class, &key, ts, &chord_bars, &pattern_id)?;
        let spb = ts
            .steps_per_bar(pattern.grid)
            .ok_or_else(|| ArrangeError::Plan("grid does not divide the bar".into()))?;
        let clips = clips_for(&active, &pattern, spb);
        score.patterns.push(pattern);
        score.layers.push(Layer {
            id: layer_id,
            name: if pl.name.trim().is_empty() {
                display_name(&preset.name)
            } else {
                pl.name.trim().to_string()
            },
            class,
            instrument: InstrumentRef {
                kind: preset.kind,
                preset: preset.name.clone(),
                params: BTreeMap::new(),
            },
            clips,
            locked: false,
            muted: false,
            solo: false,
            gain_db: pl
                .gain_db
                .map(|g| g.clamp(-60.0, 24.0))
                .unwrap_or_else(|| default_gain(class, pl.dynamics)),
            pan: 0.0,
            effects: default_effects(preset, class, pl),
            automation: vec![],
            seed_offset: 0,
        });
    }

    let intensity = if score.sections.is_empty() {
        0.5
    } else {
        score.sections.iter().map(|s| s.intensity).sum::<f32>() / score.sections.len() as f32
    };
    score.meta = Meta {
        brief: ctx.brief.map(str::to_string),
        emotion: Some(plan.emotion.trim().to_lowercase()),
        intensity: Some((intensity * 100.0).round() / 100.0),
        tags: vec!["planned".to_string()],
        reference_profiles: vec![],
        provenance: BTreeMap::new(),
    };
    Ok(score)
}

/// Chord root degree (in the scale) and symbol for every bar, index 0 = bar 1.
fn chord_per_bar(plan: &ScorePlan, key: &Key, bars: u32) -> Vec<(i32, String)> {
    let tonic = (0, diatonic_symbol(key, 0));
    let mut out = vec![tonic; bars as usize];
    for c in &plan.chords {
        let Some(sym) = parse_chord(&c.symbol) else {
            continue;
        };
        let degree = if key.mode == Mode::Chromatic {
            (sym.root.semitone() as i32 - key.tonic.semitone() as i32).rem_euclid(12)
        } else {
            match scale_degree(key, sym.root) {
                Some(d) => d,
                None => continue,
            }
        };
        for b in c.bars.start.max(1)..=c.bars.end.min(bars) {
            out[(b - 1) as usize] = (degree, c.symbol.trim().to_string());
        }
    }
    out
}

fn chord_spans(per_bar: &[(i32, String)]) -> Vec<ChordSpan> {
    let mut spans: Vec<ChordSpan> = Vec::new();
    for (i, (_, sym)) in per_bar.iter().enumerate() {
        let bar = i as u32 + 1;
        match spans.last_mut() {
            Some(last) if &last.symbol == sym && last.bars.end + 1 == bar => last.bars.end = bar,
            _ => spans.push(ChordSpan {
                bars: BarRange {
                    start: bar,
                    end: bar,
                },
                beat: 0.0,
                symbol: sym.clone(),
            }),
        }
    }
    spans
}

/// Contiguous runs of active bars become clips, each phased to the global bar grid so the
/// pattern plays the same notes in a bar wherever a clip starts.
fn clips_for(active: &BTreeSet<u32>, pattern: &Pattern, spb: u32) -> Vec<Clip> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &b in active {
        match runs.last_mut() {
            Some(r) if r.1 + 1 == b => r.1 = b,
            _ => runs.push((b, b)),
        }
    }
    runs.into_iter()
        .map(|(start, end)| Clip {
            bars: BarRange { start, end },
            pattern: pattern.id.clone(),
            pattern_offset_steps: ((start - 1) * spb) % pattern.length_steps.max(1),
            transpose: 0,
            velocity_scale: 1.0,
        })
        .collect()
}

fn base_velocity(accent: Accent) -> f32 {
    match accent {
        Accent::Strong => VELOCITY_STRONG,
        Accent::Normal => VELOCITY_NORMAL,
        Accent::Soft => VELOCITY_SOFT,
        Accent::Bright => VELOCITY_BRIGHT,
    }
}

fn dynamics_scale(d: Dynamics) -> f32 {
    match d {
        Dynamics::Soft => 0.85,
        Dynamics::Normal => 1.0,
        Dynamics::Strong => 1.1,
    }
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// Classes whose degrees follow the chord root when `follow_chords` is set, and which stack
/// triads.
fn stacks_triads(class: GeneratorClass) -> bool {
    matches!(
        class,
        GeneratorClass::Chords | GeneratorClass::Harmony | GeneratorClass::Pad
    )
}

/// Default octave of degree 0 for a register.
pub fn register_octave(class: GeneratorClass, register: Register) -> i32 {
    let base = match class {
        GeneratorClass::Bass => 2,
        GeneratorClass::Drone => 2,
        _ => 3,
    };
    match register {
        Register::Low => base - 1,
        Register::Mid => base,
        Register::High => base + 1,
    }
}

/// Moves a pitch by octaves into the preset's range, then onto the key.
fn fit_pitch(mut p: i32, preset: &PresetSpec, key: &Key) -> u8 {
    let (lo, hi) = (preset.range.0 as i32, preset.range.1 as i32);
    while p < lo && p + 12 <= 127 {
        p += 12;
    }
    while p > hi && p - 12 >= 0 {
        p -= 12;
    }
    let p = p.clamp(0, 127) as u8;
    if key.mode == Mode::Chromatic {
        p
    } else {
        key.snap(p)
    }
}

/// Drum note for a hit: the preset's GM note, or for a kit DON = kick, TIN = snare, else hat.
fn drum_pitch(preset: &PresetSpec, accent: Accent) -> u8 {
    preset.gm_note.unwrap_or(match accent {
        Accent::Strong => 36,
        Accent::Bright => 38,
        _ => 42,
    })
}

/// Smallest pattern length in bars (a multiple of the motif length) over which the chord
/// sequence repeats, capped at the piece length.
fn harmonic_period(motif_bars: u32, chord_bars: &[(i32, String)]) -> u32 {
    let total = chord_bars.len() as u32;
    let mut p = motif_bars.max(1);
    while p < total {
        let periodic = (0..total as usize).all(|b| chord_bars[b].0 == chord_bars[b % p as usize].0);
        if periodic {
            return p;
        }
        p += motif_bars.max(1);
    }
    total.max(1)
}

/// Normalises a chord root degree to the nearest octave around the tonic (-n/2..=n/2).
fn near_root(degree: i32, n: i32) -> i32 {
    let d = degree.rem_euclid(n);
    if d > n / 2 { d - n } else { d }
}

#[allow(clippy::too_many_arguments)]
fn build_pattern(
    pl: &PlanLayer,
    preset: &PresetSpec,
    class: GeneratorClass,
    key: &Key,
    ts: praxis_score::TimeSignature,
    chord_bars: &[(i32, String)],
    id: &str,
) -> Result<Pattern, ArrangeError> {
    let m = &pl.motif;
    let spb = ts
        .steps_per_bar(m.grid)
        .ok_or_else(|| ArrangeError::Plan(format!("layer `{}`: grid does not fit", pl.id)))?;
    let motif_bars = m.length_bars.max(1);
    let motif_steps = motif_bars * spb;
    let drum = preset.kind == InstrumentKind::Drum || unpitched_class(class);
    let follow = m.follow_chords && !drum && !chord_bars.is_empty();
    let period_bars = if follow {
        harmonic_period(motif_bars, chord_bars)
    } else {
        motif_bars
    };
    let length_steps = period_bars * spb;
    let sustain = m.sustain_steps.unwrap_or(if preset.sustaining {
        DEFAULT_SUSTAIN
    } else {
        1.0
    });
    let dyn_scale = dynamics_scale(pl.dynamics);
    let octave = m
        .octave
        .unwrap_or_else(|| register_octave(class, pl.register));
    let n_scale = key.mode.intervals().len() as i32;
    let mut notes: Vec<Note> = Vec::new();

    let root_at = |abs_step: u32| -> i32 {
        if follow {
            let bar = (abs_step / spb) as usize % chord_bars.len();
            near_root(chord_bars[bar].0, n_scale)
        } else {
            0
        }
    };

    if !m.notes.is_empty() {
        // Explicit notes, repeated over the harmonic period.
        for rep in 0..(period_bars / motif_bars).max(1) {
            for n in &m.notes {
                if !(n.step.is_finite() && n.step >= 0.0) {
                    continue;
                }
                let step = n.step.round() as u32;
                if step >= motif_steps {
                    continue;
                }
                let abs = rep * motif_steps + step;
                let pitch = if drum {
                    n.pitch.clamp(0, 127) as u8
                } else {
                    fit_pitch(n.pitch, preset, key)
                };
                let v = n.velocity.unwrap_or_else(|| base_velocity(n.accent)) * dyn_scale;
                notes.push(Note {
                    step: abs,
                    pitch,
                    velocity: round3(v.clamp(0.05, 1.0)),
                    accent: n.accent,
                    length: n.length.unwrap_or(sustain).clamp(0.05, 64.0),
                    offset: 0.0,
                });
            }
        }
    } else {
        let steps = parse_spoken(&m.spoken)
            .map_err(|e| ArrangeError::Plan(format!("layer `{}`: {e}", pl.id)))?;
        let slen = steps.len() as u32;
        if slen == 0 {
            return Err(ArrangeError::Plan(format!(
                "layer `{}`: empty spoken rhythm",
                pl.id
            )));
        }
        let degrees: Vec<i32> = if m.degrees.is_empty() {
            vec![0]
        } else {
            m.degrees.clone()
        };
        // Index of the spoken token played at a motif position.
        let token_at = |pos: u32| -> Option<&SpokenStep> {
            if motif_steps.is_multiple_of(slen) {
                steps.get((pos % slen) as usize)
            } else if slen <= spb {
                steps.get(((pos % spb) % slen) as usize)
            } else {
                steps.get(pos as usize)
            }
        };
        // Indices (into `notes`) of the notes the last hit produced, for ties.
        let mut last_hit: Vec<usize> = Vec::new();
        let mut hit_index = 0usize;
        for abs in 0..length_steps {
            let pos = abs % motif_steps;
            if pos == 0 {
                hit_index = 0;
            }
            match token_at(pos) {
                Some(SpokenStep::Hit { accent, velocity }) => {
                    let v = round3((velocity * dyn_scale).clamp(0.05, 1.0));
                    let start = notes.len();
                    if drum {
                        notes.push(Note {
                            step: abs,
                            pitch: drum_pitch(preset, *accent),
                            velocity: v,
                            accent: *accent,
                            length: sustain,
                            offset: 0.0,
                        });
                    } else {
                        let d = degrees[hit_index % degrees.len()] + root_at(abs);
                        let chord: Vec<i32> = if follow && stacks_triads(class) {
                            vec![d, d + 2, d + 4]
                        } else {
                            vec![d]
                        };
                        for deg in chord {
                            let pitch = fit_pitch(key.degree_to_midi(deg, octave), preset, key);
                            if notes[start..].iter().any(|n| n.pitch == pitch) {
                                continue;
                            }
                            notes.push(Note {
                                step: abs,
                                pitch,
                                velocity: v,
                                accent: *accent,
                                length: sustain,
                                offset: 0.0,
                            });
                        }
                    }
                    last_hit = (start..notes.len()).collect();
                    hit_index += 1;
                }
                Some(SpokenStep::Tie) => {
                    for &i in &last_hit {
                        notes[i].length = (notes[i].length + 1.0).min(64.0);
                    }
                }
                Some(SpokenStep::Rest) | None => last_hit.clear(),
            }
        }
    }
    notes.sort_by_key(|a| (a.step, a.pitch));
    notes.dedup_by(|a, b| a.step == b.step && a.pitch == b.pitch);
    Ok(Pattern {
        id: id.to_string(),
        grid: m.grid,
        length_steps,
        notes,
        spoken: if m.spoken.trim().is_empty() {
            None
        } else {
            Some(m.spoken.trim().to_string())
        },
    })
}

fn default_gain(class: GeneratorClass, dynamics: Dynamics) -> f32 {
    use GeneratorClass as C;
    let g = match class {
        C::Melody | C::Motif => 0.0,
        C::Chords | C::Harmony => -3.0,
        C::Arp => -4.0,
        C::Bass => -3.0,
        C::Drums => -4.0,
        C::Percussion => -8.0,
        C::Pad | C::Texture | C::Ambience | C::Drone => -8.0,
        _ => -6.0,
    };
    match dynamics {
        Dynamics::Soft => g - 1.0,
        Dynamics::Normal => g,
        Dynamics::Strong => g + 1.0,
    }
}

fn params(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

/// A small room on sustaining instruments; a gentle EQ with a low-pass on soft kicks.
fn default_effects(preset: &PresetSpec, class: GeneratorClass, pl: &PlanLayer) -> Vec<Effect> {
    let mut fx = Vec::new();
    if preset.kind == InstrumentKind::Drum {
        let is_kick = preset.gm_note == Some(36);
        if is_kick {
            let soft = preset.name == "soft_kick" || pl.dynamics == Dynamics::Soft;
            let p = if soft {
                params(&[("hp_hz", 30.0), ("lp_hz", 2500.0)])
            } else {
                params(&[("hp_hz", 30.0)])
            };
            fx.push(Effect {
                id: "tone".into(),
                kind: EffectKind::Eq,
                params: p,
                bypass: false,
                sidechain: None,
                impulse: None,
            });
        } else if preset
            .gm_note
            .is_some_and(|n| n == 42 || n == 46 || n == 70)
        {
            fx.push(Effect {
                id: "tone".into(),
                kind: EffectKind::Eq,
                params: params(&[("hp_hz", 300.0)]),
                bypass: false,
                sidechain: None,
                impulse: None,
            });
        }
    } else if preset.sustaining && !unpitched_class(class) {
        fx.push(Effect {
            id: "room".into(),
            kind: EffectKind::Reverb,
            params: params(&[
                ("decay_s", 1.8),
                ("predelay_ms", 12.0),
                ("size", 0.35),
                ("mix", 0.18),
            ]),
            bypass: false,
            sidechain: None,
            impulse: None,
        });
    }
    fx
}

/// "felt_piano" -> "Felt piano".
pub fn display_name(preset: &str) -> String {
    let s = preset.replace('_', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// A valid Score id from free text.
pub fn slug(text: &str, fallback: &str) -> String {
    let mut s = String::new();
    for c in text.trim().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if (c == '_' || c == '-' || c.is_whitespace()) && !s.is_empty() && !s.ends_with('-')
        {
            s.push(if c == '_' { '_' } else { '-' });
        }
    }
    let s = s.trim_end_matches(['-', '_']).to_string();
    let s: String = s.chars().take(56).collect();
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

fn unique_id(base: &str, seen: &mut BTreeSet<String>) -> String {
    let mut id = base.to_string();
    let mut n = 2;
    while seen.contains(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    seen.insert(id.clone());
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_names() {
        assert_eq!(slug("Piano alone", "x"), "piano-alone");
        assert_eq!(slug("  ", "x"), "x");
        assert_eq!(slug("felt_piano", "x"), "felt_piano");
        assert_eq!(display_name("soft_kick"), "Soft kick");
        let mut seen = BTreeSet::new();
        assert_eq!(unique_id("a", &mut seen), "a");
        assert_eq!(unique_id("a", &mut seen), "a-2");
    }

    #[test]
    fn harmonic_periods() {
        let c = |d: &[i32]| d.iter().map(|x| (*x, String::new())).collect::<Vec<_>>();
        assert_eq!(harmonic_period(1, &c(&[0, 5, 0, 5])), 2);
        assert_eq!(harmonic_period(2, &c(&[0, 5, 0, 5])), 2);
        assert_eq!(harmonic_period(1, &c(&[0, 0, 0])), 1);
        assert_eq!(harmonic_period(1, &c(&[0, 5, 3])), 3);
        assert_eq!(near_root(5, 7), -2);
        assert_eq!(near_root(3, 7), 3);
    }
}
