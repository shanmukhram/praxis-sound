# Status

Updated after every merge. Newest first. Branch: `build/integration` (pushed to `origin`).

## Current phase: 1 (Phase 0 exit passed)

Wave 1 (WP-A, B1, B2, C1, D1, E, F, P) runs as background agents in worktrees. From wave 2 on,
work packages run as visible sessions (owner's choice), briefed from `docs/briefs/`. WP-X (MCP
server for agents, ADR-0010) is planned after WP-K, P, D1, F2.

| WP | Status | Branch | Notes |
| --- | --- | --- | --- |
| WP-0 foundations | **done**: CI green on Windows, macOS, Linux and wasm32 (run 42868034466) | build/integration | empty score renders a silent 12 s WAV |
| WP-A score-ops | next | wp/a-score-ops | |
| WP-B1 dsp-core | next | wp/b1-dsp-core | |
| WP-B2 dsp-effects | next | wp/b2-dsp-effects | |
| WP-C1 sampler | next | wp/c1-sampler | |
| WP-D1 analyze-verify | next | wp/d1-analyze-verify | |
| WP-E audio-io | **merged** (fbdb8d1) | wp/e-audio-io | WAV/FLAC/OGG/MP3 native, M4A/MP4/MOV/WebM via ffmpeg (hardened), FLAC + stems export, MIDI export; 47 tests. Gaps: no ffmpeg timeout; resampler ~0.5 sample offset |
| WP-F engine-core | next | wp/f-engine-core | |
| WP-P planner | next | wp/p-planner | |

## What works now

- `praxis schema --write`, `praxis validate <score>`, `praxis render <score> -o out.wav`
  (empty scores only until WP-F lands).
- Score v1 types, JSON Schema in `schema/`, validation with stable issue codes, theory helpers,
  time map and note expansion, duration solver (16 bars in 30 s -> 128 BPM).
- Fixtures: `fixtures/empty.score.json`, `fixtures/acceptance1_bell.score.json` (Phase 1),
  `fixtures/acceptance1.score.json` (Phase 2, felt piano + soft kick).

## Known gaps

- Everything marked "next" above. `apply`, `lint`, `rhythm`, `verify_render` are contract
  placeholders that return empty results until their WPs land.

## Log

- Merged WP-E audio-io; accepted its contract corrections (stem tuple, hardened ffmpeg command).

- Phase 0 exit: CI green on all three OSes. ADR-0010: agents are first-class users (MCP server).

- Phase 0 foundations written: workspace, contracts, CI, ADRs, docs.
