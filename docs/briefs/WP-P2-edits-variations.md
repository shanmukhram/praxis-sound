# Brief: WP-P2 edits + variations

Branch: `wp/p2-edits`. Read first: `docs/BRIEFING.md` (binding), `PRD.md` (FR-6, FR-7, FR-8 and
the Phase 4 exit), `docs/PLAN.md` (WP-P2), `docs/contracts/planner.md`, `docs/contracts/patch.md`,
merged `crates/praxis-planner/src/**` (your package's base) and `crates/praxis-score/src/apply.rs`.

## Goal and why it matters
"Keep the first half, change the last four bars to add a subtle melody" must change only those
bars (FR-7); a locked layer cannot be altered by any later generation or edit (FR-6); several
alternates of any selection, each with its own seed, auditioned A/B and kept or discarded (FR-8).
Phase 4 exits when "change the last four bars" leaves bars 1-12 sample-identical.

## You own
`crates/praxis-planner/src/edit.rs`, `src/variations.rs` (+ dirs), additions to the plan schema
needed for edit plans (new types in new files under `src/edit/`), tests
`crates/praxis-planner/tests/edit_*.rs`, `tests/variations_*.rs`, prompt cases
`tests/prompts/edit_*.json`.

## Build
- `EditPlan` schema (forced tool call): scope (bars by number or phrase resolved by the planner:
  "last four bars", "second half", "bars 9-12"), target layers (by id/name/role; never locked
  ones), intent (add_layer, rewrite_motif, change_instrument, change_dynamics, remove_layer,
  add_melody, thin_out) and parameters. RuleProvider support for common phrasings; Claude provider
  via the same forced-tool mechanism with replay fixtures.
- `plan_edit(project_or_score, instruction, provider, catalogue, seed) -> EditOutcome { patch,
  score, scope, summary }`: build a `ScorePatch` with exact scope and `origin: model`, apply with
  `praxis_score::apply::apply`; a rejected patch goes back through repair (max 3) and never
  silently widens scope.
- `variations(score, scope, n, seed) -> Vec<Variation { seed, patch, score }>`: N alternates of
  the selected bars/layers (new seed_offset and arranger seed), each a scoped patch.
- Default lock policy: after a generation, the planner may suggest locks; edits never touch
  locked layers (and say so in the summary when the instruction names one).

## Tests
Prompt suite cases for edits; isolation: for "change the last four bars to add a subtle melody"
on the acceptance-1 Score, every layer's expanded events in bars 1-12 are identical before/after,
and (if the engine with the test factory is available as a dev-dependency through
`praxis-engine`'s `testing` module) the rendered stems are bit-identical over bars 1-12 outside
the documented tail window; locked layer edit attempts are refused; variations differ from each
other and from the original only inside scope.

## Report
As in docs/BRIEFING.md "Report back".
