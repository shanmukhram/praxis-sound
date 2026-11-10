// Mix (loudness and peak against their targets, the spectrogram, layer levels) and Export
// (FR-32). Numbers come from the current take's check report and render report.

import type { ViewContext } from '../context';
import type { ExportKind } from '../../engine/EngineAdapter';
import { drawSpectrogram } from '../spectrogram';
import { takeAudio } from '../waveform';
import { h } from '../dom';
import { scoreSeconds } from '../../score/timeline';

const pct = (v: number, lo: number, hi: number) => `${Math.max(0, Math.min(100, ((v - lo) / (hi - lo)) * 100)).toFixed(1)}%`;
const fmt = (v: number | null | undefined) => (v === null || v === undefined || !Number.isFinite(v) ? '–' : v.toFixed(1));

export function renderMix(ctx: ViewContext): HTMLElement {
  const box = h('div', { 'data-testid': 'mix-view' });
  const take = ctx.take;
  const target = ctx.score.master.loudness?.target_lufs ?? -16;
  const ceiling = ctx.score.master.loudness?.true_peak_dbtp ?? -1;
  const mock = take?.render.report.mock;
  const lufs = take?.checks.integrated_lufs ?? null;
  const peak = take?.checks.true_peak_dbtp ?? null;
  box.append(
    h(
      'div',
      { class: 'meter' },
      h('div', { class: 'section-h' }, h('h3', {}, 'Loudness'), h('span', {}, `Target ${target.toFixed(1)} LUFS`)),
      h('div', { class: 'big num', 'data-testid': 'loudness' }, fmt(lufs), h('small', {}, mock ? 'LUFS (mock estimate)' : 'LUFS')),
      h('div', { class: 'track' }, h('i', { style: `width:${lufs === null ? '0%' : pct(lufs, -30, 0)}` }), h('b', { style: `left:${pct(target, -30, 0)}` })),
      h('div', { class: 'ends2' }, h('span', {}, '−30 quiet'), h('span', {}, '0 loud')),
    ),
    h(
      'div',
      { class: 'meter' },
      h('div', { class: 'section-h' }, h('h3', {}, 'Peak'), h('span', {}, `Never above ${ceiling.toFixed(1)} dB`)),
      h('div', { class: 'big num' }, fmt(peak), h('small', {}, mock ? 'dB sample peak (mock)' : 'dB true peak')),
      h('p', { class: 'hint' }, peak !== null && peak <= ceiling + 0.05 ? 'Room for streaming and video platforms; nothing clips.' : 'Above the ceiling: the limiter needs to work harder.'),
    ),
  );
  const canvas = h('canvas', { class: 'spectro', 'data-testid': 'spectrogram', 'aria-label': 'Spectrogram of the current take' });
  box.append(h('div', { class: 'meter' }, h('div', { class: 'section-h' }, h('h3', {}, 'Spectrum over time'), h('span', {}, 'Low at the bottom')), canvas));
  if (take) void takeAudio(take).then((a) => drawSpectrogram(canvas, a)).catch(() => undefined);
  box.append(
    h(
      'div',
      { class: 'layers' },
      ...ctx.score.layers.map((l) => {
        const fx = l.effects.map((e) => (e.kind === 'eq' && e.params?.lp_hz ? `low-passed at ${(e.params.lp_hz / 1000).toFixed(1)} kHz` : e.kind === 'reverb' ? `${(e.params?.mix ?? 0) > 0.25 ? 'large' : 'small'} room reverb` : e.kind));
        return h('div', { class: 'layer' }, h('b', {}, l.name || l.id), h('small', {}, [`${l.gain_db.toFixed(1)} dB`, ...fx].join(', ')));
      }),
    ),
  );
  if (mock) box.append(h('p', { class: 'hint' }, 'Mock engine: placeholder tones, an RMS loudness estimate and sample peak. The real engine measures BS.1770 loudness and true peak.'));
  return box;
}

const FORMATS: { kind: ExportKind; label: string; note: string; group: 'audio' | 'extra' }[] = [
  { kind: 'wav24', label: 'WAV, 24-bit', note: 'Best for video editors', group: 'audio' },
  { kind: 'wav16', label: 'WAV, 16-bit', note: 'Smaller, CD quality', group: 'audio' },
  { kind: 'wav32f', label: 'WAV, 32-bit float', note: 'Headroom for further mixing', group: 'audio' },
  { kind: 'flac', label: 'FLAC', note: 'Same quality, smaller file', group: 'audio' },
  { kind: 'stems', label: 'Each layer on its own', note: '', group: 'extra' },
  { kind: 'midi', label: 'MIDI', note: 'The notes, for another music app', group: 'extra' },
  { kind: 'project', label: 'Project file', note: 'Every take and its history (.praxis)', group: 'extra' },
];

export function renderExport(ctx: ViewContext): HTMLElement {
  const take = ctx.take;
  const secs = scoreSeconds(ctx.score);
  const len = `${Math.floor(secs / 60)}:${String(Math.round(secs % 60)).padStart(2, '0')}`;
  const box = h('div', { 'data-testid': 'export-view' }, h('div', { class: 'section-h' }, h('h3', {}, take ? `Export take ${take.n}` : 'Export'), h('span', {}, `${len} ${ctx.score.playback.mode === 'loop' ? 'loop' : 'one-shot'}`)));
  const list = h('div', { class: 'export' });
  const inputs = new Map<ExportKind, HTMLInputElement>();
  const layerNames = ctx.score.layers.map((l) => (l.name || l.id).toLowerCase()).join(' and ');
  for (const f of FORMATS) {
    const input = h('input', { type: f.group === 'audio' ? 'radio' : 'checkbox', name: f.group === 'audio' ? 'fmt' : f.kind, checked: f.kind === 'wav24', 'data-testid': `export-${f.kind}` });
    inputs.set(f.kind, input);
    list.append(h('label', {}, input, h('span', {}, f.label, h('small', {}, f.kind === 'stems' ? `${layerNames} as separate files` : f.note))));
  }
  const result = h('div', { 'data-testid': 'export-result' });
  const go = h('button', { class: 'wide', 'data-testid': 'export-go' }, 'Export');
  go.addEventListener('click', async () => {
    const kinds = FORMATS.map((f) => f.kind).filter((k) => inputs.get(k)!.checked);
    const base = (ctx.score.title || 'praxis').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'praxis';
    const r = await ctx.engine.export({ kinds, destination: 'exports', baseName: take ? `${base}-take-${take.n}` : base });
    result.replaceChildren(h('p', { class: 'hint', style: `color:var(${r.ok ? '--positive' : '--negative'})` }, r.message), h('ul', { class: 'files' }, ...r.files.map((x) => h('li', {}, x.path))));
  });
  box.append(list, go, result, h('p', { class: 'hint' }, 'Exports are the sound itself. A reference clip you dropped in is never mixed into them.'));
  return box;
}
