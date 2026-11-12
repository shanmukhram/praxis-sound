# Licence audit (WP-Z, 2026-11-12)

Praxis Sound is **MIT OR Apache-2.0** (owner's decision, 2026-11-12): `LICENSE-MIT`,
`LICENSE-APACHE`, `[workspace.package] license` in the root `Cargo.toml`. Everything it ships must
be distributable under those terms. The full component list with each licence is in
`THIRD_PARTY.md`; the licence texts that ship with the app are in `THIRD-PARTY-NOTICES.txt`.

## How it is checked

| Check | Command | Result |
| --- | --- | --- |
| Rust licences, sources, bans, advisories | `cargo deny check` (`deny.toml`, cargo-deny 0.20.2; 5 targets: Windows x64, Linux x64, macOS arm64 and x64, wasm32) | `advisories ok, bans ok, licenses ok, sources ok` |
| MPL-2.0 kept out of the core | `cargo tree -i <crate> --target all -e normal,build,dev --workspace` per MPL crate; `cargo tree -p <core crate> --target all -e normal,build,dev` | every MPL crate is reached only from `praxis-app`; 0 in praxis-score, -dsp, -engine, -instruments, -analyze, -match, -planner and praxis-wasm |
| Notices | `python -I docs/release/notices.py` | 342 crates in the shipped binaries (Windows x64), 1 npm package, 223 distinct licence texts |
| npm | `app/package-lock.json`, each package's `license` | 97 packages: 1 shipped (`@tauri-apps/api`, Apache-2.0 OR MIT); 96 dev-only (MIT, Apache-2.0, ISC, BSD-3-Clause; lightningcss MPL-2.0 is Vite's CSS minifier, a build tool) |
| Packs and samples | `packs/index.json`, `THIRD_PARTY.md` | one pack, Upright Piano KW (FreePats), CC0-1.0; no other sample or impulse response is shipped or downloaded (the algorithmic reverb and the convolution reverb's impulse responses are made in code; every other instrument is synthesised) |
| Installer | `target/release/nsis/x64/installer.nsi` | NSIS 3.13 stub (zlib/libpng) with **bzip2** compression; NSIS's LZMA module (CPL-1.0) is not used |

## Allowed licences (`deny.toml`)

MIT, MIT-0, Apache-2.0 (also WITH LLVM-exception), BSD-2-Clause, BSD-3-Clause, 0BSD, ISC, Zlib,
Unicode-3.0, Unicode-DFS-2016, CC0-1.0, Unlicense, BSL-1.0, CDLA-Permissive-2.0 (webpki-roots: the
CA list as data), bzip2-1.0.6 (libbz2's BSD-style licence). Anything else is refused: GPL, LGPL,
AGPL, SSPL, BUSL, non-commercial or no-derivatives licences. A crate offered under a choice of
licences passes when one allowed licence covers it (r-efi: `MIT OR Apache-2.0 OR LGPL-2.1-or-later`).

## Named exceptions: MPL-2.0 through Tauri

MPL-2.0 is file-level copyleft: it applies to those crates' own files, which are used unmodified
from crates.io (their source stays available there, linked in `THIRD-PARTY-NOTICES.txt`), and it
is compatible with an MIT OR Apache-2.0 app.

| Crate | Path | Shipped? |
| --- | --- | --- |
| cssparser 0.37.0, cssparser-macros 0.7.1, dtoa-short 0.3.5, selectors 0.38.0 | tauri-macros / tauri-codegen -> tauri-utils -> dom_query | no: `cargo tree -e normal --target x86_64-pc-windows-msvc -p praxis-app` reaches them only through the proc-macro `tauri-macros` (Tauri's build-time code generation) |
| option-ext 0.2.0 | tauri -> dirs 7 -> dirs-sys 0.5 | yes, inside `praxis-app.exe` |
| cssparser 0.38.0, selectors 0.41.0 | in `Cargo.lock` for targets this audit does not build | no (cargo deny reports the exceptions as not encountered) |

## Advisories

No vulnerability advisories. Unmaintained notices for transitive crates only, so `deny.toml` sets
`unmaintained = "workspace"`: zbus_names, zvariant, zvariant_derive (keyring's Linux
secret-service backend; merged into zbus 6) and proc-macro-error (gtk's macros, Linux build time).
They go away when keyring and Tauri's Linux stack move on; nothing on Windows uses them.

## Not shipped, recorded

- ffmpeg (LGPL/GPL): an optional external program for M4A/MP4/MOV/WebM, found through
  `PRAXIS_FFMPEG` or `PATH`, run as a subprocess, never linked or bundled.
- WebView2: part of Windows 11; when it is missing the installer downloads Microsoft's
  bootstrapper at install time (Microsoft's licence).
- Build tools: NSIS 3.13 and nsis_tauri_utils 0.5.3 (downloaded and hash-checked by the Tauri CLI
  from Tauri's GitHub releases), cargo-deny.

## Not a legal opinion

This audit checks declared licences (SPDX expressions in each package and the licence files it
ships). It does not replace a lawyer's review before a commercial or public release.
