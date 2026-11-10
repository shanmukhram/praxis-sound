//! Audio-quality lints (warnings). OWNED BY WP-A (score-ops). Signature is a FROZEN CONTRACT.
//! These encode the owner's taste rules: rhythm first, notes connect, restraint.

use crate::types::Score;
use crate::validate::Issue;

/// Facts about presets the lints need, supplied by the caller (praxis-instruments knows them).
#[derive(Clone, Debug, Default)]
pub struct LintContext {
    /// Layer ids whose instrument rings (piano, mallets, plucks): notes should overlap.
    pub sustaining_layers: Vec<String>,
    /// Generator classes the brief explicitly asked for (others in the effect family are flagged).
    pub requested_classes: Vec<crate::types::GeneratorClass>,
}

pub fn lint(score: &Score, ctx: &LintContext) -> Vec<Issue> {
    let _ = (score, ctx);
    Vec::new()
}
