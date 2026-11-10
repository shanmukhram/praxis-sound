//! Isolation checks (rules 4 and 6): compare the note events a layer expands to.
//!
//! Events are compared with [`same_event`]: everything that reaches the engine as an integer or a
//! musical value (stable key, bar, pitch, velocity, accent, sample positions) must be equal, and
//! the `f64` seconds may differ by at most 1 ns. The tolerance exists because
//! `timeline::expand_notes` computes `start_s` as `clip_start_s + steps * step_s`, so the same
//! note reached through a split clip can differ in the last bits of `start_s` (see the timeline
//! note in the WP-A report). Sample positions differ by at most one sample only in that same
//! rounding case.

use crate::patch::{PatchError, ScorePatch};
use crate::timeline::{NoteEvent, expand_notes};
use crate::types::{BarRange, Score};
use std::collections::{BTreeMap, BTreeSet};

/// Sample rate used for event comparisons. Any rate works; equal music gives equal events.
pub(super) const SR: u32 = 48_000;

/// Tolerance on `start_s` / `length_s` in seconds.
const SECONDS_EPS: f64 = 1e-9;

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

fn close(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= SECONDS_EPS
}

/// Equal for rendering purposes (see the module docs).
pub(super) fn same_event(a: &NoteEvent, b: &NoteEvent) -> bool {
    let seconds_equal = a.start_s == b.start_s && a.length_s == b.length_s;
    let samples_ok = if seconds_equal {
        a.start_sample == b.start_sample && a.length_samples == b.length_samples
    } else {
        a.start_sample.abs_diff(b.start_sample) <= 1
            && a.length_samples.abs_diff(b.length_samples) <= 1
    };
    a.layer == b.layer
        && a.key == b.key
        && a.bar == b.bar
        && a.pitch == b.pitch
        && a.velocity.to_bits() == b.velocity.to_bits()
        && a.accent == b.accent
        && close(a.start_s, b.start_s)
        && close(a.length_s, b.length_s)
        && samples_ok
}

/// True when the events of `a` and `b` that satisfy `keep` are the same, in the same order.
pub(super) fn same_events(
    a: &[NoteEvent],
    b: &[NoteEvent],
    keep: impl Fn(&NoteEvent) -> bool,
) -> bool {
    let fa: Vec<&NoteEvent> = a.iter().filter(|e| keep(e)).collect();
    let fb: Vec<&NoteEvent> = b.iter().filter(|e| keep(e)).collect();
    fa.len() == fb.len() && fa.iter().zip(&fb).all(|(x, y)| same_event(x, y))
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
