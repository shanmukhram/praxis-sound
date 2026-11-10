# ADR-0006: Two-level planner (model writes a plan, an arranger writes the Score)

Status: accepted

The PRD says the model returns a Score or a patch through a schema-constrained call. Asking a
model to emit every note of a 16-bar Score is slow, error-prone (off-scale notes, off-grid timing,
inconsistent repeats) and wastes the rhythm-first, layering rules the owner cares about.

Decision: the model fills a smaller **ScorePlan** (key, tempo/duration, sections, layers, motifs as
spoken rhythm + scale degrees, chords, avoid-list). A deterministic **arranger** expands the plan
into a Score, enforcing the audio-quality rules by construction (grid, overlap, repetition,
layering, restraint). The model can still write explicit notes for a motif; they are validated and
repaired like everything else. This is still "the model writes Scores, never audio": the plan is a
compact Score description, and edits are still Score patches.

Consequence: an offline rule-based provider can produce the same ScorePlan from simple briefs,
so CI can run acceptance test 1 without network access or an API key. Whether a local model is
needed as an offline fallback (PRD open question) is deferred; the rule provider covers CI.
