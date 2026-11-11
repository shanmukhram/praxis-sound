// A blank sound (WP-U5) and the duration solver (FR-2), ported from
// crates/praxis-score/src/timeline.rs `duration` and crates/praxis-app/src/hand.rs `blank_score`,
// so the start form shows the same tempo the desktop shell will write.

import type { NScore, TimeSignature } from '../score/types';
import { normalizeScore, SCHEMA_VERSION } from '../score/types';
import { barQuarters } from '../score/theory';
import type { BlankSound } from './EngineAdapter';

/** What a request in words gets on a blank sound before its first take. */
export const DRAFT_ASK =
  'This sound was started by hand: add a layer in the Sound panel, write its notes, then make the first take. After that you can ask for changes in words.';

/** Tempo (quarter-note BPM) that makes `bars` bars last `seconds`: 16 bars of 4/4 in 30 s is 128. */
export function solveTempo(bars: number, ts: TimeSignature, seconds: number): number {
  return (bars * barQuarters(ts) * 60) / seconds;
}

/** Whole bars at `bpm` closest to `seconds` (at least 1), and the exact length they give. */
export function solveBars(bpm: number, ts: TimeSignature, seconds: number): { bars: number; seconds: number } {
  const barS = (barQuarters(ts) * 60) / bpm;
  const bars = Math.max(1, Math.round(seconds / barS));
  return { bars, seconds: bars * barS };
}

/** Rounded to two decimals, as hand.rs does. */
export function roundBpm(bpm: number): number {
  return Math.round(bpm * 100) / 100;
}

/** The tempo a blank sound gets: the one given, else the solver's for the length. */
export function blankTempo(b: BlankSound): number | null {
  if (b.bpm !== undefined && Number.isFinite(b.bpm)) return roundBpm(b.bpm);
  if (b.seconds !== undefined && b.seconds > 0 && Number.isFinite(b.seconds)) return roundBpm(solveTempo(b.bars, b.timeSignature, b.seconds));
  return null;
}

/** Why a blank sound cannot start like this, in words; null when it can. */
export function blankProblem(b: BlankSound): string | null {
  if (!Number.isInteger(b.bars) || b.bars < 1 || b.bars > 2048) return 'Bars must be a whole number from 1 to 2048.';
  const ts = b.timeSignature;
  if (!(ts.numerator >= 1 && ts.numerator <= 32) || ![2, 4, 8, 16].includes(ts.denominator)) return 'That time signature is not supported.';
  const bpm = blankTempo(b);
  if (bpm === null) return 'Give a tempo, or a length to fit the bars into.';
  if (!(bpm >= 20 && bpm <= 400)) return `That makes ${bpm} BPM; the tempo must be between 20 and 400.`;
  return null;
}

/** The blank score (no layers) a sound starts as; throws with `blankProblem`'s words. */
export function blankScore(b: BlankSound): NScore {
  const problem = blankProblem(b);
  if (problem) throw new Error(problem);
  return normalizeScore({
    schema_version: SCHEMA_VERSION,
    title: b.title?.trim() || 'Untitled sound',
    seed: 1,
    meta: { provenance: { source: 'built by hand' }, reference_profiles: [], tags: [] },
    tempo: { bpm: blankTempo(b)! },
    time_signature: { ...b.timeSignature },
    key: { ...b.key },
    bars: b.bars,
    playback: b.loop ? { mode: 'loop' } : { mode: 'one_shot', tail_seconds: 4 },
  });
}
