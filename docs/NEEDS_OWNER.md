# Needs the owner

Only things the owner can give. Work continues on everything they do not block.

| # | Item | Blocks | Status |
| --- | --- | --- | --- |
| 1 | **Anthropic API key** for the Claude planner and the assistant agent. | Live Claude planning, the assistant's live run | **done 2026-11-11**: stored by the owner with `praxis key set` (Windows Credential Manager, service `praxis-sound`) |
| 2 | **Listening sign-off, Phase 2**: `demos/phase-2/acceptance1_mixed_loop.flac` (+ x3 seam file). | Phase 2 exit | **done 2026-11-11**: owner listened and said "its perfect" |
| 3 | **Real-hardware check, Phase 3** (`docs/phase-3-hardware-check.md`). | Phase 3 sign-off | **done 2026-11-11**: owner built a sound by hand in the release app and tested play, loop, mute/solo, swaps, seek, loop range and export on their speakers: "everything is good" |
| 4 | **Labelled reference clips** you own with known tempo/key (optional; we self-make the test set). | Nothing | optional |
| 5 | **Licence**: decided 2026-11-12, closed-source commercial with a one-time purchase (ADR-0012). The seven MPL-2.0 crates via Tauri are fine for that (unmodified, source available). Still the owner's, outside the code: an EULA, the store/payment provider, the trademark check. | Nothing in the code | **decided** |
| 6 | **Listening review, Phase 7**: `demos/phase-7/cue_hits_2min.flac` (2-minute cue with three hits). | Phase 7 exit | **done 2026-11-12**: "Sounds right" |
| 7 | **Blind listening review, Phase 6**: the 10 pairs in `demos/phase-6/`. | Phase 6 exit | **done 2026-11-12**: "i like them all sounds good" (sound quality judged; the match itself is measured) |

## Decided by the owner (2026-11-10)

- Product name: Praxis Sound.
- Licence: closed source, sold for a one-time payment (decided 2026-11-12, ADR-0012). Repo stays private; dependencies must allow closed-source commercial distribution.
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
- Sounds live in `C:\Users\admin\Praxis Sound` by default (2026-11-11), not Documents: Windows'
  Controlled Folder Access (ransomware protection, on for this PC) blocked the unsigned app from
  writing there and it closed at start. If its folder cannot be created, the app now falls back to
  `%LOCALAPPDATA%\Praxis Sound` instead of closing.
- Autonomy (2026-11-11): "from now on dont wait to ask me just go ahed and finish the project."
  The orchestrator decides routine engineering and product choices and keeps packages running to
  the end of the PRD; it asks only for PRD sign-offs (Phase 6 blind listening review, Phase 7
  listening review), large paid spend, and anything that plays audio on the owner's speakers.
- Downloads approved (2026-11-11): "you have authority to download i approve". Build tools such
  as wasm-bindgen-cli and Tauri's NSIS bundler are installed from official sources without asking
  and recorded in THIRD_PARTY.md.
