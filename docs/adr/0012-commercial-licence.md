# ADR-0012: Closed-source commercial licence, one-time purchase

Status: accepted (owner, 2026-11-12). Supersedes the "pending" part of ADR-0008.

The owner decided: "we want to ship it someday with a one time payment for the app."

## Decision

- Praxis Sound is proprietary software sold for a one-time payment. Crates declare
  `license = "LicenseRef-Proprietary"`; the repository has a `LICENSE` stating all rights reserved.
- **Dependencies** must allow closed-source commercial distribution. Allowed: MIT, Apache-2.0,
  BSD-2/3-Clause, ISC, Zlib, Unicode, CC0-1.0, Unlicense, BSL-1.0, 0BSD. Refused: GPL, LGPL, AGPL,
  SSPL, BUSL and any non-commercial or no-derivatives licence. `cargo deny check` enforces it.
- **MPL-2.0** (seven crates, only through Tauri in the desktop shell): allowed as named exceptions.
  MPL is file-level copyleft: selling a closed-source app is fine as long as those files are not
  modified and their source is available; the third-party notices say where (crates.io).
- **Notices**: every dependency's licence text and every pack's licence ship with the app in
  `THIRD-PARTY-NOTICES.txt`, reachable from the app.
- **Assets**: sample packs and impulse responses only under CC0 or attribution-only licences;
  nothing non-commercial. ffmpeg stays an optional external program, never bundled.
- **Claude**: users bring their own Anthropic API key (FR-34); the app ships no key.

## Still the owner's (not code)

- An end-user licence agreement (EULA) for the sale, the store or payment provider, and a
  trademark check of "Praxis Sound" (ADR-0001). These are legal and business documents for the
  owner to get written; nothing in the code depends on them.
