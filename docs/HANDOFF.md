# Handoff: Praxis Sound build (kept current; last updated 2026-11-11: wave 3 running, see section 5)

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
Phase 2 closed (owner sign-off 2026-11-11). Phase 3 in progress.

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

## 5. In flight (updated 2026-11-11: wave 3 running as four sessions)

Old merged branches and worktrees were deleted; only `build/integration` and the four wave-3
worktrees exist. Every wave-3 package works in its own worktree under `.claude/worktrees/` on the
branch below (the app may add a prefix/suffix: check `git branch`). Briefs are in `docs/briefs/`.

| WP | Session | Branch | Brief | Notes |
| --- | --- | --- | --- | --- |
| WP-K2 edits, variations, takes | `local_08d46492-ce2a-48b7-8784-c8906f5e6215` (wp-k2-takes) | **merged** (c84b212) | WP-K2-edits-takes.md | command layer + CLI + MCP; shared take history; frozen gain; CLI/planner tidy-ups (follow-ups 2-4) |
| WP-D1b onset robustness | `local_c98b80e0-ef63-4d51-ab55-616e42b9e51f` (wp-d1b-onsets) | `claude/d1b-onsets-39c1f1` | WP-D1b-onset-robustness.md | **merged** (6b744c9): grid 100% |
| WP-R real-time engine | `local_86d87740-e7d6-4839-94ac-5a0e5ef22ca7` (wp-r-realtime) | **merged** (1261d59) | WP-R-realtime.md | Phase 3 |
| WP-U3 Tauri app | `local_c4804bca-8078-46c7-bef7-1ff48c2a7379` (wp-u3-adapter) | **merged** (e5d6bc6) | WP-U3-real-adapter.md | **waits on WP-K2**: when K2 is merged, message this session to merge `build/integration` and wire the take ops |

**Build limit (owner, 2026-11-11): at most 10 of 20 threads for builds in total.** Four parallel
full builds made the PC lag, so all sessions were stopped once. Now only two sessions compile at
a time; `.claude/worktrees/.cargo/config.toml` (git-ignored, not in the repo; recreate it if
missing) caps every session worktree at `jobs = 5` and `RUST_TEST_THREADS = 5`. The
orchestrator does not run its own cargo builds while two sessions are building.
Build speed-ups (local, not committed): sccache 0.19 (winget `Mozilla.sccache`, binary under
`%LOCALAPPDATA%\Microsoft\WinGet\Packages\Mozilla.sccache_*\`) as `build.rustc-wrapper`, and
`rust-lld.exe` as the msvc linker, set in each new worktree's own `.cargo/config.toml` (done for
WP-R and WP-U3; WP-K2 and WP-D1b were mid-build and kept the defaults). Once a package builds
cleanly with them, move both settings into `.claude/worktrees/.cargo/config.toml` for all.
Dev Drive is the owner's call (not set up); Cranelift was rejected (tests run optimised). Running now:
WP-P2b tone and space edits (released to build). Merged in wave 3: WP-D1b, WP-R, WP-K2, WP-U3,
WP-U4 (real-time playback in the app) and WP-H (sound-effect presets). WP-P2b is `local_0bb0fb8b-9b03-4896-a09b-59cd7c6a1d28` (brief
`docs/briefs/WP-P2b-tone-space-edits.md`). Waiting for a slot (they read and write code but must
not build until messaged "slot free"): WP-AG sound assistant agent (`local_a5e4c36f-0854-402a-ae61-2d057494b210`, brief
`docs/briefs/WP-AG-in-app-agent.md`) and WP-U5 build a sound by hand
(`local_ef4aaa95-5591-45a4-976b-056f5cfe01de`, `docs/briefs/WP-U5-hand-building.md`, closes
Phase 3). Release order: WP-P2b (building now), then WP-AG, then WP-U5. Never play audio on the
owner's speakers without asking (owner was upset once; see memory and the briefs). After
WP-AG merges, ask the owner for the API key (`praxis key set`, typed by the owner, stored in the
Windows credential store) and run `docs/agents/live-run.md`.

Merge order: WP-D1b and WP-R whenever they are ready; WP-K2 before finishing WP-U3. WP-U3 and
WP-R both may touch `Cargo.lock` (and U3 the root `Cargo.toml`): resolve by keeping both
sides' entries and letting cargo regenerate the lock.

- **Phase 2 closed**: the owner signed off `demos/phase-2/acceptance1_mixed_loop.flac` on 2026-11-11.

Known follow-ups not yet assigned: none beyond the phases in section 6. (Follow-ups 1-5 from
the pause are in WP-D1b, WP-K2, or done: old branches/worktrees deleted.)

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

Done: Phases 0, 1 and 2 (owner signed off the Phase 2 demo 2026-11-11); WP-A, B1, B2, C1, C2, D1, E, F,
F2, K, P, P2, U1, U2, X all merged.

1. **Phase 2 sign-off**: done 2026-11-11 (owner: "its perfect"). Phase 2 is closed.
2. **Wave 3 (running, section 5)**: review and merge WP-D1b, WP-R and WP-K2 as each reports;
   after WP-K2 merges, message the WP-U3 session to merge `build/integration` and finish the
   wiring; then review and merge WP-U3.
3. **Phase 3 exit**: build acceptance 1 by hand in the window, play it through the real-time
   engine (WP-R wired into the app: a small follow-up package replacing `<audio>` playback),
   export; the owner's real-hardware check (NEEDS_OWNER item 3, `play` example from WP-R).
4. Run the full workspace suite and CI at the Phase 3 exit.
5. Phase 4 prerequisites are done offline (edits, variations, isolation); only the live run remains.
6. **Phase 4 exit**: a live Claude run of the prompt and edit suites. Ask the owner for the
   Anthropic API key at that point (not before).
7. **Phase 5** WP-D2 Sound Profile + labelled test set; **Phase 6** WP-H SFX patches + WP-M
   match loop; **Phase 7** WP-P3 scoring depth; **Phase 8** WP-W wasm/browser + WP-Z performance
   and release, per `docs/PLAN.md`.

Verification preference (owner): after a package, run only its new or affected tests plus the
acceptance test; run the full workspace suite at phase exits or when shared code changes.

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
