# Brief: WP-D2 Sound Profile, clip classification and reference library (Phase 5)

Branch from `build/integration`. Read first: `docs/BRIEFING.md`, PRD FR-11..FR-14, FR-18, FR-19 and
the Phase 5 exit, `docs/contracts/sound-profile.md` (the whole profile table), the frozen types in
`crates/praxis-analyze/src/profile.rs`, the existing features in `crates/praxis-analyze/src/features/`
(tempo, key, onset, chroma, loudness, seam, attack), `verify.rs` (do not change its behaviour), the
decoders in `crates/praxis-audio-io/src/decode.rs`, and the CLI's interim `ops/analyze.rs`.

## Build

- **The profile**, every field in contracts/sound-profile.md: tempo + beats, key, loudness/LRA/true
  peak/crest, envelope (ADSR fit), spectral (centroid, rolloff, flatness, 10 octave bands, tilt,
  centroid over time), groove (onsets folded on the grid, swing), pitch (YIN), tonality, stereo
  (side/mid, correlation), reverb decay, sections (novelty over a chroma+MFCC self-similarity
  matrix), class (music / sfx / ambience), and the fixed `features` vector with `FEATURE_NAMES`
  filled (append-only from now on). `source.pcm_hash` is blake3 of the decoded PCM. Pure analysis
  in `praxis-analyze` (wasm-safe: no threads, no time, no files).
- **Similarity and originality** (`similarity.rs`, `originality.rs`) as the contract says:
  character similarity 0..1 over normalised features; originality check comparing interval
  sequences of two pitch tracks (n-gram overlap + DTW, transposition-invariant), with a threshold
  calibrated on a test set you build (close copies vs unrelated melodies) and documented.
- **Labelled test set** (`crates/praxis-analyze/tests/clips/`): self-made, because the owner buys
  nothing and has no labelled clips. Generate clips in the test from scores rendered by the real
  engine: tempos 60-180 BPM, all 24 keys, several instruments (felt piano, synths, drums), with and
  without reverb, noise and level changes, at several lengths; plus sound effects (the sfx presets)
  and ambience (noise beds, textures) for classification. Commit only the generating scores and
  `labels.json`; no audio in git. The accuracy test runs in CI: tempo within 2 BPM and correct key
  on at least 90% (PRD success criterion), and classification accuracy reported (target 90%).
- **CLI and library**: `praxis analyze <clip> [-o profile.json]` returns the full profile
  (`complete: true`); a reference library under `PRAXIS_HOME/library/` with `praxis library
  add|list|show|tag|remove` (profiles only, never audio, FR-18/FR-19); MCP `praxis_analyze`
  returns the full profile and `praxis_library` mirrors the CLI.

## You own

`crates/praxis-analyze/src/` except `verify.rs` behaviour and the frozen types in `profile.rs`
(you fill `FEATURE_NAMES` and may add modules, declared in `lib.rs`), `crates/praxis-analyze/tests/**`,
`crates/praxis-cli/src/ops/{analyze,library}.rs` plus their lines in `ops/mod.rs` and `main.rs`,
`crates/praxis-cli/tests/` new files, `crates/praxis-mcp/src/` for `praxis_analyze` / `praxis_library`,
and the profile and library sections of `docs/contracts/{sound-profile,cli}.md`. If a frozen type
must change, stop and propose it.

## Rules

`PRAXIS_KEYCHAIN=off` for every test and binary run (the owner's paid key is in the keychain);
never play audio; 5 jobs / 5 test threads via `.claude/worktrees/.cargo/config.toml`, never
override; `PRAXIS_PACKS=C:\Users\admin\Desktop\praxis sound\packs`.

## Done when

The labelled-set test passes (tempo +-2 BPM and key on >= 90%; report the numbers and the
confusion cases); profiles for a music clip, a sound effect and an ambience render plausibly
(report them); originality flags close copies and passes unrelated melodies on its test set; fmt,
workspace clippy, `cargo test -p praxis-analyze -p praxis-cli -p praxis-mcp --release`, acceptance
1 and the wasm build pass. Report as in docs/BRIEFING.md.
