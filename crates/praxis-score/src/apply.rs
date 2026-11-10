//! Patch application. OWNED BY WP-A (score-ops). Signature is a FROZEN CONTRACT.
//! Rules (docs/contracts/patch.md): reject ops on locked layers, reject out-of-scope ops,
//! copy-on-write bar edits, validate the result, and return an exact inverse.
//!
//! How the seven rules are kept:
//!
//! 1. **Atomic.** Ops run on a clone of the score; the first error discards the clone. The result
//!    must pass [`validate::validate`], otherwise [`PatchError::Invalid`].
//! 2. **Locks.** Every op that changes a layer checks `locked` first ([`PatchError::LockedLayer`]).
//!    Ops that change a pattern (`replace_pattern`, `set_key` with transposition) fail when a
//!    locked layer plays it. `set_lock {locked: false}` needs `origin: user`.
//! 3. **Scope.** Global ops need an unscoped patch (no `scope.bars` and no `scope.layers`). Bar ops
//!    must lie inside `scope.bars`; layer ops must name a layer of `scope.layers` when that list is
//!    non-empty. Nothing is clipped: an op that reaches outside fails with `OutOfScope`. With
//!    `scope.bars` set, ops that would change every bar of a layer (`update_layer` other than a
//!    rename, `add_layer`/`remove_layer` with clips outside the bars) are out of scope too.
//! 4. **Copy-on-write.** Bar ops split clips at the range edges and never mutate a pattern; the
//!    right-hand piece of a split gets `pattern_offset_steps` so it keeps its phase. Clips are then
//!    sorted by start and merged across the two range edges when the merge is exact.
//! 5. **Exact inverse.** Each op records the ops that undo it; the inverse is those groups in
//!    reverse order. Bar ops are undone with `set_clips` (+ `remove_pattern` for a pattern they
//!    added), so the original clip list comes back exactly. A removal is undone by an `add_*` op
//!    with `index` set to the removed item's position.
//! 6. **Isolation.** After all ops, a final check compares the note events of every layer outside
//!    the bar scope and every layer outside the layer scope (see `apply/isolation.rs`).
//! 7. **Deterministic ids.** Patterns created by `write_bars` whose id is missing or already taken
//!    by a different pattern get `<layer>_b<start>-<end>`, then `_2`, `_3`... (`apply/ids.rs`).
//!
//! The inverse patch is unscoped, keeps the forward origin, and is marked `origin: user` when it
//! has to unlock a layer (undo is a user action).

mod clips;
mod ids;
mod isolation;
mod ops;

use crate::patch::{Applied, Origin, PatchError, PatchOp, Scope, ScorePatch};
use crate::types::Score;
use crate::validate::{self, Issue};

/// Applies `patch` to `score`. On success the result validates and `inverse` restores `score`
/// exactly (`apply(apply(s, p).score, inverse).score == s`).
pub fn apply(score: &Score, patch: &ScorePatch) -> Result<Applied, PatchError> {
    let mut s = score.clone();
    let mut groups: Vec<Vec<PatchOp>> = Vec::with_capacity(patch.ops.len());
    for (index, op) in patch.ops.iter().enumerate() {
        let cx = ops::Cx { patch, index, op };
        groups.push(ops::apply_op(&mut s, &cx)?);
    }
    isolation::check_patch(score, &s, patch)?;
    let report = validate::validate(&s);
    if !report.is_valid() {
        return Err(PatchError::Invalid(describe(&report.errors)));
    }
    let ops: Vec<PatchOp> = groups.into_iter().rev().flatten().collect();
    let unlocks = ops
        .iter()
        .any(|o| matches!(o, PatchOp::SetLock { locked: false, .. }));
    let description = if patch.description.is_empty() {
        "undo".to_string()
    } else {
        format!("undo: {}", patch.description)
    };
    let inverse = ScorePatch {
        description,
        origin: if unlocks { Origin::User } else { patch.origin },
        scope: Scope::default(),
        ops,
    };
    Ok(Applied { score: s, inverse })
}

/// Short human-readable summary of validation errors for [`PatchError::Invalid`].
fn describe(errors: &[Issue]) -> String {
    const SHOWN: usize = 5;
    let mut parts: Vec<String> = errors
        .iter()
        .take(SHOWN)
        .map(|i| format!("{:?} at {}: {}", i.code, i.path, i.message))
        .collect();
    if errors.len() > SHOWN {
        parts.push(format!("and {} more", errors.len() - SHOWN));
    }
    parts.join("; ")
}
