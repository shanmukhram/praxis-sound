//! What a change did, measured: the bars each layer changed in (from the expanded notes and the
//! layer settings), the bars whose rendered master is bit-identical, and the take card's
//! "untouched" sentence built from those measurements.

use praxis_dsp::StereoBuffer;
use praxis_score::timeline::{NoteEvent, TimeMap, expand_notes};
use praxis_score::{Layer, Score};
use std::collections::{BTreeMap, BTreeSet};

/// Sample rate used only to compare note events.
const COMPARE_SR: u32 = 48_000;

fn events_by_bar(score: &Score, id: &str) -> BTreeMap<u32, Vec<NoteEvent>> {
    let mut out: BTreeMap<u32, Vec<NoteEvent>> = BTreeMap::new();
    let Some(li) = score.layers.iter().position(|l| l.id == id) else {
        return out;
    };
    for mut e in expand_notes(score, COMPARE_SR) {
        if e.layer == li {
            e.layer = 0;
            out.entry(e.bar).or_default().push(e);
        }
    }
    out
}

fn extent(layer: &Layer) -> Option<[u32; 2]> {
    Some([
        layer.clips.iter().map(|c| c.bars.start).min()?,
        layer.clips.iter().map(|c| c.bars.end).max()?,
    ])
}

/// The layer's settings that change how it sounds (not its clips, which the note comparison
/// covers, and not its lock).
fn sound_settings(l: &Layer) -> Layer {
    let mut x = l.clone();
    x.clips.clear();
    x.locked = false;
    x
}

/// Bars each layer changed in from `before` to `after`: `{layer id: [first, last]}`.
///
/// A layer whose notes differ counts the bars where they differ. A layer whose sound settings
/// (instrument, gain, pan, effects, mute, solo...) differ, or that was added or removed, counts
/// its whole extent. A change of tempo, key, metre, length or master counts every layer.
pub fn changed_layers(before: &Score, after: &Score) -> BTreeMap<String, [u32; 2]> {
    let mut out = BTreeMap::new();
    let global = before.tempo != after.tempo
        || before.time_signature != after.time_signature
        || before.bars != after.bars
        || before.key != after.key
        || before.master != after.master
        || before.playback != after.playback
        || before.seed != after.seed;
    let ids: BTreeSet<&str> = before
        .layers
        .iter()
        .chain(after.layers.iter())
        .map(|l| l.id.as_str())
        .collect();
    for id in ids {
        let (a, b) = (before.layer(id), after.layer(id));
        let whole = match (a, b) {
            (Some(x), Some(y)) => global || sound_settings(x) != sound_settings(y),
            _ => true,
        };
        let range = if whole {
            let ext = [a.and_then(extent), b.and_then(extent)];
            let lo = ext.iter().flatten().map(|r| r[0]).min();
            let hi = ext.iter().flatten().map(|r| r[1]).max();
            lo.zip(hi).map(|(lo, hi)| [lo, hi])
        } else {
            let ea = events_by_bar(before, id);
            let eb = events_by_bar(after, id);
            let bars: Vec<u32> = ea
                .keys()
                .chain(eb.keys())
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter(|bar| ea.get(bar) != eb.get(bar))
                .collect();
            bars.first().zip(bars.last()).map(|(lo, hi)| [*lo, *hi])
        };
        if let Some(r) = range {
            out.insert(id.to_string(), r);
        }
    }
    out
}

/// Runs of bars (`[first, last]`) in which `a` and `b` are bit-identical on both channels.
/// Empty when the two renders differ in length.
pub fn identical_bars(
    score: &Score,
    a: &StereoBuffer,
    b: &StereoBuffer,
    sample_rate: u32,
) -> Vec<[u32; 2]> {
    if a.len() != b.len() || a.is_empty() {
        return Vec::new();
    }
    let tm = TimeMap::new(score, sample_rate);
    let len = a.len();
    let same = |lo: usize, hi: usize| {
        (lo..hi).all(|i| {
            a.left[i].to_bits() == b.left[i].to_bits()
                && a.right[i].to_bits() == b.right[i].to_bits()
        })
    };
    let mut runs: Vec<[u32; 2]> = Vec::new();
    for bar in 1..=score.bars {
        let lo = (tm.bar_start_sample(bar) as usize).min(len);
        let hi = (tm.bar_start_sample(bar + 1) as usize).min(len);
        if hi <= lo || !same(lo, hi) {
            continue;
        }
        match runs.last_mut() {
            Some(r) if r[1] + 1 == bar => r[1] = bar,
            _ => runs.push([bar, bar]),
        }
    }
    runs
}

/// `"1"` or `"1–12"`.
pub fn bar_span(r: [u32; 2]) -> String {
    if r[0] == r[1] {
        format!("{}", r[0])
    } else {
        format!("{}\u{2013}{}", r[0], r[1])
    }
}

/// `"bar 3"` or `"bars 13–16"`.
pub fn bars_label(r: [u32; 2]) -> String {
    if r[0] == r[1] {
        format!("bar {}", r[0])
    } else {
        format!("bars {}", bar_span(r))
    }
}

/// The take card's "untouched" sentence: the bars measured bit-identical to take `prev` (or, when
/// no bar is, the bars outside the change that are identical note for note), then each existing
/// layer the change left alone.
pub fn untouched_text(
    before: &Score,
    after: &Score,
    changed: &BTreeMap<String, [u32; 2]>,
    identical: &[[u32; 2]],
    prev: u32,
) -> String {
    let mut out: Vec<String> = Vec::new();
    let join = |runs: &[[u32; 2]]| {
        let many = runs.len() > 1 || runs.iter().any(|r| r[0] != r[1]);
        format!(
            "{} {}",
            if many { "Bars" } else { "Bar" },
            runs.iter()
                .map(|r| bar_span(*r))
                .collect::<Vec<_>>()
                .join(" and ")
        )
    };
    if !identical.is_empty() {
        out.push(format!(
            "{} sample-identical to take {prev}.",
            join(identical)
        ));
    } else if !changed.is_empty() {
        let lo = changed.values().map(|r| r[0]).min().unwrap_or(1);
        let hi = changed.values().map(|r| r[1]).max().unwrap_or(after.bars);
        let mut spans = Vec::new();
        if lo > 1 {
            spans.push([1, lo - 1]);
        }
        if hi < after.bars {
            spans.push([hi + 1, after.bars]);
        }
        if !spans.is_empty() {
            out.push(format!(
                "{} identical note for note to take {prev}.",
                join(&spans)
            ));
        }
    }
    for l in &after.layers {
        if !changed.contains_key(&l.id) && before.layer(&l.id).is_some() {
            let name = if l.name.is_empty() { &l.id } else { &l.name };
            out.push(format!("{name} unchanged."));
        }
    }
    out.join(" ")
}
