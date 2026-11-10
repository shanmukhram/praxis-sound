# Contract: Score patches, undo and projects

Code: `crates/praxis-score/src/patch.rs` (types, frozen), `apply.rs` (WP-A), `project.rs`
(types frozen, logic WP-A). Schema: `schema/patch.schema.json`.

Every change to a Score, by a person or the model, is a `ScorePatch`: a list of ops plus a
`scope` (bars and layers it may touch) and an `origin` (user, model, system).

## Rules `apply` must keep (each has a test)

1. **Atomic.** All ops apply or none do. The result must pass `validate`; otherwise `Invalid`.
2. **Locks.** Any op that changes a locked layer, or a pattern used by a locked layer, fails with
   `LockedLayer`. `set_lock {locked: false}` succeeds only for `origin: user`.
3. **Scope.** If `scope.bars` is set, global ops (`PatchOp::is_global`) fail with `OutOfScope`, and
   bar ops (`write_bars`, `clear_bars`, `place_clip`) must lie inside it. If `scope.layers` is
   non-empty, layer ops must name one of them. Ops are rejected, never silently clipped.
4. **Copy-on-write.** `write_bars`/`clear_bars`/`place_clip` split the layer's clips at the range
   edges. Bars outside the range keep their clips and patterns; a pattern shared with other clips
   is never mutated in place. `replace_pattern` fails if any out-of-scope or locked clip uses it.
5. **Exact inverse.** `apply(apply(s, p).score, inverse) == s` (structural equality), for every
   valid patch. Property-tested with generated scores and patches.
6. **Isolation.** For any patch scoped to bars B and layers L: every layer not in L is
   structurally identical, and every bar outside B of a layer in L expands (via
   `timeline::expand_notes`) to exactly the same note events. Property-tested.
7. **Deterministic ids.** New pattern ids created by splitting are derived from
   (layer id, bar range), e.g. `piano_b13-16`, with a numeric suffix on collision.

## Audio-level isolation (engine, tested in WP-F)

Rendering a patched score scoped to bars B of layer X:
- every other layer's stem is bit-identical over the whole render;
- layer X's stem is bit-identical outside `[start(B), end(B) + tail(X))`, where `tail(X)` is the
  instrument release plus the layer's effect tail;
- with `master.frozen_gain_db` set, the master is bit-identical outside the same window, widened
  by the master chain's tail and limiter lookahead/release.

"Effect tails aside" in the PRD means exactly that window.

## Projects (`.praxis`, FR-31)

`Project { format: "praxis-project", version: 1, score, undo, redo, next_seq, versions, references }`.
- `commit(patch)` applies, pushes `{seq, patch, inverse}` onto `undo`, clears `redo`.
- `undo()` applies the top inverse and moves the entry to `redo`; `redo()` reverses that.
- `save_version(name)` snapshots the score; `restore_version(name)` is itself a patch (undoable).
- References store profile paths or library names, never audio (FR-18).
