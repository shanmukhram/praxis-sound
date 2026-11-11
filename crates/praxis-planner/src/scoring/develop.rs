//! The arranger's scored path: after the base pattern of every layer is built, each bar of each
//! layer is assigned what it plays, and clips are cut from those assignments.
//!
//! - **Density by intensity** (the motif develops, recognisably): below 0.4 a layer plays its
//!   sparse form (the notes on beats and the accented ones, sustain reaching the next kept
//!   note), up to 0.75 the motif as written, above that the lifted form (accented notes of a
//!   melodic layer doubled an octave up). Every form keeps the motif's pitches and accents.
//! - **Dynamics**: each bar's clip `velocity_scale` follows the tension curve (0.8 + 0.3 x
//!   intensity); a build ramps bar by bar towards the next part.
//! - **Filter**: pitched, sustaining, non-bass layers get a low-pass `tension` (resonance 0) whose
//!   cutoff follows the curve (1800 Hz x 2^(3 x intensity), 1.2-16 kHz).
//! - **Transitions** into a part: `drop` (the bar before plays only the lead layer), `hold` (the
//!   bar before holds its first chord for the whole bar, drums rest), `pickup` (a layer that
//!   enters plays the last beat of the bar before). Never in a bar that holds a hit.
//! - **Hits**: in the hit's bar the anchor layer (chords, else the motif or melody, else the bass)
//!   plays a strong chord of the bar's harmony on the hit's sixteenth, nudged by the note offset
//!   so its onset is exactly the hit; a playing bass adds its root and a playing kick a kick.
//!   The Score gets a `hit` marker at each hit's time.

use super::harmony::{triad_pcs, voice_lead};
use super::hits::{Landing, landing};
use crate::catalogue::PresetSpec;
use crate::plan::{PlanSection, ScorePlan, Transition};
use praxis_score::validate::unpitched_class;
use praxis_score::{
    Accent, AutoPoint, Automation, BarRange, Clip, Curve, Effect, EffectKind, GeneratorClass,
    InstrumentKind, Marker, MarkerKind, Note, Pattern, Score, SectionRole,
};
use std::collections::{BTreeMap, BTreeSet};

/// What the arranger knows about one layer of the Score.
#[derive(Clone, Debug)]
pub struct LayerInfo {
    /// Index into `score.layers`.
    pub layer: usize,
    /// The plan layer's id (sections list these).
    pub plan_id: String,
    pub preset: PresetSpec,
    pub class: GeneratorClass,
    /// Octave of degree 0 for this layer.
    pub octave: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Band {
    Sparse,
    Full,
    Lifted,
}

fn band(x: f32) -> Band {
    if x < 0.4 {
        Band::Sparse
    } else if x < 0.75 {
        Band::Full
    } else {
        Band::Lifted
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Slot {
    Silent,
    Base(Band),
    Pickup,
    Hold,
    /// A one-bar pattern already built (hit bars).
    Bar(Pattern),
}

/// Velocity scale for an intensity, in steps of 0.05.
pub fn velocity_scale(x: f32) -> f32 {
    let v = (0.8 + 0.3 * x).clamp(0.6, 1.1);
    (v * 20.0).round() / 20.0
}

/// Low-pass cutoff for an intensity.
pub fn cutoff_hz(x: f32) -> f64 {
    let c = (1800.0 * 2f64.powf(3.0 * x as f64)).clamp(1200.0, 16000.0);
    (c / 10.0).round() * 10.0
}

/// Steps of `grid` per beat of `ts` (dotted quarter in compound metres).
pub fn beat_steps(ts: praxis_score::TimeSignature, spb: u32) -> u32 {
    let beats = if ts.denominator == 8 && ts.numerator.is_multiple_of(3) && ts.numerator > 3 {
        (ts.numerator / 3) as u32
    } else {
        ts.numerator.max(1) as u32
    };
    if spb.is_multiple_of(beats) {
        (spb / beats).max(1)
    } else {
        1
    }
}

/// Notes of `p` in the bar that starts at pattern step `phase` (local steps 0..spb).
fn bar_notes(p: &Pattern, phase: u32, spb: u32) -> Vec<Note> {
    p.notes
        .iter()
        .filter(|n| n.step >= phase && n.step < phase + spb)
        .map(|n| {
            let mut n = n.clone();
            n.step -= phase;
            n
        })
        .collect()
}

/// Largest silence between one note's ring end and the next onset in a repeating pattern, in
/// steps.
fn max_gap(notes: &[Note], len: u32) -> f32 {
    if notes.is_empty() {
        return len as f32;
    }
    let mut v: Vec<(f32, f32)> = notes
        .iter()
        .map(|n| {
            (
                n.step as f32 + n.offset,
                n.step as f32 + n.offset + n.length,
            )
        })
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut ring = v[0].1;
    let mut worst = 0.0f32;
    for &(s, e) in &v[1..] {
        worst = worst.max(s - ring);
        ring = ring.max(e);
    }
    worst.max(v[0].0 + len as f32 - ring)
}

/// The sparse form: notes on beats and accented notes; sustaining notes ring to the next kept
/// onset. `None` when it would leave more than a beat of silence or equals the motif.
fn sparse(p: &Pattern, beat: u32, sustaining: bool, id: &str) -> Option<Pattern> {
    let kept: Vec<Note> = p
        .notes
        .iter()
        .filter(|n| n.step % beat == 0 || matches!(n.accent, Accent::Strong | Accent::Bright))
        .cloned()
        .collect();
    if kept.len() == p.notes.len() || kept.is_empty() {
        return None;
    }
    let onsets: BTreeSet<u32> = kept.iter().map(|n| n.step).collect();
    let mut notes = kept;
    if sustaining {
        for n in &mut notes {
            let next = onsets
                .range(n.step + 1..)
                .next()
                .copied()
                .unwrap_or(p.length_steps + onsets.first().copied().unwrap_or(0));
            let reach = (next - n.step) as f32 + 0.6;
            if reach > n.length {
                n.length = reach.min(64.0);
            }
        }
    }
    if max_gap(&notes, p.length_steps) > (beat as f32).max(max_gap(&p.notes, p.length_steps)) {
        return None;
    }
    Some(Pattern {
        id: id.to_string(),
        grid: p.grid,
        length_steps: p.length_steps,
        notes,
        spoken: None,
    })
}

/// The lifted form of a melodic layer: accented notes doubled an octave up (in range).
fn lifted(p: &Pattern, preset: &PresetSpec, id: &str) -> Option<Pattern> {
    let mut notes = p.notes.clone();
    let mut added = false;
    for n in &p.notes {
        if !matches!(n.accent, Accent::Strong | Accent::Bright) {
            continue;
        }
        let up = n.pitch as i32 + 12;
        if up > preset.range.1 as i32
            || notes
                .iter()
                .any(|m| m.step == n.step && m.pitch as i32 == up)
        {
            continue;
        }
        notes.push(Note {
            step: n.step,
            pitch: up as u8,
            velocity: ((n.velocity * 0.75 * 1000.0).round() / 1000.0).max(0.05),
            accent: Accent::Soft,
            length: n.length,
            offset: n.offset,
        });
        added = true;
    }
    if !added {
        return None;
    }
    notes.sort_by_key(|n| (n.step, n.pitch));
    Some(Pattern {
        id: id.to_string(),
        grid: p.grid,
        length_steps: p.length_steps,
        notes,
        spoken: p.spoken.clone(),
    })
}

fn melodic(class: GeneratorClass) -> bool {
    matches!(
        class,
        GeneratorClass::Melody | GeneratorClass::Motif | GeneratorClass::Arp
    )
}

fn is_kick(preset: &PresetSpec) -> bool {
    preset.kind == InstrumentKind::Drum
        && (preset.gm_note.is_none() || matches!(preset.gm_note, Some(35 | 36 | 41 | 43 | 45)))
}

fn anchor_rank(class: GeneratorClass) -> Option<u32> {
    use GeneratorClass as C;
    match class {
        C::Chords | C::Harmony => Some(0),
        C::Motif | C::Melody => Some(1),
        C::Arp => Some(2),
        C::Bass => Some(3),
        C::Drone | C::Texture | C::Ambience | C::Pad => Some(4),
        _ => None,
    }
}

/// How the music moves into section `k` (> 0).
fn transition_into(plan: &ScorePlan, secs: &[&PlanSection], k: usize) -> Transition {
    let prev = secs[k - 1];
    let cur = secs[k];
    let explicit = if plan.form.len() == secs.len() {
        plan.form[k].transition
    } else {
        Transition::Auto
    };
    let entering = cur.layers.iter().any(|l| !prev.layers.contains(l));
    match explicit {
        Transition::Auto => {
            if cur.role == SectionRole::Peak && prev.layers.len() >= 2 {
                Transition::Drop
            } else if matches!(cur.role, SectionRole::Release | SectionRole::Outro)
                && cur.intensity < prev.intensity
            {
                Transition::Hold
            } else if entering && super::rising(cur.role) {
                Transition::Pickup
            } else {
                Transition::None
            }
        }
        Transition::Drop if prev.layers.len() < 2 => Transition::None,
        t => t,
    }
}

/// Expands the scored plan's arrangement into the layers' clips, variant patterns, filter
/// curves and hit accents, and adds the hit markers.
pub fn develop(score: &mut Score, plan: &ScorePlan, infos: &[LayerInfo], chord_roots: &[i32]) {
    let total = score.bars;
    let ts = score.time_signature;
    let key = score.key;
    let mut secs: Vec<&PlanSection> = plan.sections.iter().collect();
    secs.sort_by_key(|s| s.bars.start);
    if secs.is_empty() || total == 0 {
        return;
    }
    // Section of each bar (index 0 = bar 1) and its intensity on the tension curve.
    let mut sec_of = vec![0usize; total as usize];
    for (k, s) in secs.iter().enumerate() {
        for b in s.bars.start.max(1)..=s.bars.end.min(total) {
            sec_of[(b - 1) as usize] = k;
        }
    }
    let intensity_at = |b: u32| -> f32 {
        let k = sec_of[(b - 1) as usize];
        let s = secs[k];
        let x = s.intensity.clamp(0.0, 1.0);
        if s.role == SectionRole::Build
            && let Some(next) = secs.get(k + 1)
        {
            let len = (s.bars.end - s.bars.start + 1).max(1) as f32;
            let t = (b - s.bars.start) as f32 / len;
            return x + (next.intensity.clamp(0.0, 1.0) - x) * t;
        }
        x
    };

    // Hits.
    let grid = super::hit_grid(plan);
    let hit_spb = ts.steps_per_bar(grid).unwrap_or(16);
    let landings: Vec<(f64, String, Landing)> = plan
        .hits
        .iter()
        .filter(|h| h.time_s.is_finite() && h.time_s > 0.0)
        .filter_map(|h| {
            landing(h.time_s, ts, score.tempo.bpm, grid)
                .filter(|l| l.bar <= total)
                .map(|l| (h.time_s, h.label.clone(), l))
        })
        .collect();
    let hit_bars: BTreeSet<u32> = landings.iter().map(|(_, _, l)| l.bar).collect();

    // Transitions: bar -> kind (the last bar of the part before).
    let mut transitions: BTreeMap<u32, (Transition, usize)> = BTreeMap::new();
    for k in 1..secs.len() {
        let t = transition_into(plan, &secs, k);
        let bar = secs[k].bars.start - 1;
        let prev = secs[k - 1];
        if t == Transition::None || bar <= prev.bars.start || hit_bars.contains(&bar) {
            continue;
        }
        transitions.insert(bar, (t, k));
    }

    // The lead layer of a bar: the first plan layer playing in it.
    let order: Vec<&str> = plan.layers.iter().map(|l| l.id.as_str()).collect();
    let lead_at = |b: u32| -> Option<&str> {
        let s = secs[sec_of[(b - 1) as usize]];
        order
            .iter()
            .copied()
            .find(|id| s.layers.iter().any(|l| l == id))
    };

    // Per layer: the slot of every bar.
    let mut slots: Vec<Vec<Slot>> = Vec::with_capacity(infos.len());
    for info in infos {
        let pitched = info.preset.pitched && !unpitched_class(info.class);
        let mut v = Vec::with_capacity(total as usize);
        for b in 1..=total {
            let k = sec_of[(b - 1) as usize];
            let active = secs[k].layers.contains(&info.plan_id);
            let normal = if active {
                Slot::Base(band(intensity_at(b)))
            } else {
                Slot::Silent
            };
            let slot = match transitions.get(&b) {
                Some((Transition::Drop, _)) if active => {
                    if lead_at(b) == Some(info.plan_id.as_str()) {
                        normal
                    } else {
                        Slot::Silent
                    }
                }
                Some((Transition::Hold, _)) if active => {
                    if pitched {
                        Slot::Hold
                    } else {
                        Slot::Silent
                    }
                }
                Some((Transition::Pickup, k2)) if !active => {
                    if pitched && secs[*k2].layers.contains(&info.plan_id) {
                        Slot::Pickup
                    } else {
                        Slot::Silent
                    }
                }
                _ => normal,
            };
            v.push(slot);
        }
        slots.push(v);
    }

    // Base patterns and their variants.
    struct Forms {
        base: Pattern,
        spb: u32,
        sparse: Option<Pattern>,
        lifted: Option<Pattern>,
    }
    let mut forms: Vec<Option<Forms>> = Vec::with_capacity(infos.len());
    for info in infos {
        let layer = &score.layers[info.layer];
        let base = layer
            .clips
            .first()
            .and_then(|c| score.pattern(&c.pattern))
            .cloned();
        let Some(base) = base else {
            forms.push(None);
            continue;
        };
        let Some(spb) = ts.steps_per_bar(base.grid) else {
            forms.push(None);
            continue;
        };
        let beat = beat_steps(ts, spb);
        let sustaining = info.preset.sustaining;
        let sparse_p = sparse(&base, beat, sustaining, &format!("{}_sparse", layer.id));
        let lifted_p = if melodic(info.class) && info.preset.pitched {
            lifted(&base, &info.preset, &format!("{}_lifted", layer.id))
        } else {
            None
        };
        forms.push(Some(Forms {
            base,
            spb,
            sparse: sparse_p,
            lifted: lifted_p,
        }));
    }

    // Notes a slot plays in bar `b`, local to the bar, on the layer's grid.
    let slot_notes = |li: usize, b: u32, slot: &Slot| -> Vec<Note> {
        let Some(f) = &forms[li] else { return vec![] };
        let phase = ((b - 1) * f.spb) % f.base.length_steps.max(1);
        match slot {
            Slot::Silent => vec![],
            Slot::Base(band) => {
                let p = match band {
                    Band::Sparse => f.sparse.as_ref().unwrap_or(&f.base),
                    Band::Full => &f.base,
                    Band::Lifted => f.lifted.as_ref().unwrap_or(&f.base),
                };
                bar_notes(p, phase, f.spb)
            }
            Slot::Pickup => {
                let beat = beat_steps(ts, f.spb);
                bar_notes(&f.base, phase, f.spb)
                    .into_iter()
                    .filter(|n| n.step >= f.spb.saturating_sub(beat))
                    .collect()
            }
            Slot::Hold => {
                let notes = bar_notes(&f.base, phase, f.spb);
                let first = notes.iter().map(|n| n.step).min();
                notes
                    .into_iter()
                    .filter(|n| Some(n.step) == first)
                    .map(|mut n| {
                        n.step = 0;
                        n.length = f.spb as f32 + 0.5;
                        n.velocity = ((n.velocity * 0.9 * 1000.0).round() / 1000.0).max(0.05);
                        n
                    })
                    .collect()
            }
            Slot::Bar(p) => p.notes.clone(),
        }
    };

    // Hit accents.
    for (k, (_, _, l)) in landings.iter().enumerate() {
        let b = l.bar;
        let bi = (b - 1) as usize;
        let sounding = |li: usize| !matches!(slots[li][bi], Slot::Silent);
        let pitched = |li: usize| infos[li].preset.pitched && !unpitched_class(infos[li].class);
        let anchor = (0..infos.len())
            .filter(|li| pitched(*li) && sounding(*li) && forms[*li].is_some())
            .filter_map(|li| anchor_rank(infos[li].class).map(|r| (r, li)))
            .min()
            .map(|(_, li)| li)
            .or_else(|| (0..infos.len()).find(|li| pitched(*li) && forms[*li].is_some()));
        let mut targets: Vec<usize> = anchor.into_iter().collect();
        for li in 0..infos.len() {
            if Some(li) == anchor || !sounding(li) || forms[li].is_none() {
                continue;
            }
            let c = infos[li].class;
            if (c == GeneratorClass::Bass && pitched(li)) || is_kick(&infos[li].preset) {
                targets.push(li);
            }
        }
        let root = chord_roots.get(bi).copied().unwrap_or(0);
        for li in targets {
            let info = &infos[li];
            let Some(f) = &forms[li] else { continue };
            // A bar that already holds a hit (two hits in one bar) is on the hit grid already.
            let (old, from_spb) = match &slots[li][bi] {
                Slot::Bar(p) => (p.notes.clone(), hit_spb),
                s => (slot_notes(li, b, s), f.spb),
            };
            let at = l.step as f32 + l.offset;
            // Convert to the hit grid; notes that do not fall on it are dropped.
            let mut notes: Vec<Note> = Vec::new();
            for n in old {
                let num = n.step * hit_spb;
                if !num.is_multiple_of(from_spb) {
                    continue;
                }
                let factor = hit_spb as f32 / from_spb as f32;
                let mut m = n.clone();
                m.step = num / from_spb;
                m.length = (n.length * factor).min(64.0);
                m.offset = (n.offset * factor).clamp(-0.5, 0.5);
                if (m.step as f32 + m.offset - at).abs() < 1.0 {
                    continue;
                }
                notes.push(m);
            }
            let beat = beat_steps(ts, hit_spb) as f32;
            let ring = if info.preset.sustaining {
                2.0 * beat + 0.5
            } else {
                beat
            };
            let accent = |pitch: u8, length: f32| Note {
                step: l.step,
                pitch,
                velocity: 1.0,
                accent: Accent::Strong,
                length: length.min(64.0),
                offset: l.offset,
            };
            if info.preset.kind == InstrumentKind::Drum || unpitched_class(info.class) {
                notes.push(accent(info.preset.gm_note.unwrap_or(36), 1.0));
            } else if info.class == GeneratorClass::Bass {
                let p = key.degree_to_midi(near(root, &key), info.octave);
                notes.push(accent(fit(p, &info.preset, &key), ring));
            } else {
                let center = key.degree_to_midi(near(root, &key), info.octave) + 5;
                let lo = (info.preset.range.0 as i32).max(center - 12);
                let hi = (info.preset.range.1 as i32).min(center + 14);
                for p in voice_lead(&triad_pcs(&key, root), None, center, lo, hi) {
                    let p = fit(p, &info.preset, &key);
                    if !notes.iter().any(|n| n.step == l.step && n.pitch == p) {
                        notes.push(accent(p, ring));
                    }
                }
            }
            notes.sort_by_key(|n| (n.step, n.pitch));
            notes.dedup_by(|a, b| a.step == b.step && a.pitch == b.pitch);
            let layer_id = &score.layers[info.layer].id;
            slots[li][bi] = Slot::Bar(Pattern {
                id: format!("{layer_id}_hit{}", k + 1),
                grid,
                length_steps: hit_spb,
                notes,
                spoken: None,
            });
        }
    }

    // Patterns and clips.
    let mut new_patterns: Vec<Pattern> = Vec::new();
    for (li, info) in infos.iter().enumerate() {
        let Some(f) = &forms[li] else { continue };
        let layer_id = score.layers[info.layer].id.clone();
        let mut used: BTreeSet<String> = BTreeSet::new();
        let mut one_bar: BTreeMap<String, Pattern> = BTreeMap::new();
        // (bar, pattern id, period pattern?, velocity scale)
        let mut cells: Vec<(u32, String, bool, f32)> = Vec::new();
        for b in 1..=total {
            let slot = &slots[li][(b - 1) as usize];
            let vs = velocity_scale(intensity_at(b));
            let cell = match slot {
                Slot::Silent => None,
                Slot::Base(band) => {
                    let p = match band {
                        Band::Sparse => f.sparse.as_ref().unwrap_or(&f.base),
                        Band::Full => &f.base,
                        Band::Lifted => f.lifted.as_ref().unwrap_or(&f.base),
                    };
                    Some((p.id.clone(), true))
                }
                Slot::Pickup | Slot::Hold => {
                    let notes = slot_notes(li, b, slot);
                    if notes.is_empty() {
                        None
                    } else {
                        let tag = if *slot == Slot::Pickup {
                            "pickup"
                        } else {
                            "hold"
                        };
                        let id = format!("{layer_id}_{tag}_b{b}");
                        one_bar.insert(
                            id.clone(),
                            Pattern {
                                id: id.clone(),
                                grid: f.base.grid,
                                length_steps: f.spb,
                                notes,
                                spoken: None,
                            },
                        );
                        Some((id, false))
                    }
                }
                Slot::Bar(p) => {
                    one_bar.insert(p.id.clone(), p.clone());
                    Some((p.id.clone(), false))
                }
            };
            if let Some((id, period)) = cell {
                used.insert(id.clone());
                cells.push((b, id, period, vs));
            }
        }
        let mut clips: Vec<Clip> = Vec::new();
        for (b, id, period, vs) in cells {
            if let Some(c) = clips.last_mut()
                && period
                && c.pattern == id
                && c.bars.end + 1 == b
                && (c.velocity_scale - vs).abs() < 1e-6
            {
                c.bars.end = b;
                continue;
            }
            let offset = if period {
                ((b - 1) * f.spb) % f.base.length_steps.max(1)
            } else {
                0
            };
            clips.push(Clip {
                bars: BarRange { start: b, end: b },
                pattern: id,
                pattern_offset_steps: offset,
                transpose: 0,
                velocity_scale: vs,
            });
        }
        for p in [Some(&f.base), f.sparse.as_ref(), f.lifted.as_ref()]
            .into_iter()
            .flatten()
        {
            if used.contains(&p.id) && p.id != f.base.id {
                new_patterns.push(p.clone());
            }
        }
        new_patterns.extend(one_bar.into_values());
        if !used.contains(&f.base.id) {
            score.patterns.retain(|p| p.id != f.base.id);
        }
        let layer = &mut score.layers[info.layer];
        layer.clips = clips;

        // The tension filter.
        let pitched = info.preset.pitched && !unpitched_class(info.class);
        if pitched
            && info.preset.sustaining
            && !matches!(info.class, GeneratorClass::Bass | GeneratorClass::Drone)
            && !layer.effects.iter().any(|e| e.id == "tension")
        {
            let mut points: Vec<AutoPoint> = Vec::new();
            let mut push = |bar: f64, value: f64| {
                if let Some(last) = points.last_mut()
                    && (last.bar - bar).abs() < 1e-9
                {
                    last.value = value;
                    return;
                }
                points.push(AutoPoint { bar, value });
            };
            for (k, s) in secs.iter().enumerate() {
                let c = cutoff_hz(s.intensity);
                push(s.bars.start as f64, c);
                let end = s.bars.end as f64 + 1.0;
                match secs.get(k + 1) {
                    Some(next) if s.role == SectionRole::Build => {
                        push(end, cutoff_hz(next.intensity))
                    }
                    Some(_) => push(end - 0.5, c),
                    None => push(end, c),
                }
            }
            let first = points.first().map(|p| p.value).unwrap_or(8000.0);
            let mut params = BTreeMap::new();
            params.insert("mode".to_string(), 0.0);
            params.insert("cutoff_hz".to_string(), first);
            params.insert("resonance".to_string(), 0.0);
            params.insert("mix".to_string(), 1.0);
            layer.effects.insert(
                0,
                Effect {
                    id: "tension".into(),
                    kind: EffectKind::Filter,
                    params,
                    bypass: false,
                    sidechain: None,
                    impulse: None,
                },
            );
            layer.automation.push(Automation {
                target: "effects.tension.cutoff_hz".into(),
                curve: Curve::Smooth,
                points,
            });
        }
    }
    score.patterns.extend(new_patterns);

    // Markers.
    for (k, (t, label, _)) in landings.iter().enumerate() {
        score.markers.push(Marker {
            id: format!("hit-{}", k + 1),
            time_s: *t,
            label: if label.trim().is_empty() {
                format!("Hit {}", k + 1)
            } else {
                label.trim().to_string()
            },
            kind: MarkerKind::Hit,
        });
    }
}

/// A chord root degree moved to the octave around the tonic.
fn near(root: i32, key: &praxis_score::Key) -> i32 {
    let n = key.mode.intervals().len() as i32;
    let d = root.rem_euclid(n);
    if d > n / 2 { d - n } else { d }
}

/// A pitch moved by octaves into the preset's range, then onto the key.
fn fit(mut p: i32, preset: &PresetSpec, key: &praxis_score::Key) -> u8 {
    let (lo, hi) = (preset.range.0 as i32, preset.range.1 as i32);
    while p < lo && p + 12 <= 127 {
        p += 12;
    }
    while p > hi && p - 12 >= 0 {
        p -= 12;
    }
    let p = p.clamp(0, 127) as u8;
    if key.mode == praxis_score::Mode::Chromatic {
        p
    } else {
        key.snap(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::Grid;

    fn note(step: u32, accent: Accent, length: f32) -> Note {
        Note {
            step,
            pitch: 60,
            velocity: 0.7,
            accent,
            length,
            offset: 0.0,
        }
    }

    #[test]
    fn sparse_keeps_beats_and_accents_and_rings_on() {
        let p = Pattern {
            id: "p".into(),
            grid: Grid::Eighth,
            length_steps: 8,
            notes: (0..8)
                .map(|s| {
                    note(
                        s,
                        if s == 0 {
                            Accent::Strong
                        } else if s == 5 {
                            Accent::Bright
                        } else {
                            Accent::Soft
                        },
                        1.6,
                    )
                })
                .collect(),
            spoken: None,
        };
        let s = sparse(&p, 2, true, "s").expect("sparse");
        let steps: Vec<u32> = s.notes.iter().map(|n| n.step).collect();
        assert_eq!(steps, vec![0, 2, 4, 5, 6]);
        assert!(s.notes.iter().all(|n| n.length >= 1.6));
        assert_eq!(s.notes[0].length, 2.6);
        assert!(max_gap(&s.notes, 8) <= 0.0);
    }

    #[test]
    fn curve_values() {
        assert_eq!(velocity_scale(0.3), 0.9);
        assert_eq!(velocity_scale(0.9), 1.05);
        assert_eq!(cutoff_hz(0.0), 1800.0);
        assert_eq!(cutoff_hz(1.0), 14400.0);
        assert_eq!(band(0.3), Band::Sparse);
        assert_eq!(band(0.5), Band::Full);
        assert_eq!(band(0.9), Band::Lifted);
    }
}
