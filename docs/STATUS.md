# Status

Updated after every merge. Newest first. Branch: `build/integration` (pushed to `origin`).

## Current phase: 2 (Phase 1 exit passed; demos in demos/phase-1/)

Wave 1 (WP-A, B1, B2, C1, D1, E, F, P) runs as background agents in worktrees. From wave 2 on,
work packages run as visible sessions (owner's choice), briefed from `docs/briefs/`. WP-X (MCP
server for agents, ADR-0010) is planned after WP-K, P, D1, F2.

| WP | Status | Branch | Notes |
| --- | --- | --- | --- |
| WP-0 foundations | **done**: CI green on Windows, macOS, Linux and wasm32 (run 42868034466) | build/integration | empty score renders a silent 12 s WAV |
| WP-A score-ops | **merged** (782c780 + follow-up 6e2a87b: indexed adds, set_seed, exact isolation) | wp/a2-score-ops | apply with exact inverse, scope/lock rules, 5 lints, project undo/redo/versions; 67 tests incl. proptests |
| WP-B1 dsp-core | **merged** (671dfdb) | wp/b1-dsp-core | BLEP oscillators (8-sample latency), wavetable, FM, ADSR/DAHDSR, SVF/ladder, noise, smoothing, resampler; 0 allocations after construction |
| WP-B2 dsp-effects | **merged** (505669e) | wp/b2-dsp-effects | all 11 effects, true-peak limiter, BS.1770 meter (0.0000 LU vs ebur128); trait gained set_tempo/set_impulse |
| WP-C1 sampler | **merged** (4cff5ba); onset-alignment fix running | wp/c1b-onset-align | SFZ + SF2 parsers, sampler (48 voices, velocity crossfade, release samples, loops, seeded round robin), felt/upright presets, pack install with blake3; pitch within 0.47 cent of samples. Onset fix merged (ed693c4): notes within 0.2-1.7 ms of note time (was +2.6 to +9.1 ms) |
| WP-D1 analyze-verify | **merged** (c730c4f) | wp/d1-analyze-verify | verify_render: tempo, key, grid, loudness, true peak, clipping, seam, silence; synthetic tempo within 0.03 BPM, 10/10 keys |
| WP-E audio-io | **merged** (fbdb8d1) | wp/e-audio-io | WAV/FLAC/OGG/MP3 native, M4A/MP4/MOV/WebM via ffmpeg (hardened), FLAC + stems export, MIDI export; 47 tests. Gaps: no ffmpeg timeout; resampler ~0.5 sample offset |
| WP-F engine-core | **merged** (5075c5b) | wp/f-engine-core | offline render, stems, avoid-list, loop wrap, determinism; 88x real time; effects plumbing awaits WP-F2 |
| WP-U1 front end (mock engine) | **merged** (f073ab7, merge ce36ac9) | wp/u1-frontend | app/: timeline, step sequencer, piano roll, waveform/spectrogram, undo/versions, export dialog vs a mock engine; 121 Vitest + 5 Playwright. Rebuilt to the approved ADR-0011 conversation-first design (WP-U2, merged) |
| WP-P planner | **merged** (627e121) | wp/p-planner | ScorePlan, RuleProvider, arranger, repair, ClaudeProvider (replay fixtures only), compose_plan for agents; 27/27 prompt suite; acceptance-1 brief -> Score first try |

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

- Merged WP-X MCP server (06a1d05): `praxis-mcp` stdio binary with praxis_plan_schema, presets, compose, generate, render, check, analyze, export, validate (all via praxis_cli::ops); agent guide docs/agents/praxis-skill/SKILL.md and docs/agents/mcp.json.example. tokio + praxis-cli moved into workspace deps. 456 release tests pass. Pending tools: praxis_edit, praxis_variations (wire to WP-P2), praxis_takes / praxis_open_view (with WP-U3).

- Merged WP-P2 edits + variations (331acac): plan_edit / compose_edit / plan_project_edit, EditPlan schema, locked layers refused out loud, repairs never widen scope, variations with own seeds; 15/15 edit prompt cases; bars 1-12 identical after "change the last four bars" (note level). Follow-ups noted: RuleProvider routing of edit tool, prompt_suite filter, planner dev-dep on praxis-engine for render-level isolation.
- Merged WP-C2 synth + drums (394cb94): 11 synth presets + 11 drum presets + GM kit; pitch within 0.01 cent (warm_bass -3.5 c, warm_pad +2.7 c), soft_kick 100% energy < 150 Hz, -40 dB in 230 ms, 0 allocations. Acceptance 1 now renders the kick and passes all 8 checks (-15.54 LUFS, 93% on grid). CLI test updated: bell fixture now renders. 452 release tests pass.

- Merged WP-K CLI (670353e): shared command layer praxis_cli::ops; render (+flac, checks in payload), validate, schema, check, generate (offline/claude/--plan), analyze, export-midi, packs, key, presets; 16 binary tests. 403 release tests pass; acceptance 1 passes all checks. Note: the hand-written fixtures/acceptance1.score.json render fails grid (87%) and loudness until WP-F2.

- Owner stopped all four wave-2 sessions mid-work; their work is uncommitted in their worktrees (exact files and how to resume: docs/HANDOFF.md section 5).

- Orchestration paused to continue in the terminal; four wave-2 sessions still running in the desktop app (see docs/HANDOFF.md section 5). WP-U3 adapter brief written.

- Wave 2 started as sessions: WP-C2 synth + drums, WP-F2 mix bus, WP-K CLI, WP-P2 edits + variations (details and session ids in docs/HANDOFF.md section 5).

- Merged WP-U2 (12c9e8e): `npm run dev` in app/ opens the approved conversation-first workstation (mock engine). 138 unit + 8 Playwright tests at 1440/390 rerun and passing; screenshots in app/screenshots/ match docs/design/. Next for the UI: Tauri adapter filling takes from praxis generate|edit|check, and MCP praxis_takes/praxis_open_view.

- WP-U2 conversation UI: engine side committed (27cd5be on wp/u2-conversation-ui); visible screen uncommitted in worktree funny-elion-3d4d1d; session paused on the account usage limit. Handoff written: docs/HANDOFF.md.

- Merged WP-U1 front end and the WP-C1 onset fix. Acceptance test 1 now passes end to end with the real felt piano (kick pending WP-C2): tempo 127.99, A minor, 93% on grid (median 1.6 ms), -15.6 LUFS, seamless. earshot realness 0.46 (dry, no room) -> WP-F2 reverb must lift it. Preview in demos/phase-2/.

- Merged WP-D1, WP-P, WP-C1, WP-A follow-up (all of wave 1). First end-to-end acceptance-1 run (offline brief -> felt piano render -> verify): structure all pass; tempo 128.00 and key A minor pass; grid FAILS (47.7% on grid, median 11.4 ms) because sampled attacks land late. Fix sent to WP-C1; `crates/praxis-cli/tests/acceptance1.rs` lands with it.

- Phase 1 exit: bell acceptance-1 render bit-identical across reruns, seam ratio 0.12, 30.000 s, earshot ~129.2 BPM A minor. Demos in demos/phase-1/.
- Merged WP-F, WP-A, WP-B1, WP-B2. Contract: integer-step note timing; Processor::set_tempo/set_impulse; cross-crate effect-param test.

- Merged WP-E audio-io; accepted its contract corrections (stem tuple, hardened ffmpeg command).

- Phase 0 exit: CI green on all three OSes. ADR-0010: agents are first-class users (MCP server).

- Phase 0 foundations written: workspace, contracts, CI, ADRs, docs.
