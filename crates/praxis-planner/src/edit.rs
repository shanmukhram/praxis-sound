//! Scoped edits (FR-7): an instruction ("keep the first half, change the last four bars to add
//! a subtle melody") becomes an [`EditPlan`] through one tool call (`edit_score`), the plan is
//! resolved against the Score (exact bars, target layers, never locked ones), turned into a
//! `ScorePatch` with `origin: model` and that exact scope, and applied with
//! `praxis_score::apply::apply`, which enforces locks, scope and isolation.
//!
//! - [`plan_edit`]: an instruction through a [`Provider`]; issues (and patches `apply` rejects)
//!   go back to the provider for at most [`MAX_REPAIR_ROUNDS`] rounds, and a repaired plan may
//!   never reach outside the bars and layers of the first plan that resolved.
//! - [`compose_edit`]: an EditPlan written directly by an agent, no provider.
//!
//! See the "Edits" section of docs/contracts/planner.md.

pub mod build;
pub mod material;
pub mod plan;
pub mod resolve;
pub mod rules;
pub mod scope;

pub use plan::{
    ChangeAmount, DynamicsChange, EDIT_TOOL_NAME, EditIntent, EditIssue, EditIssueCode, EditParams,
    EditPlan, EditScope, SpaceChange, ToneChange, edit_issues_text, edit_schema,
};
pub use resolve::Resolved;
pub use rules::{EditRules, parse_edit};

use crate::catalogue::{Catalogue, class_name};
use crate::provider::{Message, Provider, ProviderError, Role, ToolRequest};
use crate::repair::{MAX_REPAIR_ROUNDS, Repair};
use praxis_score::patch::{PatchError, Scope, ScorePatch};
use praxis_score::project::Project;
use praxis_score::{BarRange, Score};
use resolve::bars_text;

/// What the edit planner can do, in plain words; the summary of a `not_understood` edit ends
/// with it.
pub const EDIT_CAPABILITIES: &str = "I can add a layer or a melody, remove or mute a layer, \
swap an instrument, make layers softer or louder, thin a part out, make the sound darker \
(muffled, warmer) or brighter, give it more or less room (reverb), or write new notes (a new \
motif, a different rhythm, a variation), for the whole sound or for some bars (\"in the last \
four bars\", \"bars 9-12\")";

/// Provider id of the offline rules. `plan_edit` answers edit requests to a provider with this id
/// (`RuleProvider`) with [`EditRules`].
pub const RULE_PROVIDER_ID: &str = "offline-rules";

/// What a scoped edit produced.
#[derive(Clone, Debug, PartialEq)]
pub struct EditOutcome {
    /// The applied patch (`origin: model`, scoped to `scope`). Empty `ops` when nothing changed
    /// (for example every named layer is locked; `summary` says why).
    pub patch: ScorePatch,
    /// The patch that undoes it exactly.
    pub inverse: ScorePatch,
    /// The edited Score.
    pub score: Score,
    /// The patch scope: bars (unset for a whole-score edit) and layers.
    pub scope: Scope,
    /// The bars the edit was allowed to change (the whole score when `scope.bars` is unset).
    pub bars: BarRange,
    /// What changed, what was kept, and any refusal, in plain words.
    pub summary: String,
    /// Locked layers the instruction named; they were not changed.
    pub refused: Vec<String>,
    /// Layers the edit added.
    pub new_layers: Vec<String>,
    /// The final edit plan.
    pub plan: EditPlan,
    /// Provider calls (0 for [`compose_edit`]).
    pub attempts: u32,
    /// Issues after each provider call that had any.
    pub rounds: Vec<Vec<EditIssue>>,
    /// Deterministic repairs made to new material.
    pub repairs: Vec<Repair>,
    pub provider_id: String,
}

impl EditOutcome {
    /// True when the patch changed the Score.
    pub fn changed(&self) -> bool {
        !self.patch.ops.is_empty()
    }

    /// False when the instruction asked for no change the planner can make (`not_understood`):
    /// the patch is empty and `summary` says what edits are possible.
    pub fn understood(&self) -> bool {
        self.plan.intent != EditIntent::NotUnderstood
    }
}

/// Why an edit failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EditError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    /// The edit plan could not be resolved, built or applied. Each issue has a JSON pointer into
    /// the plan, what is wrong and what is allowed.
    #[error("the edit has problems:\n{}", edit_issues_text(.issues))]
    Invalid {
        issues: Vec<EditIssue>,
        rounds: Vec<Vec<EditIssue>>,
    },
}

impl EditError {
    /// The final issues (empty for provider errors).
    pub fn issues(&self) -> &[EditIssue] {
        match self {
            EditError::Invalid { issues, .. } => issues,
            EditError::Provider(_) => &[],
        }
    }
}

/// The Score as the edit planner sees it: timing, key, sections and layers with their bars.
pub fn score_listing(score: &Score) -> String {
    let mut s = format!(
        "{} bars of {}/{} at {} BPM in {}.\n",
        score.bars,
        score.time_signature.numerator,
        score.time_signature.denominator,
        score.tempo.bpm,
        score.key.display()
    );
    for sec in &score.sections {
        s.push_str(&format!(
            "Section `{}` ({}): {}.\n",
            sec.id,
            sec.name,
            bars_text(sec.bars)
        ));
    }
    for l in &score.layers {
        let bars: Vec<String> = l.clips.iter().map(|c| bars_text(c.bars)).collect();
        s.push_str(&format!(
            "Layer `{}` \"{}\": {} {}, plays {}{}.\n",
            l.id,
            l.name,
            class_name(l.class),
            l.instrument.preset,
            if bars.is_empty() {
                "nowhere".to_string()
            } else {
                bars.join(", ")
            },
            if l.locked {
                "; LOCKED (never change it)"
            } else {
                ""
            }
        ));
    }
    if !score.constraints.avoid.is_empty() {
        s.push_str(&format!(
            "Avoid-list: {}.\n",
            score
                .constraints
                .avoid
                .iter()
                .map(|c| class_name(*c))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    s
}

/// System prompt for an edit request.
pub fn edit_system_prompt(score: &Score, catalogue: &Catalogue) -> String {
    format!(
        "You are the edit planner of Praxis Sound. You never produce audio. You turn the user's \
edit instruction into one EditPlan by calling the `{EDIT_TOOL_NAME}` tool exactly once; the \
planner turns it into a patch that may change only the bars and layers you name.\n\n\
Rules:\n\
1. Scope exactly what the user asked. Give the bars as `scope.phrase` (\"last 4 bars\", \
\"second half\", \"bars 9-12\") or `scope.bars`; put what the user wants kept in `scope.keep`. \
Never widen the scope.\n\
2. Never name a locked layer as a target; if the user asks to change one, still name it in \
`layers` so the planner can explain that it is locked.\n\
3. Arrangement is layering: prefer adding a layer in the scope to rewriting existing parts.\n\
4. Restraint: subtle means soft dynamics; no risers, sweeps, impacts, booms, pads, whooshes or \
fills unless the user asks.\n\
5. Tone and space change the sound, never the notes. Muffled, muted, darker, warmer, duller, \
mellow or a softer tone is `change_tone` with `params.tone` = `darker`; brighter, clearer, \
crisper or more air is `brighter`. More room, bigger, a hall, wetter or more space is \
`change_space` with `params.space` = `wetter`; drier, less reverb or closer is `drier`; no \
reverb at all is `dry`. Give the size in `params.amount` (`a_little`, `normal`, `much`). Presets \
with tone controls: {}.\n\
6. Use `rewrite_motif` only when the user asks for new notes (a new or different melody, \
motif or rhythm, a variation). Never rewrite notes to answer a request about sound, level or \
space.\n\
7. If the instruction asks for no change you can express, use `not_understood` and say why in \
`notes`; nothing changes and the user is told what edits are possible.\n\
8. Use only presets from this catalogue (name, kind, class):\n{}\n\
The score:\n{}",
        tone_listing(catalogue),
        catalogue.prompt_listing(),
        score_listing(score)
    )
}

/// Presets with tone controls and their defaults, for the system prompt.
fn tone_listing(catalogue: &Catalogue) -> String {
    catalogue
        .presets
        .iter()
        .filter_map(|p| {
            let t = p.tone?;
            Some(match t.felt_default {
                Some(f) => format!("{} (tone {:.2}, felt {:.2})", p.name, t.tone_default, f),
                None => format!("{} (tone {:.2})", p.name, t.tone_default),
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The tool request for an edit instruction.
pub fn edit_request(
    instruction: &str,
    score: &Score,
    catalogue: &Catalogue,
    seed: u64,
) -> ToolRequest {
    ToolRequest {
        system: edit_system_prompt(score, catalogue),
        messages: vec![Message {
            role: Role::User,
            content: instruction.to_string(),
        }],
        tool_name: EDIT_TOOL_NAME.to_string(),
        tool_description: "Submit the EditPlan for the instruction: intent, the bars to change \
                           (and to keep), the target layers and parameters. Tone and space \
                           requests use change_tone / change_space and never change notes."
            .to_string(),
        input_schema: edit_schema(),
        max_tokens: 4_000,
        seed,
    }
}

/// The user message asking the provider to fix its edit plan.
pub fn edit_repair_prompt(issues: &[EditIssue]) -> String {
    format!(
        "The edit plan you returned has these problems:\n{}\n\nCall the `{EDIT_TOOL_NAME}` tool \
         again with the whole corrected plan. Change only what these problems require, and \
         never widen the bars or layers.",
        edit_issues_text(issues)
    )
}

/// Bars and layers a repaired plan must stay within: the bars of the first plan whose scope
/// resolved, and the target layers of the first plan that resolved completely.
#[derive(Clone, Debug, PartialEq, Default)]
struct Bound {
    bars: Option<(BarRange, bool)>,
    targets: Option<Vec<String>>,
}

impl Bound {
    /// Issues when `bars`/`targets` reach outside the bound; records them when it is unset.
    fn check_bars(&mut self, bars: BarRange, whole: bool) -> Option<EditIssue> {
        let Some((b, b_whole)) = self.bars else {
            self.bars = Some((bars, whole));
            return None;
        };
        if (whole && !b_whole) || bars.start < b.start || bars.end > b.end {
            return Some(EditIssue::new(
                EditIssueCode::Widened,
                "/scope",
                format!(
                    "the edit was scoped to {}; the corrected plan reaches {}. Stay inside {}",
                    bars_text(b),
                    bars_text(bars),
                    bars_text(b)
                ),
            ));
        }
        None
    }

    fn check_targets(&mut self, targets: &[String]) -> Option<EditIssue> {
        let Some(b) = &self.targets else {
            self.targets = Some(targets.to_vec());
            return None;
        };
        let extra: Vec<String> = targets
            .iter()
            .filter(|t| !b.contains(t))
            .map(|s| format!("`{s}`"))
            .collect();
        if extra.is_empty() {
            return None;
        }
        Some(EditIssue::new(
            EditIssueCode::Widened,
            "/layers",
            format!(
                "the edit was limited to layers [{}]; do not add {}",
                b.join(", "),
                extra.join(", ")
            ),
        ))
    }
}

/// One attempt: parse, resolve, bound check, build, apply.
fn try_edit(
    value: &serde_json::Value,
    score: &Score,
    catalogue: &Catalogue,
    seed: u64,
    bound: &mut Bound,
) -> Result<(EditOutcome, Vec<Repair>), Vec<EditIssue>> {
    let plan: EditPlan = serde_json::from_value(value.clone()).map_err(|e| {
        vec![EditIssue::new(
            EditIssueCode::Schema,
            "",
            format!(
                "{e}; the input must match the `{EDIT_TOOL_NAME}` schema (intent is one of \
                 add_layer, add_melody, rewrite_motif, change_instrument, change_dynamics, \
                 remove_layer, thin_out, change_tone, change_space, not_understood)"
            ),
        )]
    })?;
    if plan.intent == EditIntent::NotUnderstood {
        return Ok((not_understood(plan, score), vec![]));
    }
    // The scope is bound as soon as it resolves, even if the rest of the plan has problems.
    if let Ok((bars, whole, _)) = resolve::resolve_bars(&plan, score)
        && let Some(issue) = bound.check_bars(bars, whole)
    {
        return Err(vec![issue]);
    }
    let r = resolve::resolve(&plan, score, catalogue)?;
    if let Some(issue) = bound.check_targets(&r.targets) {
        return Err(vec![issue]);
    }
    let built = build::build_patch(&r, score, catalogue, seed)?;
    let applied = praxis_score::apply::apply(score, &built.patch).map_err(|e| {
        vec![EditIssue::new(
            EditIssueCode::Rejected,
            rejected_path(&e),
            format!("the patch was rejected: {e}"),
        )]
    })?;
    let summary = summarize(&r, &built, score);
    let outcome = EditOutcome {
        scope: built.patch.scope.clone(),
        patch: built.patch,
        inverse: applied.inverse,
        score: applied.score,
        bars: r.bars,
        summary,
        refused: r.refused.clone(),
        new_layers: built.new_layers,
        plan,
        attempts: 0,
        rounds: vec![],
        repairs: vec![],
        provider_id: String::new(),
    };
    Ok((outcome, built.repairs))
}

/// The outcome of a `not_understood` plan: no patch, the Score as it was, and a summary that
/// says nothing changed and what edits are possible.
fn not_understood(plan: EditPlan, score: &Score) -> EditOutcome {
    let why = if plan.notes.is_empty() {
        "the instruction asks for no change I can make".to_string()
    } else {
        plan.notes.join("; ")
    };
    let empty = ScorePatch {
        description: "not_understood: no change".to_string(),
        origin: praxis_score::patch::Origin::Model,
        scope: Scope::default(),
        ops: vec![],
    };
    EditOutcome {
        patch: empty.clone(),
        inverse: empty,
        score: score.clone(),
        scope: Scope::default(),
        bars: BarRange {
            start: 1,
            end: score.bars.max(1),
        },
        summary: format!("Nothing changed: {why}. {EDIT_CAPABILITIES}."),
        refused: vec![],
        new_layers: vec![],
        plan,
        attempts: 0,
        rounds: vec![],
        repairs: vec![],
        provider_id: String::new(),
    }
}

fn rejected_path(e: &PatchError) -> String {
    match e {
        PatchError::LockedLayer(_) | PatchError::UnlockNotAllowed(_) => "/layers".into(),
        PatchError::OutOfScope { .. } => "/scope".into(),
        _ => String::new(),
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn summarize(r: &resolve::Resolved, b: &build::Built, score: &Score) -> String {
    let mut parts: Vec<String> = Vec::new();
    if b.actions.is_empty() {
        parts.push("No change".to_string());
    } else {
        parts.push(capitalise(&b.actions.join("; ")));
        if !r.whole {
            let mut kept = Vec::new();
            if r.bars.start > 1 {
                kept.push(bars_text(BarRange {
                    start: 1,
                    end: r.bars.start - 1,
                }));
            }
            if r.bars.end < score.bars {
                kept.push(bars_text(BarRange {
                    start: r.bars.end + 1,
                    end: score.bars,
                }));
            }
            if !kept.is_empty() {
                parts.push(format!("{} are unchanged", capitalise(&kept.join(" and "))));
            }
        }
        if matches!(
            r.plan.intent,
            EditIntent::ChangeTone | EditIntent::ChangeSpace
        ) {
            parts.push("The notes are unchanged".to_string());
        }
        let others: Vec<&str> = score
            .layers
            .iter()
            .filter(|l| !b.patch.scope.layers.contains(&l.id))
            .map(|l| l.id.as_str())
            .collect();
        if !others.is_empty() && !b.patch.scope.layers.is_empty() {
            parts.push(format!(
                "Layer{} {} untouched",
                if others.len() == 1 { "" } else { "s" },
                others
                    .iter()
                    .map(|s| format!("`{s}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
                    + if others.len() == 1 { " is" } else { " are" }
            ));
        }
    }
    if !r.refused.is_empty() {
        let names = r
            .refused
            .iter()
            .map(|s| format!("`{s}`"))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!(
            "Left {names} alone: {} locked, and edits never change a locked layer (unlock {} \
             first to edit {})",
            if r.refused.len() == 1 {
                "it is"
            } else {
                "they are"
            },
            if r.refused.len() == 1 { "it" } else { "them" },
            if r.refused.len() == 1 { "it" } else { "them" },
        ));
    } else if b.actions.is_empty() {
        parts.push("nothing in the scope needed changing".to_string());
    }
    for remark in &r.remarks {
        parts.push(capitalise(remark));
    }
    parts.join(". ") + "."
}

/// Plans and applies a scoped edit from an instruction. The same Score, instruction, provider
/// and seed give the same outcome.
pub fn plan_edit(
    score: &Score,
    instruction: &str,
    provider: &dyn Provider,
    catalogue: &Catalogue,
    seed: u64,
) -> Result<EditOutcome, EditError> {
    let rules = EditRules::new();
    let provider: &dyn Provider = if provider.id() == RULE_PROVIDER_ID {
        &rules
    } else {
        provider
    };
    let mut req = edit_request(instruction, score, catalogue, seed);
    let mut rounds: Vec<Vec<EditIssue>> = Vec::new();
    let mut bound = Bound::default();
    let mut last_value: Option<serde_json::Value> = None;
    let mut attempts = 0u32;
    loop {
        let value = provider.call_tool(&req)?;
        attempts += 1;
        let repeated = last_value.as_ref() == Some(&value);
        match try_edit(&value, score, catalogue, seed, &mut bound) {
            Ok((mut out, repairs)) => {
                out.attempts = attempts;
                out.rounds = rounds;
                out.repairs = repairs;
                out.provider_id = provider.id();
                return Ok(out);
            }
            Err(issues) => {
                rounds.push(issues.clone());
                // A provider that answers the same plan again cannot repair it.
                if repeated || attempts > MAX_REPAIR_ROUNDS {
                    return Err(EditError::Invalid { issues, rounds });
                }
                let text = serde_json::to_string(&value).unwrap_or_default();
                req.messages.push(Message {
                    role: Role::Assistant,
                    content: format!("Called `{EDIT_TOOL_NAME}` with: {text}"),
                });
                req.messages.push(Message {
                    role: Role::User,
                    content: edit_repair_prompt(&issues),
                });
                last_value = Some(value);
            }
        }
    }
}

/// Applies an EditPlan written directly (by an agent through the CLI or MCP server): the same
/// resolution, patch building and `apply` as [`plan_edit`], with no provider and no repair
/// rounds. On [`EditError::Invalid`] each issue's `path` points into the plan.
pub fn compose_edit(
    edit_plan_json: &serde_json::Value,
    score: &Score,
    catalogue: &Catalogue,
    seed: u64,
) -> Result<EditOutcome, EditError> {
    let mut bound = Bound::default();
    match try_edit(edit_plan_json, score, catalogue, seed, &mut bound) {
        Ok((mut out, repairs)) => {
            out.repairs = repairs;
            out.provider_id = "direct-plan".to_string();
            Ok(out)
        }
        Err(issues) => Err(EditError::Invalid {
            issues,
            rounds: vec![],
        }),
    }
}

/// [`plan_edit`] on a project's Score, committing the patch (so it can be undone) when it
/// changes anything.
pub fn plan_project_edit(
    project: &mut Project,
    instruction: &str,
    provider: &dyn Provider,
    catalogue: &Catalogue,
    seed: u64,
) -> Result<EditOutcome, EditError> {
    let out = plan_edit(&project.score, instruction, provider, catalogue, seed)?;
    if out.changed() {
        project
            .commit(out.patch.clone())
            .map_err(|e| EditError::Invalid {
                issues: vec![EditIssue::new(
                    EditIssueCode::Rejected,
                    rejected_path(&e),
                    format!("the project rejected the patch: {e}"),
                )],
                rounds: out.rounds.clone(),
            })?;
    }
    Ok(out)
}
