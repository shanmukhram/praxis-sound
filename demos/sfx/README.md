# Sound-effect demos (WP-H)

The six procedural sound effects (`crates/praxis-instruments/src/sfx/`), each at its default
macros (`length`, `brightness`, `weight`, `pitch` all 0.5), plus a riser landing on a downbeat.
Every score is 4/4 at 120 BPM in A minor, rendered dry (no reverb) by the engine with loudness
normalisation off, so you hear each preset's own level. 48 kHz, 24-bit FLAC.

| File | Score | What it should sound like |
| --- | --- | --- |
| `whoosh.flac` | `whoosh.score.json` | One pass-by, 1 s, on A3. Air that swells as it approaches, peaks at 0.6 s and falls away, a quiet tonal core bending down through A (Doppler) and a pan from one side to the other. Not a trailer "swoosh": soft, mid-range. |
| `impact.flac` | `impact.score.json` | One hit on A (the key's tonic): a short noise crack, a low body that drops into pitch over ~10 ms and rings on A2, a sub on A1 and a short dark tail. 2 s, ends in silence. |
| `riser.flac` | `riser.score.json` | One-bar riser (4 beats): a noise band climbing, a tone holding A3 and a second one gliding up to E4 (the fifth), swelling at the very end and stopping exactly on the bar-2 downbeat (2.000 s). |
| `drone.flac` | `drone.score.json` | A 4-bar (8 s) drone on A3 rendered as a loop: soft harmonics that move slowly, a slow beat between left and right, breath-like air on top, a sub on A1. |
| `drone_loop_x3.flac` | (the same render, three times) | Listen at 0:08 and 0:16: the loop restarts with no click, gap, dip or fade. |
| `glitch.flac` | `glitch.score.json` | 0.4 s of stuttering, bit-crushed blips on A3 (some jump an octave), with short gaps and small pan moves. Every crush stays on A's harmonics, so it is in key. |
| `tick.flac` | `tick.score.json` | A clock: quarter-note ticks for two bars, the downbeat a little louder. A dry click with a faint ping on A, every tick exactly on time. |
| `riser_bar5.flac` | `riser_bar5.score.json` | A quiet tick clock on quarter notes for 6 bars (12 s); an 8-beat riser over bars 3-4 (4.0-8.0 s) that lands exactly on the bar-5 downbeat (8.0 s), where an impact on A hits. |

## The macros (every preset)

| Macro | 0 -> 1 | Default 0.5 |
| --- | --- | --- |
| `length` | duration: whoosh 0.25-4 s, impact 0.5-8 s, glitch 0.08-2 s, tick 10-250 ms (exponential); riser 1-16 **beats**; drone attack/release 0.25-8 s | 1 s, 2 s, 0.4 s, 50 ms; 4 beats; 1.41 s |
| `brightness` | filter centres and cut-offs, harmonic roll-off (spectral centroid rises) | moderate |
| `weight` | low layer: impact body level and length, subs, rumble, knock (energy below 150 Hz rises; 0 = none) | moderate |
| `pitch` | tuned parts by whole scale steps of the key (-7..+7; impact and tick ping over tonic/fifth positions), noise colour -1..+1 octave | the note |

## Measurements

From `cargo test -p praxis-instruments --release --test sfx_presets --test sfx_engine`:

- `length` sets the duration exactly (0.000-0.03% error at 0, 0.25, 0.5, 0.75, 1).
- Pitched presets land within 0.9 cent of the note (drone, riser, glitch, impact on tonic notes,
  whoosh at the pass); the `pitch` macro moves them by the expected A-minor scale steps.
- Brightness raises the centroid at every step (e.g. whoosh 475 -> 4871 Hz, riser 345 -> 1854 Hz,
  tick 535 -> 3132 Hz); weight raises the energy below 150 Hz at every step (e.g. whoosh
  +14.7 -> +55.9 dB, tick -63 -> -6 dB).
- The riser's last sound is the sample before the bar-5 downbeat (sample 383999, bar 5 =
  384000) and its loudest 25 ms end within 0.8 ms of it; over 400 seeded takes the worst is
  1.8 ms.
- A looped drone is identical cycle to cycle (difference 0.0).
- Levels at velocity 0.8: peaks 0.19 (tick) to 0.48 (glitch); no DC above 0.03% of peak.

## Listening check (earshot, 2026-11-11)

`describe` (top readings, z >= 3 is strong):

| Demo | earshot heard |
| --- | --- |
| whoosh | "a fast whoosh" (z 3.7), "a short swish", "a swoosh passing by"; AudioSet: Whoosh 0.34 |
| impact | "a snare hit" (z 4.6), "a kick drum", "a big drum hit"; pitch ~55 Hz (A1) |
| riser | "an uplifter sweep" (z 4.0), "a reverse cymbal swell" (z 3.5), "a rising riser building tension" |
| drone | "a vacuum cleaner" (z 3.3), "an electrical hum", "a singing bowl"; best music reading "a warm analog synth pad" (z 3.5); pitch ~220 Hz (A3) |
| glitch | "a digital glitch" (z 3.0), "an error buzz"; strongly "a square wave chiptune blip" (z 4.9); pitch ~220 Hz |
| tick | "a scroll wheel tick" (z 3.3), "a wooden block hit", "a clock ticking"; AudioSet: Tick 0.22 |

`timeline` on `riser_bar5`: 120 BPM grid (clarity 0.92); the riser/impact land on the 8.0 s cue
(onset 14 ms early, within tolerance). On `drone_loop_x3`: no onset at the 8 s and 16 s seams.

`realness`: all seven read as code-synthesized (p_real 0.01-0.07). They are procedural patches,
rendered dry; the drone in particular reads as a steady hum more than a real texture.

## Regenerate

```bash
cargo test -p praxis-instruments --release --test sfx_engine -- --ignored
```

writes float WAVs to `target/tmp/sfx-demos/`; the FLACs are
`ffmpeg -i <name>.wav -c:a flac -sample_fmt s32 -bits_per_raw_sample 24 <name>.flac`, and the x3
file adds `-stream_loop 2` before `-i`.
