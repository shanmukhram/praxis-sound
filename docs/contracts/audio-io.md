# Contract: audio-io

Frozen: `crates/praxis-audio-io/src/wav.rs` (`write_wav`, `read_wav`, `WavFormat`), `lib.rs`.

## WP-E provides

- `decode::decode_file(path) -> Result<Decoded { audio: StereoBuffer, sample_rate, channels,
  duration_s, source_kind: Audio|Video }, DecodeError>`: WAV (hound), FLAC (claxon), OGG Vorbis
  (lewton), MP3 (nanomp3) natively; M4A/AAC, MP4, MOV, WebM and anything else by running
  `ffmpeg -i <file> -vn -ac 2 -f f32le -ar 48000 pipe:1` when an `ffmpeg` executable is on PATH
  (or `PRAXIS_FFMPEG`). Clear error `ffmpeg_missing` otherwise. Format detection by magic bytes,
  not extension.
- `decode::resample(&StereoBuffer, from, to)` with rubato (deterministic, offline).
- `encode::write_flac(path, &StereoBuffer, sr, bits 16|24)` with flacenc.
- `encode::write_stems(dir, &[Stem], sr, format)`.
- `midi::score_to_midi(&Score) -> Vec<u8>`: SMF type 1, 480 PPQ, one track per layer (drums on
  channel 10), tempo and time signature meta events, note velocities from the expanded events.
- `device` (feature `device`, Phase 3): cpal output stream driving a callback that pulls from the
  real-time engine; buffer size request (256 default); device listing.
