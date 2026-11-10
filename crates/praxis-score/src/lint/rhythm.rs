//! Rhythm-first lints (owner rule 1): isolated notes and motifs that never repeat.

use super::{sfx_class, warn};
use crate::timeline::{TimeMap, expand_notes};
use crate::types::{Grid, Layer, Playback, Score};
use crate::validate::{Issue, IssueCode, unpitched_class};
use std::collections::{BTreeMap, BTreeSet};

/// Sample rate for note expansion; only seconds are used.
const SR: u32 = 48_000;

/// A silence longer than this many quarter-note beats between one note's end (or the latest
/// sustain still ringing) and the next onset counts as a gap. The owner heard notes separated by
/// more than a beat of silence as "out of time".
const GAP_BEATS: f64 = 1.0;

/// The layer is flagged when at least this share of its note-to-note transitions are gaps. One
/// phrase-ending rest per few bars stays well below it; sparse, disconnected notes are at 100 %.
const GAP_SHARE: f64 = 0.25;

/// Bar runs in which a layer plays: clips merged when they touch. Silence between runs is
/// arrangement (the layer drops out), not isolated notes, so gaps are only measured inside a run.
fn runs(layer: &Layer) -> Vec<(u32, u32)> {
    let mut spans: Vec<(u32, u32)> = layer
        .clips
        .iter()
        .map(|c| (c.bars.start, c.bars.end))
        .collect();
    spans.sort();
    let mut out: Vec<(u32, u32)> = Vec::new();
    for (s, e) in spans {
        match out.last_mut() {
            Some(last) if s <= last.1.saturating_add(1) => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// `isolated_notes`: pitched, non-sound-effect layers where at least [`GAP_SHARE`] of the
/// transitions between consecutive notes leave more than [`GAP_BEATS`] of silence that no
/// sustaining note bridges. In a loop whose run covers the whole score, the wrap from the last
/// note back to the first counts as a transition too (owner rule 6: the last bar leads into the
/// first). A run with a single note has no transitions and is not judged.
pub(super) fn isolated_notes(score: &Score, out: &mut Vec<Issue>) {
    let events = expand_notes(score, SR);
    let tm = TimeMap::new(score, SR);
    let beat_s = 60.0 / score.tempo.bpm;
    let limit = GAP_BEATS * beat_s + 1e-9;
    let total_s = tm.bar_pos_to_s(score.bars as f64 + 1.0);
    for (li, layer) in score.layers.iter().enumerate() {
        if unpitched_class(layer.class) || sfx_class(layer.class) {
            continue;
        }
        let mut pairs = 0usize;
        let mut gaps = 0usize;
        let mut widest = 0.0f64;
        for (first, last) in runs(layer) {
            let mut notes: Vec<(f64, f64)> = events
                .iter()
                .filter(|e| e.layer == li && e.bar >= first && e.bar <= last)
                .map(|e| (e.start_s, e.start_s + e.length_s))
                .collect();
            notes.sort_by(|a, b| a.0.total_cmp(&b.0));
            let Some(&(first_start, first_end)) = notes.first() else {
                continue;
            };
            let mut ringing_until = first_end;
            for &(start, end) in &notes[1..] {
                let gap = start - ringing_until;
                pairs += 1;
                if gap > limit {
                    gaps += 1;
                    widest = widest.max(gap);
                }
                ringing_until = ringing_until.max(end);
            }
            let wraps =
                matches!(score.playback, Playback::Loop) && first <= 1 && last >= score.bars;
            if wraps && notes.len() > 1 {
                let gap = first_start + total_s - ringing_until;
                pairs += 1;
                if gap > limit {
                    gaps += 1;
                    widest = widest.max(gap);
                }
            }
        }
        if gaps > 0 && gaps as f64 >= GAP_SHARE * pairs as f64 {
            warn(
                out,
                IssueCode::IsolatedNotes,
                format!("/layers/{li}"),
                format!(
                    "layer `{}`: {gaps} of {pairs} note transitions leave more than {GAP_BEATS} beat of \
                     silence (widest {:.2} beats); keep notes on a repeating grid and let sustain connect them",
                    layer.id,
                    widest / beat_s
                ),
            );
        }
    }
}

/// Onset position unit: 1/48 of a whole note, the finest grid every [`Grid`] lands on exactly.
fn ticks_per_step(grid: Grid) -> u32 {
    match grid {
        Grid::Quarter => 12,
        Grid::Eighth => 6,
        Grid::Sixteenth => 3,
        Grid::EighthTriplet => 4,
        Grid::SixteenthTriplet => 2,
    }
}

/// Onset rhythm of each sounding bar of a layer: the set of onset positions (ticks from the bar
/// start, micro-timing offsets ignored). Uses the same clip/phase mapping as `expand_notes`.
fn bar_cells(score: &Score, layer: &Layer) -> BTreeMap<u32, BTreeSet<u32>> {
    let mut cells: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for clip in &layer.clips {
        let Some(p) = score.pattern(&clip.pattern) else {
            continue;
        };
        let Some(spb) = score.time_signature.steps_per_bar(p.grid) else {
            continue;
        };
        if p.length_steps == 0 || clip.bars.end < clip.bars.start {
            continue;
        }
        let last = clip.bars.end.min(score.bars);
        if last < clip.bars.start {
            continue;
        }
        let len = p.length_steps as u64;
        let off = clip.pattern_offset_steps as u64 % len;
        let clip_steps = (last - clip.bars.start + 1) as u64 * spb as u64;
        let tps = ticks_per_step(p.grid);
        for n in p.notes.iter().filter(|n| n.step < p.length_steps) {
            // Clip step `rel` plays pattern step `(rel + off) % len`.
            let mut rel = (n.step as u64 + len - off) % len;
            while rel < clip_steps {
                let bar = clip.bars.start + (rel / spb as u64) as u32;
                let step_in_bar = (rel % spb as u64) as u32;
                cells.entry(bar).or_default().insert(step_in_bar * tps);
                rel += len;
            }
        }
    }
    cells
}

/// Fewest sounding bars a layer needs before "never repeats" means anything.
const MIN_BARS_FOR_REPETITION: usize = 2;

/// `no_motif_repetition`: a layer (except sound-effect classes) with at least
/// [`MIN_BARS_FOR_REPETITION`] sounding bars in which no two bars share the same onset rhythm.
/// A motif-based part repeats a one-bar cell somewhere; a part that never does is improvised
/// noodling, which the owner rejected (rule 1: a repeating motif with deliberate accents).
pub(super) fn no_motif_repetition(score: &Score, out: &mut Vec<Issue>) {
    for (li, layer) in score.layers.iter().enumerate() {
        if sfx_class(layer.class) {
            continue;
        }
        let cells = bar_cells(score, layer);
        if cells.len() < MIN_BARS_FOR_REPETITION {
            continue;
        }
        let distinct: BTreeSet<&BTreeSet<u32>> = cells.values().collect();
        if distinct.len() == cells.len() {
            warn(
                out,
                IssueCode::NoMotifRepetition,
                format!("/layers/{li}"),
                format!(
                    "layer `{}`: none of its {} sounding bars repeats another bar's rhythm; \
                     build it from a repeating rhythmic cell",
                    layer.id,
                    cells.len()
                ),
            );
        }
    }
}
