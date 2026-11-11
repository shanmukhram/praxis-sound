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

## Similarity and originality

- `similarity::similarity(a, b) -> f32` in 0..1, a weighted distance over normalised features
  (character: tempo, groove, spectral balance, envelope, tonality, width, reverb), not waveform.
- `originality::check(reference, render) -> OriginalityReport { passed, melodic_similarity,
  rhythmic_similarity, reason }`. Melodic similarity compares interval sequences of the two pitch
  tracks (n-gram overlap and DTW on intervals, transposition-invariant). A close copy (above the
  threshold set in WP-D, documented with its test set) fails. Reference audio never reaches an
  export: profiles hold no audio, and the engine has no input that accepts a reference buffer.

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
