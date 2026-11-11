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
    PlanError, Repair, fix_plan, plan_with_quality_repair, plan_with_repair, repairs_summary,
    schema_issues,
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
    let scoring = crate::scoring::summary(plan, score);
    if !scoring.is_empty() {
        out.push(' ');
        out.push_str(&scoring);
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
        "You are the planner of Praxis Sound, a sound-design workstation. You never produce audio. You \
turn the user's brief into a compact ScorePlan by calling the `{TOOL_NAME}` tool exactly once; a \
deterministic arranger expands the plan into notes.\n\
\n\
Rules (the owner's taste; plans that break them are rejected):\n\
1. Rhythm first: every layer is one repeating motif on a step grid (spoken rhythm: DON strong, \
din soft, don normal, TIN bright, - rest, ~ tie) with deliberate accents. Keep every layer busy \
enough that no note is followed by more than one beat of silence: a bass or a slow part holds \
its notes with ties (~) or sustain_steps that reach the next hit instead of leaving rests. If \
the brief gives a spoken rhythm, copy it exactly as `spoken`, without padding it with rests.\n\
2. Notes connect: sustaining instruments use sustain_steps above 1.0 (1.6 is a good default).\n\
3. Arrangement is layering: a later section adds or removes whole layers; it never rewrites a \
part that is already playing. List in each section the ids of the layers that play in it.\n\
4. Restraint: one emotion word, few elements. Use only the instruments the brief names: one \
layer per named instrument, and no layer the brief did not ask for (no added bass, keys, pad, \
drums or second melody to fill out the sound). If the brief names one instrument, the plan has \
one layer. A brief that names no instrument gets one to three that fit it. No risers, sweeps, \
impacts, booms, pads, whooshes or fills unless the brief literally asks for them. \"Cinematic\", \
\"epic\" or \"mysterious\" describe mood; they are not a licence for trailer effects.\n\
5. Every layer plays from bar 1 to the last bar unless the brief says when it enters or leaves \
(\"bars 9-16: add a kick\", \"in the second half\"). Without such words use one section, or list \
every layer in every section.\n\
6. Emotion: the brief's own mood word when it has one (\"suspenseful\" -> tense).\n\
7. Always in key: scale degrees are in the key by construction; explicit notes must be too.\n\
8. Duration: if the brief gives bars and a length in seconds, give bars and duration_s and leave \
tempo_bpm empty (16 bars in 30 s is 128 BPM). If it gives a tempo and a length, leave bars \
empty.\n\
9. Use only presets from this catalogue (name, kind, class):\n\
{}\n\
An instrument the brief names that the catalogue lacks goes in `notes` (\"strings are not in the \
catalogue\"); never replace it with other layers.\n\
\n\
Scoring a cue (only when the brief asks for a cue, a form, sections such as intro / build / \
peak / release or verse / chorus, a chord progression, or hit points):\n\
10. Form: give `form` parts in order (role intro, verse, main, build, peak, breakdown, release, \
transition or outro; `weight` = relative length; optional `intensity` 0..1) and leave \
`sections` empty: the planner solves each part's bars to the length. Leave a part's `layers` \
empty so the layers enter by intensity in the order you list them (the first layer alone in the \
quietest part, all of them in the loudest); list them only when the brief says what plays \
where. `transition` is auto (a drop into a peak, a pickup when a layer enters, a held chord into \
a release), none, pickup, drop or hold; never a riser unless the brief asks for one.\n\
11. Hit points: \"hits at 0:42, 1:05 and 1:38\" -> `hits` [{{\"time_s\": 42}}, \
{{\"time_s\": 65}}, {{\"time_s\": 98}}]. Give `duration_s` (and `tempo_bpm`, or a \
`tempo_range` when the brief gives no tempo); the planner picks the exact tempo so every hit \
lands on the beat grid, accents it and starts a form part there.\n\
12. Harmony: a `progression` (chord symbols in the key or Roman numerals such as i VI III VII, \
`bars_per_chord` 1 or 2) repeats from each part's start; leave `chords` empty with it. Every \
chord must be in the key (no borrowed chords). Melodic and chord layers set `follow_chords`.\n\
13. Emotion presets for cues (mode, tempo range, register, instruments in entry order, form \
shape); use the preset's instruments only when the brief names none:\n\
{}",
        catalogue.prompt_listing(),
        crate::scoring::emotion::prompt_listing()
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
    // The offline planner is a pure function of the brief and is never asked twice; a model gets
    // the quality checks as repair rounds too.
    let out = if provider.id() == crate::edit::RULE_PROVIDER_ID {
        plan_with_repair(provider, req, &ctx)?
    } else {
        let named = crate::offline::brief_instruments(brief);
        let quality = |p: &ScorePlan| quality_issues(p, &ctx, &named, seed);
        plan_with_quality_repair(provider, req, &ctx, &quality)?
    };
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
    // Scoring fields (form, hits, progression) become sections, tempo and chords; the outcome's
    // plan is the realised one, which arranges to the same Score again.
    let plan = crate::scoring::realise(&plan, seed);
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

/// Quality issues of a valid plan, for a model's repair rounds: the brief's restraint rules
/// (a layer for an instrument the brief does not name; a layer that enters late or leaves early
/// when the brief never says when) and the lints of the arranged Score (`isolated_notes`,
/// `no_motif_repetition`, `missing_sustain_overlap`, `too_many_elements`,
/// `unrequested_effect_class`). Each issue points into the plan and says plainly what to change.
pub fn quality_issues(
    plan: &ScorePlan,
    ctx: &PlanContext,
    named: &crate::offline::BriefInstruments,
    seed: u64,
) -> Vec<PlanIssue> {
    use crate::plan::PlanIssueCode as C;
    let mut out = Vec::new();
    let mut plan = plan.clone();
    plan.requested = ctx.requested.iter().copied().collect();
    let actx = ArrangeContext {
        catalogue: ctx.catalogue,
        requested: ctx.requested,
        seed,
        brief: None,
    };
    // An arranger failure is reported by `finish` as today; there is nothing to lint.
    let Ok(score) = arrange(&plan, &actx) else {
        return out;
    };
    let plan_index = |id: &str| plan.layers.iter().position(|l| l.id == id);

    // Restraint: only the instruments the brief names. Judged only when the brief names some
    // and the plan covers at least one of them (a plan whose every layer is a substitute is a
    // reading of the brief, not an extra layer).
    if named.names_any() {
        let covered: Vec<bool> = score
            .layers
            .iter()
            .map(|l| named.covers(&l.instrument.preset, l.class))
            .collect();
        if covered.iter().any(|c| *c) {
            let mut asked: Vec<String> = named.families.iter().cloned().collect();
            asked.extend(
                named
                    .unmapped
                    .iter()
                    .map(|u| format!("{u} (not in the catalogue)")),
            );
            for (l, _) in score.layers.iter().zip(&covered).filter(|(_, c)| !**c) {
                let Some(i) = plan_index(&l.id) else { continue };
                out.push(PlanIssue::new(
                    C::UnrequestedLayer,
                    format!("/layers/{i}"),
                    format!(
                        "layer `{}` ({}) is an instrument the brief does not ask for; the \
                         brief names only: {}. Remove this layer (and its id from the \
                         sections). Use one layer per named instrument; an instrument the \
                         catalogue lacks goes in `notes`, not into other layers",
                        l.id,
                        l.instrument.preset,
                        asked.join(", ")
                    ),
                ));
            }
        }
    }

    // Every layer plays throughout unless the brief says when (a form says when).
    if !named.timed && plan.form.is_empty() {
        for l in &score.layers {
            let bars: BTreeSet<u32> = l
                .clips
                .iter()
                .flat_map(|c| c.bars.start..=c.bars.end.min(score.bars))
                .collect();
            if bars.len() as u32 >= score.bars {
                continue;
            }
            let (Some(first), Some(last)) = (bars.first(), bars.last()) else {
                continue;
            };
            out.push(PlanIssue::new(
                C::LayerBars,
                "/sections",
                format!(
                    "layer `{}` plays only in bars {first}-{last} of {}, but the brief does \
                     not say when it enters or leaves. List it in every section so it plays \
                     from bar 1 to bar {}",
                    l.id, score.bars, score.bars
                ),
            ));
        }
    }

    // Lints on the arranged Score, as the finished plan will have them.
    let lint_ctx = lint_context(&score, ctx);
    for w in lint(&score, &lint_ctx) {
        let layer = w
            .path
            .strip_prefix("/layers/")
            .and_then(|s| s.split('/').next())
            .and_then(|s| s.parse::<usize>().ok())
            .and_then(|li| score.layers.get(li))
            .and_then(|l| plan_index(&l.id));
        let at = |tail: &str| match layer {
            Some(i) => format!("/layers/{i}{tail}"),
            None => "/layers".to_string(),
        };
        let (code, path, advice) = match w.code {
            IssueCode::IsolatedNotes => (
                C::IsolatedNotes,
                at("/motif"),
                "Keep this layer on a repeating grid with no silence longer than one beat \
                 between notes: add hits or tie notes with `~` in `spoken`, and let sustain \
                 (sustain_steps) connect each note to the next",
            ),
            IssueCode::NoMotifRepetition => (
                C::NoMotifRepetition,
                at("/motif"),
                "Use one short motif (one or two bars of spoken rhythm) that repeats",
            ),
            IssueCode::MissingSustainOverlap => (
                C::MissingSustainOverlap,
                at("/motif/sustain_steps"),
                "This instrument rings: set sustain_steps above 1.0 (1.6) so each note \
                 overlaps the next",
            ),
            IssueCode::TooManyElements => (
                C::TooManyElements,
                "/layers".to_string(),
                "Use fewer layers (only the instruments the brief names, at most four at once) \
                 and one emotion word",
            ),
            IssueCode::UnrequestedEffectClass => (
                C::UnrequestedEffectClass,
                at(""),
                "The brief did not ask for this effect: remove the layer",
            ),
            _ => continue,
        };
        out.push(PlanIssue::new(
            code,
            path,
            format!("{} {advice}", w.message),
        ));
    }
    out
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
