# ADR-0004: Decoding without symphonia (licence)

Status: accepted

The PRD names symphonia. Its licence is MPL-2.0, a weak copyleft licence; the owner's rule is
"permissive licences only". Decision: decode natively with permissive crates and use ffmpeg as an
external program for the rest:
- WAV: hound (Apache-2.0); FLAC: claxon (Apache-2.0); OGG Vorbis: lewton (MIT/Apache);
  MP3: nanomp3 (MIT/Apache).
- M4A/AAC, MP4, MOV, WebM: `ffmpeg` executable run as a subprocess when present (not linked, not
  shipped). Without it, those formats report `ffmpeg_missing`.
If the owner accepts MPL-2.0 for unmodified dependencies, symphonia would give native AAC/MP4;
recorded in docs/NEEDS_OWNER.md.
