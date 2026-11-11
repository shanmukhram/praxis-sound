# Brief: WP-P4 live planner quality (closes Phase 4)

Branch from `build/integration`. Read first: `docs/BRIEFING.md` (all rules, especially the
owner's audio-quality rules: rhythm first, notes connect, layering, restraint), PRD Phase 4 exit,
`docs/contracts/planner.md`, `docs/agents/live-run-results.md` ("Second live run": the exact
failures), the planner in `crates/praxis-planner/src/` (`brief.rs`, `repair.rs`, `plan.rs`,
`claude.rs`, `edit.rs`, `edit/`), and the suites `tests/prompt_suite.rs`, `tests/edit_prompt_suite.rs`
and their live mode (`PRAXIS_SUITE_PROVIDER=claude`).

## Why

Through the real Claude planner (`ClaudeProvider`, `claude-opus-5-5`) the brief suite passed
15/27 (every acceptance-1 phrasing passed) and the edit suite 19/20. Phase 4's exit asks the
prompt suite to pass. The failures:

1. **Unrequested layers** (restraint): `ban_list` (glass + soft keys), `unmapped_strings` (piano +
   keys + bass), `requested_riser` (an extra sub bass).
2. **Isolated notes**: model-written bass/glass parts leave gaps over a beat (`isolated_notes` lint
   in `duration_60s_at_90`, `layering_three_stages`, `trap_cinematic_mysterious`). Lint warnings
   are not part of the model's repair loop.
3. **Layers not playing throughout** when the brief gives no bars: `explicit_chords` (sub bass only
   5-8), `six_eight_with_kick` (kick from bar 3).
4. **Expectations too strict for a model**: emotion synonyms (`trap_epic_trailer` ominous vs dark,
   `trap_suspense_bells` suspenseful vs tense), a spoken rhythm padded with trailing rests
   (`spoken_bell_dorian`, `spoken_inline_sixteenths`), `edit_not_understood` ("make it pop" read
   as brighter).

## Build

- **Lint in the repair loop.** For model providers, quality lints on the arranged Score
  (`isolated_notes`, `too_many_elements`, `missing_sustain_overlap`, `unrequested_effect_class`,
  `no_motif_repetition`) go back to the provider as issues in a repair round, with plan pointers
  and a plain instruction (for example "keep the bass on a repeating grid and let sustain connect
  the notes"), within the existing round limit; then deterministic fixes as today. The offline
  provider's behaviour must not change (its suite stays exactly as it is).
- **The system prompt** (`claude.rs`, and the edit prompt) states the restraint rules plainly:
  only the instruments the brief names (an instrument with no preset is noted in `notes`, not
  replaced by several), no unrequested layers, every layer plays from bar 1 to the end unless the
  brief says when it enters or leaves, at most the layers asked for, sustain that connects notes on
  sustaining presets. Add a validator issue (or a repair) for an unrequested extra layer where the
  brief is explicit ("a glass motif ... no drums, no bass").
- **Fair expectations, without weakening what matters.** In the suites' checks: accept an
  emotion from a documented synonym set per expected emotion; compare spoken rhythms after
  normalising trailing rests and repetition to the motif length; for `edit_not_understood`,
  either accept a clearly scoped, note-preserving tone change for "make it pop" or change the case
  to an instruction that has no reasonable reading, and say which and why. Layer counts, presets,
  bars, keys, tempo, grid, banned classes and isolation stay exactly as strict.
- Do not change frozen contracts (`provider.rs`, score types). Update the planner contract's
  repair-loop and prompt sections to match what you build.

## You own

`crates/praxis-planner/**` (src and tests, including `tests/prompts/**`) and the planner sections
of `docs/contracts/planner.md`. Nothing else.

## Live runs (paid; the owner approved about US$1-3 for this package)

- Develop offline: replay fixtures and the rule provider. Add replay tests for the new repair
  round (a recorded model answer with an isolated-notes bass, then the repaired answer).
- You may run the live suites (`PRAXIS_SUITE_PROVIDER=claude`, key from the keychain) **at most
  three times in total**, and keep total live spend under **US$4**. Log each live call's token
  usage (from the API response) in live mode so the cost is measured, and report it.
- Every other test or binary run sets `PRAXIS_KEYCHAIN=off`. Never play audio.
- Build limits: 5 jobs / 5 test threads via `.claude/worktrees/.cargo/config.toml`; never override.

## Done when

- The live brief suite passes 27/27 and the live edit suite 20/20 (or every remaining failure is
  explained as a genuine ambiguity in the brief, with the case rewritten to be unambiguous and the
  reason in your report). The offline suites pass unchanged.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p praxis-planner --release` (offline), `cargo test -p praxis-cli --release --test
  acceptance1`, and the wasm build pass.
- Report as in docs/BRIEFING.md: before/after per failing case, the live cost per run, and the
  final live results.
