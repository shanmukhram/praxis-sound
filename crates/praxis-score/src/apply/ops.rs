//! One op at a time: checks (existence, locks, scope), the change, and the ops that undo it.
//!
//! Every function returns the inverse group for its op, in the order those inverse ops must run.

use super::clips::{canonicalize, carve, inside};
use super::ids::{fresh_id, resolve_write_pattern};
use super::isolation::{layer_events, outside, same_events};
use crate::patch::{LayerUpdate, Origin, PatchError, PatchOp, Scope, ScorePatch};
use crate::timeline::TimeMap;
use crate::types::*;
use crate::validate::unpitched_class;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

/// The op being applied and where it sits in its patch.
pub(super) struct Cx<'a> {
    pub patch: &'a ScorePatch,
    pub index: usize,
    pub op: &'a PatchOp,
}

impl Cx<'_> {
    fn scope(&self) -> &Scope {
        &self.patch.scope
    }

    fn out_of_scope(&self, reason: impl Into<String>) -> PatchError {
        PatchError::OutOfScope {
            index: self.index,
            op: self.op.name().into(),
            reason: reason.into(),
        }
    }

    fn unknown(&self, what: &str, id: &str) -> PatchError {
        PatchError::UnknownId {
            index: self.index,
            op: self.op.name().into(),
            what: what.into(),
            id: id.into(),
        }
    }

    fn duplicate(&self, what: &str, id: &str) -> PatchError {
        PatchError::DuplicateId {
            index: self.index,
            op: self.op.name().into(),
            what: what.into(),
            id: id.into(),
        }
    }

    fn invalid(&self, msg: impl Display) -> PatchError {
        PatchError::Invalid(format!("op {} ({}): {msg}", self.index, self.op.name()))
    }

    fn layer_allowed(&self, id: &str) -> bool {
        let l = &self.scope().layers;
        l.is_empty() || l.iter().any(|x| x == id)
    }

    /// Layer ops must name a layer of `scope.layers` when it is non-empty.
    fn layer_scope(&self, id: &str) -> Result<(), PatchError> {
        if self.layer_allowed(id) {
            Ok(())
        } else {
            Err(self.out_of_scope(format!("layer `{id}` is not in scope.layers")))
        }
    }

    /// Bar ranges must lie inside `scope.bars` when it is set.
    fn bars_scope(&self, r: BarRange, what: &str) -> Result<(), PatchError> {
        match self.scope().bars {
            Some(b) if !inside(r, b) => Err(self.out_of_scope(format!(
                "{what} bars {}-{} are not inside scope bars {}-{}",
                r.start, r.end, b.start, b.end
            ))),
            _ => Ok(()),
        }
    }
}

fn layer_index(s: &Score, cx: &Cx, id: &str) -> Result<usize, PatchError> {
    s.layers
        .iter()
        .position(|l| l.id == id)
        .ok_or_else(|| cx.unknown("layer", id))
}

fn pattern_index(s: &Score, cx: &Cx, id: &str) -> Result<usize, PatchError> {
    s.patterns
        .iter()
        .position(|p| p.id == id)
        .ok_or_else(|| cx.unknown("pattern", id))
}

fn unlocked(s: &Score, li: usize) -> Result<(), PatchError> {
    if s.layers[li].locked {
        Err(PatchError::LockedLayer(s.layers[li].id.clone()))
    } else {
        Ok(())
    }
}

/// A bar range an op writes must exist in the score.
fn range_in_score(s: &Score, cx: &Cx, r: BarRange) -> Result<(), PatchError> {
    if r.start == 0 || r.end < r.start || r.end > s.bars {
        Err(cx.invalid(format!(
            "bar range {}-{} is outside the score (1-{})",
            r.start, r.end, s.bars
        )))
    } else {
        Ok(())
    }
}

/// Smallest signed interval in semitones from `from` to `to`, -5..=6 (a tritone goes up).
pub(super) fn tonic_interval(from: PitchClass, to: PitchClass) -> i32 {
    let d = (to.semitone() as i32 - from.semitone() as i32).rem_euclid(12);
    if d > 6 { d - 12 } else { d }
}

pub(super) fn apply_op(s: &mut Score, cx: &Cx) -> Result<Vec<PatchOp>, PatchError> {
    if cx.op.is_global() && (cx.scope().bars.is_some() || !cx.scope().layers.is_empty()) {
        return Err(cx.out_of_scope(
            "global ops need an unscoped patch (no scope.bars and no scope.layers)",
        ));
    }
    match cx.op {
        // ---- global ops ----
        PatchOp::SetTitle { title } => {
            let old = std::mem::replace(&mut s.title, title.clone());
            Ok(vec![PatchOp::SetTitle { title: old }])
        }
        PatchOp::SetMeta { meta } => {
            let old = std::mem::replace(&mut s.meta, meta.clone());
            Ok(vec![PatchOp::SetMeta { meta: old }])
        }
        PatchOp::SetTempo { bpm } => {
            let old = std::mem::replace(&mut s.tempo.bpm, *bpm);
            Ok(vec![PatchOp::SetTempo { bpm: old }])
        }
        PatchOp::SetKey {
            key,
            transpose_notes,
        } => set_key(s, cx, *key, *transpose_notes),
        PatchOp::SetTimeSignature { time_signature } => {
            let old = std::mem::replace(&mut s.time_signature, *time_signature);
            Ok(vec![PatchOp::SetTimeSignature {
                time_signature: old,
            }])
        }
        PatchOp::SetBars { bars } => {
            let old = std::mem::replace(&mut s.bars, *bars);
            Ok(vec![PatchOp::SetBars { bars: old }])
        }
        PatchOp::SetPlayback { playback } => {
            let old = std::mem::replace(&mut s.playback, playback.clone());
            Ok(vec![PatchOp::SetPlayback { playback: old }])
        }
        PatchOp::SetConstraints { constraints } => {
            let old = std::mem::replace(&mut s.constraints, constraints.clone());
            Ok(vec![PatchOp::SetConstraints { constraints: old }])
        }
        PatchOp::SetMaster { master } => {
            let old = std::mem::replace(&mut s.master, master.clone());
            Ok(vec![PatchOp::SetMaster { master: old }])
        }
        // ---- score-level lists ----
        PatchOp::SetChords { chords } => set_chords(s, cx, chords),
        PatchOp::AddSection { section } => {
            if s.sections.iter().any(|x| x.id == section.id) {
                return Err(cx.duplicate("section", &section.id));
            }
            cx.bars_scope(section.bars, "section")?;
            s.sections.push(section.clone());
            Ok(vec![PatchOp::RemoveSection {
                id: section.id.clone(),
            }])
        }
        PatchOp::UpdateSection { section } => {
            let i = s
                .sections
                .iter()
                .position(|x| x.id == section.id)
                .ok_or_else(|| cx.unknown("section", &section.id))?;
            cx.bars_scope(s.sections[i].bars, "section")?;
            cx.bars_scope(section.bars, "section")?;
            let old = std::mem::replace(&mut s.sections[i], section.clone());
            Ok(vec![PatchOp::UpdateSection { section: old }])
        }
        PatchOp::RemoveSection { id } => {
            let i = s
                .sections
                .iter()
                .position(|x| &x.id == id)
                .ok_or_else(|| cx.unknown("section", id))?;
            cx.bars_scope(s.sections[i].bars, "section")?;
            let removed = s.sections.remove(i);
            let later = s.sections[i..].to_vec();
            Ok(restore_chain(
                &later,
                removed,
                |x| PatchOp::RemoveSection { id: x.id.clone() },
                |x| PatchOp::AddSection { section: x },
            ))
        }
        PatchOp::AddMarker { marker } => {
            if s.markers.iter().any(|x| x.id == marker.id) {
                return Err(cx.duplicate("marker", &marker.id));
            }
            marker_scope(s, cx, marker)?;
            s.markers.push(marker.clone());
            Ok(vec![PatchOp::RemoveMarker {
                id: marker.id.clone(),
            }])
        }
        PatchOp::RemoveMarker { id } => {
            let i = s
                .markers
                .iter()
                .position(|x| &x.id == id)
                .ok_or_else(|| cx.unknown("marker", id))?;
            marker_scope(s, cx, &s.markers[i])?;
            let removed = s.markers.remove(i);
            let later = s.markers[i..].to_vec();
            Ok(restore_chain(
                &later,
                removed,
                |x| PatchOp::RemoveMarker { id: x.id.clone() },
                |x| PatchOp::AddMarker { marker: x },
            ))
        }
        // ---- pattern ops ----
        PatchOp::AddPattern { pattern } => {
            if s.pattern(&pattern.id).is_some() {
                return Err(cx.duplicate("pattern", &pattern.id));
            }
            s.patterns.push(pattern.clone());
            Ok(vec![PatchOp::RemovePattern {
                id: pattern.id.clone(),
            }])
        }
        PatchOp::ReplacePattern { pattern } => replace_pattern(s, cx, pattern),
        PatchOp::RemovePattern { id } => remove_pattern(s, cx, id),
        // ---- layer ops ----
        PatchOp::AddLayer { layer } => add_layer(s, cx, layer),
        PatchOp::RemoveLayer { id } => remove_layer(s, cx, id),
        PatchOp::UpdateLayer { id, update } => update_layer(s, cx, id, update),
        PatchOp::SetLock { id, locked } => {
            let li = layer_index(s, cx, id)?;
            cx.layer_scope(id)?;
            if !*locked && cx.patch.origin != Origin::User {
                return Err(PatchError::UnlockNotAllowed(id.clone()));
            }
            let old = std::mem::replace(&mut s.layers[li].locked, *locked);
            Ok(vec![PatchOp::SetLock {
                id: id.clone(),
                locked: old,
            }])
        }
        PatchOp::WriteBars {
            layer,
            bars,
            pattern,
        } => write_bars(s, cx, layer, *bars, pattern),
        PatchOp::ClearBars { layer, bars } => {
            let li = bar_op_target(s, cx, layer, *bars)?;
            let old = s.layers[li].clips.clone();
            let carved = carve(s, &old, *bars).map_err(|e| cx.invalid(e))?;
            s.layers[li].clips = canonicalize(s, carved, *bars);
            Ok(vec![PatchOp::SetClips {
                layer: layer.clone(),
                clips: old,
            }])
        }
        PatchOp::PlaceClip { layer, clip } => {
            let li = bar_op_target(s, cx, layer, clip.bars)?;
            if s.pattern(&clip.pattern).is_none() {
                return Err(cx.unknown("pattern", &clip.pattern));
            }
            let old = s.layers[li].clips.clone();
            let mut carved = carve(s, &old, clip.bars).map_err(|e| cx.invalid(e))?;
            carved.push(clip.clone());
            s.layers[li].clips = canonicalize(s, carved, clip.bars);
            Ok(vec![PatchOp::SetClips {
                layer: layer.clone(),
                clips: old,
            }])
        }
        PatchOp::SetClips { layer, clips } => set_clips(s, cx, layer, clips),
    }
}

/// Undo group for removing item `removed` that had `later` after it: remove the later items
/// (last first, so each removal is of the last item), re-add `removed`, re-add the later items.
/// When `later` is empty this is just the re-add.
fn restore_chain<T: Clone>(
    later: &[T],
    removed: T,
    remove: impl Fn(&T) -> PatchOp,
    add: impl Fn(T) -> PatchOp,
) -> Vec<PatchOp> {
    let mut ops: Vec<PatchOp> = later.iter().rev().map(remove).collect();
    ops.push(add(removed));
    ops.extend(later.iter().cloned().map(add));
    ops
}

fn set_key(
    s: &mut Score,
    cx: &Cx,
    key: Key,
    transpose_notes: bool,
) -> Result<Vec<PatchOp>, PatchError> {
    let old = s.key;
    let mut inv = Vec::new();
    let d = tonic_interval(old.tonic, key.tonic);
    if transpose_notes && d != 0 {
        inv = transpose_pitched(s, cx, d)?;
    }
    s.key = key;
    inv.push(PatchOp::SetKey {
        key: old,
        transpose_notes: false,
    });
    Ok(inv)
}

/// Moves every pitched layer by `d` semitones. Patterns played only by pitched layers have their
/// notes moved; a pattern shared with an unpitched (drum) layer is left alone and the pitched
/// layers' clips get `transpose += d` instead, so drums never move. Unused patterns are untouched.
fn transpose_pitched(s: &mut Score, cx: &Cx, d: i32) -> Result<Vec<PatchOp>, PatchError> {
    if let Some(l) = s
        .layers
        .iter()
        .find(|l| l.locked && !unpitched_class(l.class) && !l.clips.is_empty())
    {
        return Err(PatchError::LockedLayer(l.id.clone()));
    }
    let mut pitched: BTreeSet<String> = BTreeSet::new();
    let mut unpitched: BTreeSet<String> = BTreeSet::new();
    for l in &s.layers {
        let set = if unpitched_class(l.class) {
            &mut unpitched
        } else {
            &mut pitched
        };
        for c in &l.clips {
            set.insert(c.pattern.clone());
        }
    }
    let mut inv = Vec::new();
    for p in &mut s.patterns {
        if !pitched.contains(&p.id) || unpitched.contains(&p.id) {
            continue;
        }
        let old = p.clone();
        for n in &mut p.notes {
            let moved = n.pitch as i32 + d;
            n.pitch = u8::try_from(moved)
                .ok()
                .filter(|v| *v <= 127)
                .ok_or_else(|| {
                    cx.invalid(format!(
                        "transposing pattern `{}` by {d} moves pitch {} out of 0..=127",
                        old.id, n.pitch
                    ))
                })?;
        }
        inv.push(PatchOp::ReplacePattern { pattern: old });
    }
    for l in &mut s.layers {
        if unpitched_class(l.class) {
            continue;
        }
        let old = l.clips.clone();
        let mut changed = false;
        for c in &mut l.clips {
            if unpitched.contains(&c.pattern) {
                c.transpose = i8::try_from(c.transpose as i32 + d).map_err(|_| {
                    cx.invalid(format!("clip transpose on layer `{}` overflows", l.id))
                })?;
                changed = true;
            }
        }
        if changed {
            inv.push(PatchOp::SetClips {
                layer: l.id.clone(),
                clips: old,
            });
        }
    }
    Ok(inv)
}

fn set_chords(s: &mut Score, cx: &Cx, chords: &[ChordSpan]) -> Result<Vec<PatchOp>, PatchError> {
    if let Some(b) = cx.scope().bars {
        let keep = |c: &&ChordSpan| !inside(c.bars, b);
        let before: Vec<&ChordSpan> = s.chords.iter().filter(keep).collect();
        let after: Vec<&ChordSpan> = chords.iter().filter(keep).collect();
        if before != after {
            return Err(cx.out_of_scope(format!(
                "chords outside scope bars {}-{} would change",
                b.start, b.end
            )));
        }
    }
    let old = std::mem::replace(&mut s.chords, chords.to_vec());
    Ok(vec![PatchOp::SetChords { chords: old }])
}

/// With `scope.bars` set, a marker must sit inside those bars.
fn marker_scope(s: &Score, cx: &Cx, m: &Marker) -> Result<(), PatchError> {
    let Some(b) = cx.scope().bars else {
        return Ok(());
    };
    let tm = TimeMap::new(s, super::isolation::SR);
    let t0 = tm.bar_pos_to_s(b.start as f64);
    let t1 = tm.bar_pos_to_s(b.end as f64 + 1.0);
    let at_end = b.end >= s.bars && m.time_s <= t1;
    if m.time_s >= t0 && (m.time_s < t1 || at_end) {
        Ok(())
    } else {
        Err(cx.out_of_scope(format!(
            "marker `{}` at {} s is not inside scope bars {}-{}",
            m.id, m.time_s, b.start, b.end
        )))
    }
}

fn replace_pattern(s: &mut Score, cx: &Cx, pattern: &Pattern) -> Result<Vec<PatchOp>, PatchError> {
    let pi = pattern_index(s, cx, &pattern.id)?;
    for l in &s.layers {
        for c in l.clips.iter().filter(|c| c.pattern == pattern.id) {
            if l.locked {
                return Err(PatchError::LockedLayer(l.id.clone()));
            }
            if !cx.layer_allowed(&l.id) {
                return Err(cx.out_of_scope(format!(
                    "pattern `{}` is also played by layer `{}`, which is not in scope.layers; use write_bars",
                    pattern.id, l.id
                )));
            }
            if let Some(b) = cx.scope().bars
                && !inside(c.bars, b)
            {
                return Err(cx.out_of_scope(format!(
                    "pattern `{}` is also played in bars {}-{} of layer `{}`, outside scope bars {}-{}; use write_bars",
                    pattern.id, c.bars.start, c.bars.end, l.id, b.start, b.end
                )));
            }
        }
    }
    let old = std::mem::replace(&mut s.patterns[pi], pattern.clone());
    Ok(vec![PatchOp::ReplacePattern { pattern: old }])
}

fn remove_pattern(s: &mut Score, cx: &Cx, id: &str) -> Result<Vec<PatchOp>, PatchError> {
    let pi = pattern_index(s, cx, id)?;
    for l in &s.layers {
        if l.clips.iter().any(|c| c.pattern == id) {
            if l.locked {
                return Err(PatchError::LockedLayer(l.id.clone()));
            }
            return Err(cx.invalid(format!(
                "pattern `{id}` is still played by layer `{}`",
                l.id
            )));
        }
    }
    let removed = s.patterns.remove(pi);
    if pi == s.patterns.len() {
        return Ok(vec![PatchOp::AddPattern { pattern: removed }]);
    }
    Ok(pattern_restore_chain(s, pi, removed))
}

/// Undo group for removing a pattern that was not last (`s` is the state after the removal, the
/// removed pattern sat at `pi`). `add_pattern` only appends, so the patterns after `pi` are
/// removed and re-added behind it. Those still in use are first swapped for temporary copies
/// (`<id>-undo`) on every layer that plays them, unlocking and re-locking locked layers; the
/// copies are identical, so no note event changes at any point.
fn pattern_restore_chain(s: &Score, pi: usize, removed: Pattern) -> Vec<PatchOp> {
    let later: Vec<Pattern> = s.patterns[pi..].to_vec();
    let used: BTreeSet<&str> = s
        .layers
        .iter()
        .flat_map(|l| l.clips.iter().map(|c| c.pattern.as_str()))
        .collect();
    let mut taken: BTreeSet<String> = s.patterns.iter().map(|p| p.id.clone()).collect();
    taken.insert(removed.id.clone());
    let mut tmp: BTreeMap<String, String> = BTreeMap::new();
    let mut ops = Vec::new();
    for q in later.iter().filter(|q| used.contains(q.id.as_str())) {
        let id = fresh_id(&mut taken, &q.id);
        tmp.insert(q.id.clone(), id.clone());
        let mut copy = q.clone();
        copy.id = id;
        ops.push(PatchOp::AddPattern { pattern: copy });
    }
    let affected: Vec<&Layer> = s
        .layers
        .iter()
        .filter(|l| l.clips.iter().any(|c| tmp.contains_key(&c.pattern)))
        .collect();
    for l in &affected {
        if l.locked {
            ops.push(PatchOp::SetLock {
                id: l.id.clone(),
                locked: false,
            });
        }
        let clips = l
            .clips
            .iter()
            .map(|c| {
                let mut c = c.clone();
                if let Some(t) = tmp.get(&c.pattern) {
                    c.pattern = t.clone();
                }
                c
            })
            .collect();
        ops.push(PatchOp::SetClips {
            layer: l.id.clone(),
            clips,
        });
    }
    ops.extend(
        later
            .iter()
            .rev()
            .map(|q| PatchOp::RemovePattern { id: q.id.clone() }),
    );
    ops.push(PatchOp::AddPattern { pattern: removed });
    ops.extend(
        later
            .iter()
            .cloned()
            .map(|q| PatchOp::AddPattern { pattern: q }),
    );
    for l in &affected {
        ops.push(PatchOp::SetClips {
            layer: l.id.clone(),
            clips: l.clips.clone(),
        });
        if l.locked {
            ops.push(PatchOp::SetLock {
                id: l.id.clone(),
                locked: true,
            });
        }
    }
    for q in later.iter() {
        if let Some(t) = tmp.get(&q.id) {
            ops.push(PatchOp::RemovePattern { id: t.clone() });
        }
    }
    ops
}

fn add_layer(s: &mut Score, cx: &Cx, layer: &Layer) -> Result<Vec<PatchOp>, PatchError> {
    if s.layer(&layer.id).is_some() {
        return Err(cx.duplicate("layer", &layer.id));
    }
    cx.layer_scope(&layer.id)?;
    for c in &layer.clips {
        cx.bars_scope(c.bars, "clip")?;
    }
    known_patterns(s, cx, &layer.clips)?;
    s.layers.push(layer.clone());
    let mut inv = Vec::new();
    if layer.locked {
        inv.push(PatchOp::SetLock {
            id: layer.id.clone(),
            locked: false,
        });
    }
    inv.push(PatchOp::RemoveLayer {
        id: layer.id.clone(),
    });
    Ok(inv)
}

fn remove_layer(s: &mut Score, cx: &Cx, id: &str) -> Result<Vec<PatchOp>, PatchError> {
    let li = layer_index(s, cx, id)?;
    unlocked(s, li)?;
    cx.layer_scope(id)?;
    if let Some(b) = cx.scope().bars
        && let Some(c) = s.layers[li].clips.iter().find(|c| !inside(c.bars, b))
    {
        return Err(cx.out_of_scope(format!(
            "layer `{id}` plays bars {}-{}, outside scope bars {}-{}",
            c.bars.start, c.bars.end, b.start, b.end
        )));
    }
    let removed = s.layers.remove(li);
    // Layers after `li` are removed (unlocking locked ones) and re-added behind it, with their
    // original `locked` flags, because add_layer only appends.
    let later = s.layers[li..].to_vec();
    let mut ops = Vec::new();
    for l in later.iter().rev() {
        if l.locked {
            ops.push(PatchOp::SetLock {
                id: l.id.clone(),
                locked: false,
            });
        }
        ops.push(PatchOp::RemoveLayer { id: l.id.clone() });
    }
    ops.push(PatchOp::AddLayer { layer: removed });
    ops.extend(later.into_iter().map(|layer| PatchOp::AddLayer { layer }));
    Ok(ops)
}

/// True when `u` changes anything but the name of `l` (such changes affect every bar).
fn changes_sound(l: &Layer, u: &LayerUpdate) -> bool {
    fn differs<T: PartialEq>(new: &Option<T>, cur: &T) -> bool {
        new.as_ref().is_some_and(|v| v != cur)
    }
    differs(&u.class, &l.class)
        || differs(&u.instrument, &l.instrument)
        || differs(&u.muted, &l.muted)
        || differs(&u.solo, &l.solo)
        || differs(&u.gain_db, &l.gain_db)
        || differs(&u.pan, &l.pan)
        || differs(&u.effects, &l.effects)
        || differs(&u.automation, &l.automation)
        || differs(&u.seed_offset, &l.seed_offset)
}

fn update_layer(
    s: &mut Score,
    cx: &Cx,
    id: &str,
    u: &LayerUpdate,
) -> Result<Vec<PatchOp>, PatchError> {
    let li = layer_index(s, cx, id)?;
    unlocked(s, li)?;
    cx.layer_scope(id)?;
    if let Some(b) = cx.scope().bars
        && changes_sound(&s.layers[li], u)
    {
        return Err(cx.out_of_scope(format!(
            "with scope bars {}-{} update_layer may only rename layer `{id}`; other properties change every bar",
            b.start, b.end
        )));
    }
    fn swap<T: Clone>(new: &Option<T>, cur: &mut T, inv: &mut Option<T>) {
        if let Some(v) = new {
            *inv = Some(std::mem::replace(cur, v.clone()));
        }
    }
    let l = &mut s.layers[li];
    let mut inv = LayerUpdate::default();
    swap(&u.name, &mut l.name, &mut inv.name);
    swap(&u.class, &mut l.class, &mut inv.class);
    swap(&u.instrument, &mut l.instrument, &mut inv.instrument);
    swap(&u.muted, &mut l.muted, &mut inv.muted);
    swap(&u.solo, &mut l.solo, &mut inv.solo);
    swap(&u.gain_db, &mut l.gain_db, &mut inv.gain_db);
    swap(&u.pan, &mut l.pan, &mut inv.pan);
    swap(&u.effects, &mut l.effects, &mut inv.effects);
    swap(&u.automation, &mut l.automation, &mut inv.automation);
    swap(&u.seed_offset, &mut l.seed_offset, &mut inv.seed_offset);
    Ok(vec![PatchOp::UpdateLayer {
        id: id.to_string(),
        update: inv,
    }])
}

/// Shared checks for write_bars / clear_bars / place_clip: the layer exists, is unlocked and in
/// scope, and the range is in the score and inside `scope.bars`.
fn bar_op_target(s: &Score, cx: &Cx, layer: &str, bars: BarRange) -> Result<usize, PatchError> {
    let li = layer_index(s, cx, layer)?;
    unlocked(s, li)?;
    cx.layer_scope(layer)?;
    range_in_score(s, cx, bars)?;
    cx.bars_scope(bars, "op")?;
    Ok(li)
}

fn write_bars(
    s: &mut Score,
    cx: &Cx,
    layer: &str,
    bars: BarRange,
    pattern: &Pattern,
) -> Result<Vec<PatchOp>, PatchError> {
    let li = bar_op_target(s, cx, layer, bars)?;
    let (pid, add) = resolve_write_pattern(s, layer, bars, pattern).map_err(|e| cx.invalid(e))?;
    if add {
        let mut p = pattern.clone();
        p.id = pid.clone();
        s.patterns.push(p);
    }
    let old = s.layers[li].clips.clone();
    let mut carved = carve(s, &old, bars).map_err(|e| cx.invalid(e))?;
    carved.push(Clip {
        bars,
        pattern: pid.clone(),
        pattern_offset_steps: 0,
        transpose: 0,
        velocity_scale: 1.0,
    });
    s.layers[li].clips = canonicalize(s, carved, bars);
    let mut inv = vec![PatchOp::SetClips {
        layer: layer.to_string(),
        clips: old,
    }];
    if add {
        inv.push(PatchOp::RemovePattern { id: pid });
    }
    Ok(inv)
}

/// Clips may only name patterns that exist when the op runs. Keeping every intermediate state free
/// of dangling references is what lets the inverse replay op by op (a `remove_pattern` in an
/// inverse must never meet a clip that named the id before the pattern existed).
fn known_patterns(s: &Score, cx: &Cx, clips: &[Clip]) -> Result<(), PatchError> {
    match clips.iter().find(|c| s.pattern(&c.pattern).is_none()) {
        Some(c) => Err(cx.unknown("pattern", &c.pattern)),
        None => Ok(()),
    }
}

fn set_clips(
    s: &mut Score,
    cx: &Cx,
    layer: &str,
    clips: &[Clip],
) -> Result<Vec<PatchOp>, PatchError> {
    let li = layer_index(s, cx, layer)?;
    unlocked(s, li)?;
    cx.layer_scope(layer)?;
    known_patterns(s, cx, clips)?;
    let before = cx.scope().bars.map(|_| layer_events(s, li));
    let old = std::mem::replace(&mut s.layers[li].clips, clips.to_vec());
    if let (Some(b), Some(before)) = (cx.scope().bars, before) {
        let after = layer_events(s, li);
        if !same_events(&before, &after, |e| outside(e.bar, b)) {
            return Err(cx.out_of_scope(format!(
                "set_clips would change notes of layer `{layer}` outside scope bars {}-{}",
                b.start, b.end
            )));
        }
    }
    Ok(vec![PatchOp::SetClips {
        layer: layer.to_string(),
        clips: old,
    }])
}
