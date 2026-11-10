//! Resolution of an [`EditPlan`] against a Score: the exact bar range, the existing layers the
//! edit changes (never locked ones), the locked layers it was refused, and the preset of any new
//! layer. Everything the patch builder needs, with issues written for a model to fix.

use super::material::{is_drum, layer_preset};
use super::plan::{EditIntent, EditIssue, EditIssueCode, EditPlan};
use super::scope::{PHRASE_FORMS, resolve_phrase};
use crate::catalogue::{Catalogue, PresetSpec, class_name};
use praxis_score::{BarRange, GeneratorClass, Layer, Score};

/// An edit plan resolved against a Score.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub plan: EditPlan,
    /// The bars the edit may change.
    pub bars: BarRange,
    /// True when the scope is the whole score (the patch is then unscoped in bars, so layer-wide
    /// changes such as gain or instrument are allowed).
    pub whole: bool,
    /// Existing, unlocked layers the edit changes, in Score order.
    pub targets: Vec<String>,
    /// Locked layers the instruction named; they are left alone.
    pub refused: Vec<String>,
    /// Preset of a new layer (`add_layer`, `add_melody`, `change_instrument`).
    pub preset: Option<PresetSpec>,
    /// Remarks for the summary (scope narrowed to respect `keep`, ...).
    pub remarks: Vec<String>,
}

fn range_text(r: BarRange) -> String {
    if r.start == r.end {
        format!("bar {}", r.start)
    } else {
        format!("bars {}-{}", r.start, r.end)
    }
}

/// "bar 5" / "bars 9-12".
pub fn bars_text(r: BarRange) -> String {
    range_text(r)
}

fn overlaps(a: BarRange, b: BarRange) -> bool {
    a.start <= b.end && b.start <= a.end
}

/// True when layer `l` has a clip overlapping `bars`.
pub fn plays_in(l: &Layer, bars: BarRange) -> bool {
    l.clips.iter().any(|c| overlaps(c.bars, bars))
}

/// Normalised reference text: lower case, spaces and dashes as underscores.
fn norm(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c == ' ' || c == '-' { '_' } else { c })
        .collect()
}

fn singular(s: &str) -> &str {
    s.strip_suffix('s').filter(|t| t.len() > 2).unwrap_or(s)
}

/// True when the reference names layer `l` (by id, name, preset or a word of it, class, or a
/// role word).
pub fn matches_layer(reference: &str, l: &Layer) -> bool {
    let r = norm(reference);
    let r1 = singular(&r).to_string();
    let id = norm(&l.id);
    let name = norm(&l.name);
    let preset = norm(&l.instrument.preset);
    let class = class_name(l.class);
    let eq = |x: &str| x == r || x == r1 || singular(x) == r1;
    if eq(&id) || eq(&name) || eq(&preset) || eq(&class) {
        return true;
    }
    let words_of = |x: &str| x.split('_').any(|w| !w.is_empty() && eq(w));
    if words_of(&id) || words_of(&name) || words_of(&preset) {
        return true;
    }
    match r1.as_str() {
        "drum" | "beat" => is_drum(l),
        "melody" | "lead" | "tune" => {
            matches!(l.class, GeneratorClass::Melody | GeneratorClass::Motif)
        }
        "percussion" | "perc" => {
            matches!(l.class, GeneratorClass::Percussion | GeneratorClass::Drums) || is_drum(l)
        }
        "chord" | "harmony" => matches!(l.class, GeneratorClass::Chords | GeneratorClass::Harmony),
        "bell" => preset.contains("bell"),
        "keys" | "key" => preset.contains("keys") || preset.contains("piano"),
        _ => false,
    }
}

fn layer_list(score: &Score) -> String {
    score
        .layers
        .iter()
        .map(|l| {
            format!(
                "`{}` ({}, {}{})",
                l.id,
                l.instrument.preset,
                class_name(l.class),
                if l.locked { ", locked" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolves the bar scope. Returns the range, whether it is the whole score, and remarks.
fn resolve_bars(
    plan: &EditPlan,
    score: &Score,
) -> Result<(BarRange, bool, Vec<String>), EditIssue> {
    let n = score.bars;
    let all = BarRange { start: 1, end: n };
    let mut remarks = Vec::new();
    let keep = match &plan.scope.keep {
        Some(k) => Some(
            resolve_phrase(k, n)
                .map_err(|e| EditIssue::new(EditIssueCode::Scope, "/scope/keep", e))?,
        ),
        None => None,
    };
    let (mut bars, explicit) = if let Some(b) = plan.scope.bars {
        if b.start == 0 || b.end < b.start || b.end > n {
            return Err(EditIssue::new(
                EditIssueCode::Scope,
                "/scope/bars",
                format!(
                    "bars {}-{} are not a range within the score's bars 1-{n}",
                    b.start, b.end
                ),
            ));
        }
        (b, true)
    } else if let Some(p) = &plan.scope.phrase {
        let r = resolve_phrase(p, n)
            .map_err(|e| EditIssue::new(EditIssueCode::Scope, "/scope/phrase", e))?;
        (r, true)
    } else if let Some(k) = keep {
        // Only "keep X": the scope is what is left, when that is one range.
        if k.start <= 1 && k.end >= n {
            return Err(EditIssue::new(
                EditIssueCode::Keep,
                "/scope/keep",
                "the instruction keeps the whole score, so nothing may change; give the bars to \
                 change in `phrase` or `bars`",
            ));
        }
        let r = if k.start <= 1 {
            BarRange {
                start: k.end + 1,
                end: n,
            }
        } else if k.end >= n {
            BarRange {
                start: 1,
                end: k.start - 1,
            }
        } else {
            return Err(EditIssue::new(
                EditIssueCode::Scope,
                "/scope/keep",
                format!(
                    "keeping {} leaves two separate ranges; give the bars to change in `phrase` \
                     (one of {PHRASE_FORMS}) or `bars`",
                    range_text(k)
                ),
            ));
        };
        (r, true)
    } else {
        (all, false)
    };
    if let Some(k) = keep
        && overlaps(bars, k)
    {
        // Narrow (never widen) to the part outside the kept bars, when that is one range.
        let narrowed = if k.start <= bars.start && k.end < bars.end {
            Some(BarRange {
                start: k.end + 1,
                end: bars.end,
            })
        } else if k.end >= bars.end && k.start > bars.start {
            Some(BarRange {
                start: bars.start,
                end: k.start - 1,
            })
        } else {
            None
        };
        match narrowed {
            Some(r) => {
                remarks.push(format!(
                    "narrowed the edit from {} to {} to keep {}",
                    range_text(bars),
                    range_text(r),
                    range_text(k)
                ));
                bars = r;
            }
            None => {
                return Err(EditIssue::new(
                    EditIssueCode::Keep,
                    "/scope",
                    format!(
                        "the edit's {} overlap the kept {}; change only bars outside them",
                        range_text(bars),
                        range_text(k)
                    ),
                ));
            }
        }
    }
    let whole = bars == all && !(explicit && keep.is_some());
    Ok((bars, whole, remarks))
}

/// Resolves an edit plan against a Score.
pub fn resolve(
    plan: &EditPlan,
    score: &Score,
    catalogue: &Catalogue,
) -> Result<Resolved, Vec<EditIssue>> {
    let mut issues = Vec::new();
    let (bars, whole, remarks) = match resolve_bars(plan, score) {
        Ok(v) => v,
        Err(e) => return Err(vec![e]),
    };

    // Layer references.
    let mut targets: Vec<String> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    for (i, r) in plan.layers.iter().enumerate() {
        let hits: Vec<&Layer> = score
            .layers
            .iter()
            .filter(|l| matches_layer(r, l))
            .collect();
        if hits.is_empty() {
            issues.push(EditIssue::new(
                EditIssueCode::UnknownLayer,
                format!("/layers/{i}"),
                format!(
                    "`{r}` names no layer; the score's layers are {}",
                    layer_list(score)
                ),
            ));
            continue;
        }
        for l in hits {
            let list = if l.locked { &mut refused } else { &mut targets };
            if !list.contains(&l.id) {
                list.push(l.id.clone());
            }
        }
    }
    let order = |ids: &mut Vec<String>| {
        ids.sort_by_key(|id| score.layers.iter().position(|l| &l.id == id));
    };
    order(&mut targets);
    order(&mut refused);

    let needs_target = matches!(
        plan.intent,
        EditIntent::RemoveLayer
            | EditIntent::ChangeInstrument
            | EditIntent::RewriteMotif
            | EditIntent::ChangeDynamics
            | EditIntent::ThinOut
    );
    if needs_target && plan.layers.is_empty() {
        match plan.intent {
            EditIntent::RemoveLayer | EditIntent::ChangeInstrument => issues.push(EditIssue::new(
                EditIssueCode::NoTarget,
                "/layers",
                format!(
                    "name the layer to change; the score's layers are {}",
                    layer_list(score)
                ),
            )),
            _ => {
                // Every unlocked layer that plays in the scope.
                for l in &score.layers {
                    if plays_in(l, bars) {
                        if l.locked {
                            refused.push(l.id.clone());
                        } else {
                            targets.push(l.id.clone());
                        }
                    }
                }
            }
        }
    }
    if needs_target && issues.is_empty() && targets.is_empty() && refused.is_empty() {
        issues.push(EditIssue::new(
            EditIssueCode::NoTarget,
            "/layers",
            format!(
                "no layer plays in {}; the score's layers are {}",
                range_text(bars),
                layer_list(score)
            ),
        ));
    }
    if needs_target && !targets.is_empty() && plan.intent != EditIntent::ChangeInstrument {
        let silent: Vec<&String> = targets
            .iter()
            .filter(|id| score.layer(id).is_some_and(|l| !plays_in(l, bars)))
            .collect();
        if silent.len() == targets.len() && !(whole && plan.intent == EditIntent::RemoveLayer) {
            issues.push(EditIssue::new(
                EditIssueCode::NoTarget,
                "/layers",
                format!(
                    "{} {} not play in {}; pick bars where {} plays",
                    silent
                        .iter()
                        .map(|s| format!("`{s}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    if silent.len() == 1 { "does" } else { "do" },
                    range_text(bars),
                    if silent.len() == 1 { "it" } else { "they" }
                ),
            ));
        }
    }

    // New material's preset.
    let mut preset = None;
    match plan.intent {
        EditIntent::AddLayer | EditIntent::AddMelody | EditIntent::ChangeInstrument => {
            let p = if let Some(name) = &plan.params.preset {
                match catalogue.get(name) {
                    Some(p) => Some(p.clone()),
                    None => {
                        issues.push(EditIssue::new(
                            EditIssueCode::UnknownPreset,
                            "/params/preset",
                            format!(
                                "`{name}` is not a catalogue preset; use one of {}",
                                crate::plan::preset_names(catalogue)
                            ),
                        ));
                        None
                    }
                }
            } else {
                let class = match (plan.intent, plan.params.class) {
                    (_, Some(c)) => Some(c),
                    (EditIntent::AddMelody, None) => Some(GeneratorClass::Melody),
                    _ => None,
                };
                match class.and_then(|c| catalogue.default_for_class(c)) {
                    Some(p) => Some(p.clone()),
                    None => {
                        issues.push(EditIssue::new(
                            EditIssueCode::UnknownPreset,
                            "/params/preset",
                            format!(
                                "name the instrument in `preset`; use one of {}",
                                crate::plan::preset_names(catalogue)
                            ),
                        ));
                        None
                    }
                }
            };
            if let Some(p) = &p {
                let class = new_class(plan, p);
                if score.constraints.avoid.contains(&class)
                    || score.constraints.avoid.contains(&p.class)
                {
                    issues.push(EditIssue::new(
                        EditIssueCode::AvoidedClass,
                        "/params/preset",
                        format!(
                            "`{}` is a {} and the score's avoid-list bans that class; choose \
                             another instrument",
                            p.name,
                            class_name(p.class)
                        ),
                    ));
                }
                if plan.intent == EditIntent::ChangeInstrument {
                    for id in &targets {
                        let Some(l) = score.layer(id) else { continue };
                        if is_drum(l) != !p.pitched {
                            issues.push(EditIssue::new(
                                EditIssueCode::UnknownPreset,
                                "/params/preset",
                                format!(
                                    "`{}` cannot play layer `{id}`: one is a drum and the other                                      is pitched; choose a {} preset",
                                    p.name,
                                    if is_drum(l) { "drum" } else { "pitched" }
                                ),
                            ));
                        }
                        if layer_preset(catalogue, l).is_some_and(|old| old.name == p.name) {
                            issues.push(EditIssue::new(
                                EditIssueCode::UnknownPreset,
                                "/params/preset",
                                format!("layer `{id}` already plays `{}`", p.name),
                            ));
                        }
                    }
                }
            }
            preset = p;
        }
        _ => {}
    }
    if let Some(g) = plan.params.gain_db
        && !(-60.0..=24.0).contains(&g)
    {
        issues.push(EditIssue::new(
            EditIssueCode::Material,
            "/params/gain_db",
            "gain_db must be within -60..=24",
        ));
    }
    if let Some(a) = plan.params.amount_db
        && !(a > 0.0 && a <= 24.0)
    {
        issues.push(EditIssue::new(
            EditIssueCode::Material,
            "/params/amount_db",
            "amount_db must be a positive change of at most 24 dB",
        ));
    }
    if let Some(n) = plan.params.length_bars
        && !(1..=8).contains(&n)
    {
        issues.push(EditIssue::new(
            EditIssueCode::Material,
            "/params/length_bars",
            "length_bars must be within 1..=8",
        ));
    }
    if !issues.is_empty() {
        return Err(issues);
    }
    Ok(Resolved {
        plan: plan.clone(),
        bars,
        whole,
        targets,
        refused,
        preset,
        remarks,
    })
}

/// Class a new layer carries: a melody for `add_melody` unless the preset's class is intrinsic
/// (a drum stays a drum), otherwise the requested class or the preset's.
pub fn new_class(plan: &EditPlan, p: &PresetSpec) -> GeneratorClass {
    if crate::plan::class_is_intrinsic(p.class) {
        return p.class;
    }
    match plan.intent {
        EditIntent::AddMelody => GeneratorClass::Melody,
        _ => plan.params.class.unwrap_or(p.class),
    }
}
