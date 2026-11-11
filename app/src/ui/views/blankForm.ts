// "Start a blank sound" (WP-U5): a sound with no request and no layers yet. Key, bars, time
// signature, the tempo or a length the duration solver fits the bars into, and loop or one-shot.
// The form lives in the conversation where a new sound's first request would go.

import type { BlankSound } from '../../engine/EngineAdapter';
import { blankProblem, blankTempo } from '../../engine/blank';
import { h } from '../dom';
import { clock, keySelects, loopSwitch, meterSelect, numberInput } from './overview';

/** What the form holds between re-renders (kept by the shell). */
export interface BlankDraft {
  title: string;
  key: BlankSound['key'];
  bars: number;
  timeSignature: BlankSound['timeSignature'];
  /** Set the tempo, or fit a length. */
  by: 'tempo' | 'length';
  bpm: number;
  seconds: number;
  loop: boolean;
}

export function defaultBlank(): BlankDraft {
  return { title: '', key: { tonic: 'A', mode: 'minor' }, bars: 16, timeSignature: { numerator: 4, denominator: 4 }, by: 'length', bpm: 120, seconds: 30, loop: true };
}

export function blankSettings(d: BlankDraft): BlankSound {
  const b: BlankSound = { title: d.title.trim() || undefined, key: { ...d.key }, timeSignature: { ...d.timeSignature }, bars: d.bars, loop: d.loop };
  if (d.by === 'tempo') b.bpm = d.bpm;
  else b.seconds = d.seconds;
  return b;
}

/** "16 bars of 4/4 at 128 BPM last 0:30, as a loop." */
export function lengthLine(d: BlankDraft): string {
  const b = blankSettings(d);
  const problem = blankProblem(b);
  if (problem) return problem;
  const bpm = blankTempo(b)!;
  const secs = (d.bars * d.timeSignature.numerator * (4 / d.timeSignature.denominator) * 60) / bpm;
  return `${d.bars} ${d.bars === 1 ? 'bar' : 'bars'} of ${d.timeSignature.numerator}/${d.timeSignature.denominator} at ${bpm} BPM last ${clock(secs)}${d.loop ? ', as a seamless loop' : ', played once'}.`;
}

export function renderBlankForm(d: BlankDraft, actions: { start(b: BlankSound): void; cancel(): void; busy: boolean; error: string | null }): HTMLElement {
  const line = h('p', { class: 'blank-line', 'data-testid': 'blank-length-line' }, lengthLine(d));
  const refresh = () => {
    line.textContent = lengthLine(d);
    go.disabled = actions.busy || !!blankProblem(blankSettings(d));
  };
  const title = h('input', { type: 'text', class: 'text-in wide-in', value: d.title, placeholder: 'Untitled sound', maxlength: 60, 'data-testid': 'blank-title', 'aria-label': 'Name' });
  title.addEventListener('input', () => (d.title = title.value));
  const speed = h('div', { class: 'set-ctl' });
  const drawSpeed = () => {
    speed.replaceChildren(
      h(
        'div',
        { class: 'seg', role: 'group', 'aria-label': 'Set the speed by' },
        h('button', { type: 'button', 'aria-pressed': String(d.by === 'length'), 'data-testid': 'blank-by-length', onclick: () => ((d.by = 'length'), drawSpeed(), refresh()) }, 'Fit a length'),
        h('button', { type: 'button', 'aria-pressed': String(d.by === 'tempo'), 'data-testid': 'blank-by-tempo', onclick: () => ((d.by = 'tempo'), drawSpeed(), refresh()) }, 'Set the tempo'),
      ),
      d.by === 'length'
        ? h('span', { class: 'inline' }, numberInput(d.seconds, { min: 1, max: 3600, step: 'any', 'data-testid': 'blank-seconds', 'aria-label': 'Length in seconds' }, (v) => ((d.seconds = v), refresh())), h('span', { class: 'unit' }, 'seconds'))
        : h('span', { class: 'inline' }, numberInput(d.bpm, { min: 20, max: 400, step: 'any', 'data-testid': 'blank-bpm', 'aria-label': 'Tempo in BPM' }, (v) => ((d.bpm = v), refresh())), h('span', { class: 'unit' }, 'BPM')),
    );
  };
  drawSpeed();
  const loop = h('div', { class: 'set-ctl' });
  const drawLoop = () => loop.replaceChildren(loopSwitch(d.loop, (v) => ((d.loop = v), drawLoop(), refresh()), { loop: 'blank-loop', once: 'blank-oneshot' }));
  drawLoop();
  const go = h('button', { type: 'submit', class: 'primary', 'data-testid': 'blank-go' }, actions.busy ? 'Starting…' : 'Start the sound');
  const row = (label: string, ctl: HTMLElement) => h('div', { class: 'set-row' }, h('span', { class: 'set-label' }, label), ctl);
  const form = h(
    'form',
    { class: 'take blank-form', 'data-testid': 'blank-form', 'aria-label': 'Start a blank sound' },
    h('div', { class: 'take-head' }, h('h3', {}, 'Start a blank sound'), h('span', {}, 'Build it by hand')),
    h('p', { class: 'say' }, 'No request needed: set the frame, then add layers and write their notes in the Sound panel. Everything you change is heard at once.'),
    row('Name', h('div', { class: 'set-ctl' }, title)),
    row('Key', h('div', { class: 'set-ctl' }, ...keySelects(d.key, (k) => ((d.key = k), refresh()), { tonic: 'blank-tonic', mode: 'blank-mode' }))),
    row(
      'Bars',
      h(
        'div',
        { class: 'set-ctl' },
        numberInput(d.bars, { min: 1, max: 2048, step: 1, 'data-testid': 'blank-bars', 'aria-label': 'Number of bars' }, (v) => ((d.bars = Math.round(v)), refresh())),
        h('span', { class: 'unit' }, 'of'),
        meterSelect(d.timeSignature, (t) => ((d.timeSignature = t), refresh()), 'blank-ts'),
      ),
    ),
    row('Speed', speed),
    row('Plays as', loop),
    line,
    actions.error ? h('p', { class: 'hint bad', 'data-testid': 'blank-error' }, actions.error) : null,
    h('div', { class: 'take-actions' }, go, h('button', { type: 'button', 'data-testid': 'blank-cancel', onclick: () => actions.cancel() }, 'Cancel')),
  );
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    if (actions.busy) return;
    const b = blankSettings(d);
    if (blankProblem(b)) return refresh();
    actions.start(b);
  });
  // Typing in a number field changes the line before the field commits.
  form.addEventListener('input', (e) => {
    const t = e.target as HTMLInputElement;
    if (t.type !== 'number' || t.value.trim() === '' || !Number.isFinite(Number(t.value))) return;
    const v = Number(t.value);
    if (t.dataset.testid === 'blank-bars') d.bars = Math.round(v);
    if (t.dataset.testid === 'blank-seconds') d.seconds = v;
    if (t.dataset.testid === 'blank-bpm') d.bpm = v;
    refresh();
  });
  refresh();
  return form;
}
