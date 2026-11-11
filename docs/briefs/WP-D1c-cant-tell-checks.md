# Brief: WP-D1c checks say "can't tell" instead of failing on unclear material

Branch from `build/integration`. Read first: `docs/BRIEFING.md` (all rules),
`docs/contracts/sound-profile.md` (render verification), `crates/praxis-analyze/src/verify.rs`,
`crates/praxis-analyze/src/features/{tempo,key}.rs` (how confidences are computed), the take
records in `docs/contracts/cli.md` ("Takes"), `crates/praxis-cli/src/ops/{check,render,takes}.rs`,
the agent's use of checks in `crates/praxis-cli/src/agent/` and `docs/agents/praxis-skill/`, and
the app's wording in `app/src/ui/checks.ts` and `app/src/engine/EngineAdapter.ts` (`VerifyCheck`).

## Why (owner, 2026-11-11)

The owner built a sound by hand: an A-minor, 128 BPM loop whose piano uses only A, B, D and E (no
third) and an uneven rhythm. The take said "2 of 8 checks failed": tempo 85.34 BPM (confidence
0.42; exactly 2/3 of 128) and key "a major" (confidence 0.58). The render was exactly right (100%
of notes within 10 ms of the grid). The detectors could not tell, and the report called that a
failure. The owner chose: unsure checks say "can't tell", not "failed".

## Decision (orchestrator, contract change approved)

- `Check` gains one field, `inconclusive: bool` (serde default false, always serialised). A check
  is inconclusive when the material cannot decide it: it keeps `passed: false` and gets
  `inconclusive: true` (honest for readers that ignore the new field), and `VerifyReport.passed`,
  the CLI's `failed[]` and every "N of M passed" count only decided checks; inconclusive ones are
  listed separately (`inconclusive[]` in the CLI payload). Document it in
  `docs/contracts/sound-profile.md`.
- **Key**: inconclusive when the score's sounding pitch classes cannot tell the mode family (for
  minor vs major: no minor or major third over the tonic, and no sixth that decides it), or when
  the detection confidence is below a threshold you calibrate; the tonic is still checked and a
  wrong tonic with good confidence still fails. Use the score (it is the check's input) as well as
  the audio.
- **Tempo**: inconclusive when the detection confidence is low (calibrate), or when the detected
  tempo is a simple ratio of the score's (1/2, 2/3, 3/4, 4/3, 3/2, 2) with low confidence while the
  grid check passes. A confident wrong tempo still fails.
- Thresholds: calibrate on the existing synthetic tests and the real renders; acceptance test 1
  (tempo confidence 0.78, key 0.87) must stay decided and passing. Document the thresholds.
- Other checks (grid, loudness, true peak, clipping, seam, silence) are never inconclusive.

## You own

`crates/praxis-analyze/src/verify.rs` (the `Check` field above plus the body),
`crates/praxis-analyze/tests/verify_*.rs`, `docs/contracts/sound-profile.md` (the verification
section), the check-summary code in `crates/praxis-cli/src/ops/{check,render,takes}.rs` and
`crates/praxis-cli/src/agent/` (so the agent reads "can't tell" correctly and does not try to
"fix" it), `docs/agents/**` (one line on what inconclusive means), `app/src/ui/checks.ts`,
`app/src/engine/EngineAdapter.ts` (`VerifyCheck.inconclusive?`), `app/src/engine/MockEngine.ts`
and `app/src/engine/mock/**` if the mock builds checks, and the matching tests. Do not edit
`crates/praxis-cli/src/ops/sound.rs` or `crates/praxis-app/**`: the orchestrator has uncommitted
changes there.

## Wording

The take's line: "6 of 6 checks passed · 2 can't tell from these notes: tempo (the rhythm doesn't
clearly show 128 BPM), key (no third, so A minor vs A major can't be heard)". Plain words, as the
rest of the app (ADR-0011). The CLI human output says the same; JSON gains the field.

## Done when

- Tests: the owner's sound (recreate the score from the numbers above: pitches 57, 74, 64, 71 on an
  eighth grid as in the brief's description, soft kick on steps 0, 3, 4, 5 in bars 9-16) renders
  and checks as 6 decided passes plus tempo and key inconclusive, `passed` true; a score with a
  clear third in the wrong mode still fails key; a confident wrong tempo still fails; acceptance
  1 unchanged (8 of 8 decided, passing); the app shows "can't tell" (Vitest).
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test -p praxis-analyze -p praxis-cli --release` (keychain off: `PRAXIS_KEYCHAIN=off`),
  the wasm build, `npx tsc --noEmit`, `npx vitest run` pass. Never call the Claude API and never
  play audio.
- Report as in docs/BRIEFING.md with the calibrated thresholds and the owner's case before/after.
