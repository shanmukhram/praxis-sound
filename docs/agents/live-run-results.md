# Live run of the sound assistant: results (2026-11-11)

First run against the real API (`claude-opus-5-5`, effort medium), with the owner's key from the
Windows keychain, on the sound `live-1` (`C:\Users\admin\Praxis Sound\sounds\live-1`). The ten
requests from `live-run.md`, in order, through `praxis --json ask live-1 ...`. Renders only; no
audio was played. Cost is estimated from the returned token usage at Opus 5.5 list prices
(input $4/M, output $20/M, cache write $5/M, cache read $0.20/M).

| # | Request | What it did | Checks | Cost |
| --- | --- | --- | --- | --- |
| 1 | Calm 16-bar felt piano loop in A minor, 30 s, soft kick from bar 9, no fills or risers | First ScorePlan was refused by validation; it fixed the plan and composed again; take 1 at 128 BPM | 8/8 | $0.122 |
| 2 | Make the piano muffled | Felt piano `tone` 0.5 -> 0.25, `felt` 0.7 -> 0.9: high band -4.4 dB, centroid 560 -> 515 Hz; then lowered the piano 0.9 dB to bring the loop back to -16 LUFS and checked again. Notes identical to take 1 | 8/8 | $0.038 |
| 3 | A bit brighter than before | Used the history: `tone` 0.35, `felt` 0.85; centroid 536 Hz, "about halfway back to the original 560 Hz". Notes identical to take 1 | 8/8 | $0.025 |
| 4 | Undo what you just did to the piano | `undo_redo`: take 2 is current again; offered redo | (no take) | $0.023 |
| 5 | Make the kick a little softer in the second half | Kick -5 -> -7 dB; only the kick changed; bars 1-8 sample-identical | 8/8 | $0.026 |
| 6 | (bars 13-16 of the piano) add a subtle melody here | New soft upright-piano layer in bars 13-16 only; bars 3-12 sample-identical (1-2 hold its tail wrapped round the loop) | 8/8 | $0.065 |
| 7 | (kick locked by hand) Make the kick louder | Refused without trying: "it's locked ... Unlock it and I'll raise it" | (no take) | $0.010 |
| 8 | Make it better | `ask_user`: "What would make it better for you: a warmer or brighter tone, more space, a louder or more present melody, more movement in the piano part, or something else?" | (no take) | $0.011 |
| 9 | Three alternatives for the last four bars of the piano; keep one whose checks all pass | `make_variations` x3, all passed; kept the first; only the piano's bars 13-16 changed | 8/8 | $0.032 |
| 10 | Show me the notes | `open_view` notes | (no take) | $0.018 |

Total: 23 API requests, about **$0.37**. Every request did what `live-run.md` says it should.

## Found

1. **Turns without a take are reported as errors** (requests 4, 7, 8, 10): `praxis ask` exits 1
   with `patch_rejected` / `error` and the reply as the message. They should be ordinary replies
   (`ok: true`, no take, the reply), in the CLI, MCP and the app.
2. **Take titles are clipped from the reply** ("I made a 16-bar", "I added a soft upright-piano
   melody in a"). The agent should give a short title, or the title should be a summary.
3. After the variation in request 9, `identical_bars` is empty: the rewritten bars feed the piano's
   room reverb, whose recirculation leaves rounding-level differences everywhere (allowed by
   contracts/patch.md). The take's wording should then say "identical note for note" rather than
   naming no identical bars.
4. Locking a layer through `praxis ask --patch` records a take ("Kick locked", take 6). In the
   app a lock is a live-score change without a take; consider the same for the CLI.
