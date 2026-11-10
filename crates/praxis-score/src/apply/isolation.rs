//! Isolation checks (rules 4 and 6): compare the note events a layer expands to.
//!
//! Events are compared with exact equality (`NoteEvent: PartialEq`). `timeline::expand_notes`
//! computes every note time from one integer step count, so a note reached through a split clip
//! is bit-identical to the same note in the unsplit clip.

use crate::patch::{PatchError, ScorePatch};
use crate::timeline::{NoteEvent, expand_notes};
use crate::types::{BarRange, Score};
use std::collections::{BTreeMap, BTreeSet};

/// Sample rate used for event comparisons. Any rate works; equal music gives equal events.
pub(super) const SR: u32 = 48_000;

/// Note events of every layer, keyed by layer id (`NoteEvent::layer` set to 0 so that adding or
/// removing another layer does not change them).
pub(super) fn events_by_layer(s: &Score) -> BTreeMap<String, Vec<NoteEvent>> {
    let mut out: BTreeMap<String, Vec<NoteEvent>> = BTreeMap::new();
    for l in &s.layers {
        out.entry(l.id.clone()).or_default();
    }
    for mut e in expand_notes(s, SR) {
        let Some(l) = s.layers.get(e.layer) else {
            continue;
        };
        e.layer = 0;
        out.entry(l.id.clone()).or_default().push(e);
    }
    out
}

/// Note events of the layer at index `li`.
pub(super) fn layer_events(s: &Score, li: usize) -> Vec<NoteEvent> {
    let mut out: Vec<NoteEvent> = expand_notes(s, SR)
        .into_iter()
        .filter(|e| e.layer == li)
        .collect();
    for e in &mut out {
        e.layer = 0;
    }
    out
}

/// True when the events of `a` and `b` that satisfy `keep` are the same, in the same order.
pub(super) fn same_events(
    a: &[NoteEvent],
    b: &[NoteEvent],
    keep: impl Fn(&NoteEvent) -> bool,
) -> bool {
    let fa: Vec<&NoteEvent> = a.iter().filter(|e| keep(e)).collect();
    let fb: Vec<&NoteEvent> = b.iter().filter(|e| keep(e)).collect();
    fa == fb
}

/// True when bar `bar` lies outside `bars`.
pub(super) fn outside(bar: u32, bars: BarRange) -> bool {
    bar < bars.start || bar > bars.end
}

/// Rule 6 over the whole patch: every layer outside `scope.layers` is structurally identical and
/// expands to the same events, and every bar outside `scope.bars` expands to the same events.
/// The per-op checks already guarantee this; this is the final safety net.
pub(super) fn check_patch(
    before: &Score,
    after: &Score,
    patch: &ScorePatch,
) -> Result<(), PatchError> {
    let scope = &patch.scope;
    if scope.bars.is_none() && scope.layers.is_empty() {
        return Ok(());
    }
    let fail = |reason: String| PatchError::OutOfScope {
        index: patch.ops.len().saturating_sub(1),
        op: "patch".into(),
        reason,
    };
    let in_layers = |id: &str| scope.layers.is_empty() || scope.layers.iter().any(|l| l == id);
    let eb = events_by_layer(before);
    let ea = events_by_layer(after);
    let empty: Vec<NoteEvent> = Vec::new();
    if !scope.layers.is_empty() {
        for l in &before.layers {
            if in_layers(&l.id) {
                continue;
            }
            if after.layer(&l.id) != Some(l) {
                return Err(fail(format!(
                    "layer `{}` is outside scope.layers but would change",
                    l.id
                )));
            }
            let b = eb.get(&l.id).unwrap_or(&empty);
            let a = ea.get(&l.id).unwrap_or(&empty);
            if !same_events(b, a, |_| true) {
                return Err(fail(format!(
                    "notes of layer `{}` (outside scope.layers) would change",
                    l.id
                )));
            }
        }
        for l in &after.layers {
            if !in_layers(&l.id) && before.layer(&l.id).is_none() {
                return Err(fail(format!("layer `{}` is not in scope.layers", l.id)));
            }
        }
    }
    if let Some(bars) = scope.bars {
        let ids: BTreeSet<&String> = eb.keys().chain(ea.keys()).collect();
        for id in ids {
            let b = eb.get(id).unwrap_or(&empty);
            let a = ea.get(id).unwrap_or(&empty);
            if !same_events(b, a, |e| outside(e.bar, bars)) {
                return Err(fail(format!(
                    "notes of layer `{id}` outside bars {}-{} would change",
                    bars.start, bars.end
                )));
            }
        }
    }
    Ok(())
}
