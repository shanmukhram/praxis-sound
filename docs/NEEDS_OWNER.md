# Needs the owner

Only things the owner can give. Work continues on everything they do not block.

| # | Item | Blocks | Status |
| --- | --- | --- | --- |
| 1 | **Anthropic API key** for the Claude planner and the assistant agent. | Live Claude planning, the assistant's live run | **done 2026-11-11**: stored by the owner with `praxis key set` (Windows Credential Manager, service `praxis-sound`) |
| 2 | **Listening sign-off, Phase 2**: `demos/phase-2/acceptance1_mixed_loop.flac` (+ x3 seam file). | Phase 2 exit | **done 2026-11-11**: owner listened and said "its perfect" |
| 3 | **Real-hardware check, Phase 3**: open the release app yourself and play; checklist in `docs/phase-3-hardware-check.md`. Nothing plays unless you press Play. | Phase 3 sign-off | **ready** |
| 4 | **Labelled reference clips** you own with known tempo/key (optional; we self-make the test set). | Nothing | optional |

## Decided by the owner (2026-11-10)

- Product name: Praxis Sound.
- Licence: closed for now; owner decides later. Repo stays private; dependencies stay permissive.
- Trademark check: later, not needed now.
- Buy nothing. Everything is made through code: synthesis, procedural effects, and free CC0/CC-BY
  packs and impulse responses (approved, under 100 MB each). No paid packs.
- MPL-2.0 (symphonia): not adopted; keep permissive decoders plus optional ffmpeg (ADR-0004).
- Platform: Windows is the release target for now. CI still builds and tests macOS and Linux to
  keep the code portable, but only Windows gates releases.
- Git identity, commit rules and private GitHub repo `shanmukhram/praxis-sound`.
- CI only when it matters (2026-11-11): the private repo's free Actions minutes ran out and every
  job was blocked by billing. Pushes that change code run Windows only (release tests); docs-only
  pushes run nothing; the full Linux/Windows/macOS + wasm check runs by hand at phase exits and
  before a release (`gh workflow run ci --ref build/integration`). This replaces "every push tests
  macOS and Linux". Buy nothing still holds: no spending limit raise.
