# Status

Updated after every merge. Newest first. Branch: `build/integration` (pushed to `origin`).

## Current phase: 2 (Phase 1 exit passed; demos in demos/phase-1/)

Wave 1 (WP-A, B1, B2, C1, D1, E, F, P) runs as background agents in worktrees. From wave 2 on,
work packages run as visible sessions (owner's choice), briefed from `docs/briefs/`. WP-X (MCP
server for agents, ADR-0010) is planned after WP-K, P, D1, F2.

| WP | Status | Branch | Notes |
| --- | --- | --- | --- |
| WP-0 foundations | **done**: CI green on Windows, macOS, Linux and wasm32 (run 42868034466) | build/integration | empty score renders a silent 12 s WAV |
| WP-A score-ops | **merged** (782c780); follow-up running (contract B index-on-add, C set_seed, exact isolation) | wp/a2-score-ops | apply with exact inverse, scope/lock rules, 5 lints, project undo/redo/versions; 67 tests incl. proptests |
| WP-B1 dsp-core | **merged** (671dfdb) | wp/b1-dsp-core | BLEP oscillators (8-sample latency), wavetable, FM, ADSR/DAHDSR, SVF/ladder, noise, smoothing, resampler; 0 allocations after construction |
| WP-B2 dsp-effects | **merged** (505669e) | wp/b2-dsp-effects | all 11 effects, true-peak limiter, BS.1770 meter (0.0000 LU vs ebur128); trait gained set_tempo/set_impulse |
| WP-C1 sampler | next | wp/c1-sampler | |
| WP-D1 analyze-verify | next | wp/d1-analyze-verify | |
| WP-E audio-io | **merged** (fbdb8d1) | wp/e-audio-io | WAV/FLAC/OGG/MP3 native, M4A/MP4/MOV/WebM via ffmpeg (hardened), FLAC + stems export, MIDI export; 47 tests. Gaps: no ffmpeg timeout; resampler ~0.5 sample offset |
| WP-F engine-core | **merged** (5075c5b) | wp/f-engine-core | offline render, stems, avoid-list, loop wrap, determinism; 88x real time; effects plumbing awaits WP-F2 |
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

- Phase 1 exit: bell acceptance-1 render bit-identical across reruns, seam ratio 0.12, 30.000 s, earshot ~129.2 BPM A minor. Demos in demos/phase-1/.
- Merged WP-F, WP-A, WP-B1, WP-B2. Contract: integer-step note timing; Processor::set_tempo/set_impulse; cross-crate effect-param test.

- Merged WP-E audio-io; accepted its contract corrections (stem tuple, hardened ffmpeg command).

- Phase 0 exit: CI green on all three OSes. ADR-0010: agents are first-class users (MCP server).

- Phase 0 foundations written: workspace, contracts, CI, ADRs, docs.
