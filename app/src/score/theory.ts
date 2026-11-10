// Music theory and grid maths, ported from crates/praxis-score/src/theory.rs and timeline.rs.
// Keep these in step with the Rust code: the UI must agree with the validator and engine.

import type { Accent, GeneratorClass, Grid, Key, Mode, PitchClass, TimeSignature } from './types';

export const PITCH_CLASSES: PitchClass[] = ['C', 'C#', 'D', 'Eb', 'E', 'F', 'F#', 'G', 'Ab', 'A', 'Bb', 'B'];

export const GRIDS: Grid[] = ['quarter', 'eighth', 'sixteenth', 'eighth_triplet', 'sixteenth_triplet'];

export const GRID_LABEL: Record<Grid, string> = {
  quarter: '1/4',
  eighth: '1/8',
  sixteenth: '1/16',
  eighth_triplet: '1/8T',
  sixteenth_triplet: '1/16T',
};

export const ACCENTS: Accent[] = ['normal', 'strong', 'soft', 'bright'];

/** Spoken-rhythm names (FR-5): DON = strong, din = soft, don = normal, TIN = bright. */
export const ACCENT_SPOKEN: Record<Accent, string> = {
  normal: 'don',
  strong: 'DON',
  soft: 'din',
  bright: 'TIN',
};

const MODE_INTERVALS: Record<Mode, number[]> = {
  major: [0, 2, 4, 5, 7, 9, 11],
  minor: [0, 2, 3, 5, 7, 8, 10],
  harmonic_minor: [0, 2, 3, 5, 7, 8, 11],
  melodic_minor: [0, 2, 3, 5, 7, 9, 11],
  dorian: [0, 2, 3, 5, 7, 9, 10],
  phrygian: [0, 1, 3, 5, 7, 8, 10],
  lydian: [0, 2, 4, 6, 7, 9, 11],
  mixolydian: [0, 2, 4, 5, 7, 9, 10],
  locrian: [0, 1, 3, 5, 6, 8, 10],
  major_pentatonic: [0, 2, 4, 7, 9],
  minor_pentatonic: [0, 3, 5, 7, 10],
  chromatic: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
};

export function semitone(pc: PitchClass): number {
  return PITCH_CLASSES.indexOf(pc);
}

export function modeIntervals(mode: Mode): number[] {
  return MODE_INTERVALS[mode];
}

/** True if `pitch` (MIDI) belongs to the key's scale. Chromatic contains everything. */
export function inKey(key: Key, pitch: number): boolean {
  const rel = (((pitch - semitone(key.tonic)) % 12) + 12) % 12;
  return MODE_INTERVALS[key.mode].includes(rel);
}

export function isTonic(key: Key, pitch: number): boolean {
  return (((pitch - semitone(key.tonic)) % 12) + 12) % 12 === 0;
}

/** "A4" style name (60 = C4), matching theory::midi_name. */
export function midiName(pitch: number): string {
  return `${PITCH_CLASSES[((pitch % 12) + 12) % 12]}${Math.floor(pitch / 12) - 1}`;
}

export function keyLabel(key: Key): string {
  return `${key.tonic} ${key.mode.replace(/_/g, ' ')}`;
}

/** Unpitched classes skip the in-key check (validate::unpitched_class). */
export function isUnpitchedClass(c: GeneratorClass): boolean {
  return c === 'drums' || c === 'percussion' || c === 'tick';
}

/** Duration of one step as a fraction of a whole note. */
export function stepWholeNotes(grid: Grid): number {
  switch (grid) {
    case 'quarter':
      return 1 / 4;
    case 'eighth':
      return 1 / 8;
    case 'sixteenth':
      return 1 / 16;
    case 'eighth_triplet':
      return 1 / 12;
    case 'sixteenth_triplet':
      return 1 / 24;
  }
}

/** Bar length as a fraction of a whole note (4/4 = 1, 6/8 = 0.75). */
export function barWholeNotes(ts: TimeSignature): number {
  return ts.numerator / ts.denominator;
}

/** Bar length in quarter-note beats (the unit tempo counts). */
export function barQuarters(ts: TimeSignature): number {
  return barWholeNotes(ts) * 4;
}

/** Steps of `grid` in one bar of `ts`, or null when the grid does not divide the bar. */
export function stepsPerBar(ts: TimeSignature, grid: Grid): number | null {
  const s = barWholeNotes(ts) / stepWholeNotes(grid);
  const r = Math.round(s);
  return Math.abs(s - r) < 1e-9 && r >= 1 ? r : null;
}

/** Steps per beat of the time signature's denominator (for drawing beat lines). */
export function stepsPerBeat(ts: TimeSignature, grid: Grid): number {
  return 1 / ts.denominator / stepWholeNotes(grid);
}

/** Velocity multiplier for each accent (timeline::accent_velocity). */
export function accentVelocity(accent: Accent): number {
  switch (accent) {
    case 'normal':
      return 1;
    case 'strong':
      return 1.25;
    case 'soft':
      return 0.7;
    case 'bright':
      return 1.1;
  }
}

export function midiToHz(pitch: number): number {
  return 440 * Math.pow(2, (pitch - 69) / 12);
}
