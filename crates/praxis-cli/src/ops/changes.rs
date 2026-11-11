//! What a change did, measured: the bars each layer changed in (from the expanded notes and the
//! layer settings), the bars whose rendered master is bit-identical, and the take card's
//! "untouched" sentence built from those measurements.

use praxis_dsp::StereoBuffer;
use praxis_score::params::effect_param_value;
use praxis_score::timeline::{NoteEvent, TimeMap, expand_notes};
use praxis_score::{Automation, Curve, Effect, EffectKind, Layer, Playback, Score};
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

/// The layer's settings that change how it sounds, apart from its effects and automation (read
/// bar by bar in [`sound_change_bars`]), its clips (the note comparison covers them) and its
/// lock.
fn base_settings(l: &Layer) -> Layer {
    let mut x = l.clone();
    x.clips.clear();
    x.locked = false;
    x.effects.clear();
    x.automation.clear();
    x
}

/// Effect kinds that pass their input through unchanged at `mix` 0 and add no latency, so a new
/// one held at zero mix outside some bars changes nothing there.
const SILENT_AT_ZERO_MIX: [EffectKind; 6] = [
    EffectKind::Filter,
    EffectKind::Compressor,
    EffectKind::Delay,
    EffectKind::Reverb,
    EffectKind::ConvolutionReverb,
    EffectKind::Chorus,
];

/// One automation curve's value at a bar position, as the engine evaluates it
/// (`praxis_engine::automation::Lane`): the first value holds before the first point, the last
/// after the last point.
fn lane_value(auto: &Automation, pos: f64) -> f64 {
    let mut pts: Vec<(f64, f64)> = auto
        .points
        .iter()
        .filter(|p| p.bar.is_finite() && p.value.is_finite())
        .map(|p| (p.bar, p.value))
        .collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some(first) = pts.first() else {
        return 0.0;
    };
    let i = pts.partition_point(|p| p.0 <= pos);
    if i == 0 {
        return first.1;
    }
    if i == pts.len() {
        return pts[pts.len() - 1].1;
    }
    let (a, b) = (pts[i - 1], pts[i]);
    let span = b.0 - a.0;
    if span <= 0.0 {
        return b.1;
    }
    let f = ((pos - a.0) / span).clamp(0.0, 1.0);
    match auto.curve {
        Curve::Step => a.1,
        Curve::Linear => a.1 + (b.1 - a.1) * f,
        Curve::Smooth => {
            let e = (1.0 - (std::f64::consts::PI * f).cos()) * 0.5;
            a.1 + (b.1 - a.1) * e
        }
    }
}

/// The lane the engine uses for `target` (the first one; later duplicates are skipped).
fn lane<'a>(l: &'a Layer, target: &str) -> Option<&'a Automation> {
    l.automation.iter().find(|a| a.target == target)
}

/// A preset parameter's default, for an instrument parameter the layer leaves unset.
fn preset_default(l: &Layer, name: &str) -> Option<f64> {
    use praxis_engine::InstrumentFactory;
    praxis_instruments::Registry::with_default_packs()
        .describe(l.instrument.kind, &l.instrument.preset)?
        .params
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.default)
}

/// A parameter's value without automation. For an effect that `l` does not have, `mix` is 0
/// (the effect is not heard) and other parameters do not matter (`None`).
fn static_value(l: &Layer, target: &str) -> Option<f64> {
    if target == "gain_db" {
        return Some(l.gain_db as f64);
    }
    if target == "pan" {
        return Some(l.pan as f64);
    }
    if let Some(name) = target.strip_prefix("instrument.") {
        return l
            .instrument
            .params
            .get(name)
            .copied()
            .or_else(|| preset_default(l, name));
    }
    let (id, name) = target.strip_prefix("effects.")?.split_once('.')?;
    match l.effects.iter().find(|e| e.id == id) {
        Some(e) => Some(effect_param_value(e.kind, &e.params, name)),
        None if name == "mix" => Some(0.0),
        None => None,
    }
}

/// The bars in which the effects and automation of `x` (before) and `y` (after) sound
/// different, or `None` when the change reaches the whole layer. `x` and `y` have the same base
/// settings (instrument, gain, pan, mute, solo, ...).
///
/// The change is read bar by bar: an existing effect must stay as it was (its parameters move
/// only through automation); a new effect counts only where it is heard (bypassed: nowhere; a
/// kind that passes its input through at `mix` 0: where its `mix` is above 0); an automation
/// lane counts where its value differs from the value before (the old lane, or the parameter's
/// fixed value). Gain and pan glide over one engine block (128 samples), so a change of theirs
/// also counts the bar before it when it starts within a block of a bar line.
fn sound_change_bars(before: &Score, x: &Layer, y: &Layer) -> Option<BTreeSet<u32>> {
    let mut bars = BTreeSet::new();
    if x.effects == y.effects && x.automation == y.automation {
        return Some(bars);
    }
    // Existing effects: the same, in the same order; new ones only in between.
    let kept: Vec<&Effect> = y
        .effects
        .iter()
        .filter(|e| x.effects.iter().any(|o| o.id == e.id))
        .collect();
    if kept.len() != x.effects.len() || kept.iter().zip(&x.effects).any(|(a, b)| *a != b) {
        return None;
    }
    let mut targets: Vec<String> = Vec::new();
    let mut add = |t: &str| {
        if !targets.iter().any(|x| x == t) {
            targets.push(t.to_string());
        }
    };
    let new_effect = |id: &str| {
        y.effects
            .iter()
            .find(|e| e.id == id && !x.effects.iter().any(|o| o.id == e.id))
    };
    for e in &y.effects {
        if x.effects.iter().any(|o| o.id == e.id) || e.bypass {
            continue;
        }
        if !SILENT_AT_ZERO_MIX.contains(&e.kind) {
            return None;
        }
        add(&format!("effects.{}.mix", e.id));
    }
    for a in x.automation.iter().chain(&y.automation) {
        // A new effect's own lanes matter only where its `mix` lets it be heard.
        if let Some((id, name)) = a
            .target
            .strip_prefix("effects.")
            .and_then(|r| r.split_once('.'))
            && let Some(e) = new_effect(id)
            && (e.bypass || name != "mix")
        {
            continue;
        }
        add(&a.target);
    }

    let last = before.bars.max(1);
    let one_shot = matches!(before.playback, Playback::OneShot { .. });
    let tm = TimeMap::new(before, COMPARE_SR);
    // One engine block at the lowest common sample rate, in bars.
    let block_bars = 128.0 / (44_100.0 * tm.bar_s().max(1e-9));
    for t in &targets {
        let (la, lb) = (lane(x, t), lane(y, t));
        let (sa, sb) = (static_value(x, t), static_value(y, t));
        if la == lb && (la.is_some() || sa == sb) {
            continue;
        }
        let glides = t == "gain_db" || t == "pan";
        let known = t == "gain_db"
            || t == "pan"
            || t.starts_with("instrument.")
            || t.starts_with("effects.");
        if !known {
            return None;
        }
        let value = |l: Option<&Automation>, s: Option<f64>, pos: f64| match l {
            Some(l) => Some(lane_value(l, pos)),
            None => s,
        };
        // Every point of both lanes and every bar line splits the sound into pieces on which
        // both curves are smooth; each piece is compared at its start and three inner points.
        let mut cuts: Vec<f64> = (1..=last + 1).map(f64::from).collect();
        for p in la.iter().chain(lb.iter()).flat_map(|l| l.points.iter()) {
            if p.bar.is_finite() && p.bar > 1.0 && p.bar < (last + 1) as f64 {
                cuts.push(p.bar);
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        for w in cuts.windows(2) {
            let (p0, p1) = (w[0], w[1]);
            let differs = (0..4).any(|k| {
                let pos = p0 + (p1 - p0) * k as f64 / 4.0;
                let (va, vb) = (value(la, sa, pos), value(lb, sb, pos));
                va.is_none() || vb.is_none() || va != vb
            });
            if differs {
                bars.insert(p0.floor() as u32);
                if glides && p0 - block_bars >= 1.0 {
                    bars.insert((p0 - block_bars).floor() as u32);
                }
            }
        }
        if one_shot {
            let end = (last + 1) as f64;
            // The tail after the last bar holds the last value.
            if value(la, sa, end) != value(lb, sb, end) {
                bars.insert(last);
            }
            // Parameters start at their fixed value and glide to the lane's first value at the
            // start of the render; loops start from the cycle before, one-shots do not.
            if !glides && (sa, value(la, sa, 1.0)) != (sb, value(lb, sb, 1.0)) {
                bars.insert(1);
            }
        }
    }
    bars.retain(|b| (1..=last).contains(b));
    Some(bars)
}

/// Bars each layer changed in from `before` to `after`: `{layer id: [first, last]}`.
///
/// A layer whose notes differ counts the bars where they differ. A change to its effects or
/// automation counts the bars where it sounds different ([`sound_change_bars`]): automation that
/// stays at the old value outside some bars, or a new effect held at zero mix there, counts only
/// those bars. A layer whose other sound settings (instrument, gain, pan, mute, solo...) differ,
/// whose existing effects changed, or that was added or removed, counts its whole extent. A
/// change of tempo, key, metre, length or master counts every layer.
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
        let sound = match (a, b) {
            (Some(x), Some(y)) if !global && base_settings(x) == base_settings(y) => {
                sound_change_bars(before, x, y)
            }
            _ => None,
        };
        let range = match sound {
            None => {
                let ext = [a.and_then(extent), b.and_then(extent)];
                let lo = ext.iter().flatten().map(|r| r[0]).min();
                let hi = ext.iter().flatten().map(|r| r[1]).max();
                lo.zip(hi).map(|(lo, hi)| [lo, hi])
            }
            Some(mut bars) => {
                let ea = events_by_bar(before, id);
                let eb = events_by_bar(after, id);
                bars.extend(
                    ea.keys()
                        .chain(eb.keys())
                        .copied()
                        .filter(|bar| ea.get(bar) != eb.get(bar)),
                );
                bars.first().zip(bars.last()).map(|(lo, hi)| [*lo, *hi])
            }
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

/// The level below which a difference between two renders is float rounding, not sound
/// (contracts/patch.md: recirculating effects may leave differences below -120 dBFS outside the
/// changed bars).
pub const ROUNDING_DBFS: f64 = -120.0;

/// The largest difference between `a` and `b` in each bar, in dBFS (index 0 is bar 1;
/// `-inf` for a bit-identical bar). Empty when the two renders differ in length.
pub fn bar_differences_db(
    score: &Score,
    a: &StereoBuffer,
    b: &StereoBuffer,
    sample_rate: u32,
) -> Vec<f64> {
    if a.len() != b.len() || a.is_empty() {
        return Vec::new();
    }
    let tm = TimeMap::new(score, sample_rate);
    let len = a.len();
    (1..=score.bars)
        .map(|bar| {
            let lo = (tm.bar_start_sample(bar) as usize).min(len);
            let hi = (tm.bar_start_sample(bar + 1) as usize).min(len);
            let peak = (lo..hi)
                .map(|i| {
                    let l = (a.left[i] as f64 - b.left[i] as f64).abs();
                    let r = (a.right[i] as f64 - b.right[i] as f64).abs();
                    if l.is_nan() || r.is_nan() {
                        f64::INFINITY
                    } else {
                        l.max(r)
                    }
                })
                .fold(0.0f64, f64::max);
            20.0 * peak.log10()
        })
        .collect()
}

/// Whether the layers that changed run through a recirculating effect (reverb, delay), whose
/// tails carry rounding-level differences everywhere.
fn recirculates(after: &Score, changed: &BTreeMap<String, [u32; 2]>) -> bool {
    let tails = |e: &Effect| {
        !e.bypass
            && matches!(
                e.kind,
                EffectKind::Reverb | EffectKind::ConvolutionReverb | EffectKind::Delay
            )
    };
    after.master.effects.iter().any(tails)
        || after
            .layers
            .iter()
            .filter(|l| changed.contains_key(&l.id))
            .any(|l| l.effects.iter().any(tails))
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
    untouched_text_measured(before, after, changed, identical, &[], prev)
}

/// [`untouched_text`] with each bar's largest difference from take `prev`
/// ([`bar_differences_db`]). When no bar is bit-identical only because a recirculating effect
/// carries rounding-level differences (below [`ROUNDING_DBFS`]) into the bars outside the change,
/// it says so with the measured residue: "Bars 3–12 identical note for note to take 4; only
/// reverb rounding differs (at most -131 dBFS)."
pub fn untouched_text_measured(
    before: &Score,
    after: &Score,
    changed: &BTreeMap<String, [u32; 2]>,
    identical: &[[u32; 2]],
    bar_db: &[f64],
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
        // Bars outside the change whose render differs only by rounding.
        let rounding: Vec<u32> = spans
            .iter()
            .flat_map(|s| s[0]..=s[1])
            .filter(|&bar| {
                bar_db
                    .get(bar as usize - 1)
                    .is_some_and(|d| *d < ROUNDING_DBFS)
            })
            .collect();
        if !rounding.is_empty() {
            let mut runs: Vec<[u32; 2]> = Vec::new();
            for &bar in &rounding {
                match runs.last_mut() {
                    Some(r) if r[1] + 1 == bar => r[1] = bar,
                    _ => runs.push([bar, bar]),
                }
            }
            let worst = rounding
                .iter()
                .map(|&bar| bar_db[bar as usize - 1])
                .fold(f64::NEG_INFINITY, f64::max);
            let what = if recirculates(after, changed) {
                "reverb rounding"
            } else {
                "float rounding"
            };
            out.push(format!(
                "{} identical note for note to take {prev}; only {what} differs (at most {:.0} \
                 dBFS).",
                join(&runs),
                worst
            ));
        } else if !spans.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_score::AutoPoint;
    use serde_json::{Value, json};

    fn loop_score() -> Score {
        let plan: Value = serde_json::from_str(include_str!(
            "../../../praxis-planner/tests/prompts/acceptance1.plan.json"
        ))
        .unwrap();
        praxis_planner::compose_plan(&plan, &praxis_planner::Catalogue::standard(), 1)
            .unwrap()
            .score
    }

    fn lane(target: &str, curve: Curve, pts: &[(f64, f64)]) -> Automation {
        Automation {
            target: target.into(),
            curve,
            points: pts
                .iter()
                .map(|&(bar, value)| AutoPoint { bar, value })
                .collect(),
        }
    }

    fn reverb(id: &str, mix: Option<f64>, bypass: bool) -> Effect {
        serde_json::from_value(json!({
            "id": id,
            "kind": "reverb",
            "params": mix.map(|m| json!({"mix": m})).unwrap_or(json!({})),
            "bypass": bypass,
        }))
        .unwrap()
    }

    fn piano(s: &mut Score) -> &mut Layer {
        s.layers.iter_mut().find(|l| l.id == "piano").unwrap()
    }

    #[test]
    fn automation_that_keeps_the_old_value_counts_only_its_bars() {
        let mut before = loop_score();
        let tone = 0.5;
        piano(&mut before)
            .instrument
            .params
            .insert("tone".into(), tone);
        let changed = |f: &dyn Fn(&mut Layer)| {
            let mut after = before.clone();
            f(piano(&mut after));
            changed_layers(&before, &after).get("piano").copied()
        };
        // A step lane: old value, then 0.2 in bars 5-8, then the old value again.
        let step = lane(
            "instrument.tone",
            Curve::Step,
            &[(1.0, tone), (5.0, 0.2), (9.0, tone)],
        );
        assert_eq!(
            changed(&|l| {
                l.automation = vec![step.clone()];
            }),
            Some([5, 8])
        );
        // A ramp that starts inside bar 4 counts bar 4.
        let ramp = lane(
            "instrument.tone",
            Curve::Linear,
            &[(4.5, tone), (5.0, 0.2), (9.0, 0.2), (9.0, tone)],
        );
        assert_eq!(
            changed(&|l| {
                l.automation = vec![ramp.clone()];
            }),
            Some([4, 8])
        );
        // Gain glides over one block, so a jump at bar 5 also reaches the end of bar 4.
        let gain = lane(
            "gain_db",
            Curve::Step,
            &[(1.0, 0.0), (5.0, -6.0), (9.0, 0.0)],
        );
        let g = |l: &mut Layer| {
            l.gain_db = 0.0;
            l.automation = vec![gain.clone()];
        };
        let mut b0 = before.clone();
        piano(&mut b0).gain_db = 0.0;
        let mut a0 = b0.clone();
        g(piano(&mut a0));
        assert_eq!(changed_layers(&b0, &a0).get("piano"), Some(&[4, 8]));
        // The fixed value itself changed: every bar the layer plays.
        let whole = changed(&|l| {
            l.instrument.params.insert("tone".into(), 0.2);
        })
        .unwrap();
        assert_eq!(whole[0], 1);
        assert!(whole[1] >= 16);
    }

    #[test]
    fn a_new_effect_counts_where_it_is_heard() {
        let before = loop_score();
        let changed = |f: &dyn Fn(&mut Layer)| {
            let mut after = before.clone();
            f(piano(&mut after));
            changed_layers(&before, &after).get("piano").copied()
        };
        let mix = lane(
            "effects.lift.mix",
            Curve::Step,
            &[(1.0, 0.0), (13.0, 0.3), (17.0, 0.0)],
        );
        // At zero mix outside bars 13-16 (and 0 as its fixed value): only those bars.
        assert_eq!(
            changed(&|l| {
                l.effects.push(reverb("lift", Some(0.0), false));
                l.automation.push(mix.clone());
            }),
            Some([13, 16])
        );
        // Its other parameters do not matter where it is not heard.
        assert_eq!(
            changed(&|l| {
                l.effects.push(reverb("lift", Some(0.0), false));
                l.automation.push(mix.clone());
                l.automation.push(lane(
                    "effects.lift.size",
                    Curve::Linear,
                    &[(1.0, 0.2), (17.0, 0.9)],
                ));
            }),
            Some([13, 16])
        );
        // Bypassed: nowhere.
        assert_eq!(
            changed(&|l| l.effects.push(reverb("lift", None, true))),
            None
        );
        // Heard everywhere (default mix, no lane): every bar.
        assert_eq!(
            changed(&|l| l.effects.push(reverb("lift", None, false))).map(|r| r[0]),
            Some(1)
        );
        // An EQ has no mix: it is heard everywhere.
        assert_eq!(
            changed(&|l| {
                l.effects.push(
                    serde_json::from_value(
                        json!({"id": "dark", "kind": "eq", "params": {"lp_hz": 3000.0}}),
                    )
                    .unwrap(),
                )
            })
            .map(|r| r[0]),
            Some(1)
        );
    }

    #[test]
    fn rounding_level_differences_read_as_rounding() {
        let before = loop_score();
        let mut after = before.clone();
        after
            .layers
            .iter_mut()
            .find(|l| l.id == "piano")
            .unwrap()
            .effects = vec![reverb("lift", Some(0.2), false)];
        let mut changed = BTreeMap::new();
        changed.insert("piano".to_string(), [13, 16]);
        // Bars 1-2 hold the tail (audible), 3-12 differ below -120 dBFS.
        let mut db = vec![-140.0; 16];
        db[0] = -40.0;
        db[1] = -70.0;
        db[4] = -125.5;
        let text = untouched_text_measured(&before, &after, &changed, &[], &db, 4);
        assert!(
            text.starts_with(
                "Bars 3\u{2013}12 identical note for note to take 4; only reverb rounding \
                 differs (at most -126 dBFS)."
            ),
            "{text}"
        );
        // Above the threshold: no rounding claim.
        let loud = vec![-90.0; 16];
        let text = untouched_text_measured(&before, &after, &changed, &[], &loud, 4);
        assert!(
            text.starts_with("Bars 1\u{2013}12 identical note for note to take 4."),
            "{text}"
        );
    }
}
