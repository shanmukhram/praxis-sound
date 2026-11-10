# Status

Updated after every merge. Newest first. Branch: `build/integration` (pushed to `origin`).

## Current phase: 0 -> 1

| WP | Status | Branch | Notes |
| --- | --- | --- | --- |
| WP-0 foundations | done locally; CI on 3 OSes pending first push | build/integration | tests, clippy -D warnings, fmt, wasm32 build pass on Windows; empty score renders a silent 12 s WAV |
| WP-A score-ops | next | wp/a-score-ops | |
| WP-B1 dsp-core | next | wp/b1-dsp-core | |
| WP-B2 dsp-effects | next | wp/b2-dsp-effects | |
| WP-C1 sampler | next | wp/c1-sampler | |
| WP-D1 analyze-verify | next | wp/d1-analyze-verify | |
| WP-E audio-io | next | wp/e-audio-io | |
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

- Phase 0 foundations written: workspace, contracts, CI, ADRs, docs.
