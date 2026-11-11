# Brief: WP-P2b tone and space edits ("muffled piano", "more room")

Branch from `build/integration`. Read first: `docs/BRIEFING.md` (all rules),
`docs/contracts/planner.md` (sections "Edits" and "Patch rules"), `docs/contracts/patch.md`,
`docs/contracts/score.md`, `crates/praxis-score/src/patch.rs` and `types.rs` (`UpdateLayer`,
`Effect`, `Automation`), the edit code in `crates/praxis-planner/src/edit.rs` and `edit/`
(`rules.rs`, `plan.rs`, `resolve.rs`, `build.rs`), the felt-piano voicing doc at the top of
`crates/praxis-instruments/src/sampler/presets.rs` (`tone`, `felt`, `release_s`), and the Feel
sliders in `app/src/ui/views/overview.ts` (`MACROS`: the exact phrases the app sends).

## The bug (found 2026-11-11)

With the offline planner, "make the piano muffled" and "make it a little darker" both answer
"Gave `piano` a new motif throughout" and rewrite the melody. The owner wanted the same notes with
a muffled sound. The engine can do it (felt piano `tone` / `felt` parameters; filter, EQ and
reverb effects; automation over bars), but `EditPlan` has no tone or space intent, and anything
the rule reader does not recognise falls through to `rewrite_motif`. The app's Brightness and
Space sliders send exactly such phrases, so they misfire too.

## You own

`crates/praxis-planner/src/edit.rs`, `crates/praxis-planner/src/edit/**`,
`crates/praxis-planner/src/catalogue.rs` (only to describe each preset's tone parameters),
`crates/praxis-planner/tests/edit_*.rs` and `tests/prompts/edit_*`, and the "Edits" section of
`docs/contracts/planner.md` (update the intent table, params, patch rules and offline reader
order to match what you build). Nothing else. If a patch op you need does not exist, stop and
say so with the exact proposed change; `patch.rs` and `types.rs` are frozen.

## Build

1. **`change_tone`** intent. Words: muffled, muted, darker, warmer, duller, softer tone, mellow,
   brighter, clearer, crisper, more air (with "a little" / "much" amounts as for dynamics). On a
   preset with tone parameters (felt piano `tone`, `felt`; check the catalogue for others) it
   changes those through `update_layer`; otherwise it adds or adjusts a low-pass filter (or a
   high shelf) effect on the layer. Notes never change. Bar-scoped requests ("muffle the piano in
   the last four bars") automate a filter over exactly those bars, so other bars stay
   bit-identical outside the tail window (contracts/patch.md).
2. **`change_space`** intent. Words: drier, less reverb, closer, more room, bigger, hall,
   wetter, more space. Adjusts the layer's reverb (`mix`, `decay_s`, `size`) or adds the
   arranger's room reverb when there is none. "Dry" can remove it. Whole-score by default.
3. **No silent rewrites.** `rewrite_motif` only when the instruction asks for new notes
   ("change the melody", "different rhythm", "new motif", "vary it"). An instruction the rule
   reader does not recognise returns a clear "not understood" outcome with no patch and a
   plain-language message listing what it can do; it never rewrites notes.
4. **The Feel sliders.** Every phrase in `MACROS` (Mood, Brightness, Space, Energy; down and up)
   maps to a sensible intent and is tested: Brightness to `change_tone`, Space to `change_space`,
   Energy and Mood to the existing intents (document the choice). The Claude provider's edit tool
   schema and system prompt gain the new intents too (replay fixtures for tests).
5. Fix found by WP-K2: in `resolve.rs` `matches_layer`, a reference equal to an existing layer
   id selects only that layer (today `piano` also matches the `upright_piano` layer).

## Done when

- Tests: "make the piano muffled" leaves every note event identical, lowers the rendered
  spectral centroid of the piano stem measurably (render with `praxis-engine`, a dev-dependency
  already), and its inverse restores the score exactly; "brighter" raises it; "darker in the last
  four bars" keeps bars 1-12 of the master bit-identical outside the tail window; "give it more
  room" raises the reverb mix and changes no notes; an unknown instruction returns "not
  understood" with an empty patch; all eight slider phrases produce the documented intents; the
  `matches_layer` fix has a test. The existing edit prompt suite still passes (update cases whose
  expected intent was the old fallback, and say which).
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p praxis-planner --release`, `cargo test -p praxis-cli --release --test takes`,
  and the wasm build pass. Set `PRAXIS_PACKS` to `C:\Users\admin\Desktop\praxis sound\packs`.
- Report as in docs/BRIEFING.md, with the before/after answer for "make the piano muffled".
