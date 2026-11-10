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
| tempo | detected BPM within 2 of `score.tempo.bpm` (octave errors count as failures) |
| key | detected tonic equals `score.key.tonic` and the detected major/minor equals the score mode's family (dorian, phrygian, locrian, harmonic/melodic minor count as minor; lydian, mixolydian as major); relative major/minor fails; chromatic and unpitched scores skip |
| grid | >= 90% of onsets within 10 ms of a grid step of the score's finest pattern grid |
| loudness | integrated within 0.5 LU of `master.loudness.target_lufs` |
| true_peak | <= `true_peak_dbtp` + 0.05 |
| clipping | no sample with \|x\| >= 1.0 |
| loop_seam | (loops only) the jump between the last and first sample is no larger than ordinary sample-to-sample jumps at that point in the music, and no broadband transient or broadband drop appears at the seam (first-difference energy, transient flux and drop flux at the seam at most 2 dB above their maximum elsewhere in the clip, excluding 50 ms around the seam) |
| silence | the render is not silent where the score has notes |
