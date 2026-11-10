// Piano roll (FR-29) for the pattern a layer plays in the selected bars. Rows of the score's key
// are highlighted (tonic rows marked); notes outside the key on a pitched layer are flagged,
// because the validator rejects them (`off_scale`). Click empty space to add a note, click a note
// to select it, drag its right edge to change length, Delete removes the selected note.

import type { AppState, Controller } from './state';
import type { Grid, NLayer, NScore, Note } from '../score/types';
import { inKey, isTonic, isUnpitchedClass, keyLabel, midiName, stepsPerBeat } from '../score/theory';
import { derivePattern, editPatternInRange, findNote, playedVelocity, toggleNote, updateNote } from '../score/patchBuilder';
import { cssVar, fitCanvas, h, layerColor } from './dom';
import { pitchLabel } from './stepSequencer';

const MIN_ROW_H = 8;
const MAX_ROW_H = 14;
const KEYS_W = 64;

interface Geometry {
  lo: number;
  hi: number;
  steps: number;
  cw: number;
  rowH: number;
}

export class PianoRoll {
  readonly el: HTMLElement;
  private info: HTMLElement;
  private canvas: HTMLCanvasElement;
  private geo: Geometry | null = null;
  private resize: { note: Note; startX: number; length: number } | null = null;

  constructor(private readonly app: Controller) {
    this.info = h('div', { class: 'hint', style: 'padding:6px 12px', 'data-testid': 'roll-info' });
    this.canvas = h('canvas', {
      tabindex: 0,
      'data-testid': 'piano-roll',
      'aria-label': 'Piano roll: click to add a note, click a note to select, drag its right edge to set length, Delete removes',
    });
    this.el = h('div', { class: 'roll' }, this.info, this.canvas);
    this.canvas.addEventListener('mousedown', (e) => void this.onDown(e));
    window.addEventListener('mousemove', (e) => this.onMove(e));
    window.addEventListener('mouseup', () => void this.onUp());
    this.canvas.addEventListener('keydown', (e) => {
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        void this.removeSelected();
      }
    });
    new ResizeObserver(() => this.draw(this.app.state)).observe(this.canvas);
  }

  update(s: AppState): void {
    this.draw(s);
  }

  private context(s: AppState): { score: NScore; layer: NLayer; notes: Note[]; steps: number; spb: number; grid: Grid } | null {
    const score = s.score;
    const layer = score?.layers.find((l) => l.id === s.selection.layerId);
    if (!score || !layer) return null;
    const d = derivePattern(score, layer.id, s.selection.bars);
    if (!d) return null;
    return { score, layer, notes: d.pattern.notes, steps: d.pattern.length_steps, spb: d.stepsPerBar, grid: d.pattern.grid };
  }

  draw(s: AppState): void {
    const c = this.context(s);
    if (!c) {
      this.info.textContent = 'Select a layer and bars on the timeline to edit notes.';
      fitCanvas(this.canvas, 40);
      this.geo = null;
      return;
    }
    const { score, layer, notes, steps } = c;
    const pitched = !isUnpitchedClass(layer.class) && score.key.mode !== 'chromatic';
    const pitches = notes.map((n) => n.pitch);
    let lo = pitches.length ? Math.min(...pitches) - 4 : 48;
    let hi = pitches.length ? Math.max(...pitches) + 4 : 72;
    if (hi - lo < 24) {
      const mid = Math.round((hi + lo) / 2);
      lo = mid - 12;
      hi = mid + 12;
    }
    lo = Math.max(0, lo);
    hi = Math.min(127, hi);
    const rows = hi - lo + 1;
    // Fit the rows to the panel height where possible; scroll beyond the minimum row height.
    const avail = (this.el.parentElement?.clientHeight ?? 400) - this.info.offsetHeight - 6;
    const ROW_H = Math.max(MIN_ROW_H, Math.min(MAX_ROW_H, Math.floor(avail / rows)));
    const { ctx, w } = fitCanvas(this.canvas, rows * ROW_H);
    const cw = (w - KEYS_W) / steps;
    this.geo = { lo, hi, steps, cw, rowH: ROW_H };

    const offScale = pitched ? notes.filter((n) => !inKey(score.key, n.pitch)) : [];
    this.info.textContent = [
      pitched ? `Key ${keyLabel(score.key)}: scale rows lighter, tonic rows marked` : isUnpitchedClass(layer.class) ? 'Drum layer: no scale check' : 'Chromatic: no scale check',
      offScale.length ? `${offScale.length} off-scale note${offScale.length > 1 ? 's' : ''} flagged (${offScale.map((n) => `${midiName(n.pitch)}@${n.step + 1}`).join(', ')})` : pitched ? 'all notes in key' : '',
      layer.locked ? 'layer is locked: unlock it (L) to edit' : '',
    ]
      .filter(Boolean)
      .join(' · ');
    this.info.dataset.offscale = String(offScale.length);

    const laneIn = cssVar('--lane-in');
    const laneOut = cssVar('--lane-out');
    const line = cssVar('--line');
    const strong = cssVar('--line-strong');
    const muted = cssVar('--muted');
    const accent = cssVar('--accent');
    const bad = cssVar('--bad');
    const text = cssVar('--text');
    ctx.font = '10px ui-monospace, Consolas, monospace';
    ctx.textBaseline = 'middle';

    for (let p = hi; p >= lo; p--) {
      const y = (hi - p) * ROW_H;
      const scaleRow = !pitched || inKey(score.key, p);
      ctx.fillStyle = scaleRow ? laneIn : laneOut;
      ctx.fillRect(KEYS_W, y, w - KEYS_W, ROW_H);
      if (pitched && isTonic(score.key, p)) {
        ctx.fillStyle = accent;
        ctx.globalAlpha = 0.1;
        ctx.fillRect(KEYS_W, y, w - KEYS_W, ROW_H);
        ctx.globalAlpha = 1;
        ctx.fillRect(KEYS_W - 3, y + 1, 3, ROW_H - 2);
      }
      ctx.fillStyle = line;
      ctx.fillRect(KEYS_W, y + ROW_H - 0.5, w - KEYS_W, 0.5);
      const black = [1, 3, 6, 8, 10].includes(((p % 12) + 12) % 12);
      ctx.fillStyle = black ? laneOut : laneIn;
      ctx.fillRect(0, y, KEYS_W - 4, ROW_H);
      ctx.fillStyle = scaleRow ? text : muted;
      const pc = ((p % 12) + 12) % 12;
      const labelled = ROW_H >= 12 ? !black || isUnpitchedClass(layer.class) : pc === 0 || (pitched && isTonic(score.key, p));
      if (labelled) ctx.fillText(pitchLabel(layer, p).slice(0, 10), 4, y + ROW_H / 2);
    }
    const beat = stepsPerBeat(score.time_signature, c.grid);
    for (let i = 0; i <= steps; i++) {
      const x = KEYS_W + i * cw;
      const isBar = i % c.spb === 0;
      const isBeat = Number.isInteger(beat) && i % beat === 0;
      ctx.fillStyle = isBar ? strong : line;
      ctx.globalAlpha = isBar || isBeat ? 1 : 0.4;
      ctx.fillRect(Math.round(x), 0, 1, rows * ROW_H);
      ctx.globalAlpha = 1;
    }

    const color = layerColor(score.layers.indexOf(layer));
    for (const n of notes) {
      if (n.pitch < lo || n.pitch > hi) continue;
      const x = KEYS_W + n.step * cw;
      const y = (hi - n.pitch) * ROW_H;
      const nw = Math.max(3, n.length * cw - 1);
      const off = pitched && !inKey(score.key, n.pitch);
      ctx.globalAlpha = 0.45 + 0.55 * playedVelocity(n);
      ctx.fillStyle = off ? bad : color;
      ctx.fillRect(x + 1, y + 1, nw, ROW_H - 2);
      ctx.globalAlpha = 1;
      if (n.accent === 'strong' || n.accent === 'bright') {
        ctx.fillStyle = n.accent === 'bright' ? cssVar('--warn') : text;
        ctx.fillRect(x + 1, y + 1, 2, ROW_H - 2);
      }
      const sel = s.selectedNote && s.selectedNote.step === n.step && s.selectedNote.pitch === n.pitch;
      if (sel || off) {
        ctx.strokeStyle = off ? bad : text;
        ctx.lineWidth = sel ? 2 : 1;
        ctx.strokeRect(x + 1, y + 1, nw, ROW_H - 2);
      }
      if (off) {
        ctx.fillStyle = text;
        ctx.fillText('!', x + 4, y + ROW_H / 2);
      }
    }
  }

  private at(e: MouseEvent): { step: number; pitch: number; x: number } | null {
    if (!this.geo) return null;
    const r = this.canvas.getBoundingClientRect();
    const x = e.clientX - r.left - KEYS_W;
    const y = e.clientY - r.top;
    if (x < 0) return null;
    const step = Math.floor(x / this.geo.cw);
    const pitch = this.geo.hi - Math.floor(y / this.geo.rowH);
    if (step < 0 || step >= this.geo.steps || pitch < this.geo.lo || pitch > this.geo.hi) return null;
    return { step, pitch, x };
  }

  private async onDown(e: MouseEvent): Promise<void> {
    const c = this.context(this.app.state);
    const hit = this.at(e);
    if (!c || !hit || !this.geo) return;
    const cw = this.geo.cw;
    const covering = c.notes.find((n) => n.pitch === hit.pitch && hit.x >= n.step * cw && hit.x <= (n.step + n.length) * cw);
    if (covering) {
      this.app.set({ selectedNote: { step: covering.step, pitch: covering.pitch }, focusPitch: covering.pitch });
      if (hit.x >= (covering.step + covering.length) * cw - 6) this.resize = { note: covering, startX: e.clientX, length: covering.length };
      return;
    }
    const ok = await this.edit('Add note', (ns) => toggleNote(ns, hit.step, hit.pitch, { velocity: 0.7, accent: 'normal', length: 1 }));
    if (ok) this.app.set({ selectedNote: { step: hit.step, pitch: hit.pitch } });
  }

  private onMove(e: MouseEvent): void {
    if (!this.resize || !this.geo) return;
    const delta = (e.clientX - this.resize.startX) / this.geo.cw;
    this.resize.length = Math.max(0.25, Math.round((this.resize.note.length + delta) * 4) / 4);
  }

  private async onUp(): Promise<void> {
    const r = this.resize;
    this.resize = null;
    const c = this.context(this.app.state);
    if (!r || !c || r.length === r.note.length) return;
    await this.edit('Set length', (ns) => updateNote(ns, r.note.step, r.note.pitch, { length: r.length }));
  }

  private async removeSelected(): Promise<void> {
    const s = this.app.state;
    const c = this.context(s);
    if (!c || !s.selectedNote) return;
    const { step, pitch } = s.selectedNote;
    if (!findNote(c.notes, step, pitch)) return;
    const ok = await this.edit('Remove note', (ns) => ns.filter((x) => !(x.step === step && x.pitch === pitch)));
    if (ok) this.app.set({ selectedNote: null });
  }

  /** Edits the notes of the selected range as they are once earlier edits have landed. */
  private edit(what: string, change: (notes: Note[]) => Note[]): Promise<boolean> {
    return this.app.edit((score, st) => {
      const layer = score.layers.find((l) => l.id === st.selection.layerId);
      const d = layer ? derivePattern(score, layer.id, st.selection.bars) : null;
      return layer && d ? editPatternInRange(score, layer.id, st.selection.bars, change(d.pattern.notes), what) : null;
    });
  }
}
