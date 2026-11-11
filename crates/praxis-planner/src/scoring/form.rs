//! Multi-section form (FR-9): form parts solved to the piece's bars, with part boundaries moved
//! onto hit points, and the layers of each part chosen by its intensity (layering: the first
//! layer alone at the quietest part, every layer at the loudest; a part never rewrites a layer,
//! it only adds or removes whole ones).

use super::emotion::role_intensity;
use super::emotion::role_word;
use crate::plan::{FormPart, PlanSection, ScorePlan};
use praxis_score::BarRange;

/// Phrase length the solver rounds part lengths to.
fn phrase_unit(total: u32) -> u32 {
    if total >= 32 {
        4
    } else if total >= 12 {
        2
    } else {
        1
    }
}

/// Splits `total` bars over parts by weight (fixed `bars` parts take theirs first when they
/// fit), each at least one bar, lengths rounded to `unit` where the total allows.
fn allocate(parts: &[&FormPart], total: u32, unit: u32) -> Vec<u32> {
    let n = parts.len();
    if n == 0 {
        return vec![];
    }
    if total <= n as u32 {
        // One bar each, the rest to the last part (cannot happen when callers check).
        let mut v = vec![1; n];
        if let Some(l) = v.last_mut() {
            *l += total.saturating_sub(n as u32);
        }
        return v;
    }
    let mut out = vec![0u32; n];
    let fixed: u32 = parts.iter().filter_map(|p| p.bars).sum();
    let free_parts: Vec<usize> = (0..n).filter(|i| parts[*i].bars.is_none()).collect();
    let use_fixed = fixed + free_parts.len() as u32 <= total;
    let mut rest = total;
    if use_fixed {
        for (i, p) in parts.iter().enumerate() {
            if let Some(b) = p.bars {
                out[i] = b.max(1);
                rest -= out[i];
            }
        }
    }
    let shared: Vec<usize> = if use_fixed {
        free_parts
    } else {
        (0..n).collect()
    };
    if shared.is_empty() {
        // Every part is fixed: the last absorbs the difference.
        let sum: u32 = out.iter().sum();
        if let Some(l) = out.last_mut() {
            *l = (*l as i64 + total as i64 - sum as i64).max(1) as u32;
        }
        return out;
    }
    let weights: Vec<f64> = shared
        .iter()
        .map(|i| {
            let w = parts[*i].weight as f64;
            if w.is_finite() && w > 0.0 { w } else { 1.0 }
        })
        .collect();
    let wsum: f64 = weights.iter().sum();
    let units = rest / unit;
    let use_units = units as usize >= shared.len();
    let (pool, scale) = if use_units { (units, unit) } else { (rest, 1) };
    // Largest remainder, each share at least one.
    let raw: Vec<f64> = weights.iter().map(|w| w / wsum * pool as f64).collect();
    let mut share: Vec<u32> = raw.iter().map(|r| (r.floor() as u32).max(1)).collect();
    let mut given: i64 = share.iter().sum::<u32>() as i64;
    let mut order: Vec<usize> = (0..shared.len()).collect();
    order.sort_by(|a, b| {
        let fa = raw[*a] - raw[*a].floor();
        let fb = raw[*b] - raw[*b].floor();
        fb.total_cmp(&fa).then(a.cmp(b))
    });
    let mut k = 0;
    while given < pool as i64 {
        share[order[k % order.len()]] += 1;
        given += 1;
        k += 1;
    }
    while given > pool as i64 {
        // Take from the largest share above one.
        if let Some(j) = (0..share.len())
            .filter(|j| share[*j] > 1)
            .max_by_key(|j| (share[*j], usize::MAX - *j))
        {
            share[j] -= 1;
            given -= 1;
        } else {
            break;
        }
    }
    for (s, i) in share.iter().zip(&shared) {
        out[*i] = s * scale;
    }
    // Bars left over by the unit rounding go to the last shared part.
    let sum: u32 = out.iter().sum();
    if sum < total
        && let Some(last) = shared.last()
    {
        out[*last] += total - sum;
    }
    out
}

/// Section start bars (`starts[0]` = 1) of `parts` over `total` bars, with boundaries moved onto
/// `targets` (bars where a hit wants a section to start) when the parts can still fit.
pub fn solve_starts(parts: &[FormPart], total: u32, targets: &[u32]) -> Vec<u32> {
    let refs: Vec<&FormPart> = parts.iter().collect();
    let n = parts.len();
    if n == 0 {
        return vec![];
    }
    let n = n.min(total.max(1) as usize);
    let refs = &refs[..n];
    let unit = phrase_unit(total);
    let lengths = allocate(refs, total, unit);
    let mut starts: Vec<u32> = Vec::with_capacity(n);
    let mut b = 1;
    for l in &lengths {
        starts.push(b);
        b += l;
    }
    // Anchors: (part index, start bar), fixed by hits. The assignment places as many hits as
    // possible on boundaries (in order), then moves the boundaries as little as possible.
    let mut best: (usize, i64, Vec<(usize, u32)>) = (0, 0, vec![]);
    let mut chosen: Vec<(usize, u32)> = Vec::new();
    assign(&starts, targets, 0, total, n, &mut chosen, 0, &mut best);
    let mut anchors: Vec<(usize, u32)> = vec![(0, 1)];
    anchors.extend(best.2);
    anchors.push((n, total + 1));
    // Re-space the parts between anchors by weight.
    let mut out = vec![1u32; n];
    for w in anchors.windows(2) {
        let ((i0, b0), (i1, b1)) = (w[0], w[1]);
        if i1 <= i0 {
            continue;
        }
        let seg = allocate(&refs[i0..i1], b1 - b0, unit);
        let mut b = b0;
        for (k, l) in seg.iter().enumerate() {
            out[i0 + k] = b;
            b += l;
        }
    }
    out
}

/// Exhaustive search over hit -> boundary assignments (hits and parts are few).
#[allow(clippy::too_many_arguments)]
fn assign(
    starts: &[u32],
    targets: &[u32],
    i: usize,
    total: u32,
    n: usize,
    chosen: &mut Vec<(usize, u32)>,
    dist: i64,
    best: &mut (usize, i64, Vec<(usize, u32)>),
) {
    if chosen.len() > best.0 || (chosen.len() == best.0 && dist < best.1) {
        *best = (chosen.len(), dist, chosen.clone());
    }
    if i >= targets.len() {
        return;
    }
    let t = targets[i];
    let (prev_j, prev_bar) = chosen.last().copied().unwrap_or((0, 1));
    for j in (prev_j + 1)..n {
        if t > prev_bar
            && t <= total
            && t - prev_bar >= (j - prev_j) as u32
            && total + 1 - t >= (n - j) as u32
        {
            chosen.push((j, t));
            let d = (starts[j] as i64 - t as i64).abs();
            assign(starts, targets, i + 1, total, n, chosen, dist + d, best);
            chosen.pop();
        }
    }
    assign(starts, targets, i + 1, total, n, chosen, dist, best);
}

/// Intensity of a part: its own, else the role's usual level.
pub fn part_intensity(p: &FormPart) -> f32 {
    p.intensity
        .filter(|x| x.is_finite())
        .map(|x| x.clamp(0.0, 1.0))
        .unwrap_or_else(|| role_intensity(p.role))
}

/// How many of `n` layers play at intensity `x`, given the form's quietest and loudest parts.
pub fn layer_count(x: f32, min: f32, max: f32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    if max - min < 1e-3 {
        return n;
    }
    let r = ((x - min) / (max - min)).clamp(0.0, 1.0);
    (1 + ((n - 1) as f32 * r).round() as usize).clamp(1, n)
}

/// A part's display name: its own, or the role's name with a running letter for repeated
/// verse/main parts ("A", "B" style forms keep the names the plan gives).
fn part_name(p: &FormPart, index: usize, parts: &[FormPart]) -> String {
    if !p.name.trim().is_empty() {
        return p.name.trim().to_string();
    }
    let word = role_word(p.role);
    let same = parts.iter().filter(|q| q.role == p.role).count();
    let mut s = crate::arrange::display_name(word);
    if same > 1 {
        let nth = parts[..index].iter().filter(|q| q.role == p.role).count() + 1;
        s = format!("{s} {nth}");
    }
    s
}

/// Sections of a form plan over `total` bars with boundaries pulled to `targets`.
pub fn sections(plan: &ScorePlan, total: u32, targets: &[u32]) -> Vec<PlanSection> {
    let parts = &plan.form;
    let starts = solve_starts(parts, total, targets);
    let n = starts.len();
    let intens: Vec<f32> = parts[..n].iter().map(part_intensity).collect();
    let (min, max) = intens
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(*x), b.max(*x)));
    let ids: Vec<String> = plan.layers.iter().map(|l| l.id.clone()).collect();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let start = starts[i];
        let end = if i + 1 < n { starts[i + 1] - 1 } else { total };
        let p = &parts[i];
        let layers: Vec<String> = if p.layers.is_empty() {
            ids[..layer_count(intens[i], min, max, ids.len())].to_vec()
        } else {
            // Keep plan order, only known ids.
            ids.iter()
                .filter(|id| p.layers.iter().any(|l| l == *id))
                .cloned()
                .collect()
        };
        out.push(PlanSection {
            name: part_name(p, i, parts),
            bars: BarRange { start, end },
            role: p.role,
            intensity: (intens[i] * 100.0).round() / 100.0,
            layers,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Transition;
    use praxis_score::SectionRole;

    fn part(role: SectionRole, weight: f32) -> FormPart {
        FormPart {
            role,
            name: String::new(),
            weight,
            bars: None,
            intensity: None,
            layers: vec![],
            transition: Transition::Auto,
        }
    }

    fn arc() -> Vec<FormPart> {
        vec![
            part(SectionRole::Intro, 2.0),
            part(SectionRole::Build, 3.0),
            part(SectionRole::Peak, 3.0),
            part(SectionRole::Release, 2.0),
        ]
    }

    #[test]
    fn parts_fill_the_piece_in_phrases() {
        let s = solve_starts(&arc(), 48, &[]);
        assert_eq!(s, vec![1, 9, 25, 41]);
        let s = solve_starts(&arc(), 45, &[]);
        assert_eq!(s[0], 1);
        assert!(s.windows(2).all(|w| w[0] < w[1]));
        assert!(*s.last().unwrap() <= 45);
        let s = solve_starts(&arc(), 3, &[]);
        assert_eq!(s, vec![1, 2, 3]);
    }

    #[test]
    fn hits_take_the_nearest_boundaries() {
        let s = solve_starts(&arc(), 48, &[18, 30, 41]);
        assert_eq!(s, vec![1, 18, 30, 41]);
        // A hit too early for the parts before it is skipped.
        let s = solve_starts(&arc(), 48, &[1, 30]);
        assert_eq!(s[2], 30);
    }

    #[test]
    fn layers_enter_with_intensity() {
        assert_eq!(layer_count(0.3, 0.3, 0.85, 3), 1);
        assert_eq!(layer_count(0.6, 0.3, 0.85, 3), 2);
        assert_eq!(layer_count(0.85, 0.3, 0.85, 3), 3);
        assert_eq!(layer_count(0.5, 0.5, 0.5, 3), 3);
    }
}
