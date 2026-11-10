# Phase 2 demos (in progress)

## `preview_felt_piano_only.flac` (preview, not the Phase 2 exit)

Acceptance test 1 planned from one brief by the offline planner and rendered with the real
**felt piano** (CC0 Upright Piano KW, voiced soft). The soft kick is not built yet, so bars 9-16
are the piano alone. Reverb, EQ, loudness normalisation and the limiter are not wired yet (mix bus,
WP-F2), so the piano is completely dry. The Score is in `preview.score.json`.

What to listen for: an even eighth-note motif that loops cleanly at 0:30, strong on the first beat
and bright on beat 3, notes ringing into each other, soft felt tone, nothing late or loose.

Measured (praxis check, 2026-11-11): tempo 127.99 BPM (score 128), key A minor, 93% of 129 onsets
within 10 ms of the grid (median 1.6 ms), -15.6 LUFS, true peak -3.0 dBTP, no clipping, seamless
loop. earshot: "solo piano music" (z +3.4), realness uncertain (0.46) because the dry render has no
room sound between notes; the mix-bus reverb is expected to fix that.

The Phase 2 exit demo (felt piano + soft kick, mixed to -16 LUFS) will be added here for sign-off.
