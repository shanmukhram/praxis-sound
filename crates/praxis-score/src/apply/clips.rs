//! Copy-on-write clip editing (rule 4): split clips at range edges keeping pattern phase, and
//! canonicalise the clip list afterwards.

use crate::types::{BarRange, Clip, Score};

/// Steps per bar and pattern length for a pattern id, if the pattern exists and fits the bar.
pub(super) fn pattern_geometry(s: &Score, id: &str) -> Option<(u32, u32)> {
    let p = s.pattern(id)?;
    let spb = s.time_signature.steps_per_bar(p.grid)?;
    if p.length_steps == 0 {
        return None;
    }
    Some((spb, p.length_steps))
}

/// True when `inner` lies completely inside `outer`.
pub(super) fn inside(inner: BarRange, outer: BarRange) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

/// Removes bars `r` from `clips`. Clips crossing an edge are split; the right-hand piece gets the
/// pattern offset it needs to play exactly the steps it played before. Clips outside `r` are
/// returned unchanged and in their original order.
pub(super) fn carve(s: &Score, clips: &[Clip], r: BarRange) -> Result<Vec<Clip>, String> {
    let mut out = Vec::with_capacity(clips.len() + 2);
    for c in clips {
        if c.bars.end < r.start || c.bars.start > r.end {
            out.push(c.clone());
            continue;
        }
        if c.bars.start < r.start {
            let mut left = c.clone();
            left.bars.end = r.start - 1;
            out.push(left);
        }
        if c.bars.end > r.end {
            let (spb, len) = pattern_geometry(s, &c.pattern).ok_or_else(|| {
                format!(
                    "cannot split the clip at bars {}-{}: pattern `{}` is unknown or does not fit the bar",
                    c.bars.start, c.bars.end, c.pattern
                )
            })?;
            // Bars of the original clip that precede the right-hand piece.
            let skipped_bars = r.end.saturating_add(1).saturating_sub(c.bars.start) as u64;
            let shift = skipped_bars * spb as u64;
            let mut right = c.clone();
            right.bars.start = r.end.saturating_add(1);
            right.pattern_offset_steps =
                ((c.pattern_offset_steps as u64 + shift) % len as u64) as u32;
            out.push(right);
        }
    }
    Ok(out)
}

/// True when `b` continues `a` exactly: adjacent bars, same pattern, transpose and velocity scale,
/// and `b` starts at the pattern step where `a` would have continued. Merging such clips cannot
/// change a single note event.
fn exact_continuation(s: &Score, a: &Clip, b: &Clip) -> bool {
    if a.bars.end.checked_add(1) != Some(b.bars.start)
        || a.bars.end < a.bars.start
        || a.pattern != b.pattern
        || a.transpose != b.transpose
        || a.velocity_scale.to_bits() != b.velocity_scale.to_bits()
    {
        return false;
    }
    let Some((spb, len)) = pattern_geometry(s, &a.pattern) else {
        return false;
    };
    if b.pattern_offset_steps >= len {
        return false;
    }
    let a_steps = (a.bars.end - a.bars.start + 1) as u64 * spb as u64;
    (a.pattern_offset_steps as u64 + a_steps) % len as u64 == b.pattern_offset_steps as u64
}

/// Sorts clips by start (stable) and merges exact continuations across the edges of the edited
/// range `r` (at bar `r.start` and bar `r.end + 1`). Clips elsewhere keep their split points, so
/// bars outside the range keep their clips (rule 4).
pub(super) fn canonicalize(s: &Score, mut clips: Vec<Clip>, r: BarRange) -> Vec<Clip> {
    clips.sort_by_key(|c| (c.bars.start, c.bars.end));
    let mut out: Vec<Clip> = Vec::with_capacity(clips.len());
    for c in clips {
        let at_edge = c.bars.start == r.start || Some(c.bars.start) == r.end.checked_add(1);
        if at_edge
            && let Some(prev) = out.last_mut()
            && exact_continuation(s, prev, &c)
        {
            prev.bars.end = c.bars.end;
            continue;
        }
        out.push(c);
    }
    out
}
