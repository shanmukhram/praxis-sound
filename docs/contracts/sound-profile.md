# Contract: Sound Profile and render verification

Code (frozen): `crates/praxis-analyze/src/profile.rs`, `verify.rs`.

A Sound Profile is the measurement of one clip (FR-12). It stores numbers only, never audio, so
it can be kept for copyrighted references and reused as a library style (FR-19). JSON, schema via
`schemars`.

## Fields and methods (WP-D)

| Field | Method |
| --- | --- |
| tempo, beats | onset-strength envelope (spectral flux, 10 ms hop), tempo by autocorrelation + comb filterbank over 60-200 BPM with octave correction, beat tracking by dynamic programming |
| key | 12-bin chroma from a constant-Q-like log-frequency mapping of the STFT; correlation with Krumhansl-Schmuckler and Temperley profiles for 24 keys |
| loudness | BS.1770-4 integrated, LRA, true peak (`praxis_dsp::meter`), sample peak, RMS, crest |
| envelope | RMS envelope; attack/decay/sustain/release fitted from it |
| spectral | centroid, 85% rolloff, flatness, 10 octave bands, tilt (dB/oct), centroid over time |
| groove | onset strengths folded onto the detected grid per bar; swing from off-beat delay |
| pitch | YIN at 10 ms hop with confidence threshold |
| tonality | 1 - mean spectral flatness, refined by harmonic-peak ratio |
| stereo | side/mid energy ratio, L/R correlation |
| reverb | decay slope of the energy after transients (Schroeder-style backward integration) |
| sections | novelty curve of a chroma+MFCC self-similarity matrix, peaks as boundaries |
| class | music / sfx / ambience from duration, onset regularity, tempo confidence, tonality, stationarity |
| features | fixed vector; names in `FEATURE_NAMES`, append-only |

### Building a profile (WP-D2)

`praxis_analyze::sound_profile(left, right, sample_rate, &ProfileOptions {file_name, channels,
beats_per_bar})` measures every field above (`crates/praxis-analyze/src/analysis.rs`; methods in
the module docs of `features/*`). Pure and WebAssembly-safe: no files, clocks or threads; the
same samples give the same profile. Details beyond the table:

- **source.pcm_hash**: BLAKE3 over the little-endian bits of every left sample, then every right
  sample (the same bytes as `praxis_testkit::audio_hash`), implemented in `hash.rs` because the
  crate has no `blake3` dependency; tested against the published vectors and the `blake3` crate.
- **tempo**: no tempo with fewer than 4 onsets. Profiles use the tempo estimator with an octave
  tie ratio of 0.9 (a faster octave relative within 10 % of the strongest wins) instead of the
  0.97 that render verification keeps: with 0.97, eighth-note music at 155-175 BPM read at half
  tempo. Confidence is the estimator's.
- **envelope**: 10 ms RMS frames; profiles keep at most 2000 (power-averaged, `hop_s` grows).
  Attack, decay, sustain and release are fitted on the 50 ms-smoothed envelope (definitions in
  `features/envelope.rs`).
- **spectral**: one 85 ms Hann STFT at a 10 ms hop. Centroid and centroid track are
  power-weighted; band levels are relative to the power of all ten bands, floored at -120 dB.
- **groove**: only when the onsets are regular (at least 75 % on the sixteenth grid of the
  beats; noise and grain onsets read 0.4-0.7, sequenced music 0.99+). The grid is the coarsest
  that places 90 % of the onsets within 0.08 beat.
- **pitch**: YIN on the mono mix decimated to 16-22 kHz, 25 ms window, 40-2000 Hz, voiced when
  `d' <= 0.25` and the frame is louder than -50 dBFS and within 40 dB of the loudest frame.
- **tonality**: mean over sounding frames of `0.5 * (1 - whitened flatness / 0.5615) + 0.5 *
  peak ratio` (whitening by the local mean spectrum, so noise of any colour reads about 0).
- **reverb**: `None` when no segment between onsets decays by 20 dB (dense music).
- **sections**: 0.5 s blocks averaged over +-2 s, similarity `exp(-d^2 / 0.2)`, novelty
  normalised to -1..1 with a floor of 0.15; clips under 16 s are one section. A layer entering
  quietly under the same material (acceptance 1's soft kick at bar 9) is not a boundary; a new
  arrangement is (an engine render changing at 16 s reads a boundary at 18 s).
- **class**: a hand-set linear model and softmax over active duration, tempo confidence, onset
  regularity, onset count, tonality, the interquartile range of the 400 ms level and the spread
  of the centroid (`features/classify.rs`). Only a regular pulse counts as music.

`FEATURE_NAMES` (append-only from now on; `features[i]` is the value of `FEATURE_NAMES[i]`, 0 when
a part is missing): `duration_s` (active span, within 45 dB of the loudest 10 ms frame),
`tempo_bpm`, `tempo_confidence`, `key_confidence`, `key_minor` (1 minor, 0 major, 0.5 no key),
`integrated_lufs`, `loudness_range_lu`, `crest_db`, `attack_s`, `decay_s`, `sustain`,
`release_s`, `centroid_hz`, `rolloff_hz`, `flatness`, `band_31_db` .. `band_16k_db` (10),
`tilt_db_per_oct`, `brightness_sd_oct`, `onset_rate_hz`, `regularity`, `swing`,
`onsets_per_bar`, `offbeat_weight` (mean off-beat step weight over on-beat), `voiced_fraction`,
`pitch_median_midi`, `pitch_range_st` (10th to 90th percentile), `tonality`, `stereo_width`,
`stereo_correlation`, `reverb_rt60_s`, `reverb_confidence`, `level_iqr_db`, `sections`,
`class_music`, `class_sfx`, `class_ambience` (45 entries).

### Labelled set (WP-D2)

Self-made, because the owner buys nothing: 81 scores in `crates/praxis-analyze/tests/clips/`
with `labels.json` (no audio in git), generated by `crates/praxis-cli/tests/clipgen` and
rendered by the real engine in `crates/praxis-cli/tests/labelled_set.rs` (praxis-analyze has no
engine). 48 music clips cover every one of the 24 keys twice at 60-180 BPM with felt and upright
piano, pluck, soft keys, mallet, FM keys, bell and glass, some with bass, melody and a drum kit,
reverb, a level swell, -20 dB level changes, white noise at -24 dB, 4, 8 and 16 bars, loops and
one-shots, at 48 and 44.1 kHz; plus 4 drum loops (tempo only), 17 sound effects (whoosh,
impact, riser, glitch, tick) and 12 ambiences (noise beds, granular textures, drones). The
test requires tempo within 2 BPM on at least 90 % of the music clips, the right key on at least
90 % of the keyed clips, and the right class on at least 90 % of all clips. Clips that need the
Upright Piano KW pack are skipped when it is missing.

Measured 2026-11-11 (`cargo test -p praxis-cli --release --test labelled_set -- --nocapture`):

| | With the piano pack | Without it |
| --- | --- | --- |
| tempo within 2 BPM | 49 of 52 (94.2 %) | 37 of 40 (92.5 %) |
| key | 47 of 48 (97.9 %) | 35 of 36 (97.2 %) |
| tempo and key both right | 44 of 48 (91.7 %) | 32 of 36 (88.9 %) |
| class | 81 of 81 (100 %) | 69 of 69 (100 %) |

Confusions: tempo octave errors on F minor at 64 BPM (read 128), D major at 175 (87.5) and Ab
major at 155 (77.5); Ab minor read as Ab major (confidence 0.49). The class weights, the profile
tempo tie ratio and the groove gate were set on this set, so 100 % class accuracy is an upper
bound for clips like these, not a promise for real recordings.

## Similarity and originality

- `similarity::similarity(a, b) -> f32` in 0..1, a weighted distance over normalised features
  (character: tempo, groove, spectral balance, envelope, tonality, width, reverb), not waveform.
  WP-D2: read from `features` by name; group weights tempo 1.5, groove 1.5, spectral 2,
  envelope 1.5, tonality 1, width 0.5, reverb 0.75, kind 1 (the class probabilities); each
  group's distance is capped at 1 (scales in `similarity.rs`); `similarity = 1 - weighted mean
  distance`. Symmetric; a profile with itself is 1.
- `originality::check(reference, render) -> OriginalityReport { passed, melodic_similarity,
  rhythmic_similarity, reason }`. Melodic similarity compares interval sequences of the two pitch
  tracks (n-gram overlap and DTW on intervals, transposition-invariant). A close copy (above the
  threshold set in WP-D, documented with its test set) fails. Reference audio never reaches an
  export: profiles hold no audio, and the engine has no input that accepts a reference buffer.
  WP-D2: notes from the pitch track (split at pitch moves and onsets), intervals rounded to
  semitones; `melodic_similarity = 0.3 * n-gram overlap (3- and 4-grams) + 0.7 * subsequence DTW
  similarity` (warping steps cost 0.5 extra); the check fails at `MELODIC_THRESHOLD = 0.6`.
  Fewer than 6 intervals on either side passes ("not enough melody to compare"). Rhythmic
  similarity (n-grams of inter-onset-interval ratios) is reported, never decisive. Calibration
  (`crates/praxis-analyze/tests/originality.rs`, 12 seeded melodies): all 60 close copies fail
  (transposed, slower or faster, staccato, two notes moved a tone, the first half in another
  key; least similar 0.702), all 36 unrelated melodies pass (another melody, another at another
  tempo, the same rhythm with new pitches; most similar 0.492).

## Render verification (`verify_render`)

Checks a master render against its Score; `praxis check` prints it. Checks and defaults:

| Check | Pass when |
| --- | --- |
| tempo | detected BPM within 2 of `score.tempo.bpm` (a confident octave error fails; see "Can't tell" below) |
| key | detected tonic equals `score.key.tonic` and the detected major/minor equals the score mode's family (dorian, phrygian, locrian, harmonic/melodic minor count as minor; lydian, mixolydian as major); a confident relative major/minor fails; chromatic and unpitched scores skip |
| grid | >= 90% of onsets within 10 ms of a grid step of the score's finest pattern grid |
| loudness | integrated within 0.5 LU of `master.loudness.target_lufs` |
| true_peak | <= `true_peak_dbtp` + 0.05 |
| clipping | no sample with \|x\| >= 1.0 |
| loop_seam | (loops only) the jump between the last and first sample is no larger than ordinary sample-to-sample jumps at that point in the music, and no broadband transient or broadband drop appears at the seam (first-difference energy, transient flux and drop flux at the seam at most 2 dB above their maximum elsewhere in the clip, excluding 50 ms around the seam) |
| silence | the render is not silent where the score has notes |

### Can't tell (`Check.inconclusive`, WP-D1c)

`Check` has `inconclusive: bool` (serde default `false`, always serialised). A check is
inconclusive when the material cannot decide it: it keeps `passed: false` (honest for readers
that ignore the field), its `measured` ends with `"; can't tell: <reason in plain words>"`
(`verify::CANT_TELL`), and it counts neither as passed nor as failed:
`VerifyReport::passed()` is true when every *decided* check passed, and every "N of M" count
covers decided checks only. `VerifyReport::summary()` is the take's line, e.g. "6 of 6 checks
passed · 2 can't tell from these notes: tempo (the rhythm doesn't clearly show 128 BPM), key (no
third, so A minor vs A major can't be heard)". The CLI's `check` payload (render, check, takes)
adds `failed[]` (decided failures), `inconclusive[]` (names) and `summary`.

Only tempo and key can be inconclusive; grid, loudness, true peak, clipping, loop seam and silence
are always decided. A detection that matches the score always passes.

| Check | Inconclusive when (otherwise a mismatch fails) |
| --- | --- |
| tempo | the detection is wrong and its confidence is below **0.25** (no clear beat at all); or the detected BPM is within 2 of a simple ratio of the score's (1/2, 2/3, 3/4, 4/3, 3/2, 2), its confidence is below **0.6**, and the grid check passed (the notes sit on the score's grid, but their rhythm also fits the related tempo). "no periodic onsets" still fails. |
| key | the detected tonic is the score's but the sounding notes (pitched, audible layers of the score) have no minor or major third over the tonic and no minor sixth, so major vs minor cannot be heard (even when the detector happens to agree); or the detection is wrong and its confidence is below **0.4**. A wrong tonic or mode at 0.4 or more fails; "no tonal content" fails. |

Calibration (2026-11-11). Confidences as `estimate_tempo` / `estimate_key` report them; "real"
is the engine render, "synth" the analyzer tests' additive synthesiser.

| Case | Tempo: detected (confidence) | Key: detected (confidence) | Verdict |
| --- | --- | --- | --- |
| Acceptance 1, real (`praxis render fixtures/acceptance1.score.json`) | 127.99 (0.62) | a minor (0.83) | 8 of 8 passed |
| Acceptance 1, offline planner, real | 128.00 (0.75) | a minor (0.87) | 8 of 8 passed |
| Acceptance 1, synth | 128.01 (0.49) | a minor (0.63) | 8 of 8 passed |
| Acceptance 1 a tone too high, synth | 128.00 (0.48) | b minor (**0.55**) | key fails (lowest confident wrong key) |
| Eighth loop vs a 64 / 100 / 85.33 BPM score, synth | 127.99 (**0.85**) | | tempo fails |
| 174 BPM drum loop (offline planner), real | 87.04 (**0.92**, ratio 1/2) | | tempo fails |
| Owner's hand-built loop (A, B, D, E; uneven rhythm), real | 85.34 (**0.42**, ratio 2/3; grid 100%) | a major (0.58; no third) | 6 of 6 passed, tempo and key can't tell |
| 60 BPM felt-piano bed (offline planner), real | 120.00 (**0.49**, ratio 2; grid passes) | c# minor (0.77) | tempo can't tell |
| 70 BPM bass, only Bb and D (offline planner), real | 201.16 (**0.21**) | bb major (0.70) | tempo can't tell; key and grid fail |
| Owner's C-major piano sketch, take 1, real | 84.01 (0.60) | e minor (**0.16**) | key can't tell |
| Same, take 2, real | 84.00 (0.62) | e minor (0.73) | key fails (notes C, E, G, A) |
| Correct detections, all renders | 0.24 to 0.92 | 0.20 to 0.87 | pass (confidence not used) |

Thresholds sit between the most confident "can't tell" and the least confident real error:
tempo ratio 0.49 vs 0.85, key 0.16 vs 0.55. Tests: `crates/praxis-analyze/tests/verify_*.rs`,
`crates/praxis-cli/tests/cant_tell.rs` (the owner's loop through the engine).

## Reference library (WP-D2, FR-19)

Profiles are kept as named, tagged styles in `PRAXIS_HOME/library/<id>.profile.json`, one
`SoundProfile` per file (`style_name`, `tags`); never audio. The same decoded clip (same
`pcm_hash`) is kept once. Commands and payloads: docs/contracts/cli.md, "Sound Profile and
library".
