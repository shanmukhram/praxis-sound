//! Audio-quality lints (warnings). OWNED BY WP-A (score-ops). Signature is a FROZEN CONTRACT.
//! These encode the owner's taste rules: rhythm first, notes connect, restraint.
//!
//! | Code | Owner rule | Fires when (thresholds documented next to each constant) |
//! | --- | --- | --- |
//! | `isolated_notes` | 1 rhythm first | >= 25 % of note-to-note transitions on a pitched layer leave more than one beat of silence |
//! | `no_motif_repetition` | 1 rhythm first | a layer with >= 2 sounding bars never repeats a bar's onset rhythm |
//! | `missing_sustain_overlap` | 2 notes connect | a layer in `sustaining_layers` has no note longer than one step |
//! | `too_many_elements` | 4 restraint | more than 4 layers sound at once in a section, or more than one emotion |
//! | `unrequested_effect_class` | 4 restraint | a riser/sweep/impact/boom/pad/whoosh layer the brief did not ask for |
//!
//! All lints are warnings: the planner's repair loop acts on them, the validator does not reject.
//! Output order is deterministic (lint by lint, layers in score order).

mod restraint;
mod rhythm;

use crate::types::{GeneratorClass, Score};
use crate::validate::{Issue, IssueCode};

/// Facts about presets the lints need, supplied by the caller (praxis-instruments knows them).
#[derive(Clone, Debug, Default)]
pub struct LintContext {
    /// Layer ids whose instrument rings (piano, mallets, plucks): notes should overlap.
    pub sustaining_layers: Vec<String>,
    /// Generator classes the brief explicitly asked for (others in the effect family are flagged).
    pub requested_classes: Vec<crate::types::GeneratorClass>,
}

pub fn lint(score: &Score, ctx: &LintContext) -> Vec<Issue> {
    let mut out = Vec::new();
    rhythm::isolated_notes(score, &mut out);
    rhythm::no_motif_repetition(score, &mut out);
    missing_sustain_overlap(score, ctx, &mut out);
    restraint::too_many_elements(score, &mut out);
    restraint::unrequested_effect_class(score, ctx, &mut out);
    out
}

/// Sound-effect classes: one-off gestures, not motifs, so the rhythm lints skip them.
pub(crate) fn sfx_class(class: GeneratorClass) -> bool {
    use GeneratorClass::*;
    matches!(
        class,
        Riser | Sweep | Impact | Boom | Whoosh | Glitch | Sfx | Fill
    )
}

fn warn(out: &mut Vec<Issue>, code: IssueCode, path: String, message: String) {
    out.push(Issue {
        code,
        path,
        message,
    });
}

/// Note length (in steps) a note must exceed to ring into the next step (owner rule 2: sustain
/// overlaps wherever the instrument would ring, i.e. `length > 1.0`).
const OVERLAP_LENGTH: f32 = 1.0;

/// `missing_sustain_overlap`: a sustaining layer (from `ctx.sustaining_layers`) whose notes all
/// have `length <= 1.0` steps, so no note ever rings into the next. Layers without notes and ids
/// that name no layer are ignored.
fn missing_sustain_overlap(score: &Score, ctx: &LintContext, out: &mut Vec<Issue>) {
    for (i, l) in score.layers.iter().enumerate() {
        if !ctx.sustaining_layers.iter().any(|id| id == &l.id) {
            continue;
        }
        let mut lengths = l
            .clips
            .iter()
            .filter_map(|c| score.pattern(&c.pattern))
            .flat_map(|p| p.notes.iter().map(|n| n.length))
            .peekable();
        if lengths.peek().is_none() {
            continue;
        }
        let longest = lengths.fold(0.0f32, f32::max);
        if longest <= OVERLAP_LENGTH {
            warn(
                out,
                IssueCode::MissingSustainOverlap,
                format!("/layers/{i}"),
                format!(
                    "layer `{}` rings but no note is longer than one step (longest {longest}); \
                     give notes length > 1.0 so each sustains into the next",
                    l.id
                ),
            );
        }
    }
}
