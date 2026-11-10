# Praxis Sound build plan

Source of truth for requirements: `PRD.md`. This file breaks every phase into work packages (WPs).
Each WP lists the FRs it covers, the files it owns (nobody else edits them), what it depends on,
and the check that proves it done. Current state lives in `docs/STATUS.md`; rules every WP follows
are in `docs/BRIEFING.md`; contracts in `docs/contracts/`; decisions in `docs/adr/`.

**How to resume with no memory:** read STATUS.md, then this file. Pick the first WP whose
dependencies are merged and whose status is not done. Brief it from its entry here plus
BRIEFING.md and the contracts it names. Review every result yourself: read the diff, run the
tests, render the demos, run `praxis check`. Merge into `build/integration` only what passes.

Branches: integration branch `build/integration` (pushed to `origin`, a private GitHub repo).
`main` is untouched until the owner says otherwise. Each WP works on `wp/<id>-<name>`.

## Dependency graph (WPs, not phases)

```
WP-0 foundations (done)
 ├─ WP-A score-ops ─────────────┬─ WP-P2 edits+variations ─┐
 ├─ WP-B1 dsp-core ── WP-C2 synth+drums ──┐                │
 ├─ WP-B2 dsp-effects ── WP-F2 engine-mix ─┼─ WP-I1 acceptance-1 end to end ── Phase 4 exit
 ├─ WP-C1 sampler+felt piano ─────────────┤                │
 ├─ WP-F engine-core ─────────────────────┘                │
 ├─ WP-D1 analyze-verify ── WP-D2 sound-profile ── WP-M match ── Phase 6 exit
 ├─ WP-E audio-io ── WP-R realtime ── WP-U workstation UI ── Phase 3 exit
 └─ WP-P planner ─────── WP-P2 ── WP-P3 scoring depth ── Phase 7 exit
WP-H sfx patches (after B1, B2)     WP-K cli (grows with each WP) ── WP-X MCP server + agent guide
WP-W wasm + browser (after U)       WP-Z performance + release (last)
```

## Phase 0: Foundations (exit: CI green on 3 OSes; CLI renders an empty Score to a silent WAV)

### WP-0 foundations (orchestrator) - DONE when CI green
Workspace with all crates; CI (fmt, clippy -D warnings, debug + release tests, schema freshness,
wasm32 build, empty-score render) on Windows, macOS, Linux; ADRs 0001-0009; Score schema v1 +
validation + round-trip and schema tests; golden harness (`praxis-testkit`); frozen contracts in
code and `docs/contracts/`; `praxis render|validate|schema`.
Check: `cargo test --workspace`; `praxis --json render fixtures/empty.score.json -o e.wav` gives a
silent stereo WAV of the right length; GitHub Actions green on all three runners.

## Phase 1: Engine core (exit: hand-written acceptance-1 Score with a bell tone renders with a clean loop seam, bit-identical across reruns)

### WP-A score-ops
FR-6, FR-7 (core), FR-31; property tests for schema and patches.
Owns: `crates/praxis-score/src/apply.rs`, `lint.rs`, `project.rs` (logic only; types frozen),
`crates/praxis-score/tests/patch_*.rs`, `tests/project_*.rs`, `tests/lint_*.rs`.
Depends: WP-0.
Done: every rule in contracts/patch.md has a test, including proptests for exact inverse and
bar/layer isolation over generated scores; lints implement the owner's taste rules (isolated notes,
no repetition, missing sustain overlap on sustaining layers, too many elements, unrequested
effect classes); project undo/redo/versions with tests.

### WP-B1 dsp-core
FR-21 (building blocks). Owns: `crates/praxis-dsp/src/{osc,env,filter,noise,smooth,interp}.rs`
and any `crates/praxis-dsp/src/{osc,env,filter,noise,smooth,interp}/` submodules,
`crates/praxis-dsp/tests/core_*.rs`. Depends: WP-0.
Done: API table in contracts/dsp.md implemented and unit-tested (pitch accuracy < 0.1 cent,
BLEP alias energy bound, envelope timing within one sample, filter responses within 0.5 dB of
theory), no allocation after construction.

### WP-B2 dsp-effects
FR-23, FR-25 (meters). Owns: `crates/praxis-dsp/src/fx/**`, `crates/praxis-dsp/src/meter/**`,
`crates/praxis-dsp/tests/fx_*.rs`. Depends: WP-0 (uses frozen biquad/delayline).
Done: every EffectKind type with indexed params per `params.rs`, honest tails and latency;
compressor sidechain; true-peak limiter never exceeds its ceiling on adversarial signals;
LoudnessMeter within 0.1 LU of `ebur128` on test signals; `fx::create`.

### WP-C1 sampler + felt piano
FR-20. Owns: `crates/praxis-instruments/src/sampler.rs`, `src/sampler/**`,
`crates/praxis-instruments/tests/sampler_*.rs`, `packs/index.json`, `packs/README.md`,
the pack-install logic in `crates/praxis-instruments/src/sampler/install.rs`.
Depends: WP-0. Downloads approved by owner for CC0/CC-BY packs < 100 MB.
Done: SFZ and SF2 parsing per contracts/instruments.md with parser tests (incl. a generated tiny
SF2 and SFZ in test fixtures); sampler voices with velocity layers, release, loops, round robin;
`felt_piano` and `upright_piano` presets from Upright Piano KW; renders of single notes tested for
pitch (analysed fundamental within 5 cents) and no clicks at note start/end.

### WP-D1 analyze-verify
Verification tooling (section "verification" of the brief). Owns:
`crates/praxis-analyze/src/verify.rs` body (types frozen), `src/features.rs` + `src/features/**`,
`crates/praxis-analyze/tests/verify_*.rs`. Depends: WP-0 (meters: may use `ebur128` directly
until WP-B2 lands).
Done: `verify_render` implements every check in contracts/sound-profile.md; tested on synthetic
renders built in the test (click trains at known BPM, chords in known keys, a loop with a
deliberate click at the seam, a clipped signal).

### WP-E audio-io
FR-11 (decoding), FR-32 (FLAC, stems, MIDI). Owns: `crates/praxis-audio-io/src/{decode,encode,midi}.rs`
(+ dirs), `crates/praxis-audio-io/tests/**`. Depends: WP-0.
Done: decode round trips (write WAV/FLAC, decode, compare), OGG/MP3 decoded from small fixtures
generated with ffmpeg in tests when available, video audio extraction via ffmpeg, FLAC export,
MIDI export validated by re-parsing with midly.

### WP-F engine-core
FR-27 (offline), FR-26 (loop wrap), FR-3 (engine enforcement), FR-24 (automation of layer
params). Owns: `crates/praxis-engine/src/render.rs` and new engine modules except `api.rs`,
`instrument.rs`, `lib.rs` (orchestrator re-exports new modules on request), `crates/praxis-engine/tests/**`,
`tests/golden/engine_*.json`. Depends: WP-0.
Done: sequencer with block splitting at events; per-layer instrument + gain/pan + stems; mute/solo;
avoid-list blocking; loop wrap with warm-up cycles; determinism test (two renders bit-identical);
edit-isolation test at stem level; a built-in test bell instrument in `src/testing.rs` (not a
preset); acceptance-1 bell fixture renders, seam verified by a local seam measurement.

### WP-P planner
FR-1, FR-2, FR-3 (plan level), FR-4, FR-5, FR-34. Owns: everything in
`crates/praxis-planner/src/` except `provider.rs`, `lib.rs`; `crates/praxis-score/src/rhythm.rs`;
`crates/praxis-planner/tests/**`, `tests/prompts/**`. Depends: WP-0.
Done: ScorePlan schema + validation + repair; RuleProvider; arranger; ClaudeProvider with
replay fixtures; prompt suite (>= 20 briefs) with automatic validators for key, grid, structure,
banned elements; acceptance-1 brief through RuleProvider yields a Score meeting every structural
condition of acceptance test 1.

## Phase 2: Instruments, effects and mix (exit: acceptance 1 with sampled felt piano + soft kick at loudness target; owner listening sign-off)

### WP-C2 synth + drums
FR-21, FR-22. Owns: `crates/praxis-instruments/src/{synth,drums}.rs` + dirs, tests `synth_*`, `drums_*`.
Depends: WP-B1. Done: presets in contracts/instruments.md incl. `bell` and `soft_kick`; each preset
renders without clicks, in tune, with tests; drum voices measured (kick fundamental, decay).

### WP-F2 engine mix bus
FR-23 (wiring), FR-24 (effect automation), FR-25, sidechain. Owns: engine modules created for
fx chains, master bus and sidechain. Depends: WP-F, WP-B2.
Done: effect chains from `Effect` specs, sidechain keyed by layer, latency compensation, master
normalisation to target within 0.5 LU and true peak <= ceiling, frozen gain, golden tests.

### WP-I1 acceptance test 1 (orchestrator)
Wire registry + engine + CLI; render `fixtures/acceptance1.score.json`; `praxis check` passes
(tempo 128, A minor, grid, -16 LUFS +-0.5, true peak <= -1, seam); demos to `demos/phase-2/`.

## Phase 3: Real-time engine and workstation (exit: acceptance 1 built by hand in the window, played without dropouts, exported)

### WP-R realtime
FR-27 (real time). Owns: `crates/praxis-engine/src/realtime/**`, `crates/praxis-audio-io/src/device.rs`.
Depends: WP-F2, WP-E. Done: RealtimeEngine with rtrb command/meter queues, graph swap without
allocation on the audio thread (tested with an allocation-counting test harness), cpal output,
transport (play/stop/seek/loop), no dropouts at 256 frames in a 60 s soak (xrun counter = 0).

### WP-U workstation UI
FR-28, FR-29, FR-30, FR-31 (UI), FR-32 (export UI). Owns: `app/**` (front end),
`crates/praxis-app/**` (Tauri shell). Depends: WP-R, WP-A, WP-E.
Done: timeline (bars, sections, layers with mute/solo/lock), step sequencer and piano roll editing
patterns through patches, waveform + spectrogram, undo/redo, versions, export dialog; browser
dev mode against a mock engine for UI tests plus Tauri build; screenshots in the report.

### WP-U3 real adapter + Tauri shell
Owns: `crates/praxis-app/**`, `app/src/engine/tauri/**`. Depends: WP-K (command layer), WP-U2.
Brief: `docs/briefs/WP-U3-real-adapter.md`. Done: requests in the app make real, checked takes
with real audio on Windows; agent takes appear live.

## Phase 4: Planner (exit: acceptance 1 from one prompt; "change the last four bars" leaves bars 1-12 sample-identical; prompt suite passes)

### WP-P2 edits + variations
FR-7, FR-8. Owns: `crates/praxis-planner/src/{edit,variations}.rs`. Depends: WP-P, WP-A.
Done: edit instructions -> scoped patches (RuleProvider + Claude); locked layers never in scope;
N variations with own seeds; render-level isolation test of "change the last four bars".

### WP-K cli
FR-33. Owns: `crates/praxis-cli/**`. Depends: grows with P, D, E, M. Done: every command in
contracts/cli.md, JSON payload tests (`assert_cmd`-style via std::process).

### WP-X agent interface (MCP server + agent guide) - ADR-0010
FR-33, FR-34 for agents. Owns: `crates/praxis-mcp/**` (new crate, added to the workspace by the
orchestrator), `docs/agents/**`, a `praxis mcp` subcommand hook agreed with WP-K. Depends:
WP-K (shared command layer), WP-P (`compose_plan`, `plan_schema`), WP-D1 (verify), WP-F2.
Brief: `docs/briefs/WP-X-agent-interface.md`.
Done: every tool in ADR-0010 implemented over stdio with JSON Schema inputs; an end-to-end test
that spawns the server, lists tools, calls `praxis_plan_schema`, sends the acceptance-1 plan to
`praxis_compose`, and gets a project, a WAV path and a passing check report; the agent guide with
worked examples; a Claude Code `.mcp.json` example. Shared logic lives in one command layer used
by both CLI and MCP (no duplicated behaviour).

## Phase 5: Reference analysis (exit: tempo +-2 BPM and correct key on >= 90% of a labelled set; profiles for music, effect, ambience)

### WP-D2 sound profile
FR-11 (pipeline), FR-12, FR-13, FR-19. Owns: `crates/praxis-analyze/src/{features,similarity,originality}/**`,
profile building, library store, `tests/clips/**` (self-made/licensed labelled set + labels.json).
Depends: WP-D1, WP-E. Done: profile per contract; classification; labelled-set accuracy test in CI.

## Phase 6: Similar-sound generation (exit: each generated sound closest to its own reference, passes originality, survives blind listening)

### WP-H sfx patches
FR-16. Owns: `crates/praxis-instruments/src/sfx.rs` + dir. Depends: WP-B1, WP-B2.
### WP-M match
FR-14, FR-15, FR-17, FR-18. Owns: `crates/praxis-match/**`. Depends: WP-D2, WP-H, WP-F2, WP-P.

## Phase 7: Scoring depth (exit: 2-minute score from a brief with three hit points lands each on its marker; listening review)

### WP-P3 scoring depth
FR-9, FR-10. Owns: planner arranger extensions (`arrange/**`), score chord helpers by request.
Depends: WP-P2, WP-F2.

## Phase 8: Performance and release (exit: every success criterion in PRD section 2 passes on release builds)

### WP-W wasm + browser build
Owns: `crates/praxis-wasm/**`, browser AudioWorklet glue in `app/src/engine-wasm/**`.
### WP-Z performance + release
Profiling (20x real time for 2-min 8-layer piece; cold start < 1 s; idle < 200 MB), installers
(Windows, macOS), pack downloader UI, CLI docs, crash reporting, licence audit.

## Success-criteria checks (PRD section 2) and where they are tested

| Criterion | Test |
| --- | --- |
| Correctness | `tests/acceptance1.rs` (prompt -> score -> render -> `verify_render`) |
| Determinism | engine determinism test + golden hashes per platform |
| Edit isolation | engine stem/master isolation tests; planner "last four bars" test |
| Loop seam | `verify_render` loop_seam check on every loop golden |
| Render speed | `praxis bench` 2-min 8-layer score >= 20x real time (release) |
| Real-time preview | WP-R soak test, xruns = 0 at 256 frames |
| Footprint | WP-Z measurements in CI artefacts |
| Reference analysis | WP-D2 labelled-set test |
| Reference match | WP-M test-set ranking + originality |
| Loudness | `verify_render` loudness/true_peak on every golden |
