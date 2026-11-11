// "Where each layer plays": the pinned change map at the top of the Sound panel. Cells light from
// `take.changed`; dragging across bars of one layer sets the scope of the next request.
//
// Transport (WP-U5): the bar numbers under the lanes seek (click one to play from there), and the
// loop strip below them sets the loop range by dragging along it. The strip is its own row, so
// setting a loop never touches the scope a drag across the lanes sets. A white line marks where
// the transport is; the row at the bottom plays and stops (Space does too).

import type { ViewContext } from '../context';
import { changeMapModel } from '../changeMapModel';
import { h } from '../dom';
import { barSeconds, scoreSeconds } from '../../score/timeline';
import { clock } from './overview';

const ICON_PLAY = '<svg width="12" height="12" viewBox="0 0 14 14" aria-hidden="true"><path d="M3 1.5v11l9.5-5.5z" fill="currentColor"/></svg>';
const ICON_STOP = '<svg width="12" height="12" viewBox="0 0 14 14" aria-hidden="true"><rect x="2" y="2" width="10" height="10" rx="1" fill="currentColor"/></svg>';

let drag: { layer: string; from: number } | null = null;
let dragTo = 0;
/** A loop being dragged on the loop strip: its first bar and the bar under the pointer. */
let loopDrag: { from: number; to: number; set: (from: number, to: number) => void } | null = null;
window.addEventListener('pointerup', () => {
  drag = null;
  if (loopDrag) {
    const d = loopDrag;
    loopDrag = null;
    d.set(d.from, d.to);
  }
});

export function renderChangeMap(ctx: ViewContext): HTMLElement {
  const map = changeMapModel(ctx.take, ctx.score, ctx.selection);
  const cols = `grid-template-columns: repeat(${map.bars}, minmax(0, 1fr))`;
  const box = h(
    'div',
    { class: 'arr', 'data-testid': 'change-map' },
    h('div', { class: 'section-h' }, h('h3', {}, 'Where each layer plays'), h('span', {}, ctx.take ? `Take ${ctx.take.n}` : ctx.state.draft ? 'No take yet' : '')),
  );
  if (map.sections.length) {
    box.append(
      h(
        'div',
        { class: 'sections', style: cols },
        ...map.sections.map((s) => h('div', { style: `grid-column: ${s.from} / ${s.to + 1}`, title: s.name }, s.name)),
      ),
    );
  }
  if (!map.lanes.length) box.append(h('div', { class: 'lane empty-lane' }, h('span', {}, 'No layers'), h('div', { class: 'cells', style: cols }, ...Array.from({ length: map.bars }, () => h('i', { class: 'cell ghost-cell' })))));
  for (const lane of map.lanes) {
    const cells = h('div', { class: 'cells', style: cols, role: 'group', 'aria-label': `${lane.name} bars` });
    for (const c of lane.cells) {
      const cell = h('button', {
        class: `cell${c.on ? ' on' : ''}${c.chg ? ' chg' : ''}${c.sel ? ' sel' : ''}`,
        'data-layer': lane.layer,
        'data-bar': c.bar,
        'data-testid': `cell-${lane.layer}-${c.bar}`,
        'aria-label': `${lane.name} bar ${c.bar}${c.on ? '' : ', silent'}${c.chg ? ', changed in this take' : ''}`,
      });
      cell.addEventListener('pointerdown', (e: PointerEvent) => {
        e.preventDefault();
        drag = { layer: lane.layer, from: c.bar };
        dragTo = c.bar;
        ctx.select({ layer: lane.layer, bars: { start: c.bar, end: c.bar } });
      });
      cell.addEventListener('pointerenter', () => {
        if (!drag || drag.layer !== lane.layer || dragTo === c.bar) return;
        dragTo = c.bar;
        ctx.select({ layer: lane.layer, bars: { start: Math.min(drag.from, c.bar), end: Math.max(drag.from, c.bar) } });
      });
      cells.append(cell);
    }
    box.append(h('div', { class: 'lane' }, h('span', { title: lane.locked ? `${lane.name} (locked)` : lane.name }, lane.name), cells));
  }
  const step = map.bars > 24 ? 4 : 1;
  const bar = barSeconds(ctx.score);
  box.append(
    h(
      'div',
      { class: 'barnums', style: cols, role: 'group', 'aria-label': 'Play from a bar' },
      ...Array.from({ length: map.bars }, (_, i) =>
        h(
          'button',
          { type: 'button', 'data-testid': `seek-${i + 1}`, 'aria-label': `Play from bar ${i + 1}`, title: `Play from bar ${i + 1}`, onclick: () => ctx.seek(i * bar) },
          (i + 1) % step === 0 || i === 0 ? String(i + 1) : '',
        ),
      ),
    ),
  );
  box.append(loopStrip(ctx, map.bars, cols));
  box.append(h('p', { class: 'maplegend', 'data-testid': 'map-legend' }, map.isEdit ? h('i', {}) : null, map.legend));
  box.append(transportRow(ctx));
  box.append(h('i', { class: 'map-ph', 'aria-hidden': 'true' }));
  return box;
}

/** The loop strip: drag along it to repeat those bars; the range shows as a filled band. */
function loopStrip(ctx: ViewContext, bars: number, cols: string): HTMLElement {
  const loop = ctx.transport.loop;
  const strip = h('div', { class: 'cells loopcells', style: cols, role: 'group', 'aria-label': 'Loop range: drag along to repeat bars' });
  const paint = (from: number, to: number) => {
    const [a, b] = [Math.min(from, to), Math.max(from, to)];
    strip.querySelectorAll<HTMLElement>('.loopcell').forEach((el) => el.classList.toggle('in', Number(el.dataset.bar) >= a && Number(el.dataset.bar) <= b));
  };
  for (let b = 1; b <= bars; b++) {
    const cell = h('button', {
      type: 'button',
      class: `loopcell${loop && b >= loop.start && b <= loop.end ? ' in' : ''}`,
      'data-bar': b,
      'data-testid': `loop-${b}`,
      'aria-label': `Loop from bar ${b}`,
    });
    cell.addEventListener('pointerdown', (e: PointerEvent) => {
      e.preventDefault();
      // Touch captures the pointer on the first cell; release it so the cells it crosses see it.
      if (cell.hasPointerCapture?.(e.pointerId)) cell.releasePointerCapture(e.pointerId);
      loopDrag = { from: b, to: b, set: (from, to) => ctx.setLoop({ start: Math.min(from, to), end: Math.max(from, to) }) };
      paint(b, b);
    });
    cell.addEventListener('pointerenter', () => {
      if (!loopDrag || loopDrag.to === b) return;
      loopDrag.to = b;
      paint(loopDrag.from, b);
    });
    cell.addEventListener('keydown', (e: KeyboardEvent) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        e.stopPropagation();
        ctx.setLoop({ start: b, end: b });
      }
    });
    strip.append(cell);
  }
  return h('div', { class: 'lane loop-lane' }, h('span', {}, 'Loop'), strip);
}

/** Play / stop, the time, and the loop in words with a way to clear it. */
function transportRow(ctx: ViewContext): HTMLElement {
  const t = ctx.transport;
  const total = scoreSeconds(ctx.score);
  const play = h('button', {
    type: 'button',
    class: 'tp-play',
    'data-testid': 'transport-play',
    'aria-label': t.playing ? 'Stop (Space)' : 'Play (Space)',
    title: t.playing ? 'Stop (Space)' : 'Play (Space)',
    onclick: () => ctx.togglePlay(),
  });
  play.innerHTML = t.playing ? ICON_STOP : ICON_PLAY;
  const loop = t.loop;
  return h(
    'div',
    { class: 'transport', 'data-testid': 'transport' },
    play,
    h('span', { class: 'num tp-time', 'data-testid': 'transport-time' }, `${clock(t.seconds)} / ${clock(total)}`),
    h(
      'span',
      { class: 'tp-loop', 'data-testid': 'loop-status' },
      loop ? `Looping ${loop.start === loop.end ? `bar ${loop.start}` : `bars ${loop.start}–${loop.end}`}` : ctx.score.playback.mode === 'loop' ? 'Loops the whole sound' : 'Plays once',
    ),
    loop ? h('button', { type: 'button', class: 'linkish', 'data-testid': 'loop-clear', onclick: () => ctx.setLoop(null) }, 'Clear loop') : null,
  );
}
