# Needs the owner

Only things the owner can give. Work continues on everything they do not block.

| # | Item | Blocks | Status |
| --- | --- | --- | --- |
| 1 | **Anthropic API key** for the Claude planner. Owner will provide it when asked; ask only when live Claude planning is the next step. The offline rule planner covers CI and acceptance test 1 meanwhile. | Live Claude planning and its prompt-suite run | ask when needed |
| 2 | **Listening sign-off, Phase 2**: listen to `demos/phase-2/acceptance1_mixed_loop.flac` (and the x3 file for the seam); the README says what to listen for. | Phase 2 exit (not the next phases) | **ready to listen** |
| 3 | **Real-hardware check** of real-time playback on Windows at Phase 3 exit. | Phase 3 sign-off | later |
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
