# Instrument packs

Sampled instruments (SFZ or SoundFont 2 with WAV/FLAC samples) live here, one directory per
pack: `packs/<name>/pack.json` plus the pack's own files. Pack audio is **not committed**; only
this README and `index.json` are. See `docs/adr/0005-sample-packs.md` and
`docs/contracts/instruments.md`.

## `index.json`

Lists every downloadable pack. Per entry:

| Field | Meaning |
| --- | --- |
| `name` | directory under `packs/` (lowercase, digits, `-`, `_`) |
| `title`, `version` | human-readable name; upstream release |
| `license`, `license_file` | SPDX id, and the file inside the archive where it was confirmed |
| `source_page`, `url` | upstream page; direct archive URL (https only) |
| `archive`, `size`, `blake3` | file name in `packs/_downloads/`, exact byte size, blake3 hex |
| `strip_prefix` | leading directory inside the archive that is dropped when unpacking |
| `entry` | main SFZ/SF2 file, relative to the pack directory |
| `presets` | preset name -> entry file, written to `pack.json` |

## Installing

`praxis packs install <name>` (CLI, WP-K) calls
`praxis_instruments::sampler::install::install_pack`, which

1. downloads `url` into `packs/_downloads/<archive>` (or reuses a copy already there),
2. checks the exact size and the blake3 hash,
3. unpacks the 7z archive into a staging directory, rejecting any entry path that would leave
   the pack directory, checks that `entry`, `license_file` and every preset file exist,
4. writes `pack.json` and moves the directory into place as `packs/<name>/`.

Until the CLI command exists, the same code runs from the test suite:

```
cargo test -p praxis-instruments --test sampler_install -- --ignored install_upright_piano_kw_from_network
```

`install_from_archive` does steps 2-4 for an archive already on disk (used by tests and CI).
Set `PRAXIS_PACKS` to use a pack directory elsewhere. Tests that need a pack print `SKIPPED ...`
and pass when it is not installed.

## Packs

### `upright-piano-kw`: FreePats Upright Piano KW (2022-02-21)

- Licence: **CC0-1.0** (public domain dedication). Confirmed in the archive's `readme.txt`
  ("Published under the terms of Creative Commons CC0 public domain dedication") and the full
  legal text in `cc0.txt`, both installed to `packs/upright-piano-kw/`.
- Source: <https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html>; archive
  `UprightPianoKW-SFZ+FLAC-20220221.7z`, 33,294,287 bytes, blake3
  `1ae0bd73dc5618682bc7d06c9c9d4dfc72458a76d504657e7125b5365a5cc3d1`.
- Content: a Kawai upright recorded in a living room; 66 stereo 44.1 kHz FLAC samples, one zone
  per minor third from A0 to C8, two velocity layers (0..80, 81..127), sustain loops below A4.
  Decoded to f32 it takes about 115 MB, shared by every instrument that uses it.
- Presets: `felt_piano` (soft voicing, see `crates/praxis-instruments/src/sampler/presets.rs`)
  and `upright_piano` (as recorded), both from `UprightPianoKW-20220221.sfz`.
- The archive's blocks are DEFLATE-compressed; the installer decodes them with its own small
  inflater (`sampler/inflate.rs`) because the workspace builds `sevenz-rust2` without its
  `deflate` feature.
