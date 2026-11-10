# Contract: instruments, presets and packs

Frozen: `crates/praxis-instruments/src/registry.rs`, `packs.rs`, `lib.rs`; the `Instrument`
traits in `crates/praxis-engine/src/instrument.rs`.

Each family module (`sampler.rs`, `synth.rs`, `drums.rs`, `sfx.rs`, each owned by one package)
exposes `pub fn presets() -> Vec<PresetEntry>` and may put submodules in a directory of the same
name (`src/sampler/sfz.rs` ...).

## Preset names (stable API; scores refer to them)

| Kind | Preset | Class | Owner | Notes |
| --- | --- | --- | --- | --- |
| sampler | `felt_piano` | motif | WP-C1 | Upright Piano KW (CC0) voiced soft: low velocity layers, felt hammer low-pass, long release, soft pedal noise optional |
| sampler | `upright_piano` | melody | WP-C1 | same pack, natural voicing |
| synth | `bell` | motif | WP-C2 | struck/FM bell; the Phase 1 test tone |
| synth | `soft_keys`, `pluck`, `mallet`, `sub_bass`, `warm_bass`, `fm_keys`, `glass`, `granular_texture`, `noise_bed` | various | WP-C2 | `pad`-class presets exist but are blocked unless asked |
| drum | `soft_kick` | drums | WP-C2 | soft, muted kick: short sine sweep + felt click, low-passed |
| drum | `kick`, `snare`, `rim`, `hat_closed`, `hat_open`, `shaker`, `tom_low`, `tom_mid`, `tom_high`, `clap` | drums / percussion | WP-C2 | synthesised; GM note numbers select voices in `kit` |
| drum | `kit` | drums | WP-C2 | multi-voice kit keyed by GM drum map |
| sfx | `whoosh`, `impact`, `riser`, `drone`, `glitch`, `tick` | same-named class | WP-H | macros: `length`, `brightness`, `weight`, `pitch`; tuned to `ctx.key` |

## Packs

`packs/<name>/pack.json` (`PackManifest`: name, version, SPDX licence, source URL, attribution,
preset -> entry file). Pack audio is **not committed**; `packs/index.json` (committed) lists
download URL, size and blake3 of each archive, and `praxis packs install <name>` fetches,
verifies and unpacks it. Tests that need a pack skip with a clear message when it is absent, and
CI installs the packs it needs first. Every pack is recorded in `THIRD_PARTY.md`.

## Sampler requirements (WP-C1)

- SFZ v1 core opcodes: `<region> <group> <global> <control>`, `sample`, `lokey/hikey/key`,
  `pitch_keycenter`, `lovel/hivel`, `tune`, `transpose`, `volume`, `pan`, `amp_veltrack`,
  `ampeg_attack/hold/decay/sustain/release`, `offset`, `loop_mode/loop_start/loop_end`,
  `trigger=release` (release samples), `rt_decay`, `seq_length/seq_position` (round robin, seeded),
  `default_path`, `#define`, `#include`.
- SoundFont 2: parse RIFF `sfbk`, `pdta` presets/instruments/zones/generators (key/vel ranges,
  root key, tuning, envelope, loops), `sdta` 16-bit samples.
- Samples: WAV (hound) and FLAC (claxon), decoded at load, resampled at playback with 4-point
  Hermite or better; voice pitch from the nearest root key.
- Velocity crossfade between layers; release samples; per-voice seeded micro-variation only via
  the note seed.
