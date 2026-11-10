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
