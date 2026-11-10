// Step sequencer (FR-4, FR-29): the pattern a layer plays in the selected bars, one lane per
// pitch, with per-step velocity, accent (normal/strong/soft/bright) and note length including
// overlap above 100%. Every change becomes a write_bars / replace_pattern patch scoped to
// exactly the selected bars and layer.

import type { AppState, Controller } from './state';
import type { Accent, NLayer, NScore, Note } from '../score/types';
import { formatRange } from '../score/types';
import { ACCENTS, ACCENT_SPOKEN, GRID_LABEL, inKey, isUnpitchedClass, midiName, semitone, stepsPerBeat } from '../score/theory';
import { clampVelocity, derivePattern, editPatternInRange, findNote, playedVelocity, toggleNote, updateNote, type DerivedPattern } from '../score/patchBuilder';
import { clear, h } from './dom';

export const GM_DRUMS: Record<number, string> = {
  35: 'Kick 2',
  36: 'Kick',
  37: 'Side stick',
  38: 'Snare',
  39: 'Clap',
  40: 'Snare 2',
  41: 'Low tom',
  42: 'Closed hat',
  44: 'Pedal hat',
  45: 'Mid tom',
  46: 'Open hat',
  49: 'Crash',
  51: 'Ride',
};

export function pitchLabel(layer: NLayer, pitch: number): string {
  if (isUnpitchedClass(layer.class)) return `${pitch} ${GM_DRUMS[pitch] ?? ''}`.trim();
  return midiName(pitch);
}

const NEXT_ACCENT: Record<Accent, Accent> = { normal: 'strong', strong: 'soft', soft: 'bright', bright: 'normal' };

export class StepSequencer {
  readonly el: HTMLElement;
  private extraLanes = new Map<string, Set<number>>();
  private velDrag: { pitch: number; step: number; top: number; height: number; value: number; bar: HTMLElement } | null = null;

  constructor(private readonly app: Controller) {
    this.el = h('div', { class: 'steps', 'data-testid': 'step-sequencer' });
    window.addEventListener('mousemove', (e) => this.onVelMove(e));
    window.addEventListener('mouseup', () => void this.onVelUp());
  }

  /** One-line description for the editor tab bar. */
  static describe(s: AppState): string {
    const score = s.score;
    const layer = score?.layers.find((l) => l.id === s.selection.layerId);
    if (!score || !layer) return '';
    const d = derivePattern(score, layer.id, s.selection.bars);
    if (!d) return '';
    const src = d.mode === 'loop' ? `${d.source} (repeats in range)` : 'note for note';
    return `${src} · ${GRID_LABEL[d.pattern.grid]} · ${d.pattern.length_steps} steps · ${formatRange(s.selection.bars)} of ${layer.name || layer.id}`;
  }

  update(s: AppState): void {
    clear(this.el);
    const score = s.score;
    const layer = score?.layers.find((l) => l.id === s.selection.layerId);
    if (!score || !layer) {
      this.el.append(h('div', { class: 'empty' }, 'Select a layer and drag across bars on the timeline to edit its pattern.'));
      return;
    }
    const d = derivePattern(score, layer.id, s.selection.bars);
    if (!d) return;
    const notes = d.pattern.notes;
    const lanes = this.lanes(score, layer, d, s.focusPitch);
    const focus = s.focusPitch !== null && lanes.includes(s.focusPitch) ? s.focusPitch : lanes[0];
    const N = d.pattern.length_steps;
    const beat = stepsPerBeat(score.time_signature, d.pattern.grid);
    const isBeat = (i: number) => Number.isInteger(beat) && i % beat === 0;
    const isBar = (i: number) => i % d.stepsPerBar === 0;

    const grid = h('div', {
      class: 'step-grid',
      role: 'grid',
      'aria-label': `Steps of ${layer.name || layer.id}`,
      style: `grid-template-columns: 78px repeat(${N}, minmax(18px, 46px))`,
    });
    grid.append(h('div', {}));
    for (let i = 0; i < N; i++) {
      const sel = s.selectedNote?.step === i;
      grid.append(
        h(
          'div',
          {
            class: `step-head${isBeat(i) ? ' beat' : ''}${sel ? ' sel' : ''}`,
            onclick: () => this.app.set({ selectedNote: findNote(notes, i, focus) ? { step: i, pitch: focus } : null }),
          },
          isBar(i) ? `${s.selection.bars.start + i / d.stepsPerBar}` : isBeat(i) ? '·' : '',
        ),
      );
    }

    for (const pitch of lanes) {
      const pitched = !isUnpitchedClass(layer.class) && score.key.mode !== 'chromatic';
      const off = pitched && !inKey(score.key, pitch);
      grid.append(
        h(
          'button',
          {
            class: `lane-label${pitch === focus ? ' active' : ''}`,
            style: 'background:none;border:none;text-align:right;padding:0 6px 0 0',
            title: off ? 'Off-scale pitch: the validator will reject notes here' : 'Show velocity and accent rows for this lane',
            onclick: () => this.app.set({ focusPitch: pitch }),
          },
          `${pitchLabel(layer, pitch)}${off ? ' !' : ''}`,
        ),
      );
      for (let i = 0; i < N; i++) {
        const n = findNote(notes, i, pitch);
        const selected = !!n && s.selectedNote?.step === i && s.selectedNote.pitch === pitch;
        const cell = h(
          'button',
          {
            class: `cell${isBeat(i) ? ' beat' : ''}${n ? ' on' : ''}${selected ? ' selected' : ''}`,
            role: 'gridcell',
            'aria-pressed': String(!!n),
            'aria-label': `${pitchLabel(layer, pitch)} step ${i + 1}${n ? `, velocity ${n.velocity}, ${n.accent}, length ${Math.round(n.length * 100)}%` : ''}`,
            'data-testid': `step-${pitch}-${i}`,
            style: n ? `opacity:${0.45 + 0.55 * playedVelocity(n)}` : '',
            onclick: () => void this.toggle(layer, i, pitch),
          },
          n && n.accent !== 'normal' ? h('span', { class: 'acc' }, ACCENT_SPOKEN[n.accent ?? 'normal']) : null,
          n
            ? h('span', {
                class: `len${n.length > 1 ? ' over' : ''}`,
                style: `width: calc(${Math.min(n.length, N - i) * 100}% + ${(Math.min(n.length, N - i) - 1) * 3}px - 4px)`,
              })
            : null,
        );
        grid.append(cell);
      }
    }

    // Velocity and accent rows for the focused lane.
    grid.append(h('div', { class: 'subrow-label' }, 'Velocity'));
    for (let i = 0; i < N; i++) {
      const n = findNote(notes, i, focus);
      const bar = h('span', { style: `height:${n ? n.velocity * 100 : 0}%` });
      grid.append(
        h(
          'div',
          {
            class: 'vel',
            'data-testid': `vel-${i}`,
            title: n ? `Velocity ${n.velocity}` : '',
            onmousedown: (e: MouseEvent) => {
              if (!n) return;
              const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
              this.velDrag = { pitch: focus, step: i, top: r.top, height: r.height, value: n.velocity, bar };
              this.onVelMove(e);
            },
          },
          bar,
        ),
      );
    }
    grid.append(h('div', { class: 'subrow-label' }, 'Accent'));
    for (let i = 0; i < N; i++) {
      const n = findNote(notes, i, focus);
      const acc = n?.accent ?? 'normal';
      grid.append(
        h(
          'button',
          {
            class: `accchip ${acc}`,
            disabled: !n,
            'data-testid': `acc-${i}`,
            title: n ? `Accent ${acc}: click to cycle normal → strong → soft → bright` : '',
            onclick: () => n && void this.commitNotes('Set accent', (ns) => updateNote(ns, i, focus, { accent: NEXT_ACCENT[(findNote(ns, i, focus)?.accent ?? 'normal') as Accent] }), { step: i, pitch: focus }),
          },
          n ? ACCENT_SPOKEN[acc] : '',
        ),
      );
    }

    this.el.append(h('div', { style: 'overflow-x:auto' }, grid, this.addLane(score, layer, d)), this.inspector(layer, s, notes));
  }

  private lanes(score: NScore, layer: NLayer, d: DerivedPattern, focus: number | null): number[] {
    const set = new Set(d.pattern.notes.map((n) => n.pitch));
    for (const p of this.extraLanes.get(layer.id) ?? []) set.add(p);
    if (focus !== null && this.extraLanes.get(layer.id)?.has(focus)) set.add(focus);
    if (set.size === 0) set.add(isUnpitchedClass(layer.class) ? 36 : 48 + semitone(score.key.tonic));
    return [...set].sort((a, b) => b - a);
  }

  private addLane(score: NScore, layer: NLayer, d: DerivedPattern): HTMLElement {
    const present = new Set(d.pattern.notes.map((n) => n.pitch));
    const options: number[] = [];
    if (isUnpitchedClass(layer.class)) options.push(...Object.keys(GM_DRUMS).map(Number));
    else {
      const base = present.size ? Math.min(...present) - 7 : 45;
      for (let p = Math.max(0, base); p < Math.min(128, base + 31); p++) if (score.key.mode === 'chromatic' || inKey(score.key, p)) options.push(p);
    }
    const sel = h('select', { 'aria-label': 'Pitch for a new lane' }, ...options.filter((p) => !present.has(p)).map((p) => h('option', { value: p }, pitchLabel(layer, p))));
    return h(
      'div',
      { style: 'margin-top:8px;display:flex;gap:6px;align-items:center' },
      h('span', { class: 'hint' }, 'Add lane'),
      sel,
      h(
        'button',
        {
          onclick: () => {
            const p = Number(sel.value);
            if (!Number.isFinite(p)) return;
            if (!this.extraLanes.has(layer.id)) this.extraLanes.set(layer.id, new Set());
            this.extraLanes.get(layer.id)!.add(p);
            this.app.set({ focusPitch: p });
          },
        },
        'Add',
      ),
      h('span', { class: 'hint' }, 'Click a step to add or remove a note. Lengths above 100% overlap the next step (yellow).'),
    );
  }

  private inspector(layer: NLayer, s: AppState, notes: Note[]): HTMLElement {
    const box = h('div', { class: 'inspector', 'data-testid': 'note-inspector' }, h('h3', {}, 'Note'));
    const sel = s.selectedNote;
    const n = sel ? findNote(notes, sel.step, sel.pitch) : undefined;
    if (!sel || !n) {
      box.append(h('div', { class: 'hint' }, 'Add or select a step to edit its velocity, accent and length.'));
      return box;
    }
    const commit = (change: Partial<Note>, what: string) =>
      void this.commitNotes(what, (ns) => updateNote(ns, sel.step, sel.pitch, change), {
        step: change.step ?? sel.step,
        pitch: change.pitch ?? sel.pitch,
      });
    const vel = h('input', { type: 'number', min: 0.01, max: 1, step: 0.05, value: n.velocity, 'data-testid': 'inspector-velocity', 'aria-label': 'Velocity' });
    vel.addEventListener('change', () => commit({ velocity: clampVelocity(Number(vel.value)) }, 'Set velocity'));
    const acc = h('select', { 'data-testid': 'inspector-accent', 'aria-label': 'Accent' }, ...ACCENTS.map((a) => h('option', { value: a, selected: a === n.accent }, `${a} (${ACCENT_SPOKEN[a]})`)));
    acc.addEventListener('change', () => commit({ accent: acc.value as Accent }, 'Set accent'));
    const len = h('input', { type: 'number', min: 5, max: 6400, step: 10, value: Math.round(n.length * 100), 'data-testid': 'inspector-length', 'aria-label': 'Length in percent of a step' });
    len.addEventListener('change', () => commit({ length: Math.max(0.05, Number(len.value) / 100) }, 'Set length'));
    box.append(
      h('div', { class: 'hint' }, `Step ${sel.step + 1} · ${pitchLabel(layer, sel.pitch)}`),
      h('label', { class: 'row' }, h('span', {}, 'Velocity'), vel),
      h('div', { class: 'hint' }, `Plays at ${playedVelocity(n).toFixed(2)} after the accent`),
      h('label', { class: 'row' }, h('span', {}, 'Accent'), acc),
      h('label', { class: 'row' }, h('span', {}, 'Length %'), len),
      n.length > 1 ? h('div', { class: 'hint overlap' }, `Overlaps ${(n.length - 1).toFixed(2)} steps into the next note`) : h('div', { class: 'hint' }, '100% = one step; above overlaps'),
      h('button', { onclick: () => void this.commitNotes('Remove note', (ns) => ns.filter((x) => !(x.step === sel.step && x.pitch === sel.pitch)), null) }, 'Remove note'),
    );
    return box;
  }

  private async toggle(layer: NLayer, step: number, pitch: number): Promise<void> {
    const drum = isUnpitchedClass(layer.class);
    let had = false;
    await this.commitNotes(
      'Toggle step',
      (notes) => {
        had = !!findNote(notes, step, pitch);
        return toggleNote(notes, step, pitch, { velocity: 0.7, accent: 'normal', length: drum ? 1 : 1.5 });
      },
      null,
    );
    if (!had && this.app.state.lastPatch?.ok) this.app.set({ selectedNote: { step, pitch }, focusPitch: pitch });
  }

  /**
   * Applies `change` to the notes the selected range plays once earlier edits have landed (via
   * Controller.edit), so a second event fired quickly never builds on a stale score.
   */
  private async commitNotes(what: string, change: (notes: Note[]) => Note[], select: { step: number; pitch: number } | null): Promise<void> {
    const ok = await this.app.edit((score, st) => {
      const layer = score.layers.find((l) => l.id === st.selection.layerId);
      const d = layer ? derivePattern(score, layer.id, st.selection.bars) : null;
      return layer && d ? editPatternInRange(score, layer.id, st.selection.bars, change(d.pattern.notes), what) : null;
    });
    if (ok) this.app.set({ selectedNote: select, focusPitch: select?.pitch ?? this.app.state.focusPitch });
  }

  private onVelMove(e: MouseEvent): void {
    const d = this.velDrag;
    if (!d) return;
    d.value = clampVelocity(1 - (e.clientY - d.top) / d.height);
    d.bar.style.height = `${d.value * 100}%`;
  }

  private async onVelUp(): Promise<void> {
    const d = this.velDrag;
    this.velDrag = null;
    if (!d) return;
    await this.commitNotes('Set velocity', (ns) => updateNote(ns, d.step, d.pitch, { velocity: d.value }), { step: d.step, pitch: d.pitch });
  }
}
