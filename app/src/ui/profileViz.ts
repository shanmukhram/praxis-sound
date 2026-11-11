// Small visuals of a Sound Profile (ADR-0011: plain words first, small pictures beside them):
// the loudness envelope with its onsets and section changes, the spectral balance in ten octave
// bands, and the groove's accents per step. Neutral greys: the amber accent stays on the take.

import type { SoundProfile } from '../engine/reference';
import { clockS } from '../engine/reference';
import { h } from './dom';

const NS = 'http://www.w3.org/2000/svg';

function el<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number>): SVGElementTagNameMap[K] {
  const e = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
}

/** dB to 0..1 over a 60 dB window under the loudest frame. */
export function envelopeHeights(framesDb: number[], window = 60): number[] {
  if (!framesDb.length) return [];
  let top = -Infinity;
  for (const f of framesDb) if (Number.isFinite(f) && f > top) top = f;
  if (!Number.isFinite(top)) return framesDb.map(() => 0);
  return framesDb.map((f) => (Number.isFinite(f) ? Math.max(0, Math.min(1, (f - (top - window)) / window)) : 0));
}

/** The envelope as an area, onsets as ticks under it, section changes as rules. */
export function envelopeSvg(p: SoundProfile): SVGSVGElement {
  const W = 1000;
  const H = 72;
  const ENV = 56;
  const svg = el('svg', { viewBox: `0 0 ${W} ${H}`, preserveAspectRatio: 'none', class: 'viz-env', role: 'img', 'data-testid': 'profile-envelope' });
  svg.setAttribute('aria-label', `Loudness over ${clockS(p.source.duration_s)}, ${p.onsets_s.length} onsets${p.sections.length > 1 ? `, ${p.sections.length} sections` : ''}`);
  const hs = envelopeHeights(p.envelope.frames_db);
  const dur = p.source.duration_s > 0 ? p.source.duration_s : Math.max(1e-3, hs.length * p.envelope.hop_s);
  if (hs.length) {
    const n = hs.length;
    const pts = hs.map((v, i) => `${((i / Math.max(1, n - 1)) * W).toFixed(1)},${(ENV - v * (ENV - 2)).toFixed(1)}`);
    svg.append(el('path', { d: `M0,${ENV} L${pts.join(' L')} L${W},${ENV} Z`, class: 'env-area' }));
  } else {
    svg.append(el('line', { x1: 0, x2: W, y1: ENV, y2: ENV, class: 'env-none' }));
  }
  for (const s of p.sections.slice(1)) {
    const x = (s.start_s / dur) * W;
    svg.append(el('line', { x1: x, x2: x, y1: 0, y2: ENV, class: 'sec-rule' }));
  }
  const g = el('g', { 'data-testid': 'profile-onsets' });
  for (const t of p.onsets_s) {
    const x = (t / dur) * W;
    if (x < 0 || x > W) continue;
    g.append(el('line', { x1: x, x2: x, y1: ENV + 5, y2: H, class: 'onset' }));
  }
  svg.append(g);
  return svg;
}

/** Ten octave bands as bars, the loudest full height, 36 dB below it empty. */
export function bandHeights(bandsDb: number[], window = 36): number[] {
  const finite = bandsDb.filter((b) => Number.isFinite(b));
  if (!finite.length) return bandsDb.map(() => 0);
  const top = Math.max(...finite);
  return bandsDb.map((b) => (Number.isFinite(b) ? Math.max(0, Math.min(1, (b - (top - window)) / window)) : 0));
}

const BAND_HZ = ['31', '63', '125', '250', '500', '1k', '2k', '4k', '8k', '16k'];

export function bandsViz(p: SoundProfile): HTMLElement {
  const hs = bandHeights(p.spectral.bands_db);
  return h(
    'div',
    { class: 'viz-bands', 'data-testid': 'profile-bands', role: 'img', 'aria-label': 'Spectral balance from low (left) to high (right)' },
    ...hs.map((v, i) => h('span', { title: `${BAND_HZ[i] ?? ''} Hz: ${Number.isFinite(p.spectral.bands_db[i]) ? p.spectral.bands_db[i].toFixed(1) : '–'} dB` }, h('i', { style: `height:${Math.max(4, Math.round(v * 100))}%` }))),
  );
}

/** The groove's accent per step, beats marked. */
export function grooveViz(p: SoundProfile): HTMLElement | null {
  const g = p.groove;
  if (!g || !g.step_weights.length) return null;
  const per = Math.max(1, Math.round(g.steps_per_bar / Math.max(1, p.tempo?.beats_per_bar ?? 4)));
  return h(
    'div',
    { class: 'viz-groove', style: `grid-template-columns: repeat(${g.step_weights.length}, minmax(0, 1fr))`, 'data-testid': 'profile-groove', role: 'img', 'aria-label': 'Accents across one bar' },
    ...g.step_weights.map((w, i) => h('span', { class: i % per === 0 ? 'beat' : '' }, h('i', { style: `height:${Math.max(6, Math.round(Math.max(0, Math.min(1, w)) * 100))}%` }))),
  );
}
