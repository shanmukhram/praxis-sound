// "Where each layer plays": the pinned change map at the top of the Sound panel. Cells light from
// `take.changed`; dragging across bars of one layer sets the scope of the next request.

import type { ViewContext } from '../context';
import { changeMapModel } from '../changeMapModel';
import { h } from '../dom';

let drag: { layer: string; from: number } | null = null;
let dragTo = 0;
window.addEventListener('pointerup', () => {
  drag = null;
});

export function renderChangeMap(ctx: ViewContext): HTMLElement {
  const map = changeMapModel(ctx.take, ctx.score, ctx.selection);
  const cols = `grid-template-columns: repeat(${map.bars}, minmax(0, 1fr))`;
  const box = h(
    'div',
    { class: 'arr', 'data-testid': 'change-map' },
    h('div', { class: 'section-h' }, h('h3', {}, 'Where each layer plays'), h('span', {}, ctx.take ? `Take ${ctx.take.n}` : '')),
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
  box.append(
    h(
      'div',
      { class: 'barnums', style: cols, 'aria-hidden': 'true' },
      ...Array.from({ length: map.bars }, (_, i) => h('span', {}, (i + 1) % step === 0 || i === 0 ? String(i + 1) : '')),
    ),
    h('p', { class: 'maplegend', 'data-testid': 'map-legend' }, map.isEdit ? h('i', {}) : null, map.legend),
  );
  return box;
}
