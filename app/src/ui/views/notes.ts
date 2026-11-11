// Notes: a piano roll of the bars around the change (WP-U1's piano roll, as a panel view) with
// the notes this take moved in amber and a "What moved" table against the previous take.
// Off-scale notes on pitched layers are flagged. Click empty space to add a note (pitched notes
// snap to the key), click a note to remove it. Each change is heard at once and joins the batch
// the next "Make a take" records (WP-U5); where the layer loops a pattern, the change lands in
// every repeat, as in the Rhythm view.

import type { ViewContext } from '../context';
import { derivePattern, editPatternInRange, toggleNote } from '../../score/patchBuilder';
import { inKey, isTonic, isUnpitchedClass, keyLabel, midiName, stepsPerBar } from '../../score/theory';
import { barSeconds, layerEvents, stepSeconds, type NoteEvent } from '../../score/timeline';
import type { BarRange, NLayer } from '../../score/types';
import { h } from '../dom';
import { defaultPitch, focus, layerPicker } from './rhythm';
import { nameOf, rangeWords, snapToKey } from '../handEdits';

const H = 220;
/** Semitones the roll shows at least. */
const ROLL_SPAN = 24;
/** How far the person moved the roll's pitch window (per layer, kept across re-renders). */
let shift = { layer: '', by: 0 };

export function renderNotes(ctx: ViewContext): HTMLElement {
  const box = h('div', { 'data-testid': 'notes-view' });
  const f = focus(ctx, 'pitched');
  if (!f) {
    box.append(h('p', { class: 'hint', style: 'margin-top:0' }, 'Add a layer in the Overview, then write its notes here.'));
    return box;
  }
  const s = ctx.score;
  const layer = f.layer;
  const name = layer.name || layer.id;
  const start = Math.max(1, f.bars.start - 2);
  const end = Math.min(f.bars.end, start + 7);
  const events = layerEvents(s, layer.id, start, end);
  const pitched = !isUnpitchedClass(layer.class) && s.key.mode !== 'chromatic';
  const prev = ctx.take ? ctx.state.takes.find((t) => t.n === ctx.take!.n - 1) : undefined;
  const key = (e: NoteEvent) => `${e.bar}:${e.stepInBar}:${e.pitch}`;
  const prevEvents = prev && ctx.take?.patch ? layerEvents(prev.score, layer.id, start, end) : null;
  const prevKeys = prevEvents ? new Set(prevEvents.map(key)) : null;
  const curKeys = new Set(events.map(key));
  const moved = prevKeys ? events.filter((e) => !prevKeys.has(key(e))) : [];

  const range = start === end ? `bar ${start}` : `bars ${start}–${end}`;
  const picker = layerPicker(ctx, layer);
  if (picker) box.append(picker);
  box.append(
    h(
      'div',
      { class: 'section-h' },
      h('h3', {}, `${name} notes, ${range}`),
      h('span', {}, pitched ? `${keyLabel(s.key)} scale only` : isUnpitchedClass(layer.class) ? 'Drum layer, no key' : 'Chromatic'),
    ),
  );

  const pitches = events.map((e) => e.pitch);
  const home = defaultPitch(ctx, [], layer);
  // Room around the notes to click new ones in (at least two octaves), moved an octave at a
  // time with "Lower notes" / "Higher notes".
  let lo = pitches.length ? Math.min(...pitches) - 5 : home - 7;
  let hi = pitches.length ? Math.max(...pitches) + 5 : home + 14;
  if (hi - lo < ROLL_SPAN) {
    const mid = Math.round((hi + lo) / 2);
    lo = mid - ROLL_SPAN / 2;
    hi = mid + ROLL_SPAN / 2;
  }
  const off = shift.layer === layer.id ? shift.by : 0;
  lo = Math.max(0, lo + off);
  hi = Math.min(127, hi + off);
  const y = (p: number) => H - 14 - ((p - lo) / (hi - lo)) * (H - 28);
  const t0 = (start - 1) * barSeconds(s);
  const dur = (end - start + 1) * barSeconds(s);
  const roll = h('div', { class: 'roll', role: 'img', 'aria-label': `Piano roll of ${name}, ${range}`, 'data-testid': 'roll', 'data-lo': lo, 'data-hi': hi });
  const move = (by: number) => {
    shift = { layer: layer.id, by: off + by };
    ctx.setViewLayer(ctx.viewLayer);
  };
  if (pitched) {
    for (let p = lo; p <= hi; p++) if (inKey(s.key, p)) roll.append(h('div', { class: `row${isTonic(s.key, p) ? ' tonic' : ''}`, style: `top:${y(p)}px` }));
  }
  if (ctx.take?.patch && f.bars.start > start) roll.append(h('div', { class: 'barline', style: `left:${(((f.bars.start - start) * barSeconds(s)) / dur) * 100}%` }));
  const movedKeys = new Set(moved.map(key));
  for (const e of events) {
    const off = pitched && !inKey(s.key, e.pitch);
    const note = h('button', {
      class: `n${movedKeys.has(key(e)) ? ' chg' : ''}${off ? ' off' : ''}`,
      style: `left:${((e.startS - t0) / dur) * 100}%;width:calc(${(e.lengthS / dur) * 100}% - 2px);top:${y(e.pitch) - 4}px`,
      title: `${midiName(e.pitch)}, bar ${e.bar}, step ${e.stepInBar + 1}${off ? ' (outside the key)' : ''}. Click to remove.`,
      'aria-label': `${midiName(e.pitch)} at bar ${e.bar}, step ${e.stepInBar + 1}`,
    });
    note.addEventListener('click', (ev) => {
      ev.stopPropagation();
      editNote(ctx, f, e.bar, e.stepInBar, e.pitch, `Removed ${midiName(e.pitch)} at bar ${e.bar}, step ${e.stepInBar + 1}`);
    });
    roll.append(note);
  }
  roll.addEventListener('click', (ev) => {
    const r = roll.getBoundingClientRect();
    const tx = t0 + ((ev.clientX - r.left) / r.width) * dur;
    const raw = Math.round(lo + ((H - 14 - (ev.clientY - r.top)) / (H - 28)) * (hi - lo));
    const pitch = pitched ? snapToKey(s.key, raw) : raw;
    const bar = Math.max(start, Math.min(end, Math.floor(tx / barSeconds(s)) + 1));
    const grid = derivePattern(s, layer.id, { start: bar, end: bar })?.pattern.grid ?? 'eighth';
    const spb = stepsPerBar(s.time_signature, grid) ?? 8;
    const step = Math.max(0, Math.min(spb - 1, Math.floor((tx - (bar - 1) * barSeconds(s)) / stepSeconds(s, grid))));
    editNote(ctx, f, bar, step, pitch, `Added ${midiName(pitch)} at bar ${bar}, step ${step + 1}`);
  });
  box.append(
    roll,
    h(
      'div',
      { class: 'roll-range' },
      h('button', { type: 'button', class: 'ghost', 'data-testid': 'notes-down', disabled: lo <= 0, onclick: () => move(-12) }, 'Lower notes'),
      h('span', { class: 'num', 'data-testid': 'roll-range' }, `${midiName(lo)} to ${midiName(hi)}`),
      h('button', { type: 'button', class: 'ghost', 'data-testid': 'notes-up', disabled: hi >= 127, onclick: () => move(12) }, 'Higher notes'),
    ),
    h(
      'div',
      { class: 'legend' },
      h('span', {}, h('i', { style: 'background:#6a6a6a' }), 'Unchanged'),
      h('span', {}, h('i', { style: 'background:#f5a524' }), ctx.take ? `Moved in take ${ctx.take.n}` : 'Moved'),
      ctx.take?.patch && f.bars.start > start ? h('span', {}, `Line marks bar ${f.bars.start}`) : null,
    ),
  );

  if (moved.length && prevEvents) {
    const rows = moved.slice(0, 6).map((e) => {
      const was = prevEvents.find((p) => p.bar === e.bar && p.stepInBar === e.stepInBar && !curKeys.has(key(p)));
      return h('tr', {}, h('td', {}, `Bar ${e.bar}, step ${e.stepInBar + 1}`), h('td', { class: 'num' }, was ? midiName(was.pitch) : 'new'), h('td', { class: 'num' }, midiName(e.pitch)));
    });
    const allIn = !pitched || moved.every((e) => inKey(s.key, e.pitch));
    box.append(
      h('div', { class: 'section-h', style: 'margin-top:18px' }, h('h3', {}, 'What moved'), h('span', {}, `${moved.length} ${moved.length === 1 ? 'note' : 'notes'}${pitched ? (allIn ? `, all in ${keyLabel(s.key)}` : ', some outside the key') : ''}`)),
      h('table', { class: 'moves', 'data-testid': 'moves' }, h('thead', {}, h('tr', {}, h('th', {}, 'Where'), h('th', { class: 'num' }, 'Was'), h('th', { class: 'num' }, 'Now'))), h('tbody', {}, ...rows)),
      moved.length > 6 ? h('p', { class: 'hint' }, `and ${moved.length - 6} more in ${range}.`) : '',
    );
  } else {
    const offCount = events.filter((e) => pitched && !inKey(s.key, e.pitch)).length;
    box.append(
      h(
        'p',
        { class: 'hint' },
        ctx.take ? `No notes moved here in take ${ctx.take.n}. ` : '',
        pitched ? (offCount ? `${offCount} notes outside ${keyLabel(s.key)} are outlined in red.` : `Every note is in ${keyLabel(s.key)}.`) : '',
        ' Click to add a note, click a note to remove it. Every change is heard at once; make a take when it sounds right.',
      ),
    );
  }
  return box;
}

/**
 * Adds or removes the note at (bar, step, pitch). Where the shown bars loop one pattern, the edit
 * is made to that pattern over the shown bars (every repeat changes, like the Rhythm view);
 * otherwise only that bar changes. Heard at once, batched for the next take.
 */
function editNote(ctx: ViewContext, f: { layer: NLayer; bars: BarRange }, bar: number, stepInBar: number, pitch: number, words: string): void {
  const layer = f.layer.id;
  const unpitched = isUnpitchedClass(f.layer.class);
  const defaults = { velocity: 0.6, accent: 'normal' as const, length: unpitched ? 1 : 1.6 };
  const loop = derivePattern(ctx.score, layer, f.bars);
  if (loop && loop.mode === 'loop' && bar >= f.bars.start && bar <= f.bars.end) {
    const step = ((bar - f.bars.start) * loop.stepsPerBar + stepInBar) % loop.pattern.length_steps;
    const patch = editPatternInRange(ctx.score, layer, f.bars, toggleNote(loop.pattern.notes, step, pitch, defaults), words);
    const repeats = ((f.bars.end - f.bars.start + 1) * loop.stepsPerBar) / loop.pattern.length_steps;
    if (patch) ctx.live(patch, `${words} of ${nameOf(f.layer)}${repeats > 1 ? ` (every repeat in ${rangeWords(f.bars)})` : ''}`, { layer, bars: f.bars }, noteGroup(f.layer));
    return;
  }
  const bars = { start: bar, end: bar };
  const d = derivePattern(ctx.score, layer, bars);
  if (!d) return;
  const patch = editPatternInRange(ctx.score, layer, bars, toggleNote(d.pattern.notes, stepInBar, pitch, defaults), words);
  if (patch) ctx.live(patch, `${words} of ${nameOf(f.layer)}`, { layer, bars }, noteGroup(f.layer));
}

/** Note clicks on one layer read once in a take's request: "edited 12 notes of Felt piano". */
function noteGroup(l: NLayer): { id: string; words: string } {
  return { id: `notes:${l.id}`, words: `edited {n} notes of ${nameOf(l)}` };
}
