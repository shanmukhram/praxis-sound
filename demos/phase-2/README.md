# Phase 2 demos: real instruments, effects and mix

**Phase 2 exit, for the owner's listening sign-off:** acceptance test 1 renders with the sampled
felt piano and the soft kick, mixed to its loudness target.

| File | What it is |
| --- | --- |
| `acceptance1_mixed_loop.flac` | **The one to judge.** Planned from one brief by the offline planner and rendered by the engine with real instruments and the mix bus. 30-second loop, 16 bars of 4/4 at 128 BPM in A minor. Bars 1-8: felt piano alone. Bars 9-16: the same piano part with a soft kick underneath. |
| `acceptance1_mixed_loop_x3.flac` | The same loop played three times back to back, so you can hear the seams at 0:30 and 1:00. |
| `acceptance1.score.json` | The Score it was rendered from. |
| `preview_felt_piano_only.flac` | Earlier preview: the piano alone, completely dry (no room, no mix), for comparison. |

## What to listen for

- **Real piano**: a soft, felt-hammer upright, not a synth tone.
- **On the beat**: an even eighth-note motif, strong on the first beat, bright on beat 3. Nothing
  late or loose.
- **Notes connect**: each note rings into the next.
- **Layering**: at 0:15 (bar 9) a soft, muted kick enters under the unchanged piano. No new melody,
  no fills.
- **Room**: a small room around the piano that builds up naturally. Listen for any metallic ring
  in the tail. The reverb's pitch modulation is off, to keep the loop exactly periodic, so tell us
  if it rings.
- **Restraint**: no risers, sweeps, impacts or pads anywhere.
- **Seam**: in the x3 file, 0:30 and 1:00 flow straight on, with no click, gap or fade.
- **Level**: comfortable, not squashed, no distortion.

## Measurements (praxis check, 2026-11-11)

| Check | Result |
| --- | --- |
| Tempo | 127.99 BPM (Score: 128) |
| Key | A minor |
| Grid | 90.9% of 132 onsets within 10 ms of the eighth grid, median deviation 1.5 ms |
| Loudness | -16.00 LUFS integrated (target -16.0 +- 0.5) |
| True peak | -2.72 dBTP (ceiling -1.0), no clipping |
| Loop seam | jump 0.65x the typical neighbouring step; no transient at the wrap |
| Silence | all 16 bars audible |
| Kick layering | energy below 120 Hz is 7.9 dB higher in bars 9-16 than in bars 1-8 |

Known: the grid passes with a thin margin. A few soft piano notes have a second swell, and with
the room on the detector sometimes counts it as an extra onset. That is an analyzer refinement on
the list, not a timing fault in the render (median deviation 1.5 ms).
