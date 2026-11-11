// Overview: the sound's settings (key, tempo, bars, time, loop or one-shot), its layers with Mute /
// Solo / Lock in words and, by hand, add / rename / instrument / bars / level / pan / remove
// (WP-U5), and four plain-language Feel macros. Mute, solo and lock are patches on the live score
// (undoable, no take); settings and layer edits are live hand edits (heard at once, batched for
// the next take); a macro change is a request, so it makes a take.

import type { ViewContext } from '../context';
import type { PresetInfo } from '../../engine/EngineAdapter';
import { groupPresets, presetLabel, QUICK_PRESETS } from '../../engine/presets';
import { lockLayer, muteLayer, soloLayer } from '../../score/patchBuilder';
import type { Key, Mode, NLayer, PitchClass, ScorePatch, TimeSignature } from '../../score/types';
import { PITCH_CLASSES } from '../../score/theory';
import { scoreSeconds } from '../../score/timeline';
import { h } from '../dom';
import {
  addLayerPatch,
  barsPatch,
  dbText,
  gainPatch,
  instrumentPatch,
  keyPatch,
  layerBarsPatch,
  layerSpan,
  nameOf,
  panPatch,
  panText,
  playbackPatch,
  removeLayerPatch,
  renameLayerPatch,
  tempoPatch,
  timeSignaturePatch,
  timeSignatureProblem,
} from '../handEdits';

const KIND: Record<string, string> = { sampler: 'Sampler', synth: 'Synth', drum: 'Drum', sfx: 'Sound effect' };

export const MACROS: { name: string; lo: string; hi: string; words: string[]; down: string; up: string }[] = [
  { name: 'Mood', lo: 'Calm', hi: 'Tense', words: ['calm', 'settled', 'uneasy', 'tense'], down: 'Make the mood calmer', up: 'Make the mood more tense' },
  { name: 'Brightness', lo: 'Dark', hi: 'Bright', words: ['dark', 'soft', 'clear', 'bright'], down: 'Make it a little darker', up: 'Make it a little brighter' },
  { name: 'Space', lo: 'Dry', hi: 'Big room', words: ['dry', 'small room', 'hall', 'big room'], down: 'Make it drier', up: 'Give it more room' },
  { name: 'Energy', lo: 'Still', hi: 'Driving', words: ['still', 'gentle', 'moving', 'driving'], down: 'Calmer, less energy', up: 'More energy' },
];

export const MACRO_DEFAULTS: Record<string, number> = { Mood: 22, Brightness: 38, Space: 30, Energy: 26 };

/** Modes in the order a person reaches for them, in words. */
export const MODES: { mode: Mode; label: string }[] = [
  { mode: 'minor', label: 'minor' },
  { mode: 'major', label: 'major' },
  { mode: 'harmonic_minor', label: 'harmonic minor' },
  { mode: 'melodic_minor', label: 'melodic minor' },
  { mode: 'dorian', label: 'dorian' },
  { mode: 'phrygian', label: 'phrygian' },
  { mode: 'lydian', label: 'lydian' },
  { mode: 'mixolydian', label: 'mixolydian' },
  { mode: 'locrian', label: 'locrian' },
  { mode: 'minor_pentatonic', label: 'minor pentatonic' },
  { mode: 'major_pentatonic', label: 'major pentatonic' },
  { mode: 'chromatic', label: 'chromatic (no key)' },
];

export const METERS: TimeSignature[] = [
  { numerator: 4, denominator: 4 },
  { numerator: 3, denominator: 4 },
  { numerator: 2, denominator: 4 },
  { numerator: 5, denominator: 4 },
  { numerator: 6, denominator: 8 },
  { numerator: 7, denominator: 8 },
  { numerator: 12, denominator: 8 },
];

const word = (m: (typeof MACROS)[number], v: number) => m.words[Math.min(3, Math.floor(v / 25))];

/** The add-layer picker stays open across re-renders until a preset is picked or it is closed. */
let pickerOpen = false;

export function clock(s: number): string {
  const m = Math.floor(s / 60);
  const sec = Math.round(s - m * 60);
  return sec === 60 ? `${m + 1}:00` : `${m}:${String(sec).padStart(2, '0')}`;
}

export function keySelects(key: Key, onChange: (k: Key) => void, ids: { tonic: string; mode: string }): HTMLElement[] {
  const tonic = h('select', { 'data-testid': ids.tonic, 'aria-label': 'Key note' }, ...PITCH_CLASSES.map((p) => h('option', { value: p, selected: p === key.tonic }, p)));
  const mode = h('select', { 'data-testid': ids.mode, 'aria-label': 'Scale' }, ...MODES.map((m) => h('option', { value: m.mode, selected: m.mode === key.mode }, m.label)));
  const fire = () => onChange({ tonic: tonic.value as PitchClass, mode: mode.value as Mode });
  tonic.addEventListener('change', fire);
  mode.addEventListener('change', fire);
  return [tonic, mode];
}

export function meterSelect(ts: TimeSignature, onChange: (t: TimeSignature) => void, testid: string): HTMLSelectElement {
  const list = METERS.some((m) => m.numerator === ts.numerator && m.denominator === ts.denominator) ? METERS : [ts, ...METERS];
  const sel = h(
    'select',
    { 'data-testid': testid, 'aria-label': 'Time signature' },
    ...list.map((m) => h('option', { value: `${m.numerator}/${m.denominator}`, selected: m.numerator === ts.numerator && m.denominator === ts.denominator }, `${m.numerator}/${m.denominator}`)),
  );
  sel.addEventListener('change', () => {
    const [n, d] = sel.value.split('/').map(Number);
    onChange({ numerator: n, denominator: d });
  });
  return sel;
}

/** Loop / One-shot as a two-button switch. */
export function loopSwitch(loop: boolean, onChange: (loop: boolean) => void, ids: { loop: string; once: string }): HTMLElement {
  return h(
    'div',
    { class: 'seg', role: 'group', 'aria-label': 'Plays as' },
    h('button', { type: 'button', 'aria-pressed': String(loop), 'data-testid': ids.loop, onclick: () => onChange(true) }, 'Loop'),
    h('button', { type: 'button', 'aria-pressed': String(!loop), 'data-testid': ids.once, onclick: () => onChange(false) }, 'One-shot'),
  );
}

export function numberInput(value: number, attrs: Record<string, string | number>, onCommit: (v: number) => void): HTMLInputElement {
  const input = h('input', { type: 'number', inputmode: 'decimal', value, class: 'num-in', ...attrs });
  input.addEventListener('change', () => {
    const v = Number(input.value);
    if (input.value.trim() !== '' && Number.isFinite(v)) onCommit(v);
    else input.value = String(value);
  });
  return input;
}

function renderSettings(ctx: ViewContext): HTMLElement {
  const s = ctx.score;
  const live = (p: ScorePatch | null) => {
    if (p) ctx.live(p, p.description ?? 'Changed the sound');
  };
  const secs = scoreSeconds(s);
  const ts = meterSelect(
    s.time_signature,
    (t) => {
      const problem = timeSignatureProblem(s, t);
      if (problem) {
        ctx.note(problem);
        ctx.setExpanded(ctx.expanded);
        return;
      }
      live(timeSignaturePatch(s, t));
    },
    'set-ts',
  );
  const row = (label: string, ...controls: (Node | string)[]) => h('div', { class: 'set-row' }, h('span', { class: 'set-label' }, label), h('div', { class: 'set-ctl' }, ...controls));
  return h(
    'div',
    { class: 'settings', 'data-testid': 'sound-settings' },
    row('Key', ...keySelects(s.key, (k) => live(keyPatch(s, k)), { tonic: 'set-tonic', mode: 'set-mode' })),
    row(
      'Tempo',
      numberInput(s.tempo.bpm, { min: 20, max: 400, step: 1, 'data-testid': 'set-bpm', 'aria-label': 'Tempo in BPM' }, (v) => live(tempoPatch(s, v))),
      h('span', { class: 'unit' }, 'BPM'),
      h('span', { class: 'set-note', 'data-testid': 'set-length' }, `${clock(secs)} in all`),
    ),
    row(
      'Bars',
      numberInput(s.bars, { min: 1, max: 2048, step: 1, 'data-testid': 'set-bars', 'aria-label': 'Number of bars' }, (v) => live(barsPatch(s, Math.round(v)))),
      h('span', { class: 'unit' }, 'of'),
      ts,
    ),
    row('Plays as', loopSwitch(s.playback.mode === 'loop', (loop) => live(playbackPatch(s, loop)), { loop: 'set-loop', once: 'set-oneshot' })),
  );
}

function instrumentSelect(ctx: ViewContext, l: NLayer): HTMLSelectElement {
  const groups = groupPresets(ctx.presets);
  const known = ctx.presets.some((p) => p.name === l.instrument.preset);
  const sel = h(
    'select',
    { 'data-testid': `layer-instrument-${l.id}`, 'aria-label': `Instrument of ${nameOf(l)}` },
    known ? null : h('option', { value: l.instrument.preset, selected: true }, presetLabel(l.instrument.preset)),
    ...groups.map((g) =>
      h(
        'optgroup',
        { label: g.name },
        ...g.presets.map((p) => h('option', { value: p.name, selected: p.name === l.instrument.preset, disabled: !p.available }, `${presetLabel(p.name)}${p.available ? '' : ' (not installed)'}`)),
      ),
    ),
  );
  sel.addEventListener('change', () => {
    const p = ctx.presets.find((x) => x.name === sel.value);
    const patch = p ? instrumentPatch(ctx.score, l.id, p) : null;
    if (patch) ctx.live(patch, patch.description ?? 'Changed the instrument', undefined);
  });
  return sel;
}

function layerDetails(ctx: ViewContext, l: NLayer): HTMLElement {
  const s = ctx.score;
  const name = h('input', { type: 'text', value: l.name, class: 'text-in', 'data-testid': `layer-name-${l.id}`, 'aria-label': `Name of ${nameOf(l)}`, maxlength: 60 });
  const rename = () => {
    const p = renameLayerPatch(s, l.id, name.value);
    if (p) ctx.live(p, p.description ?? 'Renamed a layer');
  };
  name.addEventListener('change', rename);
  name.addEventListener('keydown', (e: KeyboardEvent) => {
    if (e.key === 'Enter') name.blur();
  });
  const span = layerSpan(l) ?? { start: 1, end: s.bars };
  const setBars = (a: number, b: number) => {
    const p = layerBarsPatch(s, l.id, a, b);
    if (p) ctx.live(p, p.description ?? 'Changed where a layer plays', { layer: l.id, bars: { start: 1, end: s.bars } });
  };
  const from = numberInput(span.start, { min: 1, max: s.bars, step: 1, 'data-testid': `layer-from-${l.id}`, 'aria-label': `First bar ${nameOf(l)} plays` }, (v) => setBars(Math.round(v), span.end));
  const to = numberInput(span.end, { min: 1, max: s.bars, step: 1, 'data-testid': `layer-to-${l.id}`, 'aria-label': `Last bar ${nameOf(l)} plays` }, (v) => setBars(span.start, Math.round(v)));
  const half = Math.floor(s.bars / 2);
  const quick: { label: string; id: string; a: number; b: number }[] = [{ label: 'All bars', id: 'all', a: 1, b: s.bars }];
  if (s.bars >= 2) {
    quick.push({ label: `First half (1–${half})`, id: 'first', a: 1, b: half });
    quick.push({ label: `Second half (${half + 1}–${s.bars})`, id: 'second', a: half + 1, b: s.bars });
  }
  for (const sec of s.sections) quick.push({ label: `${sec.name || sec.id} (${sec.bars.start}–${sec.bars.end})`, id: `sec-${sec.id}`, a: sec.bars.start, b: sec.bars.end });
  const gainOut = h('output', { class: 'num' }, dbText(l.gain_db));
  const gain = h('input', { type: 'range', min: -24, max: 6, step: 0.5, value: l.gain_db, 'data-testid': `layer-gain-${l.id}`, 'aria-label': `Level of ${nameOf(l)}` });
  gain.addEventListener('input', () => (gainOut.textContent = dbText(Number(gain.value))));
  gain.addEventListener('change', () => {
    const p = gainPatch(s, l.id, Number(gain.value));
    if (p) ctx.live(p, p.description ?? 'Changed a level');
  });
  const panOut = h('output', {}, panText(l.pan));
  const pan = h('input', { type: 'range', min: -1, max: 1, step: 0.05, value: l.pan, 'data-testid': `layer-pan-${l.id}`, 'aria-label': `Pan of ${nameOf(l)}` });
  pan.addEventListener('input', () => (panOut.textContent = panText(Number(pan.value))));
  pan.addEventListener('change', () => {
    const p = panPatch(s, l.id, Number(pan.value));
    if (p) ctx.live(p, p.description ?? 'Changed a pan');
  });
  const field = (label: string, ...controls: (Node | string | null)[]) => h('div', { class: 'set-row' }, h('span', { class: 'set-label' }, label), h('div', { class: 'set-ctl' }, ...controls));
  return h(
    'div',
    { class: 'layer-edit', 'data-testid': `layer-edit-${l.id}` },
    field('Name', name),
    field('Instrument', instrumentSelect(ctx, l)),
    field('Plays in', h('span', { class: 'unit' }, 'bars'), from, h('span', { class: 'unit' }, 'to'), to),
    h(
      'div',
      { class: 'pills quick', role: 'group', 'aria-label': `Where ${nameOf(l)} plays` },
      ...quick.map((q) =>
        h('button', { type: 'button', 'aria-pressed': String(span.start === q.a && span.end === q.b), 'data-testid': `layer-range-${l.id}-${q.id}`, onclick: () => setBars(q.a, q.b) }, q.label),
      ),
    ),
    field('Level', gain, gainOut),
    field('Pan', pan, panOut),
    h(
      'div',
      { class: 'edit-foot' },
      h('button', { type: 'button', class: 'ghost danger', 'data-testid': `remove-${l.id}`, onclick: () => ctx.live(removeLayerPatch(s, l.id), `Removed ${nameOf(l)}`) }, 'Remove layer'),
      h('span', { class: 'hint' }, l.locked ? 'Unlock it to change it.' : 'Undo brings back anything removed.'),
    ),
  );
}

function presetButton(ctx: ViewContext, p: PresetInfo, big = false): HTMLButtonElement {
  return h(
    'button',
    {
      type: 'button',
      class: big ? 'preset big' : 'preset',
      disabled: !p.available,
      title: p.available ? p.description : (p.reason ?? 'Not installed on this computer'),
      'data-testid': big ? `quick-${p.name}` : `pick-${p.name}`,
      onclick: () => {
        pickerOpen = false;
        const patch = addLayerPatch(ctx.score, p);
        const id = (patch.scope?.layers ?? [])[0] ?? null;
        ctx.live(patch, patch.description ?? `Added ${presetLabel(p.name)}`, undefined);
        if (id) ctx.setViewLayer(id);
      },
    },
    presetLabel(p.name),
    big ? h('small', {}, p.available ? p.description : 'not installed') : null,
  );
}

function picker(ctx: ViewContext): HTMLElement {
  if (!ctx.presets.length) return h('div', { class: 'picker', 'data-testid': 'picker' }, h('p', { class: 'hint', style: 'margin:0' }, 'Reading the instruments…'));
  const quick = QUICK_PRESETS.map((n) => ctx.presets.find((p) => p.name === n)).filter((p): p is PresetInfo => !!p);
  return h(
    'div',
    { class: 'picker', 'data-testid': 'picker' },
    h('div', { class: 'section-h' }, h('h3', {}, 'Pick an instrument'), h('button', { type: 'button', class: 'linkish', onclick: () => ((pickerOpen = false), ctx.setExpanded(ctx.expanded)) }, 'Close')),
    quick.length ? h('div', { class: 'quick-presets' }, ...quick.map((p) => presetButton(ctx, p, true))) : null,
    ...groupPresets(ctx.presets).map((g) => h('div', { class: 'preset-group' }, h('b', {}, g.name), h('div', { class: 'preset-list' }, ...g.presets.map((p) => presetButton(ctx, p))))),
  );
}

export function renderOverview(ctx: ViewContext): HTMLElement {
  const s = ctx.score;
  const box = h(
    'div',
    {},
    h('p', { class: 'hint', style: 'margin:-6px 0 16px' }, s.layers.length ? 'Drag across bars above to change only those. Everything else stays exactly as it is.' : 'Add a layer to start: pick an instrument, then write its notes in Rhythm or Notes.'),
  );
  box.append(h('div', { class: 'section-h' }, h('h3', {}, 'Sound'), h('span', {}, 'Changes are heard at once')), renderSettings(ctx));
  box.append(h('div', { class: 'section-h' }, h('h3', {}, 'Layers'), h('span', {}, `${s.layers.length} ${s.layers.length === 1 ? 'layer' : 'layers'}`)));
  const list = h('div', { class: 'layers' });
  for (const l of s.layers) {
    const name = nameOf(l);
    const open = ctx.expanded === l.id;
    const tg = (label: string, pressed: boolean, testid: string, onclick: () => void) =>
      h('button', { 'aria-pressed': String(pressed), 'data-testid': testid, 'aria-label': `${label} ${name}`, onclick }, label === 'Lock' && pressed ? 'Locked' : label);
    const span = layerSpan(l);
    const where = span ? (span.start === 1 && span.end === s.bars ? 'every bar' : span.start === span.end ? `bar ${span.start}` : `bars ${span.start}–${span.end}`) : 'nowhere yet';
    list.append(
      h(
        'div',
        { class: `layer${open ? ' open' : ''}`, 'data-testid': `layer-${l.id}` },
        h('b', {}, name),
        h(
          'div',
          { class: 'toggles' },
          tg('Mute', l.muted, `mute-${l.id}`, () => ctx.apply(muteLayer(s, l.id, !l.muted))),
          tg('Solo', l.solo, `solo-${l.id}`, () => ctx.apply(soloLayer(s, l.id, !l.solo))),
          tg('Lock', l.locked, `lock-${l.id}`, () => ctx.apply(lockLayer(s, l.id, !l.locked))),
          h('button', { class: 'edit-toggle', 'aria-expanded': String(open), 'data-testid': `edit-${l.id}`, 'aria-label': `${open ? 'Close' : 'Edit'} ${name}`, onclick: () => ctx.setExpanded(open ? null : l.id) }, open ? 'Done' : 'Edit'),
        ),
        h('small', {}, `${KIND[l.instrument.kind] ?? l.instrument.kind} · ${presetLabel(l.instrument.preset).toLowerCase()} · plays ${where}${l.gain_db ? ` · ${dbText(l.gain_db)}` : ''}${l.locked ? ' · no edit can change it' : ''}`),
        open ? layerDetails(ctx, l) : null,
      ),
    );
  }
  box.append(list);
  if (pickerOpen) box.append(picker(ctx));
  else
    box.append(
      h(
        'button',
        {
          type: 'button',
          class: 'add-layer',
          'data-testid': 'add-layer',
          onclick: () => {
            pickerOpen = true;
            ctx.setExpanded(null);
          },
        },
        h('span', { 'aria-hidden': 'true' }, '+'),
        ' Add a layer',
      ),
    );
  // The Feel macros are requests in words: they need a take to change.
  if (ctx.take && !ctx.state.draft) {
    box.append(h('div', { class: 'section-h', style: 'margin-top:18px' }, h('h3', {}, 'Feel'), h('span', {}, 'Changes make a new take')));
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
  }
  return box;
}
