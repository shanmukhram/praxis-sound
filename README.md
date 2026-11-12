# Praxis Sound

A Rust sound-design workstation: a text brief or a dropped audio/video clip becomes editable,
layered audio (background music, scores, tonal loops and sound effects). Everything is a
**Score**; a planner and an analyzer write it, a procedural engine (samples, synthesis, effects)
renders it, so any result can be edited bar by bar and layer by layer.

Status: early build. See `docs/STATUS.md`.

## Build

```
cargo build --release
cargo test --workspace
```

## CLI

```
praxis render fixtures/empty.score.json -o out.wav
praxis validate fixtures/acceptance1.score.json
praxis schema --write
praxis --json ...        # one JSON object on stdout, see docs/contracts/cli.md
```

## Layout

| Path | What |
| --- | --- |
| `PRD.md` | requirements |
| `docs/PLAN.md`, `docs/STATUS.md` | build plan and current state |
| `docs/contracts/` | frozen interfaces between crates |
| `docs/adr/` | decisions |
| `crates/praxis-*` | the workspace crates |
| `schema/` | published JSON Schemas |
| `fixtures/` | hand-written scores |
| `demos/phase-N/` | rendered examples with listening notes |

## Licence

Praxis Sound is licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution you submit for inclusion
in the work, as defined in the Apache-2.0 license, is dual licensed as above, without any
additional terms or conditions.

Third-party components and their licences: `THIRD_PARTY.md` (the audit) and
`THIRD-PARTY-NOTICES.txt` (the licence texts shipped with the app; regenerate with
`python -I docs/release/notices.py`).
