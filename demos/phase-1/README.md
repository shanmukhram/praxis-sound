# Phase 1 demos: engine core

Phase 1 exit: the hand-written Score of acceptance test 1 (`fixtures/acceptance1_bell.score.json`)
renders with a simple test bell tone, with a clean loop seam, bit-identical across reruns.

| File | What it is |
| --- | --- |
| `acceptance1_bell_loop.flac` | The 30-second loop: 16 bars of 4/4 at 128 BPM in A minor. Bars 1-8 are the bell motif alone; bars 9-16 are the same motif with a soft kick underneath. |
| `acceptance1_bell_loop_x3.flac` | The same loop played three times back to back, so you can hear the seam at 0:30 and 1:00. |

## What to listen for

- **Rhythm**: notes land on an even eighth-note grid, a repeating two-bar motif with accents
  (strong first beat, soft in-between notes, brighter accent on beat 3).
- **Connection**: notes overlap and ring into each other rather than stopping short.
- **Layering**: at 0:15 (bar 9) the kick enters under the unchanged motif. Nothing else changes.
- **Seam**: at 0:30 and 1:00 in the x3 file, the end flows into the start with no click, no gap
  and no fade; the ring of bar 16 continues under bar 1.

## What NOT to judge yet

The bell is a test tone, deliberately simple, not a product instrument. There is no reverb, EQ,
loudness normalisation or limiter yet (Phase 2). The real felt piano and the real soft kick come
in Phase 2.

## Measurements (2026-11-10)

| Check | Result |
| --- | --- |
| Length | exactly 30.000 s (1,440,000 samples at 48 kHz) |
| Determinism | two independent renders: master and both stems bit-identical (sha256 equal) |
| Loop seam (own analysis) | jump across the wrap 0.12x the typical neighbouring jump; second difference at the wrap below the local maximum; level at sample 0 is the previous cycle's ring, not silence |
| Tempo / key (earshot meter) | ~129.2 BPM (score 128, within the 2 BPM tolerance), A minor (matches) |
| Loudness | -17.8 LUFS integrated, true peak -5.7 dBTP, 0 clipped samples (normalisation arrives in Phase 2) |
| Render speed (release) | 88x real time for this piece; 22x for a 2-minute 8-layer piece |
