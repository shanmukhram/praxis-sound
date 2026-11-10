import { describe, expect, it } from 'vitest';
import { barQuarters, stepsPerBar, stepsPerBeat, inKey, midiName, GRIDS } from '../../src/score/theory';
import type { Grid, TimeSignature } from '../../src/score/types';

const ts = (numerator: number, denominator: number): TimeSignature => ({ numerator, denominator });

// Expected steps per bar for every grid (null = the grid does not divide the bar).
const TABLE: [TimeSignature, Record<Grid, number | null>][] = [
  [ts(4, 4), { quarter: 4, eighth: 8, sixteenth: 16, eighth_triplet: 12, sixteenth_triplet: 24 }],
  [ts(3, 4), { quarter: 3, eighth: 6, sixteenth: 12, eighth_triplet: 9, sixteenth_triplet: 18 }],
  [ts(2, 4), { quarter: 2, eighth: 4, sixteenth: 8, eighth_triplet: 6, sixteenth_triplet: 12 }],
  [ts(5, 4), { quarter: 5, eighth: 10, sixteenth: 20, eighth_triplet: 15, sixteenth_triplet: 30 }],
  [ts(6, 8), { quarter: 3, eighth: 6, sixteenth: 12, eighth_triplet: 9, sixteenth_triplet: 18 }],
  [ts(7, 8), { quarter: null, eighth: 7, sixteenth: 14, eighth_triplet: null, sixteenth_triplet: 21 }],
  [ts(12, 8), { quarter: 6, eighth: 12, sixteenth: 24, eighth_triplet: 18, sixteenth_triplet: 36 }],
  [ts(2, 2), { quarter: 4, eighth: 8, sixteenth: 16, eighth_triplet: 12, sixteenth_triplet: 24 }],
  [ts(5, 16), { quarter: null, eighth: null, sixteenth: 5, eighth_triplet: null, sixteenth_triplet: null }],
];

describe('steps per bar', () => {
  for (const [sig, row] of TABLE) {
    for (const grid of GRIDS) {
      it(`${sig.numerator}/${sig.denominator} on ${grid} = ${row[grid]}`, () => {
        expect(stepsPerBar(sig, grid)).toBe(row[grid]);
      });
    }
  }

  it('acceptance 1: 16 bars of 4/4 eighths is 128 steps; 30 s at 128 BPM', () => {
    expect(16 * stepsPerBar(ts(4, 4), 'eighth')!).toBe(128);
    expect((16 * barQuarters(ts(4, 4)) * 60) / 128).toBeCloseTo(30, 9);
  });

  it('steps per beat follow the denominator', () => {
    expect(stepsPerBeat(ts(4, 4), 'sixteenth')).toBe(4);
    expect(stepsPerBeat(ts(6, 8), 'sixteenth')).toBe(2);
    expect(stepsPerBeat(ts(4, 4), 'eighth_triplet')).toBeCloseTo(3, 9);
  });
});

describe('key membership and names', () => {
  it('A minor contains the white keys only', () => {
    const k = { tonic: 'A', mode: 'minor' } as const;
    for (const p of [57, 59, 60, 62, 64, 65, 67, 69]) expect(inKey(k, p)).toBe(true);
    for (const p of [58, 61, 63, 66, 68]) expect(inKey(k, p)).toBe(false);
  });
  it('chromatic contains everything', () => {
    for (let p = 0; p < 128; p++) expect(inKey({ tonic: 'C', mode: 'chromatic' }, p)).toBe(true);
  });
  it('midi names match theory::midi_name', () => {
    expect(midiName(60)).toBe('C4');
    expect(midiName(57)).toBe('A3');
    expect(midiName(70)).toBe('Bb4');
  });
});
