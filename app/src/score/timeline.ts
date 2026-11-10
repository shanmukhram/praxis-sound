// Time mapping and note expansion, ported from crates/praxis-score/src/timeline.rs.

import type { Accent, Grid, NScore, Pattern } from './types';
import { accentVelocity, barQuarters, stepWholeNotes, stepsPerBar } from './theory';

export function barSeconds(score: Pick<NScore, 'tempo' | 'time_signature'>): number {
  return (barQuarters(score.time_signature) * 60) / score.tempo.bpm;
}

export function stepSeconds(score: Pick<NScore, 'tempo'>, grid: Grid): number {
  return (stepWholeNotes(grid) * 4 * 60) / score.tempo.bpm;
}

/** Musical length in seconds (bars only, no tail). */
export function scoreSeconds(score: NScore): number {
  return score.bars * barSeconds(score);
}

/** Seconds of tail after the bars: one-shots only, loops wrap. */
export function tailSeconds(score: NScore): number {
  return score.playback.mode === 'one_shot' ? (score.playback.tail_seconds ?? 4) : 0;
}

/** Fractional 1-based bar position for a time in seconds. */
export function secondsToBarPos(score: NScore, s: number): number {
  return 1 + s / barSeconds(score);
}

export function barPosToSeconds(score: NScore, bar: number): number {
  return (bar - 1) * barSeconds(score);
}

export interface NoteEvent {
  layer: string;
  layerIndex: number;
  /** 1-based bar the note starts in. */
  bar: number;
  stepInBar: number;
  grid: Grid;
  startS: number;
  lengthS: number;
  pitch: number;
  /** Velocity after clip scale and accent, clamped to 1. */
  velocity: number;
  accent: Accent;
  /** The source note's own fields (velocity before accent, length and offset in steps). */
  noteVelocity: number;
  noteLength: number;
  noteOffset: number;
  transpose: number;
  velocityScale: number;
}

/** Expands every clip of every layer into absolute note events, like timeline::expand_notes. */
export function expandNotes(score: NScore): NoteEvent[] {
  const out: NoteEvent[] = [];
  score.layers.forEach((layer, li) => {
    for (const clip of layer.clips) {
      const pat = score.patterns.find((p) => p.id === clip.pattern);
      if (!pat) continue;
      out.push(...expandClip(score, layer.id, li, pat, clip.bars.start, clip.bars.end, clip));
    }
  });
  out.sort((a, b) => a.startS - b.startS || a.layerIndex - b.layerIndex || a.pitch - b.pitch);
  return out;
}

function expandClip(
  score: NScore,
  layerId: string,
  layerIndex: number,
  pat: Pattern,
  start: number,
  end: number,
  clip: { pattern_offset_steps?: number; transpose?: number; velocity_scale?: number },
): NoteEvent[] {
  const out: NoteEvent[] = [];
  const spb = stepsPerBar(score.time_signature, pat.grid);
  if (spb === null || pat.length_steps === 0) return out;
  const first = Math.max(start, 1);
  const last = Math.min(end, score.bars);
  if (last < first) return out;
  const stepS = stepSeconds(score, pat.grid);
  const clipSteps = (last - first + 1) * spb;
  const len = pat.length_steps;
  const offset = (clip.pattern_offset_steps ?? 0) % len;
  const clipStartS = (first - 1) * barSeconds(score);
  for (let cycle = -offset; cycle < clipSteps; cycle += len) {
    for (const n of pat.notes) {
      if (n.step >= len) continue;
      const rel = cycle + n.step;
      if (rel < 0 || rel >= clipSteps) continue;
      const bar = first + Math.floor(rel / spb);
      const stepInBar = rel % spb;
      const accent = n.accent ?? 'normal';
      out.push({
        layer: layerId,
        layerIndex,
        bar,
        stepInBar,
        grid: pat.grid,
        startS: Math.max(0, clipStartS + (rel + (n.offset ?? 0)) * stepS),
        lengthS: n.length * stepS,
        pitch: Math.min(127, Math.max(0, n.pitch + (clip.transpose ?? 0))),
        velocity: Math.min(1, Math.max(0, n.velocity * (clip.velocity_scale ?? 1) * accentVelocity(accent))),
        accent,
        noteVelocity: n.velocity,
        noteLength: n.length,
        noteOffset: n.offset ?? 0,
        transpose: clip.transpose ?? 0,
        velocityScale: clip.velocity_scale ?? 1,
      });
    }
  }
  return out;
}

/** Note events of one layer, optionally limited to bars [from, to]. */
export function layerEvents(score: NScore, layerId: string, from = 1, to = score.bars): NoteEvent[] {
  return expandNotes(score).filter((e) => e.layer === layerId && e.bar >= from && e.bar <= to);
}

/** A comparable fingerprint of one note event (for isolation checks). */
export function eventKey(e: NoteEvent): string {
  return [e.layer, e.bar, e.stepInBar, e.grid, e.pitch, e.startS.toFixed(9), e.lengthS.toFixed(9), e.velocity.toFixed(6), e.accent].join('|');
}
