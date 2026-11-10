//! Variations (FR-8): N alternates of a selection (bars and layers), each a scoped patch with its
//! own seed, to audition A/B and keep or discard.
//!
//! A variation is a `rewrite_motif` edit of the selected layers in the selected bars, built with
//! a seed derived from (`seed`, variation index): the same arranger, in the layer's instrument,
//! register and dynamics, so the owner's taste rules hold. Every variation differs from the
//! original and from every other variation inside the selection, and is identical outside it
//! (`praxis_score::apply::apply` enforces the scope). Locked layers are never varied. With no bar
//! range (the whole score) each varied layer also gets a new `seed_offset`.

use crate::catalogue::Catalogue;
use crate::edit::build::{Built, build_patch};
use crate::edit::material::derive_seed;
use crate::edit::resolve::{Resolved, resolve};
use crate::edit::{
    EditError, EditIntent, EditIssue, EditIssueCode, EditParams, EditPlan, EditScope,
};
use praxis_score::patch::{LayerUpdate, PatchOp, Scope, ScorePatch};
use praxis_score::timeline::{NoteEvent, expand_notes};
use praxis_score::{BarRange, Score};

/// Distinct seeds tried per variation before giving up.
const TRIES: u64 = 16;

/// Sample rate used only to compare note events (any rate gives the same comparison).
const COMPARE_SR: u32 = 48_000;

/// One alternate of a selection.
#[derive(Clone, Debug, PartialEq)]
pub struct Variation {
    /// 0-based index in the set.
    pub index: usize,
    /// The variation's own seed (the arranger seed of its material; also the new `seed_offset`
    /// of each varied layer for whole-score variations).
    pub seed: u64,
    /// The scoped patch (`origin: model`) that turns the original into this variation.
    pub patch: ScorePatch,
    /// The patch that undoes it exactly.
    pub inverse: ScorePatch,
    /// The original with the patch applied.
    pub score: Score,
    /// What changed, in plain words.
    pub summary: String,
}

/// N variations of `scope` (bars and layers; empty layers means every unlocked layer that plays
/// in the bars) with the standard catalogue.
pub fn variations(
    score: &Score,
    scope: &Scope,
    n: usize,
    seed: u64,
) -> Result<Vec<Variation>, EditError> {
    variations_with(score, scope, n, seed, &Catalogue::standard())
}

/// [`variations`] with a given catalogue.
pub fn variations_with(
    score: &Score,
    scope: &Scope,
    n: usize,
    seed: u64,
    catalogue: &Catalogue,
) -> Result<Vec<Variation>, EditError> {
    let invalid = |issues: Vec<EditIssue>| EditError::Invalid {
        issues,
        rounds: vec![],
    };
    let plan = EditPlan {
        intent: EditIntent::RewriteMotif,
        scope: EditScope {
            bars: scope.bars,
            phrase: None,
            keep: None,
        },
        layers: scope.layers.clone(),
        params: EditParams::default(),
        notes: vec![],
    };
    let r = resolve(&plan, score, catalogue).map_err(invalid)?;
    if r.targets.is_empty() {
        return Err(invalid(vec![EditIssue::new(
            EditIssueCode::NoTarget,
            "/layers",
            format!(
                "every selected layer is locked ({}); unlock one to vary it",
                r.refused.join(", ")
            ),
        )]));
    }
    let bars = r.bars;
    let original = selection(score, &r.targets, bars);
    let mut seen: Vec<Vec<NoteEvent>> = vec![original];
    let mut out = Vec::with_capacity(n);
    for index in 0..n {
        let mut found = None;
        for k in 0..TRIES {
            let vseed = derive_seed(seed, &format!("variation{index}"), k);
            let built = build_patch(&r, score, catalogue, vseed).map_err(invalid)?;
            let patch = variation_patch(&r, built.patch.clone(), vseed, index);
            let applied = praxis_score::apply::apply(score, &patch).map_err(|e| {
                invalid(vec![EditIssue::new(
                    EditIssueCode::Rejected,
                    "",
                    format!("the variation patch was rejected: {e}"),
                )])
            })?;
            let sel = selection(&applied.score, &r.targets, bars);
            if !seen.contains(&sel) {
                seen.push(sel);
                found = Some((vseed, patch, applied, built));
                break;
            }
        }
        let Some((vseed, patch, applied, built)) = found else {
            return Err(invalid(vec![EditIssue::new(
                EditIssueCode::Material,
                "/layers",
                format!(
                    "found only {index} distinct variations of {} in {}; select more bars or \
                     another layer",
                    r.targets.join(", "),
                    crate::edit::resolve::bars_text(bars)
                ),
            )]));
        };
        out.push(Variation {
            index,
            seed: vseed,
            summary: summary(&r, &built, vseed),
            patch,
            inverse: applied.inverse,
            score: applied.score,
        });
    }
    Ok(out)
}

/// The patch of a variation: the rewrite, a new `seed_offset` per layer for whole-score
/// variations, and a description that names the seed.
fn variation_patch(r: &Resolved, mut patch: ScorePatch, vseed: u64, index: usize) -> ScorePatch {
    if r.whole {
        for id in &r.targets {
            patch.ops.push(PatchOp::UpdateLayer {
                id: id.clone(),
                update: LayerUpdate {
                    seed_offset: Some(derive_seed(vseed, id, 0)),
                    ..LayerUpdate::default()
                },
            });
        }
    }
    patch.description = format!(
        "variation {} (seed {vseed}): {}",
        index + 1,
        patch.description
    );
    patch
}

fn summary(r: &Resolved, built: &Built, vseed: u64) -> String {
    let mut s = format!("Seed {vseed}: {}", built.actions.join("; "));
    if !r.refused.is_empty() {
        s.push_str(&format!(
            ". Left {} alone (locked)",
            r.refused
                .iter()
                .map(|x| format!("`{x}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    s.push('.');
    s
}

/// The note events of the selected layers inside the selected bars.
fn selection(score: &Score, layers: &[String], bars: BarRange) -> Vec<NoteEvent> {
    let idx: Vec<usize> = layers
        .iter()
        .filter_map(|id| score.layers.iter().position(|l| &l.id == id))
        .collect();
    expand_notes(score, COMPARE_SR)
        .into_iter()
        .filter(|e| idx.contains(&e.layer) && e.bar >= bars.start && e.bar <= bars.end)
        .collect()
}
