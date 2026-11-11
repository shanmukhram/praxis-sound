//! Brief -> Score: the planner's top-level entry points.
//!
//! - [`plan_brief`]: a plain-language brief through a [`Provider`] (offline rules or Claude), the
//!   validation and repair loop, and the deterministic arranger.
//! - [`compose_plan`]: a ScorePlan written directly (by an agent driving the CLI or MCP server)
//!   through the same validation, deterministic repairs and arranger, with no provider.

use crate::arrange::{ArrangeContext, arrange};
pub use crate::catalogue::Catalogue;
use crate::plan::{
    PlanContext, PlanIssue, ScorePlan, TOOL_NAME, TRAILER_CLASSES, plan_schema, validate_plan,
};
use crate::provider::{Message, Provider, Role, ToolRequest};
use crate::repair::{
    PlanError, Repair, fix_plan, plan_with_repair, repairs_summary, schema_issues,
};
use praxis_score::lint::{LintContext, lint};
use praxis_score::validate::{Issue, IssueCode, validate};
use praxis_score::{GeneratorClass, Score};
use std::collections::BTreeSet;

/// Planner version recorded in provenance.
pub const PLANNER_VERSION: &str = "praxis-planner plan-v1";

/// Everything a planning run produced.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanOutcome {
    /// The arranged Score; passes `validate` with zero errors.
    pub score: Score,
    /// The final (repaired) plan the Score was arranged from.
    pub plan: ScorePlan,
    /// Provider calls made (0 for [`compose_plan`]).
    pub attempts: u32,
    /// Every deterministic repair applied, in order.
    pub repairs: Vec<Repair>,
    /// Issues found after each provider call that had any (empty when the first plan was valid).
    pub rounds: Vec<Vec<PlanIssue>>,
    pub provider_id: String,
    /// Quality lint warnings left on the Score (`praxis_score::lint`).
    pub lint: Vec<Issue>,
}

impl PlanOutcome {
    /// A short plain-language description of the sound: each layer (dynamics, instrument, motif
    /// length and grid, the bars it plays), then the timing, key and playback, and what was kept
    /// out. It is the reply a take shows in the conversation (ADR-0011).
    pub fn summary(&self) -> String {
        plan_summary(&self.plan, &self.score, self.repairs.len())
    }
}

fn grid_words(grid: praxis_score::Grid) -> &'static str {
    use praxis_score::Grid;
    match grid {
        Grid::Quarter => "quarter-note",
        Grid::Eighth => "eighth-note",
        Grid::Sixteenth => "sixteenth-note",
        Grid::EighthTriplet => "eighth-triplet",
        Grid::SixteenthTriplet => "sixteenth-triplet",
    }
}

fn number(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

fn bars_words(start: u32, end: u32) -> String {
    if start == end {
        format!("bar {start}")
    } else {
        format!("bars {start}\u{2013}{end}")
    }
}

/// [`PlanOutcome::summary`] for a plan and the Score arranged from it.
pub fn plan_summary(plan: &ScorePlan, score: &Score, repairs: usize) -> String {
    use crate::plan::Dynamics;
    let mut parts: Vec<String> = Vec::new();
    for l in &plan.layers {
        let Some(layer) = score.layers.iter().find(|x| x.id == l.id) else {
            continue;
        };
        let (Some(start), Some(end)) = (
            layer.clips.iter().map(|c| c.bars.start).min(),
            layer.clips.iter().map(|c| c.bars.end).max(),
        ) else {
            continue;
        };
        let dynamics = match l.dynamics {
            Dynamics::Soft => "soft ",
            Dynamics::Normal => "",
            Dynamics::Strong => "strong ",
        };
        let instrument = l.preset.name.replace('_', " ");
        let dynamics = if instrument.starts_with(dynamics.trim()) && !dynamics.is_empty() {
            ""
        } else {
            dynamics
        };
        let motif = if l.motif.length_bars <= 1 {
            "a one-bar motif".to_string()
        } else {
            format!("a {}-bar motif", l.motif.length_bars)
        };
        let grid = grid_words(l.motif.grid);
        let article = if grid.starts_with('e') { "an" } else { "a" };
        parts.push(format!(
            "{dynamics}{instrument} playing {motif} on {article} {grid} grid in {}",
            bars_words(start, end)
        ));
    }
    let mut out = String::new();
    if !parts.is_empty() {
        let body = parts.join(", plus ");
        let article = if body.starts_with(['a', 'e', 'i', 'o', 'u']) {
            "An"
        } else {
            "A"
        };
        out.push_str(&format!("{article} {body}. "));
    }
    let seconds = praxis_score::timeline::TimeMap::new(score, 48_000).bar_s() * score.bars as f64;
    let playback = match score.playback {
        praxis_score::Playback::Loop => "a seamless loop",
        praxis_score::Playback::OneShot { .. } => "a one-shot",
    };
    out.push_str(&format!(
        "{} bars of {}/{} at {} BPM in {}, {} seconds, {playback}.",
        score.bars,
        score.time_signature.numerator,
        score.time_signature.denominator,
        number(score.tempo.bpm),
        score.key.display(),
        number(seconds)
    ));
    let kept_out: Vec<String> = score
        .constraints
        .avoid
        .iter()
        .map(|c| crate::catalogue::class_name(*c).replace('_', " "))
        .collect();
    if !kept_out.is_empty() {
        out.push_str(&format!(" Kept out: {}.", kept_out.join(", ")));
    }
    if repairs > 0 {
        out.push_str(&format!(
            " The planner fixed {repairs} thing{} in the plan.",
            if repairs == 1 { "" } else { "s" }
        ));
    }
    out
}

/// Trailer classes a brief explicitly asks for ("add a riser into bar 9"). Mood words never
/// count; negated mentions ("no risers") never count.
pub fn requested_classes(brief: &str) -> BTreeSet<GeneratorClass> {
    crate::offline::class_mentions(brief)
        .requested
        .into_iter()
        .filter(|c| TRAILER_CLASSES.contains(c))
        .collect()
}

/// System prompt for model providers.
pub fn system_prompt(catalogue: &Catalogue) -> String {
    format!(
        "You are the planner of Praxis Sound, a sound-design workstation. You never produce \
audio. You turn the user's brief into a compact ScorePlan by calling the `{TOOL_NAME}` tool \
exactly once; a deterministic arranger expands the plan into notes.\n\n\
Rules (the owner's taste; plans that break them are rejected):\n\
1. Rhythm first: every layer is one repeating motif on a step grid (spoken rhythm: DON strong, \
din soft, don normal, TIN bright, - rest, ~ tie) with deliberate accents.\n\
2. Notes connect: sustaining instruments use sustain_steps above 1.0 (1.6 is a good default).\n\
3. Arrangement is layering: a later section adds or removes whole layers; it never rewrites a \
part that is already playing. List in each section the ids of the layers that play in it.\n\
4. Restraint: one emotion word, few elements (at most 5 layers, usually 1-3). No risers, sweeps, \
impacts, booms, pads, whooshes or fills unless the brief literally asks for them. \"Cinematic\", \
\"epic\" or \"mysterious\" describe mood; they are not a licence for trailer effects.\n\
5. Always in key: scale degrees are in the key by construction; explicit notes must be too.\n\
6. Duration: if the brief gives bars and a length in seconds, give bars and duration_s and leave \
tempo_bpm empty (16 bars in 30 s is 128 BPM). If it gives a tempo and a length, leave bars empty.\n\
7. Use only presets from this catalogue (name, kind, class):\n{}\n\
Put requested instruments the catalogue lacks in `notes` instead of substituting silently.",
        catalogue.prompt_listing()
    )
}

/// The tool request for a brief.
pub fn tool_request(brief: &str, catalogue: &Catalogue, seed: u64) -> ToolRequest {
    ToolRequest {
        system: system_prompt(catalogue),
        messages: vec![Message {
            role: Role::User,
            content: brief.to_string(),
        }],
        tool_name: TOOL_NAME.to_string(),
        tool_description: "Submit the ScorePlan for the brief: key, timing, sections, layers \
                           with spoken-rhythm motifs, chords and the avoid-list."
            .to_string(),
        input_schema: plan_schema(),
        max_tokens: 16_000,
        seed,
    }
}

/// Plans a Score from a brief. The same brief, provider and seed give the same Score.
pub fn plan_brief(
    brief: &str,
    provider: &dyn Provider,
    catalogue: &Catalogue,
    seed: u64,
) -> Result<PlanOutcome, PlanError> {
    let requested = requested_classes(brief);
    let ctx = PlanContext {
        catalogue,
        requested: &requested,
    };
    let req = tool_request(brief, catalogue, seed);
    let out = plan_with_repair(provider, req, &ctx)?;
    let mut plan = out.plan;
    // The plan's own `requested` field is ignored when a brief is present: the brief's words
    // decide (a model cannot talk itself into a riser).
    let repairs = out.repairs;
    plan.requested = requested.iter().copied().collect();
    finish(
        plan,
        &ctx,
        seed,
        Some(brief),
        provider.id(),
        out.attempts,
        repairs,
        out.rounds,
    )
}

/// Composes a Score from a ScorePlan written directly (no provider, no brief). Runs the same
/// validation, deterministic repairs (recorded in `repairs`) and arranger as [`plan_brief`].
/// Trailer classes are allowed only when listed in the plan's `requested` field.
pub fn compose_plan(
    plan_json: &serde_json::Value,
    catalogue: &Catalogue,
    seed: u64,
) -> Result<PlanOutcome, PlanError> {
    let plan: ScorePlan = serde_json::from_value(plan_json.clone())
        .map_err(|e| PlanError::Invalid(schema_issues(plan_json, &e)))?;
    let requested: BTreeSet<GeneratorClass> = plan
        .requested
        .iter()
        .copied()
        .filter(|c| TRAILER_CLASSES.contains(c))
        .collect();
    let ctx = PlanContext {
        catalogue,
        requested: &requested,
    };
    let issues = validate_plan(&plan, &ctx);
    let (plan, repairs, rounds) = if issues.is_empty() {
        (plan, vec![], vec![])
    } else {
        let (fixed, repairs) = fix_plan(&plan, &ctx);
        let remaining = validate_plan(&fixed, &ctx);
        if !remaining.is_empty() {
            return Err(PlanError::Invalid(remaining));
        }
        (fixed, repairs, vec![issues])
    };
    finish(
        plan,
        &ctx,
        seed,
        None,
        "direct-plan".to_string(),
        0,
        repairs,
        rounds,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish(
    plan: ScorePlan,
    ctx: &PlanContext,
    seed: u64,
    brief: Option<&str>,
    provider_id: String,
    attempts: u32,
    mut repairs: Vec<Repair>,
    rounds: Vec<Vec<PlanIssue>>,
) -> Result<PlanOutcome, PlanError> {
    let actx = ArrangeContext {
        catalogue: ctx.catalogue,
        requested: ctx.requested,
        seed,
        brief,
    };
    let mut score = arrange(&plan, &actx).map_err(|e| PlanError::Arrange(e.to_string()))?;

    // Lints drive arranger fixes (ADR-0006 step 4).
    let lint_ctx = lint_context(&score, ctx);
    let mut warnings = lint(&score, &lint_ctx);
    if warnings
        .iter()
        .any(|w| w.code == IssueCode::MissingSustainOverlap)
    {
        fix_sustain(&mut score, &lint_ctx, &mut repairs);
        warnings = lint(&score, &lint_ctx);
    }

    let report = validate(&score);
    if !report.is_valid() {
        let msg = report
            .errors
            .iter()
            .map(|e| format!("{} {:?}: {}", e.path, e.code, e.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(PlanError::InvalidScore(msg));
    }

    let prov = &mut score.meta.provenance;
    prov.insert("planner".into(), PLANNER_VERSION.into());
    prov.insert("provider".into(), provider_id.clone());
    prov.insert("attempts".into(), attempts.to_string());
    prov.insert("repairs".into(), repairs.len().to_string());
    if !repairs.is_empty() {
        prov.insert("repair_codes".into(), repairs_summary(&repairs));
    }
    prov.insert("seed".into(), seed.to_string());
    if !plan.notes.is_empty() {
        prov.insert("planner_notes".into(), plan.notes.join("; "));
    }
    Ok(PlanOutcome {
        score,
        plan,
        attempts,
        repairs,
        rounds,
        provider_id,
        lint: warnings,
    })
}

/// Lint facts: which layers ring, which classes were asked for.
pub fn lint_context(score: &Score, ctx: &PlanContext) -> LintContext {
    let sustaining_layers = score
        .layers
        .iter()
        .filter(|l| {
            ctx.catalogue
                .get_kind(l.instrument.kind, &l.instrument.preset)
                .is_some_and(|p| p.sustaining)
        })
        .map(|l| l.id.clone())
        .collect();
    let mut requested: BTreeSet<GeneratorClass> = ctx.requested.clone();
    requested.extend(score.layers.iter().map(|l| l.class));
    LintContext {
        sustaining_layers,
        requested_classes: requested.into_iter().collect(),
    }
}

/// Lengthens notes of sustaining layers that do not overlap.
fn fix_sustain(score: &mut Score, lint_ctx: &LintContext, repairs: &mut Vec<Repair>) {
    let patterns: BTreeSet<String> = score
        .layers
        .iter()
        .filter(|l| lint_ctx.sustaining_layers.contains(&l.id))
        .flat_map(|l| l.clips.iter().map(|c| c.pattern.clone()))
        .collect();
    for p in &mut score.patterns {
        if !patterns.contains(&p.id) {
            continue;
        }
        let mut changed = false;
        for n in &mut p.notes {
            if n.length <= 1.0 {
                n.length = crate::arrange::DEFAULT_SUSTAIN;
                changed = true;
            }
        }
        if changed {
            repairs.push(Repair {
                code: crate::plan::PlanIssueCode::Sustain,
                path: format!("/patterns/{}", p.id),
                action: "lengthened notes so the sustaining instrument overlaps".into(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RuleProvider;

    #[test]
    fn summary_says_what_plays_where_in_plain_words() {
        let out = plan_brief(
            "16 bars, 4/4, A minor, 30 seconds, seamless loop. Bars 1-8: one soft felt piano \
             playing a repeating rhythmic motif on an eighth-note grid. Bars 9-16: the identical \
             piano part plus a soft muted kick underneath. No risers.",
            &RuleProvider::new(),
            &Catalogue::standard(),
            1,
        )
        .expect("plan");
        let s = out.summary();
        assert!(s.starts_with("A soft felt piano playing"), "{s}");
        assert!(s.contains("eighth-note grid in bars 1\u{2013}16"), "{s}");
        assert!(s.contains("soft kick playing"), "{s}");
        assert!(!s.contains("soft soft"), "{s}");
        assert!(s.contains("16 bars of 4/4 at 128 BPM"), "{s}");
        assert!(s.contains("30 seconds, a seamless loop."), "{s}");
        assert!(s.contains("riser"), "{s}");
    }
}
