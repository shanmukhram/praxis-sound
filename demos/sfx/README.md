# Sound-effect demos (WP-H, reworked in WP-H2)

The six procedural sound effects (`crates/praxis-instruments/src/sfx/`), each at its default
macros (`length`, `brightness`, `weight`, `pitch` all 0.5), plus a riser landing on a downbeat.
Every score is 4/4 at 120 BPM in A minor, rendered by the engine with loudness normalisation off
and no master effects, so you hear each preset's own level and its own room. 48 kHz, 24-bit FLAC.

WP-H2 rebuilt the timbres toward recorded effects with physical-modelling ideas. Every preset now
plays into its own small per-voice room (`sfx/space.rs`: diffusion, early reflections and an
8-line delay network with an all-pass in each line), and randomness that makes takes differ
(gusts, mode tunings, debris, grain choices) comes only from the note seed. All measured
properties and tests of WP-H still hold (see Measurements).

| File | Score | What it should sound like |
| --- | --- | --- |
| `whoosh.flac` | `whoosh.score.json` | One pass-by, 1 s, on A3. Moving air: pink turbulence in three bands that brighten as the source comes closer (distant highs are absorbed) and gust, a hiss on top, a level that follows the distance, a soft tonal core and whistle bending down through A (Doppler), a pan from one side to the other, and a room. |
| `impact.flac` | `impact.score.json` | One hit on A (the key's tonic): a contact crack, a body that drops into pitch within ~50 ms and rings on A2 with drum-like upper modes, a shell knock and debris, a low slap, a gusting low roll and a sub on A1, in a hall. 2 s, ends in silence. |
| `riser.flac` | `riser.score.json` | One-bar riser (4 beats): a band of air climbing and narrowing, stirred by gusts and an accelerating gate, a three-voice unison holding A3 and another gliding up to E4 (the fifth) and drifting apart, swelling at the very end and stopping exactly on the bar-2 downbeat (2.000 s). |
| `drone.flac` | `drone.score.json` | A 4-bar (8 s) drone on A3 rendered as a loop: mostly breath (noise around the note, notched on its pitch), with drifting, beating harmonics and bowed resonances inside it, a sub on A1 and a long room. |
| `drone_loop_x3.flac` | (the same render, three times) | Listen at 0:08 and 0:16: the loop restarts with no click, gap, dip or fade. |
| `glitch.flac` | `glitch.score.json` | 0.4 s of a stuttering, corrupted buffer on A3: comb-tuned noise bursts (from crackle to buzz) over a quiet hum on the note, a few crushed FM blips, repeats (some at double speed), gaps and small pan moves. Everything pitched is on A's harmonics, so it is in key. |
| `tick.flac` | `tick.score.json` | A clock: quarter-note ticks for two bars, the downbeat a little louder. A short modal click (a hard contact on a small body) with a faint ping on A and the first moments of a room, every tick exactly on time. |
| `riser_bar5.flac` | `riser_bar5.score.json` | A quiet tick clock on quarter notes for 6 bars (12 s); an 8-beat riser over bars 3-4 (4.0-8.0 s) that lands exactly on the bar-5 downbeat (8.0 s), where an impact on A hits. |

## The macros (every preset)

| Macro | 0 -> 1 | Default 0.5 |
| --- | --- | --- |
| `length` | duration: whoosh 0.25-4 s, impact 0.5-8 s, glitch 0.08-2 s, tick 10-250 ms (exponential); riser 1-16 **beats**; drone attack/release 0.25-8 s | 1 s, 2 s, 0.4 s, 50 ms; 4 beats; 1.41 s |
| `brightness` | filter centres and cut-offs, harmonic roll-off, contact hardness, room damping (spectral centroid rises) | moderate |
| `weight` | low layer: impact body, sub and roll; subs, rumble, knock (energy below 150 Hz rises; 0 = none) | moderate |
| `pitch` | tuned parts by whole scale steps of the key (-7..+7; impact and tick ping over tonic/fifth positions), noise colour -1..+1 octave | the note |

## Measurements

From `cargo test -p praxis-instruments --release --test sfx_presets --test sfx_texture --test sfx_engine`:

- `length` sets the duration exactly (0.000-0.001% error at the default, under 1% at every step).
- Pitched presets land within 2.5 cents of the note (drone, riser, glitch, impact on tonic notes,
  whoosh at the pass); the `pitch` macro moves them by the expected A-minor scale steps.
- Brightness raises the centroid at every step (e.g. whoosh 1139 -> 5630 Hz, impact 205 -> 338 Hz,
  tick 1998 -> 6365 Hz); weight raises the energy below 150 Hz at every step (e.g. impact
  +18 -> +50 dB, tick -31 -> -4 dB).
- The riser's last sound is the sample before the bar-5 downbeat (sample 383999, bar 5 =
  384000) and its loudest 25 ms end within 0.8 ms of it; over 400 seeded takes the worst is
  2.2 ms.
- A looped drone is identical cycle to cycle (difference 0.0).
- Levels at velocity 0.8: peaks 0.21 (glitch) to 0.70 (whoosh); no DC above 0.07% of peak.
- Texture (new `sfx_texture.rs`): crest factors 14-27 dB (the WP-H glitch measured 5 dB),
  left/right correlation 0.17-0.92 (the WP-H tick was mono), seeded takes differ but keep their
  level, and the drone's second and third harmonics move 16-18 dB while the level between its
  harmonics is -21 dB (the WP-H drone, a clean harmonic stack, measured -50 dB).

## Listening check (earshot, 2026-11-12)

`realness` (p_real, before = the WP-H demos, after = these; clip thresholds lo 0.17 / hi 0.77,
the 8 s drone and 12 s riser_bar5 are means of 4 s windows):

| Demo | Before | After | earshot `describe` now (top readings, z >= 3 is strong) |
| --- | --- | --- | --- |
| whoosh | 0.07 | **0.59** | "a fast whoosh" (z 3.4), "a swoosh passing by" (3.0); AudioSet Whoosh 0.31 |
| impact | 0.01 | **0.34** | "a big drum hit" (z 3.3), "a cinematic hit with a long tail" (3.3), "a deep cinematic boom" (2.8) |
| riser | 0.03 | **0.27** | "a reverse cymbal swell" (z 4.6), "an uplifter sweep" (4.0) |
| riser_bar5 | 0.01 | **0.24** | (the riser, the clock and the landing impact) |
| drone | 0.04 | **0.73** | "a computer fan hum" (z 3.7), "a vacuum cleaner" (3.6); pitch read as A1 (the sub) |
| drone_loop_x3 | 0.04 | **0.73** | every 4 s window 0.71-0.75 |
| glitch | 0.02 | **0.34** | "an error buzz" (z 2.9), "a digital glitch" (2.7) |
| tick | 0.03 | **0.48** | "a wooden block hit" (z 3.9); AudioSet Tick 0.43, Tick-tock 0.21 |

For scale, earshot's own searched code-made bank sounds score 0.22-0.39 (glitches), 0.25
(shimmer), 0.69 (a riser) and 0.75 (an impact) on the same model. Readings are model judgements,
not a listening test: the owner's blind review decides. What still holds them back, as far as
earshot shows:

- riser: it must stop dead on the downbeat (it lands under a hit), and a riser that cuts to
  silence reads less like a library file; the same render through an ideal convolution room
  only reaches 0.33.
- impact: within its 2 s length the hall is cut by the final fade. The same render put through
  an extra offline convolution hall with a free 3 s tail read 0.82, so a denser room and a longer
  `length` are the levers; a wetter, longer per-voice hall alone did not help (0.29).
- drone: realness is high, but the words still say "fan hum": the breath layer is the next
  thing to refine.
- glitch: an earlier, noisier version without the quiet hum on the note read 0.43 but landed
  up to 13 cents off pitch on some notes; the hum keeps every take in tune for about 0.1 of
  realness.

## Regenerate

```bash
cargo test -p praxis-instruments --release --test sfx_engine -- --ignored
```

writes float WAVs to `target/tmp/sfx-demos/`; the FLACs are
`ffmpeg -i <name>.wav -c:a flac -sample_fmt s32 -bits_per_raw_sample 24 <name>.flac`, and the x3
file adds `-stream_loop 2` before `-i`.
