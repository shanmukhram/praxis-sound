# ADR-0005: Sample formats and instrument packs

Status: accepted

- Formats: SFZ (v1 core opcodes) and SoundFont 2, both open. Samples WAV or FLAC.
- Default piano: FreePats **Upright Piano KW** (CC0-1.0, SFZ+FLAC, 32 MiB), voiced into the
  `felt_piano` preset (soft velocity layers, felt-hammer low-pass, long release). A true felt
  piano with redistribution rights may replace it later, but only a free one: the owner decided
  to buy nothing and make everything through code (2026-11-10).
- Pack audio is not committed to git. `packs/index.json` (committed) lists each pack's URL,
  size, licence and blake3; `praxis packs install` downloads, verifies and unpacks it into
  `packs/<name>/`. CI installs what tests need. Every pack is recorded in THIRD_PARTY.md.
- Owner approved downloading CC0/CC-BY packs and impulse responses under 100 MB each (2026-11-10).
