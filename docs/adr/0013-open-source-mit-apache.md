# ADR-0013: Open source under MIT OR Apache-2.0

Status: accepted (owner, 2026-11-12). Supersedes ADR-0012 (commercial licence) and ADR-0008.

The owner decided: "lets make this opensource forget our paid stuff", and chose the licence
"MIT OR Apache-2.0".

## Decision

- Praxis Sound is open source, dual-licensed **MIT OR Apache-2.0** (the Rust convention): users
  may pick either. The repository root has `LICENSE-MIT` and `LICENSE-APACHE`; every crate
  declares `license = "MIT OR Apache-2.0"` through the workspace.
- **Dependencies** stay compatible with distributing the whole under MIT/Apache: permissive
  licences only (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, CC0, Unlicense, BSL-1.0, 0BSD). GPL,
  LGPL, AGPL, SSPL, BUSL and non-commercial licences are refused (`cargo deny check`). GPL projects
  such as Surge XT or Vital can be studied, not copied.
- **MPL-2.0** (seven crates, only through Tauri in the desktop shell) is compatible: file-level
  copyleft on those unmodified files; their source is on crates.io.
- **Notices**: every dependency's licence and the piano pack's CC0 notice ship with the app in
  `THIRD-PARTY-NOTICES.txt`.
- **Assets**: sample packs and impulse responses only under CC0 or attribution-only licences
  (attribution in THIRD_PARTY.md and the notices). ffmpeg stays an optional external program.
- **Claude**: users bring their own Anthropic API key; nothing in the repository holds a key.

## Going public

The whole history was checked for secrets and personal data (none: every commit uses the GitHub
no-reply address, no key or token in any version of any file). The repository
(`shanmukhram/praxis-sound`) was made public on 2026-11-12 at the owner's request. Public
repositories get free GitHub Actions minutes, so CI can run fully again.
