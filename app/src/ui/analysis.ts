// Waveform and spectrogram (FR-30), fed by the adapter's render. With the mock engine the audio is
// a placeholder synthesis and the panels say so.

import type { AppState, Controller } from './state';
import type { DecodedAudio } from '../engine/wav';
import { cssVar, fitCanvas, fmtDb, h } from './dom';

/** In-place radix-2 FFT on real/imag arrays (length a power of two). */
export function fft(re: Float32Array, im: Float32Array): void {
  const n = re.length;
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      [re[i], re[j]] = [re[j], re[i]];
      [im[i], im[j]] = [im[j], im[i]];
    }
  }
  for (let len = 2; len <= n; len <<= 1) {
    const ang = (-2 * Math.PI) / len;
    const wr = Math.cos(ang);
    const wi = Math.sin(ang);
    for (let i = 0; i < n; i += len) {
      let cr = 1;
      let ci = 0;
      for (let k = 0; k < len / 2; k++) {
        const a = i + k;
        const b = a + len / 2;
        const tr = re[b] * cr - im[b] * ci;
        const ti = re[b] * ci + im[b] * cr;
        re[b] = re[a] - tr;
        im[b] = im[a] - ti;
        re[a] += tr;
        im[a] += ti;
        const ncr = cr * wr - ci * wi;
        ci = cr * wi + ci * wr;
        cr = ncr;
      }
    }
  }
}

/** Magnitude spectrogram in dB: columns of `size/2` bins, hop `hop`. Mono mix of all channels. */
export function spectrogram(audio: DecodedAudio, size = 2048, hop = 1024): { cols: Float32Array[]; bins: number } {
  const ch = audio.channels;
  const n = ch[0]?.length ?? 0;
  const win = new Float32Array(size).map((_, i) => 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / (size - 1)));
  const cols: Float32Array[] = [];
  const re = new Float32Array(size);
  const im = new Float32Array(size);
  for (let start = 0; start + size <= n; start += hop) {
    for (let i = 0; i < size; i++) {
      let x = 0;
      for (const c of ch) x += c[start + i];
      re[i] = (x / ch.length) * win[i];
      im[i] = 0;
    }
    fft(re, im);
    const col = new Float32Array(size / 2);
    for (let k = 0; k < size / 2; k++) col[k] = 10 * Math.log10(re[k] * re[k] + im[k] * im[k] + 1e-12);
    cols.push(col);
  }
  return { cols, bins: size / 2 };
}

export class AnalysisView {
  readonly el: HTMLElement;
  private wave: HTMLCanvasElement;
  private spec: HTMLCanvasElement;
  private report: HTMLElement;
  private specImage: { audio: DecodedAudio; canvas: HTMLCanvasElement } | null = null;

  constructor(private readonly app: Controller) {
    this.wave = h('canvas', { 'data-testid': 'waveform', 'aria-label': 'Waveform of the last render' });
    this.spec = h('canvas', { 'data-testid': 'spectrogram', 'aria-label': 'Spectrogram of the last render' });
    this.report = h('div', { class: 'report', 'data-testid': 'render-report' });
    const caption = (label: string) => h('figcaption', {}, h('span', {}, label), h('span', { class: 'mock-note' }, ''));
    this.el = h(
      'section',
      { class: 'analysis', 'aria-label': 'Render analysis' },
      h('figure', {}, caption('Waveform'), this.wave),
      h('figure', {}, caption('Spectrogram (log frequency)'), this.spec),
      this.report,
    );
    new ResizeObserver(() => this.update(this.app.state)).observe(this.wave);
  }

  update(s: AppState): void {
    const mock = this.app.engine.kind === 'mock';
    this.el.querySelectorAll('.mock-note').forEach((e) => (e.textContent = mock ? 'mock render: placeholder synthesis' : ''));
    this.drawWave(s);
    this.drawSpec(s);
    this.renderReport(s);
  }

  updatePosition(s: AppState): void {
    this.drawWave(s);
    this.drawSpec(s);
  }

  private playheadX(s: AppState, w: number): number | null {
    const r = s.render?.result.report;
    if (!r || !s.score) return null;
    const total = (r.length_samples + r.tail_samples) / r.sample_rate;
    return (s.position.seconds / total) * w;
  }

  private drawWave(s: AppState): void {
    const { ctx, w, h: hh } = fitCanvas(this.wave);
    const audio = s.render?.audio;
    if (!audio) return this.placeholder(ctx, w, hh, s.rendering ? 'Rendering…' : 'Render (R) to see the waveform');
    const ch = audio.channels;
    const n = ch[0].length;
    const mid = hh / 2;
    ctx.fillStyle = cssVar('--accent');
    for (let x = 0; x < w; x++) {
      const a = Math.floor((x / w) * n);
      const b = Math.max(a + 1, Math.floor(((x + 1) / w) * n));
      let lo = 0;
      let hi = 0;
      for (let i = a; i < b; i += 4) {
        for (const c of ch) {
          if (c[i] < lo) lo = c[i];
          if (c[i] > hi) hi = c[i];
        }
      }
      ctx.fillRect(x, mid - hi * mid * 0.95, 1, Math.max(1, (hi - lo) * mid * 0.95));
    }
    this.barLines(ctx, s, w, hh);
    this.drawPlayhead(ctx, s, w, hh);
  }

  private drawSpec(s: AppState): void {
    const { ctx, w, h: hh } = fitCanvas(this.spec);
    const audio = s.render?.audio;
    if (!audio) return this.placeholder(ctx, w, hh, s.rendering ? 'Rendering…' : 'Render (R) to see the spectrogram');
    if (!this.specImage || this.specImage.audio !== audio) this.specImage = { audio, canvas: this.buildSpec(audio) };
    ctx.imageSmoothingEnabled = true;
    ctx.drawImage(this.specImage.canvas, 0, 0, w, hh);
    this.barLines(ctx, s, w, hh);
    this.drawPlayhead(ctx, s, w, hh);
  }

  /** Renders the spectrogram once into an offscreen canvas (columns x 160 log-frequency rows). */
  private buildSpec(audio: DecodedAudio): HTMLCanvasElement {
    const { cols, bins } = spectrogram(audio);
    const rows = 160;
    const c = document.createElement('canvas');
    c.width = Math.max(1, cols.length);
    c.height = rows;
    const g = c.getContext('2d')!;
    const img = g.createImageData(c.width, rows);
    const nyq = audio.sampleRate / 2;
    const fmin = 30;
    let max = -Infinity;
    for (const col of cols) for (const v of col) max = Math.max(max, v);
    for (let x = 0; x < cols.length; x++) {
      for (let y = 0; y < rows; y++) {
        const f = fmin * Math.pow(nyq / fmin, 1 - y / (rows - 1));
        const k = Math.min(bins - 1, Math.round((f / nyq) * bins));
        const t = Math.pow(Math.max(0, Math.min(1, (cols[x][k] - (max - 90)) / 90)), 1.6);
        const [r, gg, b] = ramp(t);
        const o = (y * c.width + x) * 4;
        img.data[o] = r;
        img.data[o + 1] = gg;
        img.data[o + 2] = b;
        img.data[o + 3] = 255;
      }
    }
    g.putImageData(img, 0, 0);
    return c;
  }

  private barLines(ctx: CanvasRenderingContext2D, s: AppState, w: number, hh: number): void {
    const r = s.render?.result.report;
    if (!r || !s.score) return;
    const total = (r.length_samples + r.tail_samples) / r.sample_rate;
    const barS = r.length_samples / r.sample_rate / s.score.bars;
    ctx.fillStyle = cssVar('--line-strong');
    ctx.globalAlpha = 0.5;
    for (let b = 1; b < s.score.bars; b++) ctx.fillRect(Math.round(((b * barS) / total) * w), 0, 1, hh);
    ctx.globalAlpha = 1;
  }

  private drawPlayhead(ctx: CanvasRenderingContext2D, s: AppState, w: number, hh: number): void {
    const x = this.playheadX(s, w);
    if (x === null) return;
    ctx.fillStyle = cssVar('--text');
    ctx.fillRect(Math.round(x), 0, 1.5, hh);
  }

  private placeholder(ctx: CanvasRenderingContext2D, w: number, hh: number, text: string): void {
    ctx.fillStyle = cssVar('--faint');
    ctx.font = '12px system-ui, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(text, w / 2, hh / 2);
    ctx.textAlign = 'start';
  }

  private renderReport(s: AppState): void {
    const r = s.render?.result.report;
    this.report.replaceChildren(h('h3', { style: 'margin:0 0 4px;font-size:11px;text-transform:uppercase;letter-spacing:.06em;color:var(--muted)' }, 'Render report'));
    if (!r) {
      this.report.append(h('div', { class: 'hint' }, s.rendering ? 'Rendering…' : 'No render yet.'));
      return;
    }
    const row = (k: string, v: string) => [h('dt', { class: 'k' }, k), h('dd', {}, v)];
    this.report.append(
      h(
        'dl',
        {},
        ...row('Length', `${(r.length_samples / r.sample_rate).toFixed(2)} s${r.is_loop ? ' (loop)' : ''}`),
        ...row('Loudness', `${fmtDb(r.loudness_lufs)} ${r.mock ? 'dB RMS (est.)' : 'LUFS'}`),
        ...row('Peak', `${fmtDb(r.true_peak_dbtp)} ${r.mock ? 'dBFS' : 'dBTP'}`),
        ...row('Gain', `${fmtDb(r.applied_gain_db)} dB`),
        ...row('Notes', String(r.notes_rendered)),
        ...row('Blocked', r.blocked_layers.length ? r.blocked_layers.join(', ') : 'none'),
        ...row('Time', `${(r.render_seconds * 1000).toFixed(0)} ms`),
      ),
      ...r.warnings.map((w) => h('div', { class: 'hint', style: 'color:var(--warn)' }, w)),
    );
  }
}

/** Dark-to-light ramp: near-black -> blue -> amber -> white. */
function ramp(t: number): [number, number, number] {
  const stops: [number, number, number, number][] = [
    [0, 14, 16, 20],
    [0.35, 38, 70, 130],
    [0.65, 200, 140, 70],
    [1, 250, 245, 230],
  ];
  for (let i = 1; i < stops.length; i++) {
    if (t <= stops[i][0]) {
      const [t0, r0, g0, b0] = stops[i - 1];
      const [t1, r1, g1, b1] = stops[i];
      const u = (t - t0) / (t1 - t0);
      return [r0 + (r1 - r0) * u, g0 + (g1 - g0) * u, b0 + (b1 - b0) * u];
    }
  }
  return [250, 245, 230];
}
