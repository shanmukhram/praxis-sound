// Overview: layers with Mute / Solo / Lock in words, and four plain-language Feel macros. Mute,
// solo and lock are patches on the live score (undoable, no take); a macro change is a request,
// so it makes a take.

import type { ViewContext } from '../context';
import { lockLayer, muteLayer, soloLayer } from '../../score/patchBuilder';
import { h } from '../dom';

const KIND: Record<string, string> = { sampler: 'Sampler', synth: 'Synth', drum: 'Drum', sfx: 'Sound effect' };

export const MACROS: { name: string; lo: string; hi: string; words: string[]; down: string; up: string }[] = [
  { name: 'Mood', lo: 'Calm', hi: 'Tense', words: ['calm', 'settled', 'uneasy', 'tense'], down: 'Make the mood calmer', up: 'Make the mood more tense' },
  { name: 'Brightness', lo: 'Dark', hi: 'Bright', words: ['dark', 'soft', 'clear', 'bright'], down: 'Make it a little darker', up: 'Make it a little brighter' },
  { name: 'Space', lo: 'Dry', hi: 'Big room', words: ['dry', 'small room', 'hall', 'big room'], down: 'Make it drier', up: 'Give it more room' },
  { name: 'Energy', lo: 'Still', hi: 'Driving', words: ['still', 'gentle', 'moving', 'driving'], down: 'Calmer, less energy', up: 'More energy' },
];

export const MACRO_DEFAULTS: Record<string, number> = { Mood: 22, Brightness: 38, Space: 30, Energy: 26 };

const word = (m: (typeof MACROS)[number], v: number) => m.words[Math.min(3, Math.floor(v / 25))];

export function renderOverview(ctx: ViewContext): HTMLElement {
  const s = ctx.score;
  const box = h('div', {}, h('p', { class: 'hint', style: 'margin:-6px 0 16px' }, 'Drag across bars above to change only those. Everything else stays exactly as it is.'));
  box.append(h('div', { class: 'section-h' }, h('h3', {}, 'Layers'), h('span', {}, `${s.layers.length} ${s.layers.length === 1 ? 'layer' : 'layers'}`)));
  const list = h('div', { class: 'layers' });
  for (const l of s.layers) {
    const name = l.name || l.id;
    const tg = (label: string, pressed: boolean, testid: string, onclick: () => void) =>
      h('button', { 'aria-pressed': String(pressed), 'data-testid': testid, 'aria-label': `${label} ${name}`, onclick }, label === 'Lock' && pressed ? 'Locked' : label);
    list.append(
      h(
        'div',
        { class: 'layer', 'data-testid': `layer-${l.id}` },
        h('b', {}, name),
        h(
          'div',
          { class: 'toggles' },
          tg('Mute', l.muted, `mute-${l.id}`, () => ctx.apply(muteLayer(s, l.id, !l.muted))),
          tg('Solo', l.solo, `solo-${l.id}`, () => ctx.apply(soloLayer(s, l.id, !l.solo))),
          tg('Lock', l.locked, `lock-${l.id}`, () => ctx.apply(lockLayer(s, l.id, !l.locked))),
        ),
        h('small', {}, `${KIND[l.instrument.kind] ?? l.instrument.kind} · ${l.instrument.preset.replace(/_/g, ' ')}${l.locked ? ' · no edit can change it' : ''}`),
      ),
    );
  }
  box.append(list);
  box.append(h('div', { class: 'section-h' }, h('h3', {}, 'Feel'), h('span', {}, 'Changes make a new take')));
  const knobs = h('div', { class: 'knobs' });
  for (const m of MACROS) {
    const v = ctx.knobs[m.name] ?? MACRO_DEFAULTS[m.name];
    const b = h('b', {}, word(m, v));
    const input = h('input', { id: `k-${m.name}`, type: 'range', min: 0, max: 100, value: v, 'data-testid': `knob-${m.name.toLowerCase()}` });
    input.addEventListener('input', () => (b.textContent = word(m, Number(input.value))));
    input.addEventListener('change', () => {
      const nv = Number(input.value);
      const before = ctx.knobs[m.name] ?? MACRO_DEFAULTS[m.name];
      ctx.knobs[m.name] = nv;
      if (nv !== before) ctx.ask(nv < before ? m.down : m.up);
    });
    knobs.append(
      h('div', { class: 'knob' }, h('label', { for: `k-${m.name}` }, m.name, b), input, h('div', { class: 'ends' }, h('span', {}, m.lo), h('span', {}, m.hi))),
    );
  }
  box.append(knobs);
  return box;
}
