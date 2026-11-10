//! Patch application. OWNED BY WP-A (score-ops). Signature is a FROZEN CONTRACT.
//! Rules (docs/contracts/patch.md): reject ops on locked layers, reject out-of-scope ops,
//! copy-on-write bar edits, validate the result, and return an exact inverse.

use crate::patch::{Applied, PatchError, ScorePatch};
use crate::types::Score;

/// Applies `patch` to `score`. On success the result validates and `inverse` restores `score`
/// exactly (`apply(apply(s, p).score, inverse).score == s`).
pub fn apply(score: &Score, patch: &ScorePatch) -> Result<Applied, PatchError> {
    let _ = (score, patch);
    Err(PatchError::Invalid(
        "apply: not implemented yet (WP-A)".into(),
    ))
}
