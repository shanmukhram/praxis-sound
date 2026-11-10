# Handoff: Praxis Sound build (kept current; last updated 2026-11-11 05:10 IST: paused, nothing running)

Paste the block below into a new Claude Code session opened in
`C:\Users\admin\Desktop\praxis sound`. Everything else in this file is the detail it points to.

---

```
You are taking over as lead engineer and orchestrator of Praxis Sound, a Rust sound-design
workstation in C:\Users\admin\Desktop\praxis sound (git repo, GitHub: shanmukhram/praxis-sound,
private). The owner's goal: come back to a working app, not a plan. You plan, split work into
packages, run parallel sessions, review everything yourself, merge, and keep going until PRD.md
is complete.

Read, in this order, before doing anything: docs/HANDOFF.md (this handoff, all of it), PRD.md,
docs/STATUS.md, docs/PLAN.md, docs/BRIEFING.md, docs/NEEDS_OWNER.md, docs/adr/ (all), and
docs/contracts/ (all). The product name is Praxis Sound (never "Sonic Foundry"); crates are
praxis-*; the CLI binary is `praxis`.

Hard rules (from the owner):
- Commits: `git -c user.name=shanmukhram -c user.email=51820274+shanmukhram@users.noreply.github.com commit -F <msg file>`.
  NEVER add Co-Authored-By, "Generated with Claude Code" or any AI attribution, trailers or emoji.
  After every commit check `git log -1 --format='%an <%ae>%n%B'` shows exactly that author and
  `git log -1 --format='%B' | grep -icE "co-authored|generated with|noreply@anthropic"` prints 0.
  Stage explicit paths only (never git add -A). Never --no-verify. Never touch `main`.
- Work and merge on branch `build/integration`; push it to origin when there is something to
  push (standing permission). Work packages use their own branches/worktrees `wp/<id>-<name>`.
- Standing permission: merge a session's branch into build/integration after you review it
  (scope = only its owned files, authorship + attribution checks, rerun its tests).
- The owner chose: run work packages as visible sessions (start_session), not hidden agents.
- Ask the owner only for what only they can give (see docs/NEEDS_OWNER.md). Do not re-ask
  decided items. Ask for the Anthropic API key only when live Claude planning is the next step.

Next actions: see "What to do next" in docs/HANDOFF.md.
```

---

## 1. Where things are

| Thing | Where |
| --- | --- |
| Repository | `C:\Users\admin\Desktop\praxis sound` (Windows 11, Git Bash + PowerShell) |
| GitHub | https://github.com/shanmukhram/praxis-sound (private). Remote `origin`. `gh` CLI is logged in as `shanmukhram` |
| Integration branch | `build/integration` (all merged work; pushed; CI green at `ecef2e8`) |
| `main` | empty, untouched. Do not commit there unless the owner says so |
| CI | `.github/workflows/ci.yml`: fmt, clippy -D warnings, debug + release tests, schema freshness, empty-score render on Windows/macOS/Linux, wasm32 build. `gh run list --branch build/integration` |
| Requirements | `PRD.md` (FR-1..FR-34, success criteria, phases 0-8) |
| Plan / status | `docs/PLAN.md` (every work package with owned files, deps, done check), `docs/STATUS.md` (current state, newest log first) |
| Rules for every package | `docs/BRIEFING.md` (ownership, engineering, audio-quality and git rules, report format) |
| Contracts (frozen interfaces) | `docs/contracts/*.md` and the code files marked `FROZEN CONTRACT` |
| Decisions | `docs/adr/0001..0011` |
| Briefs ready to hand out | `docs/briefs/WP-C2-synth-drums.md`, `WP-F2-mix-bus.md`, `WP-K-cli.md`, `WP-P2-edits-variations.md`, `WP-X-agent-interface.md`, `WP-U2-conversation-ui.md` |
| Owner asks | `docs/NEEDS_OWNER.md` |
| Approved UI design | `docs/design/workstation-concept.html` (clickable), screenshots in `docs/design/screenshots/`, ADR-0011 |
| Demos | `demos/phase-1/` (bell loop + x3 seam test), `demos/phase-2/preview_felt_piano_only.flac` |
| Piano samples | `packs/upright-piano-kw/` (CC0, installed locally, git-ignored; index + blake3 in `packs/index.json`) |
| Fixtures | `fixtures/empty.score.json`, `acceptance1_bell.score.json`, `acceptance1.score.json` |

## 2. What the product is (short)

A text brief or a dropped audio/video clip becomes editable, layered audio. Everything is a
**Score** (key, tempo, bars, sections, layers, patterns, instruments, effects, automation). A
planner (offline rules, or Claude) and an analyzer write the Score; a procedural engine (samples,
synthesis, effects) renders it; every edit is a scoped patch so untouched bars stay
sample-identical. The owner's AI agents are primary users (ADR-0010: MCP server, the agent can
write the plan itself, every render returns measurements). The UI is conversation-first
(ADR-0011, approved): each request makes a playable "take" with plain-language checks and a
change map; deeper views open on demand.

## 3. Owner decisions (do not re-ask)

Name Praxis Sound. Licence closed for now (decided later). Trademark later. Buy nothing: make
everything in code; free CC0/CC-BY packs and impulse responses under 100 MB each are approved.
No MPL-2.0 (no symphonia; permissive decoders + optional ffmpeg). Windows is the release
platform (macOS/Linux CI kept for portability). API key: owner gives it when asked. UI: the
conversation-first concept is approved; scrollbars must match the dark theme.

## 4. State of the build

Phase 0 done (CI green on 3 OSes). Phase 1 done (bell acceptance loop bit-identical, seamless).
Phase 2 in progress.

Merged into `build/integration` (all reviewed, tests rerun):

| Package | What it delivered |
| --- | --- |
| WP-0 foundations | workspace, Score schema v1 + JSON Schema, validation, theory, timeline, CLI render/validate/schema, golden harness, CI |
| WP-A score-ops (+follow-up) | scoped patches with exact inverses, lock/scope rules (users may mute/solo locked layers), indexed adds, set_seed, 5 taste lints, project undo/redo/versions, proptests |
| WP-B1 dsp-core | BLEP oscillators, wavetable, FM, envelopes, SVF/ladder, noise, smoothing, resampler |
| WP-B2 dsp-effects | all 11 effects, true-peak limiter, BS.1770 meter (matches ebur128) |
| WP-C1 sampler (+onset fix) | SFZ/SF2 sampler, felt_piano + upright_piano presets, pack install with blake3; notes start on their beat (within 2 ms) |
| WP-D1 analyze-verify | `verify_render`: tempo, key, grid, loudness, true peak, clipping, loop seam, silence |
| WP-E audio-io | WAV/FLAC/OGG/MP3 decode, M4A/MP4/MOV/WebM via ffmpeg, FLAC/stems/MIDI export |
| WP-F engine-core | offline renderer, stems, avoid-list enforcement, seamless loop wrap, determinism, 88x real time |
| WP-P planner | ScorePlan, offline RuleProvider, arranger, repair loop, ClaudeProvider (replay fixtures only, no live call yet), `compose_plan` for agent-written plans; 27/27 prompt suite |
| WP-U1 front end | app/ (TypeScript + Vite) against a mock engine: timeline, step sequencer, piano roll, waveform, undo, export dialog |

Key result: `crates/praxis-cli/tests/acceptance1.rs` runs acceptance test 1 end to end (one
brief -> offline planner -> real felt piano -> render -> verifier) and passes every check:
tempo 127.99, A minor, 93% of onsets on the grid (median 1.6 ms), -15.6 LUFS, seamless loop.
The soft kick layer is skipped until WP-C2 lands (the test says so and tightens automatically).

Test totals at handoff: 386 Rust tests pass in release; app: 140 unit tests on the WP-U2 branch.

## 5. In flight (updated 2026-11-11 05:10 IST: paused by the owner; nothing running)

**Nothing is running and nothing is uncommitted.** Every package so far is merged into
`build/integration` and pushed (HEAD `734b309` or later). Wave 2 (WP-C2 synth + drums, WP-F2 mix
bus, WP-K CLI, WP-P2 edits + variations) and WP-X (MCP server) are all merged.

- **Phase 2 exit demo is ready**: `demos/phase-2/acceptance1_mixed_loop.flac` (+ `_x3` seam file,
  README with what to listen for). Waiting for the owner's listening sign-off
  (docs/NEEDS_OWNER.md item 2). Measured: 127.99 BPM, A minor, -16.00 LUFS, -2.72 dBTP, seamless,
  kick adds 7.9 dB below 120 Hz from bar 9; grid 90.9% (thin margin, see below).
- **WP-U3 real adapter + Tauri shell** was started and then stopped by the owner before it wrote
  anything (its empty worktree was removed). Restart it from `docs/briefs/WP-U3-real-adapter.md`
  on a new branch `wp/u3-adapter` from `build/integration`. Authorise exactly one root
  `Cargo.toml` edit: add `crates/praxis-app` to `[workspace] members` only (not default-members),
  plus any audited third-party deps; check Tauri versions/licences first.

Known follow-ups (not started):
1. **Analyzer onset robustness**: the acceptance grid passes at 90.9% (threshold 90%). The
   detector counts a soft felt-piano note's second swell (pitch 60, velocity 0.55), made more
   visible by the room reverb, as an extra onset; 8 soft notes read ~12 ms late. Fix in
   `praxis-analyze` onset detection (do not bend the reverb further). The FDN reverb's modulation
   is off (time-invariant, exactly periodic loops); revisit only if the owner hears ringing.
2. **Wire edits into the CLI and MCP**: `praxis edit` / `praxis variations` in `praxis_cli::ops`
   and `praxis_edit` / `praxis_variations` tools in `praxis-mcp`, using
   `praxis_planner::{plan_project_edit, compose_edit, variations}`.
3. **Planner tidy-ups proposed by WP-P2**: route the edit tool inside `RuleProvider::call_tool`;
   make `prompt_suite.rs` skip `edit_*` files; add `praxis-engine` as a planner dev-dependency for
   render-level edit-isolation tests.
4. **CLI**: `validate` should accept `.praxis` projects; `pack_verify_failed` error code; a
   keychain delete helper for `key clear`.
5. Old merged branches (`wp/*`, `worktree-agent-*`) and stale worktrees under `.claude/worktrees/`
   can be deleted (`git worktree list`, `git worktree remove <path>`, `git branch -d <branch>`).

Test totals at pause: about 475 Rust tests in release (run the full suite with
`PRAXIS_PACKS` pointing at `packs/` so the piano tests run), app 138 unit + 8 Playwright,
MCP stdio test, acceptance test 1 all 8 checks.

## Continuing this orchestration in the terminal

Resume this orchestrating session in a terminal opened in the project folder:

```
cd "C:\Users\admin\Desktop\praxis sound"
claude --resume 1d1ce050-83ea-4eb7-af85-1081b75553dc
```

(or `claude --continue` for the most recent session in this folder, or `claude --resume` to pick
from a list). If it cannot be resumed, start `claude` in the folder and paste the prompt at the
top of this file. In the terminal, desktop-only tools (session cards, the browser pane, session
messaging) are not available: review branches with git, start new work as sub-agents or ask the
owner to open sessions, and take screenshots with headless Edge (section 8).

## 6. What to do next (in order)

1. **WP-U2**: done and merged. Next UI step (later): a Tauri adapter that fills takes from the
   CLI command layer, plus the MCP take/view hooks.
2. **WP-C2 synth + drums** (merged): gives `bell` and `soft_kick`; then
   acceptance test 1 renders with the kick.
3. **WP-F2 mix bus** (merged): effect chains, sidechain, master loudness to
   -16 LUFS and the true-peak limiter inside the loop wrap. Also must give the dry piano a room
   (earshot rated the dry render's realness 0.46 because there was no room sound).
4. **Phase 2 exit**: demo rendered (`demos/phase-2/`); waiting for the owner's listening sign-off.
5. **WP-K CLI** (merged), then **WP-U3 real adapter + Tauri shell** (`docs/briefs/WP-U3-real-adapter.md`, starts when WP-K merges) and **WP-X MCP server** (ADR-0010; rmcp 3.9 is already in the workspace deps;
   tools share the CLI's command layer; agent can write the plan; renders return verify reports).
6. **WP-P2 edits + variations** (merged; live Claude smoke test still needs the API key) (Phase 4 exit: "change the last four bars" leaves bars 1-12
   sample-identical). Ask the owner for the API key when a live Claude smoke test is next.
7. Then Phases 3 (real-time + Tauri shell), 5 (reference analysis), 6 (match + SFX patches),
   7 (scoring depth), 8 (performance + release) per `docs/PLAN.md`.

## 7. How to review and merge a package (what has worked)

1. `git log --format='%h %an <%ae> %s' build/integration..<branch>`; the attribution grep on the
   range prints 0.
2. `git diff --name-only build/integration...<branch>` contains only the package's owned files
   (see its PLAN entry); frozen contract files are unchanged unless you approved the exact diff.
3. Merge with `git -c user.name=... -c user.email=... merge --no-ff <branch> -m "Merge WP-..."`.
4. Rerun: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace --release --no-fail-fast`, `cargo build -p praxis-wasm --target wasm32-unknown-unknown`,
   and for audio `cargo test -p praxis-cli --release --test acceptance1 -- --nocapture`.
5. Listen-check with the earshot MCP tools (`measure`, `describe`, `realness`) on renders.
6. Accept or reject contract proposals from the report (you own the contracts), update
   `docs/STATUS.md`, commit, push `build/integration`.

## 8. Environment notes

- Rust 1.99 stable (MSVC), wasm32 target installed, Node 24, Python 3.12 (no numpy), ffmpeg 9 on
  PATH, gh CLI logged in. Disk has ~200 GB free.
- The taste-skill design guidance is installed as Markdown only (its scripts are not installed;
  no outside font or image loads). Screenshots: headless Edge
  (`msedge --headless=new --screenshot=... --window-size=1440,900 <file url>`); for 390 px wrap
  the page in an iframe (Edge's headless window has a minimum width).
- Agent worktrees live in `.claude/worktrees/` (git-ignored). Old `worktree-agent-*` and `wp/*`
  branches are merged; they can be deleted once you no longer need them.
- Multiple sessions share the scratchpad; use uniquely named commit-message files.

## 9. Keeping this file current

The owner asked for this file to track the project as it moves. After every merge or milestone:
update section 4 (state), section 5 (in flight: sessions, branches, worktrees) and section 6
(next steps), add a line to `docs/STATUS.md`'s log, commit, and push `build/integration`.
