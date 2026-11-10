// Rhythm: the step grid of one layer in DON / din / don / TIN (WP-U1's step sequencer, simplified
// for the panel). Clicking a step cycles it off -> don -> DON -> din -> TIN -> off; each change is
// a hand edit scoped to the bars shown, so it makes a take.

import type { ViewContext } from '../context';
import type { Accent, NLayer, Note } from '../../score/types';
import { derivePattern, editPatternInRange } from '../../score/patchBuilder';
import { ACCENT_SPOKEN, GRID_LABEL, isUnpitchedClass } from '../../score/theory';
import { h } from '../dom';

const NEXT: Record<string, Accent | 'off'> = { off: 'normal', normal: 'strong', strong: 'soft', soft: 'bright', bright: 'off' };
const HEIGHT: Record<Accent, number> = { strong: 100, bright: 88, normal: 64, soft: 44 };

/** The layer and bars the Rhythm and Notes views show: the selection, else what the take changed, else the first clip. */
export function focus(ctx: ViewContext, prefer: 'pitched' | 'any'): { layer: NLayer; bars: { start: number; end: number } } | null {
  const s = ctx.score;
  if (ctx.selection) {
    const layer = s.layers.find((l) => l.id === ctx.selection!.layer);
    if (layer) return { layer, bars: ctx.selection.bars };
  }
  if (ctx.take?.patch) {
    for (const [id, [a, b]] of Object.entries(ctx.take.changed)) {
      const layer = s.layers.find((l) => l.id === id);
      if (layer && (prefer === 'any' || !isUnpitchedClass(layer.class))) return { layer, bars: { start: a, end: b } };
    }
  }
  const layer = (prefer === 'pitched' ? s.layers.find((l) => !isUnpitchedClass(l.class)) : undefined) ?? s.layers[0];
  if (!layer) return null;
  const clip = layer.clips[0];
  return { layer, bars: clip ? { ...clip.bars } : { start: 1, end: Math.min(4, s.bars) } };
}

export function renderRhythm(ctx: ViewContext): HTMLElement {
  const f = focus(ctx, 'any');
  const box = h('div', { 'data-testid': 'rhythm-view' });
  if (!f) return box;
  const d = derivePattern(ctx.score, f.layer.id, f.bars);
  if (!d) return box;
  const N = d.pattern.length_steps;
  const repeats = d.mode === 'loop' ? ((f.bars.end - f.bars.start + 1) * d.stepsPerBar) / N : 1;
  const range = f.bars.start === f.bars.end ? `bar ${f.bars.start}` : `bars ${f.bars.start}–${f.bars.end}`;
  box.append(
    h(
      'div',
      { class: 'section-h' },
      h('h3', {}, `${f.layer.name || f.layer.id} rhythm, ${range}`),
      h('span', {}, `${GRID_LABEL[d.pattern.grid]} notes · ${d.mode === 'loop' ? `repeats ${repeats} ${repeats === 1 ? 'time' : 'times'}` : 'note for note'}`),
    ),
  );
  const notes = d.pattern.notes;
  const topAt = (step: number) => notes.filter((n) => n.step === step).sort((a, b) => b.velocity - a.velocity)[0];
  const common = mostCommonPitch(notes, f.layer);
  const steps = h('div', { class: 'steps', style: `grid-template-columns: repeat(${Math.min(N, 16)}, minmax(0, 1fr))` });
  for (let i = 0; i < N; i++) {
    const n = topAt(i);
    const acc = (n?.accent ?? 'normal') as Accent;
    const label = n ? ACCENT_SPOKEN[acc] : '';
    steps.append(
      h(
        'button',
        {
          class: `step ${n ? acc : 'off'}`,
          'data-testid': `step-${i}`,
          'aria-label': `Step ${i + 1}, ${n ? `${acc} (${label})` : 'silent'}`,
          title: 'Click to cycle: off, don, DON, din, TIN',
          onclick: () => {
            const next = NEXT[n ? acc : 'off'];
            let out: Note[];
            if (!n) out = [...notes, { step: i, pitch: common, velocity: 0.62, accent: 'normal', length: isUnpitchedClass(f.layer.class) ? 1 : 1.6 }];
            else if (next === 'off') out = notes.filter((x) => x.step !== i);
            else out = notes.map((x) => (x === n ? { ...x, accent: next } : x));
            const patch = editPatternInRange(ctx.score, f.layer.id, f.bars, out, `Step ${i + 1} ${next === 'off' ? 'silenced' : `set to ${ACCENT_SPOKEN[next as Accent]}`}`);
            if (patch) ctx.edit(patch, `Step ${i + 1} of ${f.layer.name || f.layer.id}, ${range}: ${next === 'off' ? 'silent' : ACCENT_SPOKEN[next as Accent]}`, { layer: f.layer.id, bars: f.bars });
          },
        },
        n ? h('i', { class: 'v', style: `height:${HEIGHT[acc]}%` }) : null,
        h('span', {}, label),
      ),
    );
  }
  box.append(
    steps,
    h(
      'div',
      { class: 'legend' },
      h('span', {}, h('i', { style: 'background:#6a6a6a' }), 'DON strong'),
      h('span', {}, h('i', { style: 'background:#3a3a3a' }), 'don, din normal and soft'),
      h('span', {}, h('i', { style: 'background:#f5a524' }), 'TIN bright'),
    ),
    h('p', { class: 'hint' }, 'Say a rhythm out loud in the chat, like "DON din don din TIN din don din". Click a step to cycle it; each click makes a take.'),
  );
  return box;
}

function mostCommonPitch(notes: Note[], layer: NLayer): number {
  if (!notes.length) return isUnpitchedClass(layer.class) ? 36 : 60;
  const count = new Map<number, number>();
  for (const n of notes) count.set(n.pitch, (count.get(n.pitch) ?? 0) + 1);
  return [...count.entries()].sort((a, b) => b[1] - a[1])[0][0];
}
