// Export dialog (FR-32): WAV 16/24/32-bit float, FLAC, per-layer stems, MIDI and the project file.
// The adapter does the work; the mock only records the request.

import type { Controller } from './state';
import type { ExportKind } from '../engine/EngineAdapter';
import { clear, h } from './dom';

const KINDS: { kind: ExportKind; label: string; checked: boolean }[] = [
  { kind: 'wav16', label: 'WAV 16-bit', checked: false },
  { kind: 'wav24', label: 'WAV 24-bit', checked: true },
  { kind: 'wav32f', label: 'WAV 32-bit float', checked: false },
  { kind: 'flac', label: 'FLAC', checked: false },
  { kind: 'stems', label: 'Stems (one WAV per layer)', checked: false },
  { kind: 'midi', label: 'MIDI', checked: false },
  { kind: 'project', label: 'Project file (.praxis)', checked: true },
];

export class ExportDialog {
  readonly el: HTMLDialogElement;
  private result: HTMLElement;
  private dest: HTMLInputElement;
  private base: HTMLInputElement;
  private boxes = new Map<ExportKind, HTMLInputElement>();

  constructor(private readonly app: Controller) {
    this.dest = h('input', { type: 'text', value: 'exports', 'aria-label': 'Destination folder', 'data-testid': 'export-dest' });
    this.base = h('input', { type: 'text', value: 'praxis', 'aria-label': 'File name', 'data-testid': 'export-name' });
    this.result = h('div', { 'data-testid': 'export-result' });
    const fs = h('fieldset', {}, h('legend', {}, 'Formats'));
    for (const k of KINDS) {
      const box = h('input', { type: 'checkbox', checked: k.checked, 'data-testid': `export-${k.kind}` });
      this.boxes.set(k.kind, box);
      fs.append(h('label', {}, box, ` ${k.label}`));
    }
    this.el = h(
      'dialog',
      { 'aria-labelledby': 'export-title', 'data-testid': 'export-dialog' },
      h('h2', { id: 'export-title' }, 'Export'),
      h('div', { class: 'hint' }, app.engine.kind === 'mock' ? 'Mock engine: the request is recorded, nothing is written.' : ''),
      fs,
      h('label', { class: 'row' }, h('span', {}, 'Folder'), this.dest),
      h('label', { class: 'row' }, h('span', {}, 'Name'), this.base),
      this.result,
      h(
        'div',
        { class: 'actions' },
        h('button', { onclick: () => this.el.close() }, 'Close'),
        h('button', { class: 'primary', 'data-testid': 'export-go', onclick: () => void this.go() }, 'Export'),
      ),
    );
  }

  open(): void {
    const title = this.app.state.score?.title ?? '';
    const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
    if (slug) this.base.value = slug;
    clear(this.result);
    this.el.showModal();
  }

  private async go(): Promise<void> {
    const kinds = KINDS.map((k) => k.kind).filter((k) => this.boxes.get(k)!.checked);
    clear(this.result);
    if (!kinds.length) {
      this.result.append(h('div', { class: 'hint', style: 'color:var(--bad)' }, 'Choose at least one format.'));
      return;
    }
    const r = await this.app.engine.export({ kinds, destination: this.dest.value.trim() || '.', baseName: this.base.value.trim() || 'praxis' });
    this.result.append(h('div', { class: 'hint', style: `color:var(${r.ok ? '--ok' : '--bad'})` }, r.message), h('ul', {}, ...r.files.map((f) => h('li', {}, f.path))));
    this.app.set({ status: { text: `Export: ${r.message}`, kind: r.ok ? 'ok' : 'bad' } });
  }
}
