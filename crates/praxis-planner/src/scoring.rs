//! Scoring depth (FR-9, FR-10; WP-P3): emotion presets, chord progressions with voice-leading,
//! multi-section form solved to the piece's length, a tension curve that drives dynamics,
//! density, a gentle filter and layer entries, musical transitions, and hit points the
//! arrangement lands on.
//!
//! A plan opts in with `form`, `hits` or `progression` ([`ScorePlan::is_scored`]); every other
//! plan arranges exactly as before. [`realise`] turns the scoring fields into ordinary plan
//! fields (tempo, bars, sections, chords) before the arranger runs; it is idempotent, so the plan
//! a [`crate::PlanOutcome`] returns arranges to the same Score again. The arranger's scored path
//! is in [`develop`].

pub mod develop;
pub mod emotion;
pub mod form;
pub mod harmony;
pub mod hits;
pub mod words;

use crate::plan::{
    MAX_BPM, MIN_BPM, PlanIssue, PlanIssueCode, ScorePlan, TempoRange, resolve_timing,
};
use crate::repair::Repair;
use praxis_score::{Grid, SectionRole};

/// Most form parts a plan may have.
pub const MAX_PARTS: usize = 12;
/// Most hit points a plan may have.
pub const MAX_HITS: usize = 8;
/// Hit points must be at least this far apart, seconds.
pub const MIN_HIT_GAP_S: f64 = 1.0;
/// Without a `tempo_range`, hit points may move the tempo by this fraction.
pub const TEMPO_FLEX: f64 = 0.04;

/// The grid hits land on: sixteenth-triplets when the plan's first pitched layer plays a
/// triplet grid, else sixteenths.
pub fn hit_grid(plan: &ScorePlan) -> Grid {
    let triplet = plan
        .layers
        .first()
        .is_some_and(|l| matches!(l.motif.grid, Grid::EighthTriplet | Grid::SixteenthTriplet));
    if triplet {
        Grid::SixteenthTriplet
    } else {
        Grid::Sixteenth
    }
}

/// Hit times in order (finite, positive ones only).
pub fn hit_times(plan: &ScorePlan) -> Vec<f64> {
    let mut t: Vec<f64> = plan
        .hits
        .iter()
        .map(|h| h.time_s)
        .filter(|t| t.is_finite() && *t > 0.0)
        .collect();
    t.sort_by(f64::total_cmp);
    t
}

/// "1:05" for 65 s.
pub fn timecode(s: f64) -> String {
    let whole = s.max(0.0);
    let m = (whole / 60.0).floor() as u64;
    let rest = whole - m as f64 * 60.0;
    if (rest - rest.round()).abs() < 0.005 {
        format!("{m}:{:02}", rest.round() as u64)
    } else {
        format!("{m}:{:05.2}", rest)
    }
}

/// Issues of the scoring fields (`form`, `hits`, `progression`, `tempo_range`). `bars` and
/// `secs` are the piece's length before hit points move the tempo.
pub fn validate(out: &mut Vec<PlanIssue>, plan: &ScorePlan, bars: u32, secs: Option<f64>) {
    use PlanIssueCode as K;
    let ids: Vec<&str> = plan.layers.iter().map(|l| l.id.as_str()).collect();
    if plan.form.len() > MAX_PARTS {
        out.push(PlanIssue::new(
            K::Form,
            "/form",
            format!("{} form parts is too many; use at most {MAX_PARTS}", plan.form.len()),
        ));
    }
    if !plan.form.is_empty() && plan.form.len() as u32 > bars {
        out.push(PlanIssue::new(
            K::Form,
            "/form",
            format!(
                "{} form parts do not fit in {bars} bars; use fewer parts or more bars",
                plan.form.len()
            ),
        ));
    }
    for (i, p) in plan.form.iter().enumerate() {
        let pp = format!("/form/{i}");
        if !(p.weight.is_finite() && p.weight > 0.0 && p.weight <= 100.0) {
            out.push(PlanIssue::new(
                K::Form,
                format!("{pp}/weight"),
                "weight must be a positive number (a relative length, e.g. 1, 2 or 3)",
            ));
        }
        if p.bars == Some(0) || p.bars.is_some_and(|b| b > bars) {
            out.push(PlanIssue::new(
                K::Form,
                format!("{pp}/bars"),
                format!("bars must be 1-{bars}, or omit it to share bars by weight"),
            ));
        }
        if let Some(x) = p.intensity
            && !(x.is_finite() && (0.0..=1.0).contains(&x))
        {
            out.push(PlanIssue::new(
                K::Intensity,
                format!("{pp}/intensity"),
                "intensity 0..=1",
            ));
        }
        for (j, l) in p.layers.iter().enumerate() {
            if !ids.contains(&l.as_str()) {
                out.push(PlanIssue::new(
                    K::UnknownLayer,
                    format!("{pp}/layers/{j}"),
                    format!(
                        "form part {i} names unknown layer `{l}`; layer ids: {}",
                        ids.join(", ")
                    ),
                ));
            }
        }
    }
    let fixed: u32 = plan.form.iter().filter_map(|p| p.bars).sum();
    if fixed > bars {
        out.push(PlanIssue::new(
            K::Form,
            "/form",
            format!("the parts' bars add up to {fixed}, more than the piece's {bars}"),
        ));
    }
    // With a form, every layer must play in some part.
    if !plan.form.is_empty() && plan.form.len() as u32 <= bars {
        let secs = form::sections(plan, bars, &[]);
        for (i, l) in plan.layers.iter().enumerate() {
            if !secs.iter().any(|s| s.layers.contains(&l.id)) {
                out.push(PlanIssue::new(
                    K::InactiveLayer,
                    format!("/layers/{i}"),
                    format!(
                        "layer `{}` plays in no form part; list it in a part's layers (or \
                         leave every part's layers empty so intensity decides)",
                        l.id
                    ),
                ));
            }
        }
    }

    if plan.hits.len() > MAX_HITS {
        out.push(PlanIssue::new(
            K::Hit,
            "/hits",
            format!("{} hit points is too many; use at most {MAX_HITS}", plan.hits.len()),
        ));
    }
    for (i, h) in plan.hits.iter().enumerate() {
        let hp = format!("/hits/{i}/time_s");
        if !(h.time_s.is_finite() && h.time_s > 0.0) {
            out.push(PlanIssue::new(
                K::Hit,
                hp,
                "time_s must be a positive number of seconds from the start",
            ));
            continue;
        }
        if let Some(s) = secs
            && h.time_s > s - 0.5
        {
            out.push(PlanIssue::new(
                K::Hit,
                hp,
                format!(
                    "hit at {} is not inside the piece ({} long); lengthen the piece or move the hit",
                    timecode(h.time_s),
                    timecode(s)
                ),
            ));
        }
    }
    let times = hit_times(plan);
    if let Some(w) = times.windows(2).find(|w| w[1] - w[0] < MIN_HIT_GAP_S) {
        out.push(PlanIssue::new(
            K::Hit,
            "/hits",
            format!(
                "hits at {} and {} are closer than {MIN_HIT_GAP_S} s; merge them",
                timecode(w[0]),
                timecode(w[1])
            ),
        ));
    }

    if let Some(p) = &plan.progression {
        if p.chords.is_empty() || p.chords.len() > 16 {
            out.push(PlanIssue::new(
                K::Progression,
                "/progression/chords",
                "a progression needs 1-16 chords",
            ));
        }
        if !(1..=8).contains(&p.bars_per_chord) {
            out.push(PlanIssue::new(
                K::Progression,
                "/progression/bars_per_chord",
                "bars_per_chord 1-8 (usually 1 or 2)",
            ));
        }
        for (i, c) in p.chords.iter().enumerate() {
            if let Err(m) = harmony::progression_symbol(&plan.key, c) {
                out.push(PlanIssue::new(
                    K::Progression,
                    format!("/progression/chords/{i}"),
                    format!(
                        "{m}; chords of {}: {}",
                        plan.key.display(),
                        diatonic_list(plan)
                    ),
                ));
            }
        }
    }
    if let Some(r) = plan.tempo_range
        && !(r.min.is_finite()
            && r.max.is_finite()
            && r.min <= r.max
            && r.min >= MIN_BPM
            && r.max <= MAX_BPM)
    {
        out.push(PlanIssue::new(
            K::Timing,
            "/tempo_range",
            format!("tempo_range needs min <= max within {MIN_BPM}-{MAX_BPM} BPM"),
        ));
    }
}

fn diatonic_list(plan: &ScorePlan) -> String {
    let n = plan.key.mode.intervals().len().min(7) as i32;
    (0..n)
        .map(|d| crate::plan::diatonic_symbol(&plan.key, d))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Deterministic fixes of the scoring fields (part of `repair::fix_plan`): idempotent, and a
/// plan whose scoring fields are valid comes back unchanged with no repairs.
pub fn fix(p: &mut ScorePlan, bars: u32) -> Vec<Repair> {
    use PlanIssueCode as K;
    let mut r = Vec::new();
    let mut note = |code, path: String, action: String| {
        r.push(Repair { code, path, action });
    };
    if p.form.len() > MAX_PARTS {
        p.form.truncate(MAX_PARTS);
        note(K::Form, "/form".into(), format!("kept the first {MAX_PARTS} parts"));
    }
    if !p.form.is_empty() && p.form.len() as u32 > bars {
        p.form.truncate(bars.max(1) as usize);
        note(K::Form, "/form".into(), format!("kept the first {bars} parts"));
    }
    let ids: Vec<String> = p.layers.iter().map(|l| l.id.clone()).collect();
    for (i, part) in p.form.iter_mut().enumerate() {
        let pp = format!("/form/{i}");
        if !(part.weight.is_finite() && part.weight > 0.0 && part.weight <= 100.0) {
            part.weight = 1.0;
            note(K::Form, format!("{pp}/weight"), "set the weight to 1".into());
        }
        if part.bars == Some(0) || part.bars.is_some_and(|b| b > bars) {
            part.bars = None;
            note(K::Form, format!("{pp}/bars"), "shared bars by weight instead".into());
        }
        if let Some(x) = part.intensity
            && !(x.is_finite() && (0.0..=1.0).contains(&x))
        {
            part.intensity = x.is_finite().then(|| x.clamp(0.0, 1.0));
            note(K::Intensity, format!("{pp}/intensity"), "clamped to 0..=1".into());
        }
        let before = part.layers.len();
        part.layers.retain(|l| ids.contains(l));
        if part.layers.len() != before {
            note(
                K::UnknownLayer,
                format!("{pp}/layers"),
                "removed references to unknown layers".into(),
            );
        }
    }
    let fixed: u32 = p.form.iter().filter_map(|x| x.bars).sum();
    if fixed > bars {
        for part in &mut p.form {
            part.bars = None;
        }
        note(K::Form, "/form".into(), "shared bars by weight instead of fixed bars".into());
    }
    // Layers no part plays: give them every part that lists layers (auto parts already
    // include every layer at their loudest).
    if !p.form.is_empty() {
        let secs = form::sections(p, bars, &[]);
        for id in &ids {
            if !secs.iter().any(|s| s.layers.contains(id)) {
                for part in &mut p.form {
                    if !part.layers.is_empty() {
                        part.layers.push(id.clone());
                    }
                }
                note(
                    K::InactiveLayer,
                    "/form".into(),
                    format!("layer `{id}` was in no part; it now plays in every part that lists layers"),
                );
            }
        }
    }

    let secs = resolve_timing(p)
        .ok()
        .map(|t| t.bars as f64 * p.time_signature.bar_quarters() * 60.0 / t.bpm);
    let before = p.hits.len();
    p.hits.retain(|h| {
        h.time_s.is_finite() && h.time_s > 0.0 && secs.is_none_or(|s| h.time_s <= s - 0.5)
    });
    p.hits.sort_by(|a, b| a.time_s.total_cmp(&b.time_s));
    let mut kept: Vec<crate::plan::HitPoint> = Vec::new();
    for h in p.hits.drain(..) {
        if kept.last().is_none_or(|k| h.time_s - k.time_s >= MIN_HIT_GAP_S) {
            kept.push(h);
        }
    }
    kept.truncate(MAX_HITS);
    if kept.len() != before {
        note(
            K::Hit,
            "/hits".into(),
            format!("kept {} of {before} hit points (inside the piece, at least {MIN_HIT_GAP_S} s apart)", kept.len()),
        );
    }
    p.hits = kept;

    if let Some(prog) = &mut p.progression {
        let key = p.key;
        let before = prog.chords.len();
        prog.chords.retain(|c| harmony::progression_symbol(&key, c).is_ok());
        prog.chords.truncate(16);
        if prog.chords.len() != before {
            note(
                K::Progression,
                "/progression/chords".into(),
                "dropped chords outside the key or unreadable".into(),
            );
        }
        if !(1..=8).contains(&prog.bars_per_chord) {
            prog.bars_per_chord = prog.bars_per_chord.clamp(1, 8);
            note(
                K::Progression,
                "/progression/bars_per_chord".into(),
                "clamped to 1-8".into(),
            );
        }
        if prog.chords.is_empty() {
            p.progression = None;
            note(
                K::Progression,
                "/progression".into(),
                "removed the empty progression".into(),
            );
        }
    }
    if let Some(range) = p.tempo_range
        && !(range.min.is_finite()
            && range.max.is_finite()
            && range.min <= range.max
            && range.min >= MIN_BPM
            && range.max <= MAX_BPM)
    {
        p.tempo_range = None;
        note(K::Timing, "/tempo_range".into(), "removed an invalid tempo range".into());
    }
    r
}

/// Turns the scoring fields into ordinary plan fields: hit points choose the tempo and bars, the
/// form becomes `sections`, the progression (or the emotion's library progression for a form)
/// becomes `chords`. Plain plans come back unchanged. Idempotent. Remarks for the user are
/// appended to `notes` the first time.
pub fn realise(plan: &ScorePlan, seed: u64) -> ScorePlan {
    let mut p = plan.clone();
    if !p.is_scored() {
        return p;
    }
    let Ok(timing) = resolve_timing(&p) else {
        return p;
    };
    let ts = p.time_signature;
    let mut bpm = timing.bpm;
    let mut bars = timing.bars;

    // 1. Hit points pick the tempo.
    let times = hit_times(&p);
    let grid = hit_grid(&p);
    if !times.is_empty() {
        let range = match p.tempo_range {
            Some(r) => (r.min.max(MIN_BPM), r.max.min(MAX_BPM)),
            None => (
                (bpm * (1.0 - TEMPO_FLEX)).max(MIN_BPM),
                (bpm * (1.0 + TEMPO_FLEX)).min(MAX_BPM),
            ),
        };
        let length = match p.duration_s.filter(|s| s.is_finite() && *s > 0.0) {
            Some(s) => hits::Length::Seconds(s),
            None => hits::Length::Bars(bars),
        };
        if let Some(s) = hits::solve_tempo(&times, ts, grid, bpm, range, length) {
            let changed = (s.bpm - bpm).abs() > 1e-9 || s.bars != bars || p.duration_s.is_some();
            if changed {
                let worst = s.max_residual_s * 1000.0;
                p.notes.push(format!(
                    "tempo {:.2} BPM, {} bars, so the {} hit point{} land on the grid (largest \
                     distance {worst:.1} ms before the accent's nudge)",
                    s.bpm,
                    s.bars,
                    times.len(),
                    if times.len() == 1 { "" } else { "s" }
                ));
            }
            bpm = s.bpm;
            bars = s.bars;
            p.tempo_bpm = Some(bpm);
            p.bars = Some(bars);
            p.duration_s = None;
            p.tempo_range = Some(TempoRange { min: bpm, max: bpm });
        }
    }

    // 2. The form becomes sections, its boundaries moved onto the hits.
    if !p.form.is_empty() {
        let targets: Vec<u32> = times
            .iter()
            .filter_map(|t| hits::landing(*t, ts, bpm, grid))
            .map(|l| if l.frac > 0.5 { l.bar + 1 } else { l.bar })
            .filter(|b| *b >= 2 && *b <= bars)
            .collect();
        p.sections = form::sections(&p, bars, &targets);
    }

    // 3. Harmony: the progression (or a library one for a form without chords).
    let one_shot = p.playback == crate::plan::PlanPlayback::OneShot;
    if p.progression.is_some() {
        let symbols = harmony::progression_symbols(&p);
        let bpc = p.progression.as_ref().map(|x| x.bars_per_chord).unwrap_or(1);
        let chords = harmony::chords_over_sections(&p, &symbols, bpc, one_shot, bars);
        if !chords.is_empty() {
            p.chords = chords;
        }
    } else if !p.form.is_empty() && p.chords.is_empty() {
        let preset = emotion::preset_for(&p.emotion);
        let symbols = harmony::library_symbols(&p.key, preset, seed);
        p.chords =
            harmony::chords_over_sections(&p, &symbols, preset.bars_per_chord, one_shot, bars);
        p.notes.push(format!(
            "chords: {} from the {} progressions, restarting with each section",
            symbols.join(" "),
            preset.name
        ));
    }
    p
}

/// One plain sentence about the form and the hits, for the take's summary.
pub fn summary(plan: &ScorePlan, score: &praxis_score::Score) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !plan.form.is_empty() && !score.sections.is_empty() {
        let secs: Vec<String> = score
            .sections
            .iter()
            .map(|s| {
                format!(
                    "{} (bars {}\u{2013}{})",
                    s.name.to_lowercase(),
                    s.bars.start,
                    s.bars.end
                )
            })
            .collect();
        parts.push(format!("Form: {}.", secs.join(", ")));
    }
    let hits: Vec<String> = score
        .markers
        .iter()
        .filter(|m| m.kind == praxis_score::MarkerKind::Hit)
        .map(|m| {
            let bar = praxis_score::timeline::TimeMap::new(score, 48_000).s_to_bar_pos(m.time_s);
            format!("{} (bar {})", timecode(m.time_s), bar.floor() as u32)
        })
        .collect();
    if !hits.is_empty() {
        parts.push(format!("Hits land at {}.", hits.join(", ")));
    }
    parts.join(" ")
}

/// Whether a section role is one the music rises into (a drop or pickup suits it).
pub fn rising(role: SectionRole) -> bool {
    matches!(
        role,
        SectionRole::Build | SectionRole::Peak | SectionRole::Main | SectionRole::Verse
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;
    use crate::plan::{FormPart, HitPoint, PlanContext, Transition, validate_plan};
    use std::collections::BTreeSet;

    fn base() -> ScorePlan {
        serde_json::from_value(serde_json::json!({
            "emotion": "tense",
            "key": {"tonic": "D", "mode": "minor"},
            "tempo_bpm": 100.0,
            "duration_s": 120.0,
            "layers": [
                {"id": "piano", "class": "motif", "preset": {"kind": "sampler", "name": "felt_piano"},
                 "motif": {"grid": "eighth", "spoken": "DON din don din TIN din don din",
                           "degrees": [0, 4, 7, 4], "follow_chords": true, "sustain_steps": 1.6}},
                {"id": "bass", "class": "bass", "preset": {"kind": "synth", "name": "sub_bass"},
                 "motif": {"grid": "eighth", "spoken": "DON ~ ~ ~ don ~ ~ ~", "degrees": [0],
                           "follow_chords": true}}
            ],
            "form": [
                {"role": "intro", "weight": 2}, {"role": "build", "weight": 3},
                {"role": "peak", "weight": 3}, {"role": "release", "weight": 2}
            ],
            "hits": [{"time_s": 42.0}, {"time_s": 65.0}, {"time_s": 98.0}]
        }))
        .expect("plan")
    }

    #[test]
    fn a_scored_plan_validates_and_realises_idempotently() {
        let p = base();
        let c = Catalogue::standard();
        let requested = BTreeSet::new();
        let ctx = PlanContext {
            catalogue: &c,
            requested: &requested,
        };
        assert_eq!(validate_plan(&p, &ctx), vec![]);
        let r = realise(&p, 0);
        assert_eq!(r.sections.len(), 4);
        assert!(r.duration_s.is_none());
        let bpm = r.tempo_bpm.unwrap();
        assert!((96.0..=104.0).contains(&bpm), "{bpm}");
        assert!(!r.chords.is_empty());
        assert_eq!(validate_plan(&r, &ctx), vec![]);
        assert_eq!(realise(&r, 0), r);
        // Each hit starts a section or falls inside one bar of a section start.
        for h in [42.0, 65.0, 98.0] {
            let l = hits::landing(h, r.time_signature, bpm, Grid::Sixteenth).unwrap();
            assert!(
                r.sections.iter().any(|s| s.bars.start == l.bar || s.bars.start == l.bar + 1),
                "{h} -> bar {}",
                l.bar
            );
        }
        // The fixer leaves a valid plan alone.
        let mut q = p.clone();
        assert!(fix(&mut q, 50).is_empty());
        assert_eq!(q, p);
    }

    #[test]
    fn bad_scoring_fields_are_reported_and_fixed() {
        let mut p = base();
        p.form.push(FormPart {
            role: SectionRole::Outro,
            name: String::new(),
            weight: -1.0,
            bars: Some(0),
            intensity: Some(3.0),
            layers: vec!["strings".into()],
            transition: Transition::Hold,
        });
        p.hits.push(HitPoint {
            time_s: 500.0,
            label: String::new(),
        });
        p.hits.push(HitPoint {
            time_s: 42.4,
            label: String::new(),
        });
        p.progression = Some(crate::plan::PlanProgression {
            chords: vec!["i".into(), "bVII".into()],
            bars_per_chord: 0,
        });
        let c = Catalogue::standard();
        let requested = BTreeSet::new();
        let ctx = PlanContext {
            catalogue: &c,
            requested: &requested,
        };
        let issues = validate_plan(&p, &ctx);
        let codes: BTreeSet<PlanIssueCode> = issues.iter().map(|i| i.code).collect();
        for k in [
            PlanIssueCode::Form,
            PlanIssueCode::Hit,
            PlanIssueCode::Progression,
            PlanIssueCode::Intensity,
            PlanIssueCode::UnknownLayer,
        ] {
            assert!(codes.contains(&k), "{k:?} in {issues:?}");
        }
        let (fixed, repairs) = crate::repair::fix_plan(&p, &ctx);
        assert!(!repairs.is_empty());
        assert_eq!(validate_plan(&fixed, &ctx), vec![]);
        assert_eq!(fixed.hits.len(), 3);
        let (again, more) = crate::repair::fix_plan(&fixed, &ctx);
        assert!(more.is_empty(), "{more:?}");
        assert_eq!(again, fixed);
    }

    #[test]
    fn timecodes() {
        assert_eq!(timecode(65.0), "1:05");
        assert_eq!(timecode(42.0), "0:42");
        assert_eq!(timecode(98.5), "1:38.50");
    }
}
