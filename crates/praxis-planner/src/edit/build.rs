//! A resolved edit as an exactly scoped `ScorePatch`: the patch's `scope.bars` is the edit's bars
//! (unset only for a whole-score edit), its `scope.layers` the target layers plus any new layer,
//! and every op stays inside both, so `praxis_score::apply::apply` enforces the isolation.
//!
//! Bar edits never mutate a pattern: changed material is a new pattern placed with `place_clip`
//! (which splits the old clips at the edges) or a whole new layer, and dynamics in a bar range are
//! a clip `velocity_scale`.
//!
//! Tone and space edits never touch notes. They change the layer's instrument parameters or
//! effects through `update_layer`. `apply` refuses an `update_layer` that changes the sound in a
//! bar-scoped patch, so a tone or space edit over some bars is a patch with `scope.bars` unset
//! whose only change is an effect that is exactly transparent outside those bars: a filter (or a
//! reverb) at `mix` 0, or the layer's own reverb mix, with a step automation lane that sets its
//! `mix` inside the bars. Outside the bars and the effect tail the layer renders bit-identically
//! (contracts/patch.md).

use super::material::{
    self, Material, MaterialSpec, base_spoken, degrees_for, derive_seed, dynamics_of, global_phase,
    is_drum, layer_preset, melody_motif, octave_of, register_of, steps_per_bar, vary_spoken,
};
use super::plan::{
    ChangeAmount, DynamicsChange, EditIntent, EditIssue, EditIssueCode, SpaceChange, ToneChange,
};
use super::resolve::{Resolved, bars_text, new_class, plays_in};
use crate::arrange::{display_name, slug};
use crate::catalogue::{Catalogue, PresetSpec, ToneControls};
use crate::motifs::{default_degrees, default_spoken, fitting_grid, follows_chords, rhythm_role};
use crate::plan::{Dynamics, Motif, Register};
use crate::repair::Repair;
use praxis_score::patch::{LayerUpdate, Origin, PatchOp, Scope, ScorePatch};
use praxis_score::{
    Accent, AutoPoint, Automation, BarRange, Clip, Curve, Effect, EffectKind, Grid, InstrumentRef,
    Layer, Pattern, Score,
};
use std::collections::BTreeMap;

/// Default change for `change_dynamics`, in dB.
pub const DEFAULT_SOFTER_DB: f32 = 4.0;
pub const DEFAULT_LOUDER_DB: f32 = 3.0;

/// The patch for a resolved edit, with what it did in words.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
    pub patch: ScorePatch,
    /// Ids of layers the patch adds.
    pub new_layers: Vec<String>,
    /// One sentence per change, for the summary.
    pub actions: Vec<String>,
    /// Deterministic repairs made to new material.
    pub repairs: Vec<Repair>,
}

/// The part of a clip inside a bar range, with the pattern offset it plays there.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub clip: Clip,
}

/// The pieces of layer `l`'s clips inside `bars`.
pub fn pieces(score: &Score, l: &Layer, bars: BarRange) -> Vec<Piece> {
    let mut out = Vec::new();
    for c in &l.clips {
        let start = c.bars.start.max(bars.start);
        let end = c.bars.end.min(bars.end);
        if start > end {
            continue;
        }
        let Some(p) = score.pattern(&c.pattern) else {
            continue;
        };
        let spb = steps_per_bar(score, p.grid).unwrap_or(1) as u64;
        let len = p.length_steps.max(1) as u64;
        let offset =
            ((c.pattern_offset_steps as u64 + (start - c.bars.start) as u64 * spb) % len) as u32;
        let mut clip = c.clone();
        clip.bars = BarRange { start, end };
        clip.pattern_offset_steps = offset;
        out.push(Piece { clip });
    }
    out
}

/// A layer id not yet used in the Score (nor in `taken`).
fn unique_layer_id(score: &Score, base: &str, taken: &[String]) -> String {
    let free = |id: &str| score.layer(id).is_none() && !taken.iter().any(|t| t == id);
    if free(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base}_{i}"))
        .find(|id| free(id))
        .unwrap_or_else(|| base.to_string())
}

/// A pattern id not yet used in the Score (nor in `taken`).
fn unique_pattern_id(score: &Score, base: &str, taken: &[String]) -> String {
    let free = |id: &str| score.pattern(id).is_none() && !taken.iter().any(|t| t == id);
    if free(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base}_{i}"))
        .find(|id| free(id))
        .unwrap_or_else(|| base.to_string())
}

fn material_issue(path: &str, e: String) -> Vec<EditIssue> {
    vec![EditIssue::new(EditIssueCode::Material, path, e)]
}

/// Octave shift (in semitones) that brings a pattern's median pitch into a preset's range.
fn range_shift(pattern: &Pattern, transpose: i8, p: &PresetSpec) -> i8 {
    if !p.pitched || pattern.notes.is_empty() {
        return 0;
    }
    let mut pitches: Vec<i32> = pattern
        .notes
        .iter()
        .map(|n| n.pitch as i32 + transpose as i32)
        .collect();
    pitches.sort_unstable();
    let mid = pitches[pitches.len() / 2];
    let (lo, hi) = (p.range.0 as i32, p.range.1 as i32);
    let mut shift = 0;
    while mid + shift < lo && shift < 48 {
        shift += 12;
    }
    while mid + shift > hi && shift > -48 {
        shift -= 12;
    }
    let lowest = pitches[0] + shift;
    let highest = pitches[pitches.len() - 1] + shift;
    if lowest < 0 || highest > 127 {
        return 0;
    }
    shift as i8
}

/// Builds the patch for a resolved edit. `seed` decides new material.
pub fn build_patch(
    r: &Resolved,
    score: &Score,
    catalogue: &Catalogue,
    seed: u64,
) -> Result<Built, Vec<EditIssue>> {
    let mut b = Built {
        patch: ScorePatch {
            description: String::new(),
            origin: Origin::Model,
            scope: Scope {
                bars: if r.whole { None } else { Some(r.bars) },
                layers: r.targets.clone(),
            },
            ops: vec![],
        },
        new_layers: vec![],
        actions: vec![],
        repairs: vec![],
    };
    let where_ = if r.whole {
        "throughout".to_string()
    } else {
        format!("in {}", bars_text(r.bars))
    };
    match r.plan.intent {
        EditIntent::AddLayer | EditIntent::AddMelody => {
            add_layer(r, score, catalogue, seed, &mut b, &where_)?
        }
        EditIntent::RemoveLayer => remove(r, score, &mut b, &where_),
        EditIntent::ChangeDynamics => dynamics(r, score, &mut b, &where_),
        EditIntent::ThinOut => thin_out(r, score, catalogue, &mut b, &where_),
        EditIntent::RewriteMotif => rewrite(r, score, catalogue, seed, &mut b, &where_)?,
        EditIntent::ChangeInstrument => change_instrument(r, score, &mut b, &where_)?,
        EditIntent::ChangeTone => tone(r, score, catalogue, &mut b, &where_),
        EditIntent::ChangeSpace => space(r, score, &mut b, &where_),
        // Answered before resolution (edit.rs); there is nothing to build.
        EditIntent::NotUnderstood => {}
    }
    if matches!(
        r.plan.intent,
        EditIntent::ChangeTone | EditIntent::ChangeSpace
    ) {
        // Layer settings change only in a patch without bars (see the module docs); the
        // automation holds the change to the edited bars.
        b.patch.scope.bars = None;
    }
    b.patch.scope.layers.extend(b.new_layers.iter().cloned());
    b.patch.description = format!(
        "{}: {}",
        intent_name(r.plan.intent),
        if b.actions.is_empty() {
            "no change".to_string()
        } else {
            b.actions.join("; ")
        }
    );
    Ok(b)
}

/// The intent's snake_case name.
pub fn intent_name(i: EditIntent) -> String {
    serde_json::to_value(i)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn dyn_word(d: Dynamics) -> &'static str {
    match d {
        Dynamics::Soft => "soft ",
        Dynamics::Normal => "",
        Dynamics::Strong => "strong ",
    }
}

fn add_layer(
    r: &Resolved,
    score: &Score,
    catalogue: &Catalogue,
    seed: u64,
    b: &mut Built,
    where_: &str,
) -> Result<(), Vec<EditIssue>> {
    let Some(preset) = r.preset.clone() else {
        return Err(material_issue(
            "/params/preset",
            "no instrument for the new layer".into(),
        ));
    };
    let params = &r.plan.params;
    let melody = r.plan.intent == EditIntent::AddMelody;
    let class = new_class(&r.plan, &preset);
    let ts = score.time_signature;
    let mut motif = if melody && preset.pitched {
        melody_motif(score, seed, params.grid)
    } else {
        let role = rhythm_role(&preset, class);
        let grid = params
            .grid
            .filter(|g| ts.steps_per_bar(*g).is_some())
            .unwrap_or_else(|| {
                if ts.steps_per_bar(Grid::Eighth).is_some() {
                    Grid::Eighth
                } else {
                    fitting_grid(ts)
                }
            });
        let spoken = default_spoken(role, ts, grid)
            .or_else(|| default_spoken(role, ts, fitting_grid(ts)))
            .unwrap_or_else(|| "DON".to_string());
        Motif {
            grid,
            length_bars: 1,
            spoken,
            degrees: default_degrees(role),
            octave: None,
            sustain_steps: None,
            follow_chords: follows_chords(role),
            notes: vec![],
        }
    };
    if let Some(g) = params.grid
        && ts.steps_per_bar(g).is_some()
        && g != motif.grid
    {
        motif.grid = g;
    }
    if let Some(s) = &params.spoken {
        motif.spoken = s.clone();
        if params.length_bars.is_none() {
            let spb = ts.steps_per_bar(motif.grid).unwrap_or(1) as usize;
            let tokens = s.split_whitespace().count();
            motif.length_bars = (tokens.div_ceil(spb.max(1))).clamp(1, 8) as u32;
        }
    }
    if !params.degrees.is_empty() {
        motif.degrees = params.degrees.clone();
        motif.follow_chords = true;
    }
    if let Some(n) = params.length_bars {
        motif.length_bars = n;
    }
    let name = params.name.clone().unwrap_or_else(|| {
        if melody {
            "Melody".to_string()
        } else {
            display_name(&preset.name)
        }
    });
    let id = unique_layer_id(score, &slug(&name, "layer"), &b.new_layers);
    let dynamics = params.dynamics.unwrap_or(Dynamics::Normal);
    let register = params.register.unwrap_or(if melody {
        Register::High
    } else {
        Register::Mid
    });
    let spec = MaterialSpec {
        id: id.clone(),
        name: name.clone(),
        class,
        preset: preset.clone(),
        register,
        dynamics,
        gain_db: params.gain_db,
        motif,
    };
    let Material {
        mut pattern,
        mut layer,
        repairs,
    } = material::build(score, &spec, catalogue).map_err(|e| material_issue("/params", e))?;
    b.repairs.extend(repairs);
    pattern.id = unique_pattern_id(score, &format!("{id}_motif"), &[]);
    layer.id = id.clone();
    layer.name = name;
    layer.locked = false;
    layer.muted = false;
    layer.solo = false;
    layer.clips = vec![Clip {
        bars: r.bars,
        pattern: pattern.id.clone(),
        pattern_offset_steps: global_phase(score, &pattern, r.bars.start),
        transpose: 0,
        velocity_scale: 1.0,
    }];
    b.actions.push(format!(
        "added a {}{} {} (`{id}`) {where_}",
        dyn_word(dynamics),
        preset.name.replace('_', " "),
        if melody { "melody" } else { "layer" },
    ));
    b.patch.ops.push(PatchOp::AddPattern {
        pattern,
        index: None,
    });
    b.patch.ops.push(PatchOp::AddLayer { layer, index: None });
    b.new_layers.push(id);
    Ok(())
}

fn remove(r: &Resolved, score: &Score, b: &mut Built, where_: &str) {
    for id in &r.targets {
        let Some(l) = score.layer(id) else { continue };
        if r.whole {
            b.patch.ops.push(PatchOp::RemoveLayer { id: id.clone() });
            // Patterns only this layer played go with it.
            let mut own: Vec<&str> = l.clips.iter().map(|c| c.pattern.as_str()).collect();
            own.sort_unstable();
            own.dedup();
            for p in own {
                let shared = score
                    .layers
                    .iter()
                    .filter(|o| o.id != *id && !r.targets.contains(&o.id))
                    .any(|o| o.clips.iter().any(|c| c.pattern == p));
                let removed_before = b
                    .patch
                    .ops
                    .iter()
                    .any(|op| matches!(op, PatchOp::RemovePattern { id: x } if x == p));
                if !shared && !removed_before {
                    b.patch
                        .ops
                        .push(PatchOp::RemovePattern { id: p.to_string() });
                }
            }
            b.actions.push(format!("removed layer `{id}`"));
        } else if plays_in(l, r.bars) {
            b.patch.ops.push(PatchOp::ClearBars {
                layer: id.clone(),
                bars: r.bars,
            });
            b.actions.push(format!("silenced `{id}` {where_}"));
        }
    }
    // Patterns must go after every layer that used them.
    b.patch
        .ops
        .sort_by_key(|op| matches!(op, PatchOp::RemovePattern { .. }));
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn dynamics(r: &Resolved, score: &Score, b: &mut Built, where_: &str) {
    let change = r.plan.params.change.unwrap_or(DynamicsChange::Softer);
    let amount = r.plan.params.amount_db.unwrap_or(match change {
        DynamicsChange::Softer => DEFAULT_SOFTER_DB,
        DynamicsChange::Louder => DEFAULT_LOUDER_DB,
    });
    let signed = match change {
        DynamicsChange::Softer => -amount,
        DynamicsChange::Louder => amount,
    };
    let word = match change {
        DynamicsChange::Softer => "softer",
        DynamicsChange::Louder => "louder",
    };
    for id in &r.targets {
        let Some(l) = score.layer(id) else { continue };
        if r.whole {
            let g = (l.gain_db + signed).clamp(-60.0, 24.0);
            if g == l.gain_db {
                continue;
            }
            b.patch.ops.push(PatchOp::UpdateLayer {
                id: id.clone(),
                update: LayerUpdate {
                    gain_db: Some(g),
                    ..LayerUpdate::default()
                },
            });
            b.actions.push(format!(
                "made `{id}` {word} by {amount:.0} dB (gain {:.1} -> {g:.1} dB)",
                l.gain_db
            ));
        } else {
            let mut any = false;
            for piece in pieces(score, l, r.bars) {
                let mut clip = piece.clip;
                let v = (clip.velocity_scale * db_to_gain(signed)).clamp(0.05, 2.0);
                if v == clip.velocity_scale {
                    continue;
                }
                clip.velocity_scale = v;
                b.patch.ops.push(PatchOp::PlaceClip {
                    layer: id.clone(),
                    clip,
                });
                any = true;
            }
            if any {
                b.actions
                    .push(format!("made `{id}` {word} by {amount:.0} dB {where_}"));
            }
        }
    }
}

fn steps_per_beat(score: &Score, grid: Grid) -> u32 {
    let spb = steps_per_bar(score, grid).unwrap_or(1);
    let beats = score.time_signature.numerator.max(1) as u32;
    if spb.is_multiple_of(beats) {
        (spb / beats).max(1)
    } else {
        spb.max(1)
    }
}

/// A thinner version of a pattern: accented notes and the beats (or, if that removes nothing,
/// accented notes and the downbeats). Sustaining instruments ring into the next kept note.
fn thinned(score: &Score, p: &Pattern, sustaining: bool) -> Option<Pattern> {
    let spb = steps_per_bar(score, p.grid).unwrap_or(1).max(1);
    let beat = steps_per_beat(score, p.grid);
    let accented = |a: Accent| matches!(a, Accent::Strong | Accent::Bright);
    let pass1: Vec<_> = p
        .notes
        .iter()
        .filter(|n| accented(n.accent) || (n.step % spb).is_multiple_of(beat))
        .cloned()
        .collect();
    let notes = if pass1.len() < p.notes.len() {
        pass1
    } else {
        p.notes
            .iter()
            .filter(|n| accented(n.accent) || n.step % spb == 0)
            .cloned()
            .collect()
    };
    if notes.is_empty() || notes.len() == p.notes.len() {
        return None;
    }
    let mut q = p.clone();
    q.notes = notes;
    q.spoken = None;
    if sustaining {
        let mut steps: Vec<u32> = q.notes.iter().map(|n| n.step).collect();
        steps.dedup();
        let len = q.length_steps.max(1);
        for n in &mut q.notes {
            let next = steps
                .iter()
                .find(|s| **s > n.step)
                .copied()
                .unwrap_or(steps[0] + len);
            let gap = (next - n.step) as f32;
            n.length = n.length.max(gap + 0.6).min(64.0);
        }
    }
    Some(q)
}

fn thin_out(r: &Resolved, score: &Score, catalogue: &Catalogue, b: &mut Built, where_: &str) {
    let mut added: BTreeMap<String, String> = BTreeMap::new();
    let mut new_ids: Vec<String> = Vec::new();
    for id in &r.targets {
        let Some(l) = score.layer(id) else { continue };
        let sustaining = layer_preset(catalogue, l).is_some_and(|p| p.sustaining);
        let mut any = false;
        for piece in pieces(score, l, r.bars) {
            let Some(src) = score.pattern(&piece.clip.pattern) else {
                continue;
            };
            let pid = match added.get(&src.id) {
                Some(pid) => pid.clone(),
                None => {
                    let Some(mut q) = thinned(score, src, sustaining) else {
                        continue;
                    };
                    q.id = unique_pattern_id(score, &format!("{}_thin", src.id), &new_ids);
                    new_ids.push(q.id.clone());
                    added.insert(src.id.clone(), q.id.clone());
                    let pid = q.id.clone();
                    b.patch.ops.push(PatchOp::AddPattern {
                        pattern: q,
                        index: None,
                    });
                    pid
                }
            };
            let mut clip = piece.clip;
            clip.pattern = pid;
            b.patch.ops.push(PatchOp::PlaceClip {
                layer: id.clone(),
                clip,
            });
            any = true;
        }
        if any {
            b.actions
                .push(format!("thinned `{id}` to its accents and beats {where_}"));
        }
    }
}

/// New motif material for an existing layer, in its instrument, register and dynamics.
pub fn rewritten_pattern(
    r: &Resolved,
    score: &Score,
    catalogue: &Catalogue,
    l: &Layer,
    src: &Pattern,
    seed: u64,
) -> Result<(Pattern, Vec<Repair>), String> {
    let preset = layer_preset(catalogue, l).cloned().ok_or_else(|| {
        format!(
            "layer `{}` plays `{}`, which the catalogue lacks",
            l.id, l.instrument.preset
        )
    })?;
    let params = &r.plan.params;
    let grid = params
        .grid
        .filter(|g| score.time_signature.steps_per_bar(*g).is_some())
        .unwrap_or(src.grid);
    let spb = score.time_signature.steps_per_bar(grid).unwrap_or(1).max(1);
    let length_bars = params
        .length_bars
        .unwrap_or_else(|| (src.length_steps / spb).clamp(1, 8));
    let drum = is_drum(l);
    let src_for_rhythm = if grid == src.grid { Some(src) } else { None };
    let base = match src_for_rhythm {
        Some(p) => base_spoken(score, l, p, Some(&preset)),
        None => default_spoken(rhythm_role(&preset, l.class), score.time_signature, grid),
    }
    .unwrap_or_else(|| "DON".to_string());
    let spoken = params
        .spoken
        .clone()
        .unwrap_or_else(|| vary_spoken(&base, spb, seed, preset.sustaining));
    let motif = Motif {
        grid,
        length_bars,
        spoken,
        degrees: if drum {
            vec![0]
        } else if params.degrees.is_empty() {
            degrees_for(seed)
        } else {
            params.degrees.clone()
        },
        octave: if drum { None } else { octave_of(score, src) },
        sustain_steps: None,
        follow_chords: !drum,
        notes: vec![],
    };
    let spec = MaterialSpec {
        id: l.id.clone(),
        name: l.name.clone(),
        class: l.class,
        preset,
        register: if drum {
            Register::Mid
        } else {
            register_of(score, l, src)
        },
        dynamics: params.dynamics.unwrap_or_else(|| dynamics_of(src)),
        gain_db: None,
        motif,
    };
    let m = material::build(score, &spec, catalogue)?;
    Ok((m.pattern, m.repairs))
}

/// The notes a pattern plays from step `offset` on, as (relative step, pitch, velocity bits) over
/// one full cycle, so patterns of different phase compare.
fn cycle(p: &Pattern, offset: u32) -> Vec<(u32, u8, u32)> {
    let len = p.length_steps.max(1);
    let mut v: Vec<(u32, u8, u32)> = p
        .notes
        .iter()
        .map(|n| {
            (
                (n.step + len - offset % len) % len,
                n.pitch,
                n.velocity.to_bits(),
            )
        })
        .collect();
    v.sort_unstable();
    v
}

fn rewrite(
    r: &Resolved,
    score: &Score,
    catalogue: &Catalogue,
    seed: u64,
    b: &mut Built,
    where_: &str,
) -> Result<(), Vec<EditIssue>> {
    let mut new_ids: Vec<String> = Vec::new();
    for (ti, id) in r.targets.iter().enumerate() {
        let Some(l) = score.layer(id) else { continue };
        let ps = pieces(score, l, r.bars);
        let Some(first) = ps.first() else { continue };
        let Some(src) = score.pattern(&first.clip.pattern) else {
            continue;
        };
        let mut found = None;
        let mut last_err = String::new();
        for k in 0..8u64 {
            let s = derive_seed(seed, &format!("rewrite:{id}"), k);
            match rewritten_pattern(r, score, catalogue, l, src, s) {
                Ok((p, reps)) => {
                    let new_cycle = cycle(&p, global_phase(score, &p, first.clip.bars.start));
                    let old_cycle = cycle(src, first.clip.pattern_offset_steps);
                    if new_cycle != old_cycle || r.plan.params.spoken.is_some() {
                        found = Some((p, reps));
                        break;
                    }
                }
                Err(e) => last_err = e,
            }
        }
        let Some((mut p, reps)) = found else {
            if last_err.is_empty() {
                continue;
            }
            return Err(material_issue(&format!("/layers/{ti}"), last_err));
        };
        b.repairs.extend(reps);
        p.id = unique_pattern_id(
            score,
            &format!("{id}_v{}-{}", r.bars.start, r.bars.end),
            &new_ids,
        );
        new_ids.push(p.id.clone());
        let pid = p.id.clone();
        let placed: Vec<PatchOp> = ps
            .iter()
            .map(|piece| PatchOp::PlaceClip {
                layer: id.clone(),
                clip: Clip {
                    bars: piece.clip.bars,
                    pattern: pid.clone(),
                    pattern_offset_steps: global_phase(score, &p, piece.clip.bars.start),
                    transpose: 0,
                    velocity_scale: piece.clip.velocity_scale,
                },
            })
            .collect();
        b.patch.ops.push(PatchOp::AddPattern {
            pattern: p,
            index: None,
        });
        b.patch.ops.extend(placed);
        b.actions.push(format!("gave `{id}` a new motif {where_}"));
    }
    Ok(())
}

fn change_instrument(
    r: &Resolved,
    score: &Score,
    b: &mut Built,
    where_: &str,
) -> Result<(), Vec<EditIssue>> {
    let Some(preset) = r.preset.clone() else {
        return Err(material_issue(
            "/params/preset",
            "no instrument to change to".into(),
        ));
    };
    let inst = InstrumentRef {
        kind: preset.kind,
        preset: preset.name.clone(),
        params: Default::default(),
    };
    for id in &r.targets {
        let Some(l) = score.layer(id) else { continue };
        let class = new_class(&r.plan, &preset);
        let class = if crate::plan::class_is_intrinsic(preset.class)
            || crate::plan::class_is_intrinsic(l.class)
        {
            class
        } else {
            l.class
        };
        if r.whole {
            let mut clips = l.clips.clone();
            let mut shifted = false;
            for c in &mut clips {
                if let Some(p) = score.pattern(&c.pattern) {
                    let s = range_shift(p, c.transpose, &preset);
                    if s != 0 {
                        c.transpose = c.transpose.saturating_add(s);
                        shifted = true;
                    }
                }
            }
            let rename = l.name == display_name(&l.instrument.preset);
            b.patch.ops.push(PatchOp::UpdateLayer {
                id: id.clone(),
                update: LayerUpdate {
                    name: rename.then(|| display_name(&preset.name)),
                    class: (class != l.class).then_some(class),
                    instrument: Some(inst.clone()),
                    ..LayerUpdate::default()
                },
            });
            if shifted {
                b.patch.ops.push(PatchOp::SetClips {
                    layer: id.clone(),
                    clips,
                });
            }
            b.actions.push(format!(
                "`{id}` now plays {} instead of {}",
                preset.name.replace('_', " "),
                l.instrument.preset.replace('_', " ")
            ));
        } else {
            let ps = pieces(score, l, r.bars);
            if ps.is_empty() {
                continue;
            }
            let new_id = unique_layer_id(
                score,
                &format!("{id}_{}", slug(&preset.name, "layer")),
                &b.new_layers,
            );
            let mut layer = l.clone();
            layer.id = new_id.clone();
            layer.name = format!("{} ({})", display_name(&preset.name), l.name);
            layer.class = class;
            layer.instrument = inst.clone();
            layer.locked = false;
            layer.muted = false;
            layer.solo = false;
            layer.automation = vec![];
            layer.clips = ps
                .into_iter()
                .map(|piece| {
                    let mut c = piece.clip;
                    if let Some(p) = score.pattern(&c.pattern) {
                        c.transpose =
                            c.transpose
                                .saturating_add(range_shift(p, c.transpose, &preset));
                    }
                    c
                })
                .collect();
            b.patch.ops.push(PatchOp::ClearBars {
                layer: id.clone(),
                bars: r.bars,
            });
            b.patch.ops.push(PatchOp::AddLayer { layer, index: None });
            b.actions.push(format!(
                "{} plays `{id}`'s part {where_} (new layer `{new_id}`)",
                preset.name.replace('_', " ")
            ));
            b.new_layers.push(new_id);
        }
    }
    Ok(())
}

// ---- tone and space ----

/// Id of the whole-layer low-pass a darker edit adds when the preset has no tone controls.
pub const TONE_FILTER_ID: &str = "tone_lp";
/// Id of the high-shelf EQ a brighter edit adds when the preset's tone is at its top.
pub const AIR_EQ_ID: &str = "air";
/// Id of the room reverb a wetter edit adds.
pub const ROOM_REVERB_ID: &str = "room";
/// Lowest cutoff a darker edit moves the tone low-pass to.
pub const MIN_TONE_LP_HZ: f64 = 250.0;
/// A brighter edit removes the tone low-pass once its cutoff would reach this.
pub const OPEN_TONE_LP_HZ: f64 = 16_000.0;
/// High-pass cutoff of a brighter edit over some bars (a tilt mixed in by `mix`).
pub const BAR_HP_HZ: f64 = 800.0;

/// Change of the preset's `tone` (and `felt`) parameter.
pub fn tone_step(a: ChangeAmount) -> f64 {
    match a {
        ChangeAmount::ALittle => 0.15,
        ChangeAmount::Normal => 0.3,
        ChangeAmount::Much => 0.45,
    }
}

/// Factor on the tone low-pass cutoff per darker step (its inverse per brighter step).
fn lp_factor(a: ChangeAmount) -> f64 {
    match a {
        ChangeAmount::ALittle => 0.75,
        ChangeAmount::Normal => 0.5,
        ChangeAmount::Much => 0.33,
    }
}

/// Cutoff of a new whole-layer tone low-pass.
fn new_lp_hz(a: ChangeAmount) -> f64 {
    match a {
        ChangeAmount::ALittle => 6000.0,
        ChangeAmount::Normal => 3000.0,
        ChangeAmount::Much => 1500.0,
    }
}

/// Cutoff of the low-pass a darker edit switches on over some bars.
pub fn bar_lp_hz(a: ChangeAmount) -> f64 {
    match a {
        ChangeAmount::ALittle => 3000.0,
        ChangeAmount::Normal => 1500.0,
        ChangeAmount::Much => 800.0,
    }
}

/// Wet fraction of the high-pass tilt a brighter edit switches on over some bars.
fn bar_hp_mix(a: ChangeAmount) -> f64 {
    match a {
        ChangeAmount::ALittle => 0.35,
        ChangeAmount::Normal => 0.6,
        ChangeAmount::Much => 0.85,
    }
}

/// High-shelf boost of the air EQ per brighter step, in dB.
fn air_db(a: ChangeAmount) -> f64 {
    match a {
        ChangeAmount::ALittle => 2.0,
        ChangeAmount::Normal => 4.0,
        ChangeAmount::Much => 6.0,
    }
}

/// Reverb mix added per wetter step.
fn wet_step(a: ChangeAmount) -> f64 {
    match a {
        ChangeAmount::ALittle => 0.06,
        ChangeAmount::Normal => 0.12,
        ChangeAmount::Much => 0.2,
    }
}

/// Factor on the reverb mix per drier step.
fn drier_factor(a: ChangeAmount) -> f64 {
    match a {
        ChangeAmount::ALittle => 0.75,
        ChangeAmount::Normal => 0.55,
        ChangeAmount::Much => 0.3,
    }
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// An effect parameter, or the parameter table's default.
fn fx_param(e: &Effect, name: &str) -> f64 {
    e.params.get(name).copied().unwrap_or_else(|| {
        praxis_score::params::effect_param(e.kind, name)
            .map(|s| s.default)
            .unwrap_or(0.0)
    })
}

fn effect(id: &str, kind: EffectKind, pairs: &[(&str, f64)]) -> Effect {
    Effect {
        id: id.to_string(),
        kind,
        params: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        bypass: false,
        sidechain: None,
        impulse: None,
    }
}

/// An effect id not used in `effects`.
fn free_effect_id(effects: &[Effect], base: &str) -> String {
    let free = |id: &str| !effects.iter().any(|e| e.id == id);
    if free(base) {
        return base.to_string();
    }
    (2..)
        .map(|i| format!("{base}_{i}"))
        .find(|id| free(id))
        .unwrap_or_else(|| base.to_string())
}

/// True when an automation lane drives one of the effect's parameters.
fn automated(automation: &[Automation], effect_id: &str) -> bool {
    let prefix = format!("effects.{effect_id}.");
    automation.iter().any(|a| a.target.starts_with(&prefix))
}

fn is_reverb(e: &Effect) -> bool {
    matches!(e.kind, EffectKind::Reverb | EffectKind::ConvolutionReverb)
}

/// A step lane that holds `outside` and switches to `inside` over exactly `bars`.
fn bar_lane(
    target: String,
    bars: BarRange,
    total_bars: u32,
    outside: f64,
    inside: f64,
) -> Automation {
    let mut points = Vec::new();
    if bars.start > 1 {
        points.push(AutoPoint {
            bar: 1.0,
            value: outside,
        });
    }
    points.push(AutoPoint {
        bar: bars.start as f64,
        value: inside,
    });
    if bars.end < total_bars {
        points.push(AutoPoint {
            bar: (bars.end + 1) as f64,
            value: outside,
        });
    }
    Automation {
        target,
        curve: Curve::Step,
        points,
    }
}

/// The `update_layer` op for a changed instrument, effects and automation (None when nothing
/// changed).
fn settings_op(
    l: &Layer,
    instrument: InstrumentRef,
    effects: Vec<Effect>,
    automation: Vec<Automation>,
) -> Option<PatchOp> {
    let update = LayerUpdate {
        instrument: (instrument != l.instrument).then_some(instrument),
        effects: (effects != l.effects).then_some(effects),
        automation: (automation != l.automation).then_some(automation),
        ..LayerUpdate::default()
    };
    if update == LayerUpdate::default() {
        return None;
    }
    Some(PatchOp::UpdateLayer {
        id: l.id.clone(),
        update,
    })
}

fn tone_word(t: ToneChange) -> &'static str {
    match t {
        ToneChange::Darker => "darker",
        ToneChange::Brighter => "brighter",
    }
}

fn tone(r: &Resolved, score: &Score, catalogue: &Catalogue, b: &mut Built, where_: &str) {
    let dir = r.plan.params.tone.unwrap_or(ToneChange::Darker);
    let amount = r.plan.params.amount.unwrap_or_default();
    for id in &r.targets {
        let Some(l) = score.layer(id) else { continue };
        let changed = if r.whole {
            whole_tone(l, catalogue, dir, amount)
        } else {
            bars_tone(l, score, r.bars, dir, amount)
        };
        if let Some((op, how)) = changed {
            b.patch.ops.push(op);
            b.actions
                .push(format!("made `{id}` {} {where_} ({how})", tone_word(dir)));
        }
    }
}

/// A whole-layer tone change: first undo an earlier effect-level change that went the other
/// way, then move the preset's tone controls, then fall back to a low-pass or an air EQ.
fn whole_tone(
    l: &Layer,
    catalogue: &Catalogue,
    dir: ToneChange,
    amount: ChangeAmount,
) -> Option<(PatchOp, String)> {
    let mut effects = l.effects.clone();
    let mut inst = l.instrument.clone();
    let mut how: Option<String> = None;
    let lp = effects.iter().position(|e| {
        e.id == TONE_FILTER_ID
            && e.kind == EffectKind::Filter
            && fx_param(e, "mode") == 0.0
            && !automated(&l.automation, &e.id)
    });
    let air = effects
        .iter()
        .position(|e| e.id == AIR_EQ_ID && e.kind == EffectKind::Eq);
    match (dir, lp, air) {
        (ToneChange::Darker, _, Some(i)) => {
            let db = round3(fx_param(&effects[i], "high_db") - air_db(amount));
            if db <= 0.05 {
                effects.remove(i);
                how = Some("took off the air boost".into());
            } else {
                effects[i].params.insert("high_db".into(), db);
                how = Some(format!("air boost down to {db:.1} dB"));
            }
        }
        (ToneChange::Brighter, Some(i), _) => {
            let hz = (fx_param(&effects[i], "cutoff_hz") / lp_factor(amount)).round();
            if hz >= OPEN_TONE_LP_HZ {
                effects.remove(i);
                how = Some("took off the tone low-pass".into());
            } else {
                effects[i].params.insert("cutoff_hz".into(), hz);
                how = Some(format!("tone low-pass opened to {hz:.0} Hz"));
            }
        }
        _ => {}
    }
    if how.is_none()
        && let Some(tc) = layer_preset(catalogue, l).and_then(|p| p.tone)
    {
        let step = tone_step(amount);
        let sign = match dir {
            ToneChange::Darker => -1.0,
            ToneChange::Brighter => 1.0,
        };
        let mut parts = Vec::new();
        let t0 = inst
            .params
            .get(ToneControls::TONE)
            .copied()
            .unwrap_or(tc.tone_default);
        let t1 = round3((t0 + sign * step).clamp(0.0, 1.0));
        if t1 != t0 {
            inst.params.insert(ToneControls::TONE.into(), t1);
            parts.push(format!("tone {t0:.2} -> {t1:.2}"));
        }
        if let Some(fd) = tc.felt_default {
            let f0 = inst.params.get(ToneControls::FELT).copied().unwrap_or(fd);
            let f1 = round3((f0 - sign * step).clamp(0.0, 1.0));
            if f1 != f0 {
                inst.params.insert(ToneControls::FELT.into(), f1);
                parts.push(format!("felt {f0:.2} -> {f1:.2}"));
            }
        }
        if !parts.is_empty() {
            how = Some(parts.join(", "));
        }
    }
    if how.is_none() {
        match (dir, lp, air) {
            (ToneChange::Darker, Some(i), _) => {
                let old = fx_param(&effects[i], "cutoff_hz");
                let hz = (old * lp_factor(amount)).max(MIN_TONE_LP_HZ).round();
                if hz < old {
                    effects[i].params.insert("cutoff_hz".into(), hz);
                    how = Some(format!("tone low-pass {old:.0} -> {hz:.0} Hz"));
                }
            }
            (ToneChange::Darker, None, _) => {
                let fid = free_effect_id(&effects, TONE_FILTER_ID);
                let hz = new_lp_hz(amount);
                effects.insert(
                    0,
                    effect(
                        &fid,
                        EffectKind::Filter,
                        &[
                            ("mode", 0.0),
                            ("cutoff_hz", hz),
                            ("resonance", 0.0),
                            ("mix", 1.0),
                        ],
                    ),
                );
                how = Some(format!("a {hz:.0} Hz low-pass"));
            }
            (ToneChange::Brighter, _, Some(i)) => {
                let old = fx_param(&effects[i], "high_db");
                let db = (old + air_db(amount)).min(12.0);
                if db > old {
                    effects[i].params.insert("high_db".into(), db);
                    how = Some(format!("air boost {old:.1} -> {db:.1} dB"));
                }
            }
            (ToneChange::Brighter, _, None) => {
                let fid = free_effect_id(&effects, AIR_EQ_ID);
                let db = air_db(amount);
                effects.insert(
                    0,
                    effect(
                        &fid,
                        EffectKind::Eq,
                        &[("high_hz", 5000.0), ("high_db", db)],
                    ),
                );
                how = Some(format!("a +{db:.0} dB air shelf at 5 kHz"));
            }
        }
    }
    let how = how?;
    settings_op(l, inst, effects, l.automation.clone()).map(|op| (op, how))
}

/// A tone change over some bars: a filter at `mix` 0 that a step lane switches on in exactly
/// those bars (a low-pass for darker, a high-pass tilt for brighter). An earlier change over the
/// same bars is deepened, or taken off when it went the other way.
fn bars_tone(
    l: &Layer,
    score: &Score,
    bars: BarRange,
    dir: ToneChange,
    amount: ChangeAmount,
) -> Option<(PatchOp, String)> {
    let mut effects = l.effects.clone();
    let mut automation = l.automation.clone();
    let base = format!("tone_b{}_{}", bars.start, bars.end);
    let range = bars_text(bars);
    let how;
    if let Some(i) = effects
        .iter()
        .position(|e| e.id == base && e.kind == EffectKind::Filter)
    {
        let darker_filter = fx_param(&effects[i], "mode") == 0.0;
        let old = fx_param(&effects[i], "cutoff_hz");
        if darker_filter != (dir == ToneChange::Darker) {
            effects.remove(i);
            let prefix = format!("effects.{base}.");
            automation.retain(|a| !a.target.starts_with(&prefix));
            how = format!("took off the earlier tone change in {range}");
        } else if dir == ToneChange::Darker {
            let hz = (old * lp_factor(amount)).max(MIN_TONE_LP_HZ).round();
            if hz >= old {
                return None;
            }
            effects[i].params.insert("cutoff_hz".into(), hz);
            how = format!("low-pass in {range} {old:.0} -> {hz:.0} Hz");
        } else {
            let hz = (old / lp_factor(amount)).min(4000.0).round();
            if hz <= old {
                return None;
            }
            effects[i].params.insert("cutoff_hz".into(), hz);
            how = format!("high-pass tilt in {range} {old:.0} -> {hz:.0} Hz");
        }
    } else {
        let fid = free_effect_id(&effects, &base);
        let (mode, hz, wet) = match dir {
            ToneChange::Darker => (0.0, bar_lp_hz(amount), 1.0),
            ToneChange::Brighter => (1.0, BAR_HP_HZ, bar_hp_mix(amount)),
        };
        effects.insert(
            0,
            effect(
                &fid,
                EffectKind::Filter,
                &[
                    ("mode", mode),
                    ("cutoff_hz", hz),
                    ("resonance", 0.0),
                    ("mix", 0.0),
                ],
            ),
        );
        automation.push(bar_lane(
            format!("effects.{fid}.mix"),
            bars,
            score.bars,
            0.0,
            wet,
        ));
        how = match dir {
            ToneChange::Darker => format!("a {hz:.0} Hz low-pass switched on in {range} only"),
            ToneChange::Brighter => format!(
                "a {hz:.0} Hz high-pass tilt at {:.0}% switched on in {range} only",
                wet * 100.0
            ),
        };
    }
    settings_op(l, l.instrument.clone(), effects, automation).map(|op| (op, how))
}

fn space(r: &Resolved, score: &Score, b: &mut Built, where_: &str) {
    let dir = r.plan.params.space.unwrap_or(SpaceChange::Wetter);
    let amount = r.plan.params.amount.unwrap_or_default();
    for id in &r.targets {
        let Some(l) = score.layer(id) else { continue };
        let changed = if r.whole {
            whole_space(l, dir, amount)
        } else {
            bars_space(l, score, r.bars, dir, amount)
        };
        if let Some((op, how)) = changed {
            b.patch.ops.push(op);
            b.actions.push(format!("{how} {where_}"));
        }
    }
}

/// The arranger's room reverb (`arrange::default_effects`), bigger for "much"; `mix` overrides
/// its wet fraction.
fn room_reverb(id: &str, amount: ChangeAmount, mix: Option<f64>) -> Effect {
    let (decay, size, wet) = match amount {
        ChangeAmount::Much => (2.6, 0.6, 0.26),
        _ => (1.8, 0.35, 0.18),
    };
    effect(
        id,
        EffectKind::Reverb,
        &[
            ("decay_s", decay),
            ("predelay_ms", 12.0),
            ("size", size),
            ("mix", mix.unwrap_or(wet)),
        ],
    )
}

fn whole_space(l: &Layer, dir: SpaceChange, amount: ChangeAmount) -> Option<(PatchOp, String)> {
    let id = &l.id;
    let mut effects = l.effects.clone();
    let mut automation = l.automation.clone();
    let reverbs: Vec<usize> = (0..effects.len())
        .filter(|&i| is_reverb(&effects[i]))
        .collect();
    let how = match dir {
        SpaceChange::Dry => {
            if reverbs.is_empty() {
                return None;
            }
            let gone: Vec<String> = reverbs
                .iter()
                .map(|&i| format!("effects.{}.", effects[i].id))
                .collect();
            effects.retain(|e| !is_reverb(e));
            automation.retain(|a| !gone.iter().any(|g| a.target.starts_with(g.as_str())));
            format!("took the reverb off `{id}` (dry)")
        }
        SpaceChange::Drier => {
            if reverbs.is_empty() {
                return None;
            }
            let f = drier_factor(amount);
            let mut parts = Vec::new();
            for &i in &reverbs {
                let e = &mut effects[i];
                let m0 = fx_param(e, "mix");
                let m1 = round3(m0 * f);
                e.params.insert("mix".into(), m1);
                if e.kind == EffectKind::Reverb {
                    let d0 = fx_param(e, "decay_s");
                    let d1 = round3((d0 * (0.5 + f / 2.0)).max(0.3));
                    let s0 = fx_param(e, "size");
                    let s1 = round3((s0 - (1.0 - f) / 2.0).max(0.0));
                    e.params.insert("decay_s".into(), d1);
                    e.params.insert("size".into(), s1);
                    parts.push(format!(
                        "`{}` mix {m0:.2} -> {m1:.2}, decay {d0:.1} -> {d1:.1} s",
                        e.id
                    ));
                } else {
                    parts.push(format!("`{}` mix {m0:.2} -> {m1:.2}", e.id));
                }
            }
            format!("made `{id}` drier ({})", parts.join("; "))
        }
        SpaceChange::Wetter if reverbs.is_empty() => {
            let rid = free_effect_id(&effects, ROOM_REVERB_ID);
            let room = room_reverb(&rid, amount, None);
            let how = format!(
                "gave `{id}` a room reverb (mix {:.2}, decay {:.1} s)",
                fx_param(&room, "mix"),
                fx_param(&room, "decay_s")
            );
            effects.push(room);
            how
        }
        SpaceChange::Wetter => {
            let step = wet_step(amount);
            let grow = 1.0 + step * 2.5;
            let mut parts = Vec::new();
            for &i in &reverbs {
                let e = &mut effects[i];
                let m0 = fx_param(e, "mix");
                let m1 = round3((m0 + step).min(0.6));
                e.params.insert("mix".into(), m1);
                if e.kind == EffectKind::Reverb {
                    let d0 = fx_param(e, "decay_s");
                    let d1 = round3((d0 * grow).min(8.0));
                    let s0 = fx_param(e, "size");
                    let s1 = round3((s0 + step).min(1.0));
                    e.params.insert("decay_s".into(), d1);
                    e.params.insert("size".into(), s1);
                    parts.push(format!(
                        "`{}` mix {m0:.2} -> {m1:.2}, decay {d0:.1} -> {d1:.1} s",
                        e.id
                    ));
                } else {
                    let d0 = fx_param(e, "decay_scale");
                    e.params
                        .insert("decay_scale".into(), round3((d0 * grow).min(2.0)));
                    parts.push(format!("`{}` mix {m0:.2} -> {m1:.2}", e.id));
                }
            }
            format!("gave `{id}` more room ({})", parts.join("; "))
        }
    };
    settings_op(l, l.instrument.clone(), effects, automation).map(|op| (op, how))
}

/// A space change over some bars: a step lane on each reverb's `mix` that holds its own value
/// outside the bars (a new room reverb sits at `mix` 0 outside them).
fn bars_space(
    l: &Layer,
    score: &Score,
    bars: BarRange,
    dir: SpaceChange,
    amount: ChangeAmount,
) -> Option<(PatchOp, String)> {
    let id = &l.id;
    let mut effects = l.effects.clone();
    let mut automation = l.automation.clone();
    let range = bars_text(bars);
    let mut parts = Vec::new();
    let reverbs: Vec<usize> = (0..effects.len())
        .filter(|&i| is_reverb(&effects[i]))
        .collect();
    if reverbs.is_empty() {
        if dir != SpaceChange::Wetter {
            return None;
        }
        let rid = free_effect_id(&effects, ROOM_REVERB_ID);
        let wet = fx_param(&room_reverb(&rid, amount, None), "mix");
        effects.push(room_reverb(&rid, amount, Some(0.0)));
        automation.push(bar_lane(
            format!("effects.{rid}.mix"),
            bars,
            score.bars,
            0.0,
            wet,
        ));
        parts.push(format!(
            "a room reverb (mix {wet:.2}) switched on in {range} only"
        ));
    } else {
        for &i in &reverbs {
            let e = &effects[i];
            let target = format!("effects.{}.mix", e.id);
            if automation.iter().any(|a| a.target == target) {
                continue;
            }
            let m0 = fx_param(e, "mix");
            let m1 = round3(match dir {
                SpaceChange::Wetter => (m0 + wet_step(amount)).min(0.6),
                SpaceChange::Drier => m0 * drier_factor(amount),
                SpaceChange::Dry => 0.0,
            });
            if m1 == m0 {
                continue;
            }
            automation.push(bar_lane(target, bars, score.bars, m0, m1));
            parts.push(format!("`{}` mix {m0:.2} -> {m1:.2} in {range} only", e.id));
        }
    }
    if parts.is_empty() {
        return None;
    }
    let word = match dir {
        SpaceChange::Wetter => "more room",
        SpaceChange::Drier => "less reverb",
        SpaceChange::Dry => "no reverb",
    };
    let how = format!("gave `{id}` {word} ({})", parts.join("; "));
    settings_op(l, l.instrument.clone(), effects, automation).map(|op| (op, how))
}
