//! Validation and repair (ADR-0006 step 2): validate the plan, send the issues back to the
//! provider (at most [`MAX_REPAIR_ROUNDS`] rounds), then apply deterministic fixes (snap to the
//! scale, quantise, drop banned layers, clamp ranges), recording every repair.

use crate::catalogue::Catalogue;
use crate::motifs::{default_spoken, fitting_grid, rhythm_role};
use crate::plan::{
    MAX_BARS, MAX_BPM, MAX_LAYERS, MIN_BPM, PlanContext, PlanIssue, PlanIssueCode, PlanSection,
    ScorePlan, class_is_intrinsic, code_name, diatonic_symbol, effective_avoid, effective_class,
    issues_text, parse_chord, resolve_timing, scale_degree,
};
use crate::provider::{Message, Provider, ProviderError, Role, ToolRequest};
use praxis_score::rhythm::{SpokenStep, parse_spoken};
use praxis_score::timeline::duration;
use praxis_score::validate::{unpitched_class, valid_id};
use praxis_score::{BarRange, Mode, SectionRole, TimeSignature};
use serde::Serialize;
use std::collections::BTreeSet;

/// Provider repair rounds after the first call.
pub const MAX_REPAIR_ROUNDS: u32 = 3;

/// Default tempo when a plan gives none.
pub const DEFAULT_BPM: f64 = 90.0;
/// Default length when a plan gives none.
pub const DEFAULT_BARS: u32 = 8;

/// One change the planner made to a plan or score on its own.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Repair {
    pub code: PlanIssueCode,
    /// JSON pointer into the plan.
    pub path: String,
    pub action: String,
}

/// Why planning failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PlanError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    /// The plan could not be read or repaired. Each issue has a JSON pointer, what is wrong and
    /// what is allowed.
    #[error("the plan has problems that cannot be repaired automatically:\n{}", issues_text(.0))]
    Invalid(Vec<PlanIssue>),
    #[error("the plan could not be arranged: {0}")]
    Arrange(String),
    /// The arranged Score failed validation (a planner bug).
    #[error("the arranged score failed validation: {0}")]
    InvalidScore(String),
}

/// Result of the provider loop.
#[derive(Clone, Debug, PartialEq)]
pub struct LoopOutcome {
    pub plan: ScorePlan,
    /// Provider calls made.
    pub attempts: u32,
    /// Deterministic repairs applied after the provider rounds.
    pub repairs: Vec<Repair>,
    /// Issues found after each provider call that had any.
    pub rounds: Vec<Vec<PlanIssue>>,
}

/// The user message that asks the provider to fix its plan.
pub fn repair_prompt(issues: &[PlanIssue]) -> String {
    format!(
        "The plan you returned has these problems:\n{}\n\nCall the `{}` tool again with the \
         whole corrected plan. Change only what these problems require.",
        issues_text(issues),
        crate::plan::TOOL_NAME
    )
}

/// Calls the provider, validates, asks for repairs, and falls back to deterministic fixes.
pub fn plan_with_repair(
    provider: &dyn Provider,
    first: ToolRequest,
    ctx: &PlanContext,
) -> Result<LoopOutcome, PlanError> {
    plan_with_quality_repair(provider, first, ctx, &|_| Vec::new())
}

/// [`plan_with_repair`] with quality checks on every valid plan (`brief::quality_issues`: lints
/// on the arranged Score and the brief's restraint rules). Quality issues go back to the provider
/// in the same repair rounds as validation issues (at most [`MAX_REPAIR_ROUNDS`]), but never
/// reject a plan: when the rounds run out, the valid plan with the fewest quality issues wins
/// (the latest on a tie), and only when no answer was valid do the deterministic fixes run.
pub fn plan_with_quality_repair(
    provider: &dyn Provider,
    first: ToolRequest,
    ctx: &PlanContext,
    quality: &dyn Fn(&ScorePlan) -> Vec<PlanIssue>,
) -> Result<LoopOutcome, PlanError> {
    let mut req = first;
    let mut attempts = 0u32;
    let mut rounds = Vec::new();
    let mut last_plan: Option<ScorePlan> = None;
    let mut last_value: Option<serde_json::Value> = None;
    // The valid plan with the fewest quality issues so far, and that count.
    let mut best: Option<(ScorePlan, usize)> = None;
    let last_issues = loop {
        let value = provider.call_tool(&req)?;
        attempts += 1;
        let repeated = last_value.as_ref() == Some(&value);
        let issues = match serde_json::from_value::<ScorePlan>(value.clone()) {
            Ok(plan) => {
                let mut issues = crate::plan::validate_plan(&plan, ctx);
                if issues.is_empty() {
                    issues = quality(&plan);
                    if best.as_ref().is_none_or(|(_, n)| issues.len() <= *n) {
                        best = Some((plan.clone(), issues.len()));
                    }
                }
                last_plan = Some(plan);
                issues
            }
            Err(e) => schema_issues(&value, &e),
        };
        if issues.is_empty()
            && let Some(plan) = last_plan.take()
        {
            return Ok(LoopOutcome {
                plan,
                attempts,
                repairs: vec![],
                rounds,
            });
        }
        rounds.push(issues.clone());
        // A provider that returns the same plan again cannot repair it (the offline provider is
        // a pure function of the brief); go straight to the deterministic fixes.
        if repeated || attempts > MAX_REPAIR_ROUNDS {
            if let Some((plan, _)) = best.take() {
                return Ok(LoopOutcome {
                    plan,
                    attempts,
                    repairs: vec![],
                    rounds,
                });
            }
            break issues;
        }
        let text = serde_json::to_string(&value).unwrap_or_default();
        req.messages.push(Message {
            role: Role::Assistant,
            content: format!("Called `{}` with: {text}", req.tool_name),
        });
        req.messages.push(Message {
            role: Role::User,
            content: repair_prompt(&issues),
        });
        last_value = Some(value);
    };
    let Some(plan) = last_plan else {
        return Err(PlanError::Invalid(last_issues));
    };
    let (fixed, repairs) = fix_plan(&plan, ctx);
    let remaining = crate::plan::validate_plan(&fixed, ctx);
    if !remaining.is_empty() {
        return Err(PlanError::Invalid(remaining));
    }
    Ok(LoopOutcome {
        plan: fixed,
        attempts,
        repairs,
        rounds,
    })
}

// ---------------------------------------------------------------------------------------------
// Schema errors with JSON pointers

fn schema_issue(path: &str, msg: String) -> PlanIssue {
    PlanIssue::new(PlanIssueCode::Schema, path, msg)
}

/// Locates why a tool input does not match the plan schema, field by field, so the issue carries
/// a JSON pointer. Falls back to the whole-document error.
pub fn schema_issues(value: &serde_json::Value, err: &serde_json::Error) -> Vec<PlanIssue> {
    use serde_json::Value;
    let mut out = Vec::new();
    let Some(obj) = value.as_object() else {
        return vec![schema_issue(
            "",
            "the plan must be a JSON object; see plan_schema()".into(),
        )];
    };
    const TOP: [&str; 15] = [
        "title",
        "emotion",
        "key",
        "time_signature",
        "tempo_bpm",
        "duration_s",
        "bars",
        "playback",
        "avoid",
        "sections",
        "layers",
        "chords",
        "notes",
        "requested",
        "",
    ];
    for k in obj.keys() {
        if !TOP.contains(&k.as_str()) {
            out.push(schema_issue(
                &format!("/{k}"),
                format!(
                    "unknown field `{k}`; allowed fields: {}",
                    TOP[..14].join(", ")
                ),
            ));
        }
    }
    for req in ["emotion", "key", "layers"] {
        if !obj.contains_key(req) {
            out.push(schema_issue(
                &format!("/{req}"),
                format!("missing required field `{req}`"),
            ));
        }
    }
    fn check<T: serde::de::DeserializeOwned>(out: &mut Vec<PlanIssue>, path: &str, v: &Value) {
        if let Err(e) = serde_json::from_value::<T>(v.clone()) {
            out.push(schema_issue(path, e.to_string()));
        }
    }
    use crate::plan::{PlanChord, PlanLayer, PlanNote, PlanPlayback, PresetChoice};
    use praxis_score::{GeneratorClass, Key, TimeSignature as Ts};
    if let Some(v) = obj.get("emotion") {
        check::<String>(&mut out, "/emotion", v);
    }
    if let Some(v) = obj.get("key") {
        check::<Key>(&mut out, "/key", v);
    }
    if let Some(v) = obj.get("time_signature") {
        check::<Ts>(&mut out, "/time_signature", v);
    }
    for f in ["tempo_bpm", "duration_s"] {
        if let Some(v) = obj.get(f) {
            check::<Option<f64>>(&mut out, &format!("/{f}"), v);
        }
    }
    if let Some(v) = obj.get("bars") {
        check::<Option<u32>>(&mut out, "/bars", v);
    }
    if let Some(v) = obj.get("playback") {
        check::<PlanPlayback>(&mut out, "/playback", v);
    }
    for f in ["avoid", "requested"] {
        if let Some(Value::Array(a)) = obj.get(f) {
            for (i, v) in a.iter().enumerate() {
                check::<GeneratorClass>(&mut out, &format!("/{f}/{i}"), v);
            }
        }
    }
    if let Some(Value::Array(a)) = obj.get("sections") {
        for (i, v) in a.iter().enumerate() {
            check::<PlanSection>(&mut out, &format!("/sections/{i}"), v);
        }
    }
    if let Some(Value::Array(a)) = obj.get("chords") {
        for (i, v) in a.iter().enumerate() {
            check::<PlanChord>(&mut out, &format!("/chords/{i}"), v);
        }
    }
    if let Some(Value::Array(a)) = obj.get("layers") {
        for (i, v) in a.iter().enumerate() {
            let lp = format!("/layers/{i}");
            if serde_json::from_value::<PlanLayer>(v.clone()).is_ok() {
                continue;
            }
            let mut deeper = false;
            if let Some(p) = v.get("preset")
                && serde_json::from_value::<PresetChoice>(p.clone()).is_err()
            {
                check::<PresetChoice>(&mut out, &format!("{lp}/preset"), p);
                deeper = true;
            }
            if let Some(c) = v.get("class")
                && serde_json::from_value::<GeneratorClass>(c.clone()).is_err()
            {
                check::<GeneratorClass>(&mut out, &format!("{lp}/class"), c);
                deeper = true;
            }
            if let Some(m) = v.get("motif") {
                if let Some(Value::Array(notes)) = m.get("notes") {
                    for (j, n) in notes.iter().enumerate() {
                        if serde_json::from_value::<PlanNote>(n.clone()).is_err() {
                            check::<PlanNote>(&mut out, &format!("{lp}/motif/notes/{j}"), n);
                            deeper = true;
                        }
                    }
                }
                if !deeper && serde_json::from_value::<crate::plan::Motif>(m.clone()).is_err() {
                    check::<crate::plan::Motif>(&mut out, &format!("{lp}/motif"), m);
                    deeper = true;
                }
            }
            if !deeper {
                check::<PlanLayer>(&mut out, &lp, v);
            }
        }
    } else if obj.contains_key("layers") {
        out.push(schema_issue("/layers", "layers must be an array".into()));
    }
    if out.is_empty() {
        out.push(schema_issue("", err.to_string()));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Deterministic fixes

struct Fixer {
    repairs: Vec<Repair>,
}

impl Fixer {
    fn note(&mut self, code: PlanIssueCode, path: impl Into<String>, action: impl Into<String>) {
        self.repairs.push(Repair {
            code,
            path: path.into(),
            action: action.into(),
        });
    }
}

fn four_four() -> TimeSignature {
    TimeSignature {
        numerator: 4,
        denominator: 4,
    }
}

/// Applies every deterministic fix the plan needs. Fixes are idempotent: a valid plan comes back
/// unchanged with no repairs.
pub fn fix_plan(plan: &ScorePlan, ctx: &PlanContext) -> (ScorePlan, Vec<Repair>) {
    use PlanIssueCode as K;
    let mut p = plan.clone();
    let mut f = Fixer { repairs: vec![] };

    let ts = p.time_signature;
    if ts.numerator == 0 || ts.numerator > 32 || ![2u8, 4, 8, 16].contains(&ts.denominator) {
        p.time_signature = four_four();
        f.note(
            K::TimeSignature,
            "/time_signature",
            "set the time signature to 4/4",
        );
    }
    fix_timing(&mut p, &mut f);
    let bars = resolve_timing(&p).map(|t| t.bars).unwrap_or(DEFAULT_BARS);

    let emotion = p.emotion.trim().to_lowercase();
    let word: String = emotion
        .split_whitespace()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_alphabetic() || *c == '-')
        .collect();
    let word = if word.is_empty() {
        "calm".to_string()
    } else {
        word
    };
    if word != p.emotion {
        f.note(
            K::Emotion,
            "/emotion",
            format!("reduced the emotion `{}` to one word `{word}`", p.emotion),
        );
        p.emotion = word;
    }

    fix_layers(&mut p, ctx, bars, &mut f);
    fix_sections(&mut p, bars, &mut f);
    fix_chords(&mut p, bars, &mut f);
    (p, f.repairs)
}

fn fix_timing(p: &mut ScorePlan, f: &mut Fixer) {
    use PlanIssueCode as K;
    let ok = |v: Option<f64>| v.is_none_or(|x| x.is_finite() && x > 0.0);
    if !ok(p.tempo_bpm) {
        f.note(K::Timing, "/tempo_bpm", "removed a non-positive tempo");
        p.tempo_bpm = None;
    }
    if !ok(p.duration_s) {
        f.note(K::Timing, "/duration_s", "removed a non-positive duration");
        p.duration_s = None;
    }
    if p.bars == Some(0) {
        f.note(K::Bars, "/bars", "removed bars = 0");
        p.bars = None;
    }
    if let Some(b) = p.bars
        && b > MAX_BARS
    {
        f.note(K::Bars, "/bars", format!("clamped {b} bars to {MAX_BARS}"));
        p.bars = Some(MAX_BARS);
    }
    let ts = p.time_signature;
    match (p.tempo_bpm, p.duration_s, p.bars) {
        (Some(bpm), Some(s), Some(b)) if (duration::solve_tempo(b, ts, s) - bpm).abs() > 0.5 => {
            let need = duration::solve_tempo(b, ts, s);
            f.note(
                K::Timing,
                "/tempo_bpm",
                format!(
                    "{bpm} BPM did not fit {b} bars in {s} s; solved the tempo from bars and \
                     duration: {need:.2} BPM"
                ),
            );
            p.tempo_bpm = None;
        }
        (None, None, Some(_)) => {
            f.note(
                K::Timing,
                "/tempo_bpm",
                format!("no tempo or duration given; used {DEFAULT_BPM} BPM"),
            );
            p.tempo_bpm = Some(DEFAULT_BPM);
        }
        (Some(_), None, None) => {
            f.note(
                K::Bars,
                "/bars",
                format!("no bars or duration given; used {DEFAULT_BARS} bars"),
            );
            p.bars = Some(DEFAULT_BARS);
        }
        (None, Some(_), None) => {
            f.note(
                K::Timing,
                "/tempo_bpm",
                format!("only a duration was given; used {DEFAULT_BPM} BPM and solved the bars"),
            );
            p.tempo_bpm = Some(DEFAULT_BPM);
        }
        (None, None, None) => {
            f.note(
                K::Timing,
                "/bars",
                format!("no timing given; used {DEFAULT_BARS} bars at {DEFAULT_BPM} BPM"),
            );
            p.tempo_bpm = Some(DEFAULT_BPM);
            p.bars = Some(DEFAULT_BARS);
        }
        _ => {}
    }
    // Tempo out of the practical range.
    if let Ok(t) = resolve_timing(p)
        && t.bpm >= MIN_BPM
        && t.bpm <= MAX_BPM
    {
        return;
    }
    let target = resolve_timing_unchecked(p)
        .map(|b| b.clamp(MIN_BPM, MAX_BPM))
        .unwrap_or(DEFAULT_BPM);
    match (p.duration_s, p.bars) {
        (Some(s), Some(_)) => {
            let (b0, _) = duration::solve_bars(target, ts, s);
            let fit = [b0, b0.saturating_sub(1), b0 + 1]
                .into_iter()
                .filter(|b| *b >= 1 && *b <= MAX_BARS)
                .find(|b| (MIN_BPM..=MAX_BPM).contains(&duration::solve_tempo(*b, ts, s)));
            if let Some(b) = fit {
                f.note(
                    K::Timing,
                    "/bars",
                    format!(
                        "the tempo was outside {MIN_BPM}-{MAX_BPM} BPM; changed bars to {b} to \
                         keep the {s} s duration"
                    ),
                );
                p.bars = Some(b);
                p.tempo_bpm = None;
            } else {
                f.note(
                    K::Timing,
                    "/duration_s",
                    format!("no bar count fits {s} s in range; dropped the duration"),
                );
                p.duration_s = None;
                p.tempo_bpm = Some(target);
            }
        }
        _ => {
            f.note(
                K::Timing,
                "/tempo_bpm",
                format!("clamped the tempo to {target} BPM"),
            );
            p.tempo_bpm = Some(target);
            if p.bars.is_none() && p.duration_s.is_none() {
                p.bars = Some(DEFAULT_BARS);
            }
        }
    }
    if resolve_timing(p).is_err() {
        f.note(
            K::Timing,
            "/tempo_bpm",
            format!("reset timing to {DEFAULT_BARS} bars at {DEFAULT_BPM} BPM"),
        );
        p.tempo_bpm = Some(DEFAULT_BPM);
        p.duration_s = None;
        p.bars = Some(DEFAULT_BARS);
    }
}

/// Tempo the plan implies, without the range check.
fn resolve_timing_unchecked(p: &ScorePlan) -> Option<f64> {
    let ts = p.time_signature;
    match (p.tempo_bpm, p.duration_s, p.bars) {
        (_, Some(s), Some(b)) => Some(duration::solve_tempo(b, ts, s)),
        (Some(bpm), _, _) => Some(bpm),
        _ => None,
    }
}

fn fix_layers(p: &mut ScorePlan, ctx: &PlanContext, bars: u32, f: &mut Fixer) {
    use PlanIssueCode as K;
    let catalogue: &Catalogue = ctx.catalogue;
    let avoid = effective_avoid(p, ctx.requested);
    let key = p.key;
    let ts = p.time_signature;
    let mut kept = Vec::new();
    let mut seen = BTreeSet::new();
    let mut renamed: Vec<(String, String)> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    for (i, layer) in p.layers.iter().enumerate() {
        let lp = format!("/layers/{i}");
        let mut l = layer.clone();
        // Preset.
        let preset = match catalogue.get(&l.preset.name) {
            Some(pr) => {
                if pr.kind != l.preset.kind {
                    f.note(
                        K::PresetKind,
                        format!("{lp}/preset/kind"),
                        format!("set the kind of `{}` to its catalogue kind", pr.name),
                    );
                    l.preset.kind = pr.kind;
                }
                pr
            }
            None => match catalogue.default_for_class(l.class) {
                Some(pr) if !avoid.contains(&pr.class) => {
                    f.note(
                        K::UnknownPreset,
                        format!("{lp}/preset/name"),
                        format!(
                            "replaced unknown preset `{}` with `{}`",
                            l.preset.name, pr.name
                        ),
                    );
                    l.preset.name = pr.name.clone();
                    l.preset.kind = pr.kind;
                    pr
                }
                _ => {
                    f.note(
                        K::UnknownPreset,
                        lp.clone(),
                        format!(
                            "dropped layer `{}`: unknown preset `{}`",
                            l.id, l.preset.name
                        ),
                    );
                    dropped.push(layer.id.clone());
                    continue;
                }
            },
        };
        if l.class != preset.class && (class_is_intrinsic(preset.class) || unpitched_class(l.class))
        {
            f.note(
                K::ClassMismatch,
                format!("{lp}/class"),
                format!("set the class of `{}` to its preset's class", l.id),
            );
            l.class = preset.class;
        }
        let class = effective_class(&l, Some(preset));
        if avoid.contains(&class) || avoid.contains(&preset.class) {
            f.note(
                K::AvoidedClass,
                lp.clone(),
                format!(
                    "dropped layer `{}` ({}): its class is on the avoid-list",
                    l.id,
                    crate::catalogue::class_name(class)
                ),
            );
            dropped.push(layer.id.clone());
            continue;
        }
        if kept.len() >= MAX_LAYERS {
            f.note(
                K::TooManyLayers,
                lp.clone(),
                format!(
                    "dropped layer `{}`: at most {MAX_LAYERS} layers (restraint)",
                    l.id
                ),
            );
            dropped.push(layer.id.clone());
            continue;
        }
        // Id.
        let mut id = if valid_id(&l.id) {
            l.id.clone()
        } else {
            crate::arrange::slug(&l.id, "layer")
        };
        let base = id.clone();
        let mut n = 2;
        while seen.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        if id != l.id {
            f.note(
                if valid_id(&l.id) {
                    K::DuplicateId
                } else {
                    K::BadId
                },
                format!("{lp}/id"),
                format!("renamed layer `{}` to `{id}`", l.id),
            );
            renamed.push((l.id.clone(), id.clone()));
            l.id = id.clone();
        }
        seen.insert(id);
        if let Some(g) = l.gain_db
            && !(g.is_finite() && (-60.0..=24.0).contains(&g))
        {
            f.note(
                K::Gain,
                format!("{lp}/gain_db"),
                "removed an out-of-range gain",
            );
            l.gain_db = None;
        }

        // Motif.
        let mp = format!("{lp}/motif");
        let m = &mut l.motif;
        if ts.steps_per_bar(m.grid).is_none() {
            let g = fitting_grid(ts);
            f.note(
                K::Grid,
                format!("{mp}/grid"),
                format!("changed the grid to {}", crate::plan::grid_name(g)),
            );
            m.grid = g;
        }
        let spb = ts.steps_per_bar(m.grid).unwrap_or(1);
        let max_len = 8.min(bars.max(1));
        if m.length_bars == 0 || m.length_bars > max_len {
            let v = m.length_bars.clamp(1, max_len);
            f.note(
                K::MotifLength,
                format!("{mp}/length_bars"),
                format!("set length_bars to {v}"),
            );
            m.length_bars = v;
        }
        for (j, d) in m.degrees.iter_mut().enumerate() {
            if !(-21..=28).contains(d) {
                let v = (*d).clamp(-21, 28);
                f.note(
                    K::Degrees,
                    format!("{mp}/degrees/{j}"),
                    format!("clamped degree {d} to {v}"),
                );
                *d = v;
            }
        }
        if let Some(o) = m.octave
            && !(0..=8).contains(&o)
        {
            f.note(
                K::Octave,
                format!("{mp}/octave"),
                format!("clamped octave {o} to {}", o.clamp(0, 8)),
            );
            m.octave = Some(o.clamp(0, 8));
        }
        if let Some(s) = m.sustain_steps {
            if !(s.is_finite() && s > 0.0 && s <= 64.0) {
                f.note(
                    K::Sustain,
                    format!("{mp}/sustain_steps"),
                    "removed an invalid sustain",
                );
                m.sustain_steps = None;
            } else if preset.sustaining && s <= 1.0 {
                f.note(
                    K::Sustain,
                    format!("{mp}/sustain_steps"),
                    format!(
                        "set sustain_steps to {} so the notes overlap",
                        crate::arrange::DEFAULT_SUSTAIN
                    ),
                );
                m.sustain_steps = Some(crate::arrange::DEFAULT_SUSTAIN);
            }
        }
        let len_steps = m.length_bars * spb;
        let pitched = preset.pitched && !unpitched_class(class);
        let mut notes = Vec::new();
        for (j, n) in m.notes.iter().enumerate() {
            let np = format!("{mp}/notes/{j}");
            let mut n = n.clone();
            if !n.step.is_finite() || n.step < 0.0 {
                f.note(
                    K::OffGrid,
                    format!("{np}/step"),
                    "dropped a note with an invalid step",
                );
                continue;
            }
            if n.step.fract() != 0.0 {
                let q = n.step.round();
                f.note(
                    K::OffGrid,
                    format!("{np}/step"),
                    format!("quantised step {} to {q}", n.step),
                );
                n.step = q;
            }
            if n.step as u32 >= len_steps {
                f.note(
                    K::OffGrid,
                    format!("{np}/step"),
                    format!("dropped a note at step {} beyond the motif", n.step),
                );
                continue;
            }
            if !(0..=127).contains(&n.pitch) {
                let v = n.pitch.clamp(0, 127);
                f.note(
                    K::NoteRange,
                    format!("{np}/pitch"),
                    format!("clamped pitch {} to {v}", n.pitch),
                );
                n.pitch = v;
            }
            if pitched {
                let (lo, hi) = (preset.range.0 as i32, preset.range.1 as i32);
                let mut q = n.pitch;
                while q < lo && q + 12 <= 127 {
                    q += 12;
                }
                while q > hi && q - 12 >= 0 {
                    q -= 12;
                }
                if q != n.pitch {
                    f.note(
                        K::NoteRange,
                        format!("{np}/pitch"),
                        format!("moved pitch {} by octaves to {q}", n.pitch),
                    );
                    n.pitch = q;
                }
                if key.mode != Mode::Chromatic && !key.contains(n.pitch as u8) {
                    let s = key.snap(n.pitch as u8) as i32;
                    f.note(
                        K::OffScale,
                        format!("{np}/pitch"),
                        format!(
                            "snapped {} to {} (in {})",
                            praxis_score::theory::midi_name(n.pitch as u8),
                            praxis_score::theory::midi_name(s as u8),
                            key.display()
                        ),
                    );
                    n.pitch = s;
                }
            }
            if let Some(v) = n.velocity
                && !(v > 0.0 && v <= 1.0)
            {
                f.note(
                    K::Velocity,
                    format!("{np}/velocity"),
                    "removed an invalid velocity",
                );
                n.velocity = None;
            }
            if let Some(len) = n.length
                && !(len.is_finite() && len > 0.0 && len <= 64.0)
            {
                f.note(
                    K::Sustain,
                    format!("{np}/length"),
                    "removed an invalid length",
                );
                n.length = None;
            }
            notes.push(n);
        }
        m.notes = notes;
        let role = rhythm_role(preset, class);
        if m.notes.is_empty() {
            let steps = parse_spoken(&m.spoken);
            let bad = match &steps {
                Err(_) => true,
                Ok(s) => !s.iter().any(|t| matches!(t, SpokenStep::Hit { .. })),
            };
            if bad {
                let s = default_spoken(role, ts, m.grid).unwrap_or_else(|| "DON".into());
                f.note(
                    K::Spoken,
                    format!("{mp}/spoken"),
                    format!("replaced the spoken rhythm with `{s}`"),
                );
                m.spoken = s;
            } else if let Ok(s) = steps {
                let len = s.len() as u32;
                if len > m.length_bars * spb {
                    let need = len.div_ceil(spb);
                    if need <= max_len {
                        f.note(
                            K::MotifLength,
                            format!("{mp}/length_bars"),
                            format!("lengthened the motif to {need} bars to fit its rhythm"),
                        );
                        m.length_bars = need;
                    } else {
                        let s = default_spoken(role, ts, m.grid).unwrap_or_else(|| "DON".into());
                        f.note(
                            K::Spoken,
                            format!("{mp}/spoken"),
                            format!("the rhythm was too long; replaced it with `{s}`"),
                        );
                        m.spoken = s;
                    }
                }
            }
            if preset.pitched && !unpitched_class(class) && m.degrees.is_empty() {
                f.note(
                    K::Degrees,
                    format!("{mp}/degrees"),
                    "used degree 0 (the tonic)",
                );
                m.degrees = vec![0];
            }
        }
        kept.push(l);
    }
    p.layers = kept;
    for s in &mut p.sections {
        for l in &mut s.layers {
            if let Some((_, new)) = renamed.iter().find(|(old, _)| old == l) {
                *l = new.clone();
            }
        }
        s.layers.retain(|l| !dropped.contains(l));
    }
}

fn fix_sections(p: &mut ScorePlan, bars: u32, f: &mut Fixer) {
    use PlanIssueCode as K;
    let ids: Vec<String> = p.layers.iter().map(|l| l.id.clone()).collect();
    if p.sections.is_empty() {
        f.note(
            K::Sections,
            "/sections",
            format!("added one section over bars 1-{bars} with every layer"),
        );
        p.sections = vec![PlanSection {
            name: "Main".into(),
            bars: BarRange {
                start: 1,
                end: bars,
            },
            role: SectionRole::Main,
            intensity: 0.5,
            layers: ids.clone(),
        }];
        return;
    }
    let mut secs: Vec<PlanSection> = Vec::new();
    for (i, s) in p.sections.iter().enumerate() {
        let sp = format!("/sections/{i}");
        let mut s = s.clone();
        let before = s.layers.len();
        s.layers.retain(|l| ids.contains(l));
        if s.layers.len() != before {
            f.note(
                K::UnknownLayer,
                format!("{sp}/layers"),
                "removed references to unknown layers",
            );
        }
        let mut dedup = Vec::new();
        for l in s.layers.drain(..) {
            if !dedup.contains(&l) {
                dedup.push(l);
            }
        }
        s.layers = dedup;
        if !(s.intensity.is_finite() && (0.0..=1.0).contains(&s.intensity)) {
            let v = if s.intensity.is_finite() {
                s.intensity.clamp(0.0, 1.0)
            } else {
                0.5
            };
            f.note(
                K::Intensity,
                format!("{sp}/intensity"),
                format!("set intensity to {v}"),
            );
            s.intensity = v;
        }
        let (mut a, mut b) = (s.bars.start, s.bars.end);
        if b < a {
            std::mem::swap(&mut a, &mut b);
        }
        let (a2, b2) = (a.max(1), b.min(bars));
        if a2 > b2 {
            f.note(
                K::Sections,
                format!("{sp}/bars"),
                format!("dropped section `{}` outside bars 1-{bars}", s.name),
            );
            continue;
        }
        if (a2, b2) != (s.bars.start, s.bars.end) {
            f.note(
                K::Sections,
                format!("{sp}/bars"),
                format!("clamped section `{}` to bars {a2}-{b2}", s.name),
            );
        }
        s.bars = BarRange { start: a2, end: b2 };
        secs.push(s);
    }
    secs.sort_by_key(|s| s.bars.start);
    // Remove overlaps by starting later sections after the earlier ones.
    let mut out: Vec<PlanSection> = Vec::new();
    for mut s in secs {
        if let Some(prev) = out.last()
            && s.bars.start <= prev.bars.end
        {
            let start = prev.bars.end + 1;
            if start > s.bars.end {
                f.note(
                    K::Sections,
                    "/sections",
                    format!("dropped section `{}`: it lay inside an earlier one", s.name),
                );
                continue;
            }
            f.note(
                K::Sections,
                "/sections",
                format!(
                    "moved the start of `{}` to bar {start} to remove an overlap",
                    s.name
                ),
            );
            s.bars.start = start;
        }
        out.push(s);
    }
    if out.is_empty() {
        f.note(
            K::Sections,
            "/sections",
            format!("added one section over bars 1-{bars} with every layer"),
        );
        p.sections = vec![PlanSection {
            name: "Main".into(),
            bars: BarRange {
                start: 1,
                end: bars,
            },
            role: SectionRole::Main,
            intensity: 0.5,
            layers: ids,
        }];
        return;
    }
    // Close gaps by extending the earlier section (or the first one back to bar 1).
    if out[0].bars.start > 1 {
        f.note(
            K::Sections,
            "/sections/0/bars",
            format!("extended `{}` back to bar 1", out[0].name),
        );
        out[0].bars.start = 1;
    }
    for i in 1..out.len() {
        let end = out[i - 1].bars.end;
        if out[i].bars.start > end + 1 {
            let new_end = out[i].bars.start - 1;
            f.note(
                K::Sections,
                "/sections",
                format!(
                    "extended `{}` to bar {new_end} to close a gap",
                    out[i - 1].name
                ),
            );
            out[i - 1].bars.end = new_end;
        }
    }
    let last = out.len() - 1;
    if out[last].bars.end < bars {
        f.note(
            K::Sections,
            "/sections",
            format!("extended `{}` to bar {bars}", out[last].name),
        );
        out[last].bars.end = bars;
    }
    // Layers that never play: give them every section (they were asked for).
    for id in &ids {
        if !out.iter().any(|s| s.layers.contains(id)) {
            f.note(
                K::InactiveLayer,
                "/sections",
                format!("layer `{id}` was in no section; it now plays in every section"),
            );
            for s in &mut out {
                s.layers.push(id.clone());
            }
        }
    }
    p.sections = out;
}

fn fix_chords(p: &mut ScorePlan, bars: u32, f: &mut Fixer) {
    use PlanIssueCode as K;
    let key = p.key;
    let mut out = Vec::new();
    for (i, c) in p.chords.iter().enumerate() {
        let cp = format!("/chords/{i}");
        let mut c = c.clone();
        let (a, b) = (c.bars.start.max(1), c.bars.end.min(bars));
        if a > b {
            f.note(
                K::Chord,
                cp.clone(),
                format!("dropped chord `{}` outside the piece", c.symbol),
            );
            continue;
        }
        if (a, b) != (c.bars.start, c.bars.end) {
            f.note(
                K::Chord,
                format!("{cp}/bars"),
                format!("clamped to bars {a}-{b}"),
            );
            c.bars = BarRange { start: a, end: b };
        }
        let Some(sym) = parse_chord(&c.symbol) else {
            f.note(
                K::Chord,
                format!("{cp}/symbol"),
                format!("dropped unreadable chord `{}`", c.symbol),
            );
            continue;
        };
        if key.mode != Mode::Chromatic {
            let root_ok = scale_degree(&key, sym.root).is_some();
            let bass_ok = sym.bass.is_none_or(|b| scale_degree(&key, b).is_some());
            if !root_ok || !bass_ok {
                let snapped = key.snap(60 + sym.root.semitone());
                let pc = praxis_score::PitchClass::of_midi(snapped);
                let degree = scale_degree(&key, pc).unwrap_or(0);
                let new = diatonic_symbol(&key, degree);
                f.note(
                    K::Chord,
                    format!("{cp}/symbol"),
                    format!("replaced `{}` with the in-key chord `{new}`", c.symbol),
                );
                c.symbol = new;
            }
        }
        out.push(c);
    }
    p.chords = out;
}

/// Short summary of repairs for provenance, e.g. "sections:1, off_scale:2".
pub fn repairs_summary(repairs: &[Repair]) -> String {
    let mut counts: std::collections::BTreeMap<String, u32> = Default::default();
    for r in repairs {
        *counts.entry(code_name(r.code)).or_default() += 1;
    }
    counts
        .iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join(", ")
}
