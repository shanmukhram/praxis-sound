# Third-party components

Every dependency and asset, with licence and the check made before adopting it. Permissive
licences only (ADR-0008). Checked against crates.io on 2026-11-10.

## Rust crates (direct)

| Crate | Version | Licence | Last release | WebAssembly | Used by |
| --- | --- | --- | --- | --- | --- |
| serde | 1.0.229 | MIT OR Apache-2.0 | 2026-07 | yes | all |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | 2026-10 | yes | all |
| schemars | 1.2.3 | MIT | 2026-10 | yes | score, analyze, planner |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | 2026-09 | yes | libraries |
| anyhow | 1.0.104 | MIT OR Apache-2.0 | 2026-07 | yes | cli |
| clap | 4.6.7 | MIT OR Apache-2.0 | 2026-09 | n/a (cli) | cli |
| hound | 3.5.1 | Apache-2.0 | 2023-09 (stable format, widely used) | yes | audio-io, instruments, testkit |
| claxon | 0.4.3 | Apache-2.0 | 2020-08 (FLAC decoding, format stable) | yes | audio-io, instruments |
| lewton | 0.10.2 | MIT OR Apache-2.0 | 2021-01 (Vorbis, format stable) | yes | audio-io |
| nanomp3 | 0.2.0 | MIT OR Apache-2.0 | 2026-09 | yes | audio-io |
| flacenc | 0.5.1 | Apache-2.0 | 2025-12 | yes | audio-io |
| midly | 0.5.3 | Unlicense | 2023-01 (SMF format stable) | yes | audio-io |
| rustfft | 6.4.1 | MIT OR Apache-2.0 | 2025-09 | yes | analyze |
| realfft | 3.5.1 | MIT | 2026-10 | yes | analyze, dsp (convolution) |
| rubato | 5.0.1 | MIT OR Apache-2.0 | 2026-10 | yes | audio-io |
| ebur128 | 0.1.11 | MIT | 2026-10 | yes | analyze (reference meter) |
| cpal | 0.18.2 | Apache-2.0 | 2026-08 | partial (desktop use only) | audio-io `device` feature |
| rtrb | 0.4.1 | MIT OR Apache-2.0 | 2026-10 | yes | engine (real-time queues) |
| ureq | 3.5.1 | MIT OR Apache-2.0 | 2026-11 | no (desktop only, `claude` feature) | planner |
| keyring | 4.2.0 | MIT OR Apache-2.0 | 2026-08 | no (desktop only, `claude` feature) | planner |
| blake3 | 1.8.7 | CC0-1.0 OR Apache-2.0 | 2026-08 | yes | testkit, analyze |
| sevenz-rust2 | 0.25.0 | Apache-2.0 | 2026-11 | yes | pack install (7z archives) |
| wasm-bindgen | 0.2.130 | MIT OR Apache-2.0 | 2026-10 | yes | wasm |
| proptest (dev) | 1.12.0 | MIT OR Apache-2.0 | 2026-10 | n/a | tests |
| jsonschema (dev) | 0.61.1 | MIT | 2026-11 | n/a | tests |

Rejected: symphonia 0.6.1 (MPL-2.0, weak copyleft; ADR-0004). fundsp 0.23 (not needed; ADR-0003).

## Instrument packs and impulse responses

| Pack | Licence | Source | Size | Used for |
| --- | --- | --- | --- | --- |
| Upright Piano KW (FreePats), SFZ+FLAC 2022-02-21 | CC0-1.0 | https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html | 32 MiB | `felt_piano`, `upright_piano` (to be installed by WP-C1) |

## External programs (not shipped, optional)

| Program | Licence | Use |
| --- | --- | --- |
| ffmpeg | LGPL/GPL (separate executable, invoked as a subprocess) | decoding M4A/AAC/MP4/MOV/WebM when installed |
