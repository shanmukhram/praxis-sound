// History (undo/redo), named versions (FR-31) and the last patch as JSON.

import type { AppState, Controller } from './state';
import { describeScope } from '../score/patchBuilder';
import { clear, h } from './dom';

export class SidePanel {
  readonly el: HTMLElement;
  private historyBox: HTMLElement;
  private versionsBox: HTMLElement;
  private patchBox: HTMLElement;
  private nameInput: HTMLInputElement;
  private historyList: HTMLElement;
  private undoBtn: HTMLButtonElement;
  private redoBtn: HTMLButtonElement;

  constructor(private readonly app: Controller) {
    // Persistent controls: rebuilding them on every update could swallow a click whose
    // mousedown and mouseup straddle a re-render.
    this.undoBtn = h('button', { onclick: () => void this.app.undo(), 'data-testid': 'undo', title: 'Undo (Ctrl+Z)' }, 'Undo');
    this.redoBtn = h('button', { onclick: () => void this.app.redo(), 'data-testid': 'redo', title: 'Redo (Ctrl+Shift+Z)' }, 'Redo');
    this.historyList = h('ol', { class: 'history-list', 'aria-label': 'Edit history, newest first' });
    this.nameInput = h('input', { type: 'text', placeholder: 'Version name', 'aria-label': 'Version name', 'data-testid': 'version-name', style: 'flex:1;min-width:0' });
    this.nameInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') this.save();
    });
    this.historyBox = h(
      'section',
      { 'data-testid': 'history' },
      h('div', { style: 'display:flex;align-items:center;gap:6px' }, h('h3', { style: 'flex:1' }, 'History'), this.undoBtn, this.redoBtn),
      this.historyList,
    );
    this.versionsBox = h('section', { 'data-testid': 'versions' });
    this.patchBox = h('section', { 'data-testid': 'patch-panel' });
    this.el = h('aside', { class: 'side', 'aria-label': 'History, versions and patch' }, this.historyBox, this.versionsBox, this.patchBox);
  }

  private save(): void {
    const name = this.nameInput.value.trim() || `Version ${this.app.history.versions.length + 1}`;
    this.app.saveVersion(name);
    this.nameInput.value = '';
  }

  update(s: AppState): void {
    const hist = this.app.history;
    this.undoBtn.disabled = !hist.canUndo;
    this.redoBtn.disabled = !hist.canRedo;
    const list = this.historyList;
    clear(list);
    for (const e of [...hist.redo].reverse()) {
      list.append(h('li', { class: 'redo', title: 'Undone: redo with Ctrl+Shift+Z' }, h('span', { class: 'seq' }, `#${e.seq}`), e.patch.description || e.patch.ops[0]?.op || ''));
    }
    [...hist.undo].reverse().forEach((e, i) => {
      list.append(h('li', { class: i === 0 ? 'current' : '' }, h('span', { class: 'seq' }, `#${e.seq}`), e.patch.description || e.patch.ops[0]?.op || ''));
    });
    if (!hist.undo.length && !hist.redo.length) list.append(h('li', { class: 'redo' }, 'No edits yet'));

    clear(this.versionsBox);
    const vs = h('div', { class: 'versions' });
    for (const v of hist.versions) {
      vs.append(
        h(
          'div',
          { class: 'version' },
          h('span', {}, v.name, h('span', { class: 'hint' }, ` at #${v.seq}`)),
          h('button', { onclick: () => void this.app.restoreVersion(v.name), 'data-testid': `restore-${v.name}` }, 'Restore'),
        ),
      );
    }
    if (!hist.versions.length) vs.append(h('div', { class: 'hint' }, 'Save a named version to come back to it; restoring is undoable.'));
    this.versionsBox.append(
      h('h3', {}, 'Versions'),
      h('div', { style: 'display:flex;gap:6px' }, this.nameInput, h('button', { onclick: () => this.save(), 'data-testid': 'save-version' }, 'Save')),
      vs,
    );

    clear(this.patchBox);
    this.patchBox.append(h('h3', {}, 'Last patch'));
    const lp = s.lastPatch;
    if (!lp) {
      this.patchBox.append(h('div', { class: 'hint' }, 'Every edit is a scoped ScorePatch; it appears here.'));
      return;
    }
    this.patchBox.append(
      h('div', { class: 'scope-readout', 'data-testid': 'patch-panel-scope' }, `scope: ${describeScope(lp.patch)}`),
      h('div', { class: lp.ok ? 'hint' : 'hint', style: `color:var(${lp.ok ? '--ok' : '--bad'})` }, lp.message),
      h('pre', { class: 'json', 'data-testid': 'patch-json' }, JSON.stringify(lp.patch, null, 2)),
    );
  }
}
