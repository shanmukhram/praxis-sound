# Handoff: Praxis Sound build (kept current; last updated 2026-11-11: wave 4 running, see section 5)

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
| CI | `.github/workflows/ci.yml`. Owner decision 2026-11-11 (free minutes ran out): code pushes run Windows release tests only, docs-only pushes run nothing, the full Linux/Windows/macOS + debug + wasm check runs by hand (`gh workflow run ci --ref build/integration`) at phase exits and before releases. Runs stay blocked until the monthly free minutes reset. `gh run list --branch build/integration` |
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

## 5. In flight (updated 2026-11-11: wave 4, Phases 4-8)

The owner said to finish the project without routine questions and approved tool downloads
(docs/NEEDS_OWNER.md). Ask only for PRD sign-offs (Phase 6 blind review, Phase 7 listening
review), large paid spend, and anything that plays audio on the owner's speakers.

At most two sessions compile at once (10 of 20 threads; `.claude/worktrees/.cargo/config.toml`
caps each at 5 jobs and adds sccache + rust-lld). Queued sessions read and write code but must not
build until messaged "slot free" (the dependent ones also wait for a named merge). Session notes only wake the orchestrator when no other started session is working, so every
session is told to SendMessage the orchestrator (`local_b37ecbec-7aed-4686-af91-6fbce30705d2`) when it finishes or needs
something, and the orchestrator subscribes with `notify_when_idle`. Session starts
are refused in automated turns, so every package was started in one owner turn; later steps are
messages to these sessions.

| WP | Session | State | Brief | Release when |
| --- | --- | --- | --- | --- |
| WP-P4 live planner quality | `local_9d36f82c-3389-4b79-bcd5-49ee65dc6bfb` | **merged**, Phase 4 closed | WP-P4-live-planner-quality.md | (running) |
| WP-D2 Sound Profile + library | `local_d4822b8f-57b1-47ae-b93b-d705b8a08ca4` | **merged** | WP-D2-sound-profile.md | (running) |
| WP-H2 realistic sfx | `local_e181048d-d661-4de8-9bd3-62cfb7c11812` | **merged** | WP-H2-sfx-realism.md | first free slot |
| WP-P3 scoring depth + hit points | `local_26d5ea83-cf3a-4e26-9484-d34526546f88` | **merged**, Phase 7 closed | WP-P3-scoring-depth.md | after WP-P4 merges: "WP-P4 merged, slot free" |
| WP-U6 reference clips in the app | `local_2c567f48-72fa-4ff1-8380-8e5ee1c2cc5f` | **merged**, Phase 5 closed | WP-U6-reference-clips-ui.md | after WP-D2: "WP-D2 merged, slot free" |
| WP-M match loop | `local_c25d0d15-36a9-439c-84d9-450faee7771e` | **merged**, Phase 6 closed | WP-M-match.md | after WP-D2 and WP-H2: "WP-D2 and WP-H2 merged, slot free" |
| WP-W browser build | `local_1eaf0bdd-3166-4fc0-bbf7-2f97f3dea4ba` | **merged** | WP-W-browser.md | after WP-U6: "WP-U6 merged, slot free" |
| WP-Z performance + release | `local_8d511f36-2c98-42bf-b40e-155091985439` | building | WP-Z-performance-release.md | last: "all packages merged, slot free" |

Rules for every review: authorship + attribution checks, owned files only, rerun the package's
tests with `PRAXIS_KEYCHAIN=off` (the owner's paid key is in the keychain) and nothing playing
audio (Playwright through a temporary muted config), then merge with `--no-ff`, push, and rebuild
the release app (`cd app && npm run tauri -- build --no-bundle`) when app code changed.

Closed: Phases 0-3 (owner sign-offs for 2 and 3). Phase 4: assistant live run passed; planner
suites live 15/27 briefs, 19/20 edits (WP-P4 is closing it). Sounds live in
`C:\Users\admin\Praxis Sound` (Windows Controlled Folder Access blocks Documents).

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
