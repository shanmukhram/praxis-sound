# Phase 6 blind listening review: "make something like this"

Ten pairs. In each pair one file is a **reference** (a clip made for the test, standing in for a
clip you would drop in) and the other is what Praxis **made to be like it** from the reference's
measurements alone (the matcher never hears or copies the reference; it only sees numbers such as
tempo, key, brightness, envelope, groove and room). Which file is which changes from pair to
pair, with no pattern.

| Pair | Files |
| --- | --- |
| 01 | `pair-01-a.flac`, `pair-01-b.flac` |
| 02 | `pair-02-a.flac`, `pair-02-b.flac` |
| 03 | `pair-03-a.flac`, `pair-03-b.flac` |
| 04 | `pair-04-a.flac`, `pair-04-b.flac` |
| 05 | `pair-05-a.flac`, `pair-05-b.flac` |
| 06 | `pair-06-a.flac`, `pair-06-b.flac` |
| 07 | `pair-07-a.flac`, `pair-07-b.flac` |
| 08 | `pair-08-a.flac`, `pair-08-b.flac` |
| 09 | `pair-09-a.flac`, `pair-09-b.flac` |
| 10 | `pair-10-a.flac`, `pair-10-b.flac` |

Pairs 01-05 are music, 06-09 sound effects, 10 an ambience bed. 48 kHz, 24-bit FLAC, both files
of a pair at about the same loudness as made (no extra normalisation).

## What to listen for

For each pair, write down:

1. **Which one is the reference?** (A or B; "can't tell" is a good answer.)
2. **Same character?** Would the made one work where you wanted the reference: same mood, pace,
   brightness, weight and space? (yes / roughly / no)
3. **Its own piece?** Does the made one avoid copying the reference's melody? (yes / no)
4. Anything that sounds wrong (out of time, out of key, harsh, muddy, a click).

The phase passes when every made sound is fair to its reference and none copies its melody.

## The answers

`answers/ANSWER_KEY.md` says which file is the reference in each pair, what the made one was
built from and its measured similarity. Open it only after listening. `answers/` also holds both
scores of every pair (`pair-NN-reference.score.json`, `pair-NN-result.score.json`).

## How these were made

`cargo test -p praxis-cli --release --test match_demos -- --ignored --nocapture` renders the
reference set (`praxis_match::refset`), matches each reference (seed 7, default options) and
writes the pairs. Measurements: docs/contracts/sound-profile.md, "Reference set and the
Phase 6 exit".
