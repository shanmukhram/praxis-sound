# ADR-0007: Edit isolation, frozen master gain and loop wrap

Status: accepted

Loudness normalisation measures the whole render, so editing bars 13-16 would change the gain of
bars 1-12 and break "every other bar sample-identical". Decision: after the first normalisation
the applied gain is stored in `master.frozen_gain_db`; renders reuse it until the user normalises
again (an explicit patch). Edit isolation is then defined precisely in docs/contracts/patch.md:
stems of other layers are bit-identical; the edited layer and the master are bit-identical
outside the edited bars plus the tail window ("effect tails aside" in the PRD).

Per-note randomness is seeded from each note's stable key (layer, bar, step, pitch), never from a
running stream, so an edit cannot shift random draws elsewhere.

Loop wrap renders warm-up cycles and captures a steady-state cycle (docs/contracts/engine.md),
which wraps instrument releases, reverb tails and compressor/limiter state across the seam.
