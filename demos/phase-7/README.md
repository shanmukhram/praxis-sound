# Phase 7 demos: scoring depth and hit points

**For the owner's listening review (the Phase 7 exit).** Every file was planned from the one-line
brief shown, by the offline planner (seed 0), and rendered by the engine with the real
instruments (the sampled felt piano, the synth keys and basses, the soft kick) and the mix bus.
Each `.score.json` next to it is the Score it was rendered from. Nothing was edited by hand.

**Phase 7 exit:** a 2-minute score generated from a brief with three hit points lands each one on
its marker and passes listening review. That is `cue_hits_2min.flac`.

| File | Brief | Length |
| --- | --- | --- |
| `cue_hits_2min.flac` | **The one to judge.** "A 2-minute tense cue in D minor: intro, build, peak and release, with hits at 0:42, 1:05 and 1:38. A felt piano motif, a warm bass and a soft kick. No risers." | 2:00 + 4 s ring-out |
| `cue_melancholy_preset.flac` | "A melancholy 90-second evolving cue." (no instruments, key or tempo named: all from the melancholy preset) | 1:32 + ring-out |
| `cue_calm_progression.flac` | "A calm cue in C major at 84 BPM, 32 bars, with the chord progression I V vi IV: a felt piano motif, warm keys and a warm bass." | 1:31 + ring-out |
| `cue_aba.flac` | "An A/B/A cue for felt piano and soft keys in G major at 90 BPM, one minute long." | 1:01 + ring-out |

## What to listen for

### `cue_hits_2min.flac` (D minor, 120 BPM, 60 bars of 2 s)

The brief gave no tempo, so the planner chose 120 BPM from the tense preset's range (88-120)
because there all three hits fall on strong beats: 0:42 and 1:38 on bar lines, 1:05 on beat 3.

| Time | What happens |
| --- | --- |
| 0:00 | **Intro.** The felt piano alone, sparse (on the beats and accents only), slightly muted (a gentle low-pass that opens as the cue builds). |
| **0:42** | **Hit 1 and the build.** A strong D minor chord on the piano with a bass root, exactly on 0:42; the warm bass enters here and stays. The piano now plays its full motif. Dynamics rise bar by bar through the build. |
| 0:56 | The piano motif lifts: its accented notes are doubled an octave up. |
| 1:02-1:04 | **Drop.** The bass rests for one bar; the piano plays alone, leading into the peak. |
| 1:04 | **Peak.** Bass back, the soft kick enters underneath. |
| **1:05** | **Hit 2**, on beat 3 of the peak's first bar: chord, bass root and kick together. |
| 1:36-1:38 | **Held chord.** Piano and bass hold one chord for the whole bar; the kick stops. |
| **1:38** | **Hit 3 and the release.** Accent on the downbeat, then the piano plays its full motif alone, quieter and darker, to the end. |

Listen for: each hit lands exactly where the picture would cut (no flam, nothing early or late);
the music grows by adding layers, never by rewriting the piano; transitions are a drop and a held
chord, not a riser; nothing sounds like a trailer effect.

### `cue_melancholy_preset.flac` (E minor, 76 BPM, bars of 3.16 s)

Nothing but the mood was asked for: the melancholy preset chose E minor, 76 BPM, a felt piano,
soft keys and a warm bass, the form intro / main / peak / release, and the progression
i-iv-VI-v (Em Am C Bm), which restarts at each section.

| Time | What happens |
| --- | --- |
| 0:00 | Piano alone, sparse. |
| 0:19 | **Main.** Soft keys enter with sustained, voice-led chords (each voice moves as little as possible). Piano full. |
| 0:41-0:44 | Drop: the keys rest one bar. |
| 0:44 | **Peak.** Keys back, the warm bass enters with passing notes between distant chord roots. |
| 1:00-1:03 | Held chord, then the **release**: piano alone, sparse again, to the end. |

### `cue_calm_progression.flac` (C major, 84 BPM, bars of 2.86 s)

The progression asked for, I-V-vi-IV (C G Am F), in every section.

| Time | What happens |
| --- | --- |
| 0:00 | Piano alone, sparse. |
| 0:23 | Warm keys enter with voice-led chords. |
| 0:43-0:46 | Drop to the piano, then the **peak** at 0:46 with the warm bass: listen for its passing note on the last beat before a far chord change (C to G, Am to F). |
| 1:06-1:09 | Held chord, then the **release**. |

### `cue_aba.flac` (G major, 90 BPM, bars of 2.67 s)

An A/B/A form: A (0:00) piano alone; B (0:21) soft keys join, a little louder and brighter;
A (0:43) piano alone again. The return of A is the same piano part as the start.

## Measurements (`praxis generate --render`, which runs `praxis check`, 2026-11-12)

| File | Tempo | Key | Grid (onsets within 10 ms) | Loudness | True peak | Checks |
| --- | --- | --- | --- | --- | --- | --- |
| `cue_hits_2min` | 120.00 BPM | D minor | 96.0% of 373, median 0.17 ms | -16.14 LUFS | -1.02 dBTP | 7 of 7 |
| `cue_melancholy_preset` | 76.00 BPM | E minor | 100% of 150, median 0.14 ms | -16.12 LUFS | -1.02 dBTP | 7 of 7 |
| `cue_calm_progression` | 84.00 BPM | C major | 97.7% of 175, median 0.15 ms | -16.07 LUFS | -1.02 dBTP | 7 of 7 |
| `cue_aba` | 90.00 BPM | G major | 100% of 180, median 0.17 ms | -16.02 LUFS | -1.02 dBTP | 7 of 7 |

**Hits, measured on the render** (`praxis analyze` onsets of `cue_hits_2min.flac`): the onset
nearest each marker is 0.17 ms from it, at 0:42, 1:05 and 1:38. The planner's own test
(`tests/scoring_render.rs`, the engine's test instruments) finds the attack (the first 10 dB rise
in 5 ms) 4, 3 and 1 ms from the three markers.

Two things found while making these, both fixed before rendering: a felt-piano motif in the low
register (the tense preset's first setting) blurred its attacks so the grid check read 80% (every
preset now keeps the lead in the middle register), and the melancholy preset's first progression
(i-VI-III-VII) leaned so hard on the relative major that the key check heard G major (its default
is now i-iv-VI-v).
