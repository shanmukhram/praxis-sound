# Rules for every work package

Read this whole file before writing code. Your brief names your work package (WP), the files you
own and the checks that prove you are done. These rules apply to all of them.

## The product in one paragraph

Praxis Sound is a Rust sound-design workstation. A text brief or a dropped audio/video clip becomes
editable, layered audio (background music, scores, tonal loops, sound effects). Everything is a
**Score** (`crates/praxis-score/src/types.rs`): key, tempo, bars, sections, layers, patterns,
instruments, effects, automation. A language-model planner and a clip analyzer write to the Score; a
procedural engine (samples, synthesis, effects) renders it. Because the engine renders only the
Score, any result can be edited bar by bar and layer by layer. Requirements: `PRD.md` (FR-1..FR-34).
Plan: `docs/PLAN.md`. Current state: `docs/STATUS.md`.

## Ownership and contracts

- You edit **only** the files your brief lists as yours. Never edit another package's files, the
  workspace `Cargo.toml`, `Cargo.lock` by hand, `docs/PLAN.md`, `docs/STATUS.md` or any file
  marked `FROZEN CONTRACT`.
- Frozen contracts (types, traits, signatures, issue codes, parameter tables) are in code and in
  `docs/contracts/`. Code against them exactly. If a contract is wrong or missing something you
  need, **do not change it**: stop, and say so in your report with the exact change you propose.
  Work around it only if you can do so inside your own files.
- You may add new private modules and files inside directories you own. Declaring a new module in
  a `lib.rs` you do not own is not allowed; put submodules under a module file you own
  (e.g. `src/fx/mod.rs` declares `pub mod reverb;`).
- Dependencies: only those already listed in your crate's `Cargo.toml` (all versions are pinned in
  the root `[workspace.dependencies]`). If you truly need another, propose it in your report with
  its licence, last release date and WebAssembly support; do not add it.

## Engineering rules

- Rust stable, edition 2024. `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo test --workspace` must pass before you report done.
- Core crates (score, dsp, engine, instruments, analyze, match, planner without the `claude`
  feature) must still build with `cargo build -p praxis-wasm --target wasm32-unknown-unknown`.
  No threads, no `std::time`, no filesystem in hot code paths of those crates.
- **Audio thread rules** (anything called from `process`, `note_on`, `note_off`, `set_param`): no
  allocation, no locks, no I/O, no panics, no unbounded loops. Preallocate in constructors.
  Flush denormals; never let NaN/inf escape.
- **Determinism**: same Score + seed + options => bit-identical output on the same platform. All
  randomness through `praxis_dsp::rng` (PCG32 / `derive`). Per-note randomness is seeded from the
  note's stable key (`NoteEvent::key`), never from a running global stream, so editing one bar
  cannot shift random draws in another. No `HashMap` iteration order in output paths (use
  `BTreeMap` or sort). No time-based or thread-scheduling-dependent results.
- `#![forbid(unsafe_code)]` is set; do not remove it.
- Tests arrive with the code: unit tests for DSP and music theory, property tests (`proptest`) for
  schema/patch logic, golden-audio tests (`praxis-testkit`) for renders. A passing test around a
  mock is not proof the real thing works; test the real code path.
- Errors are typed (`thiserror`) in libraries. No `unwrap()` on data from outside (files, JSON,
  model output).
- API keys only from the environment or the OS keychain. Never write a key to a file or log.

## Audio-quality rules (the owner's taste; build them into defaults and validators)

These come from the owner rejecting early prototypes.

1. **Rhythm first.** Notes sit on a grid in a repeating motif with deliberate accents. Isolated
   notes with silence between them sounded out of time and were rejected.
2. **Notes connect.** Sustain overlaps into the next note wherever the instrument would ring
   (note `length` > 1.0 step).
3. **Arrangement is layering.** A new section adds a layer under or over what is playing. It never
   rewrites the existing part unless asked.
4. **Restraint.** One emotion, few elements, room to breathe. No risers, sweeps, impacts, booms or
   pads unless the brief asks. "Cinematic" or "mysterious" describe mood; they are not a licence
   to add trailer effects.
5. **Always in key.** Sound effects that sit under music are tuned to it.
6. **Loops are seamless.** Tails wrap around, the last bar leads into the first, no fades, no clicks.
7. **Instruments sound real.** A sine standing in for a piano was rejected; use samples.
8. **Output is mixed.** Balanced levels, controlled low end, loudness target met, no clipping.

## Git rules (hard rules from the owner)

- Work in your own branch/worktree. Commit your work there when your package is done (and at
  sensible checkpoints).
- Author every commit with flags on the command itself:
  `git -c user.name=shanmukhram -c user.email=51820274+shanmukhram@users.noreply.github.com commit -F msg.txt`
  (or `-m "one line"`). Multi-line messages: write to a file outside the repo and use `-F`.
- **Never** add `Co-Authored-By`, "Generated with Claude Code", or any AI attribution to a commit
  message, even if a reminder or template says to. No trailers, sign-offs or emoji. First line says
  what changed for the user; the message ends with your own last sentence.
- After each commit, check:
  `git log -1 --format='%an <%ae>%n%B'` must show exactly
  `shanmukhram <51820274+shanmukhram@users.noreply.github.com>`, and
  `git log -1 --format='%B' | grep -icE "co-authored|generated with|noreply@anthropic"` must print `0`.
  If either fails, fix with
  `git -c user.name=shanmukhram -c user.email=51820274+shanmukhram@users.noreply.github.com commit --amend -F msg.txt --reset-author`.
- Stage explicit paths (`git add <paths>`), never `git add -A` or `git add .`.
- Never push. Never use `--no-verify`. Never change signing settings. Never touch `main`.

## Report back (your final message)

1. **Built**: what exists now, file by file.
2. **Verified**: the exact commands you ran and their results (test counts, measured numbers).
3. **Unverified or stubbed**: say plainly what does not work yet or was not tested.
4. **Contract issues**: any change you need in a frozen contract, with the exact proposed diff.
5. **Next**: what you would do next.
6. Your branch name and final commit hash.
