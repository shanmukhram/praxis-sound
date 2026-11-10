// Take waveforms: peaks read from the take's own render, drawn as bars like the concept. The bars
// a take changed are amber (from `take.changed`); everything else grey.

import type { Take } from '../engine/EngineAdapter';
import { decodeWav, type DecodedAudio } from '../engine/wav';

const audioCache = new Map<string, Promise<DecodedAudio>>();

/** Decodes a take's render once (by URL). */
export function takeAudio(take: Take): Promise<DecodedAudio> {
  const url = take.render.audioUrl;
  let p = audioCache.get(url);
  if (!p) {
    p = fetch(url)
      .then((r) => r.arrayBuffer())
      .then(decodeWav);
    audioCache.set(url, p);
  }
  return p;
}

/** Peak envelope in `bins` buckets over the musical length, normalised to 0..1. */
export function envelope(audio: DecodedAudio, bins: number, lengthSamples?: number): number[] {
  const n = Math.min(audio.channels[0]?.length ?? 0, lengthSamples ?? Infinity);
  const out = new Array(bins).fill(0);
  for (let b = 0; b < bins; b++) {
    const a = Math.floor((b / bins) * n);
    const z = Math.max(a + 1, Math.floor(((b + 1) / bins) * n));
    let m = 0;
    for (const ch of audio.channels) for (let i = a; i < z; i += 2) m = Math.max(m, Math.abs(ch[i]));
    out[b] = m;
  }
  const max = Math.max(...out, 1e-9);
  return out.map((v) => v / max);
}

/** Union of a take's changed bars (an edit), or null for a new sound. */
export function changedSpan(take: Take): [number, number] | null {
  if (!take.patch) return null;
  const r = Object.values(take.changed);
  if (!r.length) return null;
  return [Math.min(...r.map((x) => x[0])), Math.max(...r.map((x) => x[1]))];
}

const SVGNS = 'http://www.w3.org/2000/svg';

/**
 * An SVG waveform for a take. Bars are grey placeholders until the render is decoded, then real
 * peaks. Includes a hidden playhead (`.ph`) the player moves.
 */
export function waveSvg(take: Take, bins: number, height: number): SVGSVGElement {
  const w = 1000;
  const svg = document.createElementNS(SVGNS, 'svg');
  svg.setAttribute('viewBox', `0 0 ${w} ${height}`);
  svg.setAttribute('preserveAspectRatio', 'none');
  svg.setAttribute('role', 'img');
  svg.setAttribute('aria-label', `Waveform of take ${take.n}`);
  svg.dataset.take = String(take.n);
  const bars = take.score.bars;
  const span = changedSpan(take);
  if (span) {
    const bg = document.createElementNS(SVGNS, 'rect');
    const x0 = ((span[0] - 1) / bars) * w;
    bg.setAttribute('x', x0.toFixed(1));
    bg.setAttribute('y', '0');
    bg.setAttribute('width', (((span[1] / bars) * w) - x0).toFixed(1));
    bg.setAttribute('height', String(height));
    bg.setAttribute('fill', 'rgba(245,165,36,.08)');
    bg.classList.add('chg-bg');
    svg.append(bg);
  }
  const bw = w / bins;
  const rects: SVGRectElement[] = [];
  for (let i = 0; i < bins; i++) {
    const bar = Math.floor((i / bins) * bars) + 1;
    const hot = !!span && bar >= span[0] && bar <= span[1];
    const r = document.createElementNS(SVGNS, 'rect');
    r.setAttribute('x', (i * bw + 0.6).toFixed(1));
    r.setAttribute('width', Math.max(0.5, bw - 1.2).toFixed(1));
    r.setAttribute('rx', '1');
    r.setAttribute('fill', hot ? '#f5a524' : '#7a7a7a');
    if (hot) r.classList.add('hot');
    setBar(r, 0.08, height);
    rects.push(r);
    svg.append(r);
  }
  const ph = document.createElementNS(SVGNS, 'rect');
  ph.classList.add('ph');
  ph.setAttribute('x', '0');
  ph.setAttribute('y', '0');
  ph.setAttribute('width', '2');
  ph.setAttribute('height', String(height));
  ph.setAttribute('fill', '#ffffff');
  ph.setAttribute('opacity', '0');
  svg.append(ph);
  void takeAudio(take)
    .then((audio) => {
      const env = envelope(audio, bins, take.render.report.length_samples);
      env.forEach((v, i) => setBar(rects[i], v, height));
    })
    .catch(() => undefined);
  return svg;
}

function setBar(r: SVGRectElement, v: number, height: number): void {
  const hh = Math.max(2, v * (height - 6));
  r.setAttribute('y', ((height - hh) / 2).toFixed(1));
  r.setAttribute('height', hh.toFixed(1));
}
