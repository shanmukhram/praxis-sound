# Brief: WP-U6 drop a clip in the app: profile, matched A/B, library (Phase 5)

**Start building only after WP-D2 is merged** (it provides `ops::analyze` and the library); then
merge `build/integration` into your branch. Read first: `docs/BRIEFING.md`, PRD FR-11, FR-12,
FR-19, FR-30 and the Phase 5 exit ("profiles display for a music clip, an effect and an
ambience"), ADR-0011 (plain words, the style, depth on demand), `docs/contracts/sound-profile.md`,
the app (`app/src/ui/`, `app/src/engine/`), and the desktop shell `crates/praxis-app/src/`.

## Build

- **Drop a clip** (audio WAV/MP3/FLAC/OGG/M4A or video MP4/MOV/WebM) on the window or the
  composer's "Add a clip": the shell decodes it (ffmpeg for video, when present; a plain message
  when it is missing), analyses it and shows a **Profile view** in the Sound panel: what kind of
  clip it is, tempo and key (with "can't tell" when unsure), loudness, brightness, space, groove,
  sections, in plain words with small visuals (spectral balance bars, envelope, onset strip).
- **Loudness-matched A/B** between the reference clip and the current take (FR-30), through the
  real-time engine for the take and the file for the clip, levels matched by LUFS so the louder
  one does not win; a clear A/B toggle.
- **Reference library**: save a profile with a name and tags, list and reuse it ("make something
  like 'rain loop'" sends the profile to the request); profiles only, never audio.
- The request with a dropped clip passes the clip's profile to the assistant and the planner
  (the assistant gets a tool result describing it; matching itself is WP-M).

## You own

`app/src/ui/**`, `app/src/engine/` except `app/src/engine-wasm/**`, `app/tests/**`,
`app/screenshots/**`, `crates/praxis-app/**`.

## Rules

`PRAXIS_KEYCHAIN=off` for tests and app runs; never play audio (fake output, muted Chromium);
5 jobs / 5 test threads via the worktrees cargo config.

## Done when

Vitest and Playwright (1440 and 390) for drop, profile view, A/B and library; Rust tests for the
new shell commands; a manual run in the release app (fake output) with screenshots of the profile
view for a music clip, a sound effect and an ambience (make the clips with the engine; no
downloads); fmt, `cargo clippy -p praxis-app --all-targets -- -D warnings`, `cargo test -p
praxis-app`, tsc pass. Report as in docs/BRIEFING.md.
