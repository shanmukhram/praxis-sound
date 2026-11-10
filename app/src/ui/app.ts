// The workstation shell: state, controller and layout. Views call the controller; every score
// change goes through History.commit -> EngineAdapter.applyPatch.

import type { ApplyResult, EngineAdapter } from '../engine/EngineAdapter';
import { patchErrorMessage } from '../engine/EngineAdapter';
import type { NScore, Score, ScorePatch } from '../score/types';
import { formatRange } from '../score/types';
import { History } from '../score/history';
import { describeScope, muteLayer, soloLayer } from '../score/patchBuilder';
import { keyLabel } from '../score/theory';
import { barPosToSeconds, scoreSeconds } from '../score/timeline';
import { decodeWav } from '../engine/wav';
import type { AppState, Controller, Selection } from './state';
import { TimelineView } from './timeline';
import { StepSequencer } from './stepSequencer';
import { PianoRoll } from './pianoRoll';
import { AnalysisView } from './analysis';
import { SidePanel } from './sidePanel';
import { ExportDialog } from './exportDialog';
import { clear, fmtTime, h, isTyping } from './dom';

export interface FixtureEntry {
  name: string;
  score: unknown;
}

export class App implements Controller {
  readonly history: History;
  readonly state: AppState = {
    score: null,
    selection: { layerId: null, bars: { start: 1, end: 1 } },
    editorTab: 'steps',
    selectedNote: null,
    focusPitch: null,
    lastPatch: null,
    position: { playing: false, seconds: 0, bar: 1, loop: null },
    meters: { peakL: -Infinity, peakR: -Infinity, rmsL: -Infinity, rmsR: -Infinity },
    render: null,
    rendering: false,
    status: { text: 'Ready', kind: 'info' },
    issues: { errors: [], warnings: [] },
  };

  private topbar = h('header', { class: 'topbar' });
  private scopebar = h('div', { class: 'scopebar', 'aria-label': 'Edit range' });
  private editorTabs = h('div', { class: 'tabs', role: 'tablist' });
  private editorPanel = h('div', { class: 'tabpanel', role: 'tabpanel' });
  private statusbar = h('footer', { class: 'statusbar' });
  private positionEl = h('div', { class: 'position', 'data-testid': 'position', 'aria-live': 'off' });
  private meterEls: HTMLElement[] = [h('span'), h('span')];
  private timeline: TimelineView;
  private steps: StepSequencer;
  private roll: PianoRoll;
  private analysis: AnalysisView;
  private side: SidePanel;
  private exportDialog: ExportDialog;
  private renderTimer: ReturnType<typeof setTimeout> | null = null;
  private renderSeq = 0;
  private lastPlayheadBar = -1;

  constructor(
    root: HTMLElement,
    readonly engine: EngineAdapter,
    private readonly fixtures: FixtureEntry[],
  ) {
    this.history = new History(engine);
    this.timeline = new TimelineView(this);
    this.steps = new StepSequencer(this);
    this.roll = new PianoRoll(this);
    this.analysis = new AnalysisView(this);
    this.side = new SidePanel(this);
    this.exportDialog = new ExportDialog(this);
    root.append(
      this.topbar,
      this.timeline.el,
      this.scopebar,
      h('main', { class: 'work' }, h('section', { class: 'editor', 'aria-label': 'Pattern editor' }, this.editorTabs, this.editorPanel), this.side.el),
      this.analysis.el,
      this.statusbar,
      this.exportDialog.el,
    );
    engine.onPosition((p) => {
      this.state.position = p;
      this.updatePosition();
    });
    engine.onMeters((m) => {
      this.state.meters = m;
      this.updateMeters();
    });
    window.addEventListener('keydown', (e) => this.onKey(e));
  }

  // ------------------------------------------------------------ controller

  async loadScore(raw: unknown, label: string): Promise<void> {
    try {
      const r = await this.engine.load(raw as Score);
      this.history.reset(r.score);
      this.state.score = r.score;
      this.state.issues = { errors: r.errors, warnings: r.warnings };
      this.state.lastPatch = null;
      this.state.render = null;
      this.state.selectedNote = null;
      this.state.focusPitch = null;
      this.state.selection = defaultSelection(r.score);
      this.state.status = r.errors.length
        ? { text: `Loaded ${label} with ${r.errors.length} validation error(s): ${r.errors[0].code} at ${r.errors[0].path}`, kind: 'bad' }
        : { text: `Loaded ${label}`, kind: 'ok' };
      this.update();
      this.scheduleRender(0);
    } catch (e) {
      this.set({ status: { text: `Could not load ${label}: ${(e as Error).message}`, kind: 'bad' } });
    }
  }

  private editChain: Promise<unknown> = Promise.resolve();

  edit(build: (score: NScore, state: AppState) => ScorePatch | null): Promise<boolean> {
    const run = this.editChain.then(() => (this.state.score ? this.commit(build(this.state.score, this.state)) : false));
    this.editChain = run.catch(() => undefined);
    return run;
  }

  async commit(patch: ScorePatch | null): Promise<boolean> {
    if (!patch) return false;
    const r = await this.history.commit(patch);
    if (r.ok) {
      this.state.score = r.score;
      this.state.lastPatch = { patch, ok: true, message: 'applied', kind: 'edit' };
      this.state.status = { text: `${patch.description} — ${describeScope(patch)}`, kind: 'ok' };
      this.scheduleRender();
    } else {
      const msg = patchErrorMessage(r.error);
      this.state.lastPatch = { patch, ok: false, message: `rejected: ${msg}`, kind: 'edit' };
      this.state.status = { text: `Rejected: ${msg}`, kind: 'bad' };
    }
    this.fixSelection();
    this.update();
    return r.ok;
  }

  async undo(): Promise<void> {
    const r = await this.history.undoLast();
    if (r) this.afterHistory(r, r.entry.inverse, 'undo', `Undid #${r.entry.seq}: ${r.entry.patch.description}`);
  }

  async redo(): Promise<void> {
    const r = await this.history.redoLast();
    if (r) this.afterHistory(r, r.entry.patch, 'redo', `Redid #${r.entry.seq}: ${r.entry.patch.description}`);
  }

  private afterHistory(r: ApplyResult, patch: ScorePatch, kind: 'undo' | 'redo', text: string): void {
    if (r.ok) {
      this.state.score = r.score;
      this.state.lastPatch = { patch, ok: true, message: `${kind} applied`, kind };
      this.state.status = { text, kind: 'ok' };
      this.scheduleRender();
    } else {
      this.state.status = { text: `${kind} failed: ${patchErrorMessage(r.error)}`, kind: 'bad' };
    }
    this.fixSelection();
    this.update();
  }

  select(sel: Partial<Selection>): void {
    const prev = this.state.selection;
    const next = { ...prev, ...sel };
    const layerChanged = next.layerId !== prev.layerId;
    const changed = layerChanged || next.bars.start !== prev.bars.start || next.bars.end !== prev.bars.end;
    this.state.selection = next;
    if (changed) this.state.selectedNote = null;
    if (layerChanged) this.state.focusPitch = null;
    this.fixSelection();
    this.update();
  }

  set(partial: Partial<AppState>): void {
    Object.assign(this.state, partial);
    this.update();
  }

  seekBar(bar: number): void {
    if (!this.state.score) return;
    void this.engine.seek(barPosToSeconds(this.state.score, bar));
  }

  togglePlay(): void {
    if (this.state.position.playing) void this.engine.stop();
    else void this.engine.play();
  }

  toggleLoop(): void {
    const loop = this.state.position.loop;
    const sel = this.state.selection.bars;
    if (loop && loop.start === sel.start && loop.end === sel.end) void this.engine.setLoop(null);
    else void this.engine.setLoop({ ...sel });
  }

  async render(): Promise<void> {
    if (!this.state.score) return;
    const seq = ++this.renderSeq;
    this.state.rendering = true;
    this.analysis.update(this.state);
    try {
      const result = await this.engine.render();
      const audio = decodeWav(await (await fetch(result.audioUrl)).arrayBuffer());
      if (seq !== this.renderSeq) return;
      this.state.render = { result, audio };
    } catch (e) {
      this.state.status = { text: `Render failed: ${(e as Error).message}`, kind: 'bad' };
    } finally {
      if (seq === this.renderSeq) this.state.rendering = false;
    }
    this.analysis.update(this.state);
    this.renderStatus();
  }

  saveVersion(name: string): void {
    if (!this.state.score) return;
    this.history.saveVersion(name, this.state.score);
    this.set({ status: { text: `Saved version "${name}"`, kind: 'ok' } });
  }

  async restoreVersion(name: string): Promise<void> {
    if (!this.state.score) return;
    const before = this.history.undo.length;
    const r = await this.history.restoreVersion(name);
    if (!r) return;
    if (r.ok) {
      this.state.score = r.score;
      const entry = this.history.undo.length > before ? this.history.undo.at(-1)! : null;
      if (entry) this.state.lastPatch = { patch: entry.patch, ok: true, message: 'applied', kind: 'edit' };
      this.state.status = { text: entry ? `Restored version "${name}" (undo with Ctrl+Z)` : `"${name}" is the current state`, kind: 'ok' };
      this.scheduleRender();
    } else {
      this.state.status = { text: `Restore "${name}" rejected: ${patchErrorMessage(r.error)}`, kind: 'bad' };
    }
    this.fixSelection();
    this.update();
  }

  openExport(): void {
    this.exportDialog.open();
  }

  // ------------------------------------------------------------ rendering the shell

  private scheduleRender(delay = 350): void {
    if (this.renderTimer) clearTimeout(this.renderTimer);
    this.renderTimer = setTimeout(() => void this.render(), delay);
  }

  private fixSelection(): void {
    const score = this.state.score;
    if (!score) return;
    const sel = this.state.selection;
    if (sel.layerId && !score.layers.some((l) => l.id === sel.layerId)) sel.layerId = score.layers[0]?.id ?? null;
    const start = Math.max(1, Math.min(sel.bars.start, score.bars));
    const end = Math.max(start, Math.min(sel.bars.end, score.bars));
    sel.bars = { start, end };
  }

  update(): void {
    this.renderTopbar();
    this.renderScopebar();
    this.timeline.update(this.state);
    this.renderEditor();
    this.side.update(this.state);
    this.analysis.update(this.state);
    this.renderStatus();
    this.updatePosition();
  }

  private renderTopbar(): void {
    const s = this.state.score;
    clear(this.topbar);
    const fileInput = h('input', { type: 'file', accept: '.json,application/json', style: 'display:none', 'data-testid': 'open-file' });
    fileInput.addEventListener('change', async () => {
      const f = fileInput.files?.[0];
      if (!f) return;
      try {
        await this.loadScore(JSON.parse(await f.text()), f.name);
      } catch (e) {
        this.set({ status: { text: `Could not read ${f.name}: ${(e as Error).message}`, kind: 'bad' } });
      }
    });
    const fixtureSel = h(
      'select',
      { 'aria-label': 'Load a fixture', 'data-testid': 'fixture-select' },
      h('option', { value: '' }, 'Fixtures…'),
      ...this.fixtures.map((f) => h('option', { value: f.name }, f.name)),
    );
    fixtureSel.addEventListener('change', () => {
      const f = this.fixtures.find((x) => x.name === fixtureSel.value);
      if (f) void this.loadScore(f.score, f.name);
    });
    const playing = this.state.position.playing;
    const loopOn = !!this.state.position.loop;
    this.topbar.append(
      h('span', { class: 'brand' }, 'Praxis Sound'),
      h('span', { class: 'title', 'data-testid': 'score-title', title: s?.title ?? '' }, s?.title || 'Untitled'),
      s
        ? h(
            'div',
            { class: 'facts', 'data-testid': 'facts' },
            h('span', { class: 'fact' }, keyLabel(s.key)),
            h('span', { class: 'fact' }, `${s.tempo.bpm} BPM`),
            h('span', { class: 'fact' }, `${s.time_signature.numerator}/${s.time_signature.denominator}`),
            h('span', { class: 'fact', 'data-testid': 'bars-count' }, `${s.bars} bars`),
            h('span', { class: 'fact' }, s.playback.mode === 'loop' ? 'loop' : 'one-shot'),
          )
        : '',
      h('div', { class: 'group' }, h('button', { onclick: () => fileInput.click(), title: 'Open a Score JSON file' }, 'Open…'), fileInput, fixtureSel),
      h('div', { class: 'spacer' }),
      h(
        'div',
        { class: 'group', role: 'group', 'aria-label': 'Transport' },
        h('button', { onclick: () => this.seekBar(1), title: 'Return to start (Home)', 'aria-label': 'Return to start' }, '⏮'),
        h('button', { class: 'toggle', 'aria-pressed': String(playing), onclick: () => this.togglePlay(), 'data-testid': 'play', title: 'Play / stop (Space)' }, playing ? '■ Stop' : '▶ Play'),
        h(
          'button',
          {
            class: 'toggle',
            'aria-pressed': String(loopOn),
            onclick: () => this.toggleLoop(),
            'data-testid': 'loop',
            title: 'Loop the selected bars (L)',
          },
          loopOn ? `Loop ${formatRange(this.state.position.loop!).replace('bars ', '')}` : 'Loop',
        ),
        this.positionEl,
        h('div', { class: 'meter', 'aria-hidden': 'true', title: 'Output peak L/R' }, h('div', {}, this.meterEls[0]), h('div', {}, this.meterEls[1])),
      ),
      h('div', { class: 'spacer' }),
      h('button', { onclick: () => void this.render(), title: 'Render (R)', 'data-testid': 'render' }, 'Render'),
      h('button', { onclick: () => this.openExport(), title: 'Export (E)', 'data-testid': 'export' }, 'Export…'),
      h('span', { class: 'badge', title: this.engine.kind === 'mock' ? 'UI is running against the TypeScript mock engine. Audio is a placeholder preview, not Praxis instruments.' : '' }, this.engine.kind === 'mock' ? 'Mock engine' : this.engine.label),
    );
  }

  private renderScopebar(): void {
    clear(this.scopebar);
    const s = this.state.score;
    if (!s) return;
    const sel = this.state.selection;
    const layerSel = h(
      'select',
      { 'aria-label': 'Layer to edit', 'data-testid': 'scope-layer' },
      ...s.layers.map((l) => h('option', { value: l.id, selected: l.id === sel.layerId }, `${l.name || l.id}${l.locked ? ' (locked)' : ''}`)),
    );
    layerSel.addEventListener('change', () => this.select({ layerId: layerSel.value }));
    const from = h('input', { type: 'number', min: 1, max: s.bars, value: sel.bars.start, 'aria-label': 'First bar', 'data-testid': 'scope-from' });
    const to = h('input', { type: 'number', min: 1, max: s.bars, value: sel.bars.end, 'aria-label': 'Last bar', 'data-testid': 'scope-to' });
    // The field just edited wins: a first bar past the last moves the last bar along (and the
    // reverse), instead of swapping the two.
    const apply = (edited: 'from' | 'to') => {
      let a = Math.round(Number(from.value));
      let b = Math.round(Number(to.value));
      if (!Number.isFinite(a) || !Number.isFinite(b)) return;
      if (a > b) {
        if (edited === 'from') b = a;
        else a = b;
      }
      this.select({ bars: { start: a, end: b } });
    };
    from.addEventListener('change', () => apply('from'));
    to.addEventListener('change', () => apply('to'));
    const sections = h(
      'select',
      { 'aria-label': 'Use a section as the range' },
      h('option', { value: '' }, 'Section…'),
      h('option', { value: 'all' }, `All bars (1-${s.bars})`),
      ...s.sections.map((x) => h('option', { value: x.id }, `${x.name || x.id} (${x.bars.start}-${x.bars.end})`)),
    );
    sections.addEventListener('change', () => {
      if (sections.value === 'all') this.select({ bars: { start: 1, end: s.bars } });
      const sec = s.sections.find((x) => x.id === sections.value);
      if (sec) this.select({ bars: { ...sec.bars } });
    });
    const lp = this.state.lastPatch;
    const layer = s.layers.find((l) => l.id === sel.layerId);
    this.scopebar.append(
      h('label', {}, 'Edit ', layerSel),
      h('label', {}, 'bars ', from, ' – ', to),
      sections,
      h('span', { class: 'scope-readout', 'data-testid': 'edit-scope', title: 'Scope of the next edit' }, `next edit: ${formatRange(sel.bars)} · ${sel.layerId ?? '—'}${layer?.locked ? ' (locked)' : ''}`),
      h('span', { class: 'spacer' }),
      lp
        ? h(
            'span',
            { class: 'patch-status', 'data-testid': 'patch-status' },
            h('span', { class: lp.ok ? 'ok' : 'bad' }, lp.ok ? (lp.kind === 'edit' ? '✓ applied' : `✓ ${lp.kind}`) : '✗ rejected'),
            h('span', { class: 'scope-readout', 'data-testid': 'patch-scope' }, describeScope(lp.patch)),
            h('span', { class: 'desc', title: lp.message }, lp.patch.description ?? ''),
          )
        : h('span', { class: 'hint' }, 'Edits become scoped patches; nothing outside the scope can change.'),
    );
  }

  private renderEditor(): void {
    clear(this.editorTabs);
    const tab = (id: AppState['editorTab'], label: string, key: string) =>
      h(
        'button',
        { role: 'tab', 'aria-selected': String(this.state.editorTab === id), 'data-testid': `tab-${id}`, title: `${label} (${key})`, onclick: () => this.set({ editorTab: id }) },
        label,
      );
    this.editorTabs.append(tab('steps', 'Step sequencer', '1'), tab('roll', 'Piano roll', '2'), h('span', { class: 'info', 'data-testid': 'editor-info' }, StepSequencer.describe(this.state)));
    clear(this.editorPanel);
    if (this.state.editorTab === 'steps') {
      this.steps.update(this.state);
      this.editorPanel.append(this.steps.el);
    } else {
      this.editorPanel.append(this.roll.el);
      this.roll.update(this.state);
    }
  }

  private renderStatus(): void {
    clear(this.statusbar);
    const st = this.state.status;
    const issues = this.state.issues;
    this.statusbar.append(
      h('span', { class: `msg ${st.kind}`, 'data-testid': 'status', role: 'status' }, st.text),
      h('span', { class: 'spacer' }),
      issues.warnings.length || issues.errors.length ? h('span', {}, `${issues.errors.length} errors · ${issues.warnings.length} warnings at load`) : '',
      h(
        'span',
        {},
        ...[
          ['Space', 'play'],
          ['L', 'loop'],
          ['Ctrl+Z', 'undo'],
          ['Ctrl+Shift+Z', 'redo'],
          ['M/S', 'mute/solo'],
          ['R', 'render'],
          ['E', 'export'],
        ].flatMap(([k, v]) => [h('span', { class: 'kbd' }, k), ` ${v}  `]),
      ),
    );
  }

  private updatePosition(): void {
    const p = this.state.position;
    const s = this.state.score;
    if (s) {
      const ts = s.time_signature;
      const barInt = Math.floor(p.bar);
      const beat = Math.floor((p.bar - barInt) * ts.numerator) + 1;
      this.positionEl.textContent = `${String(barInt).padStart(3, '0')}.${beat}  ${fmtTime(p.seconds)} / ${fmtTime(scoreSeconds(s))}`;
    }
    // Repaint the canvases that show the playhead (cheap: no DOM rebuild).
    if (p.playing || p.bar !== this.lastPlayheadBar) {
      this.lastPlayheadBar = p.bar;
      this.timeline.draw(this.state);
      this.analysis.updatePosition(this.state);
    }
    const playBtn = this.topbar.querySelector<HTMLButtonElement>('[data-testid=play]');
    if (playBtn && playBtn.getAttribute('aria-pressed') !== String(p.playing)) {
      playBtn.setAttribute('aria-pressed', String(p.playing));
      playBtn.textContent = p.playing ? '■ Stop' : '▶ Play';
    }
    const loopBtn = this.topbar.querySelector<HTMLButtonElement>('[data-testid=loop]');
    if (loopBtn) {
      loopBtn.setAttribute('aria-pressed', String(!!p.loop));
      loopBtn.textContent = p.loop ? `Loop ${p.loop.start}-${p.loop.end}` : 'Loop';
    }
  }

  private updateMeters(): void {
    const m = this.state.meters;
    const pct = (db: number) => (Number.isFinite(db) ? Math.max(0, Math.min(100, ((db + 60) / 60) * 100)) : 0);
    this.meterEls[0].style.width = `${pct(m.peakL)}%`;
    this.meterEls[1].style.width = `${pct(m.peakR)}%`;
  }

  private onKey(e: KeyboardEvent): void {
    if (isTyping(e) || this.exportDialog.el.open) return;
    const mod = e.ctrlKey || e.metaKey;
    const s = this.state.score;
    const layerId = this.state.selection.layerId;
    const layer = s?.layers.find((l) => l.id === layerId);
    if (mod && e.key.toLowerCase() === 'z') {
      e.preventDefault();
      void (e.shiftKey ? this.redo() : this.undo());
    } else if (mod && e.key.toLowerCase() === 'y') {
      e.preventDefault();
      void this.redo();
    } else if (mod) {
      return;
    } else if (e.key === ' ') {
      e.preventDefault();
      this.togglePlay();
    } else if (e.key === 'Home') {
      e.preventDefault();
      this.seekBar(1);
    } else if (e.key === 'l' || e.key === 'L') {
      this.toggleLoop();
    } else if (e.key === 'r' || e.key === 'R') {
      void this.render();
    } else if (e.key === 'e' || e.key === 'E') {
      this.openExport();
    } else if ((e.key === 'm' || e.key === 'M') && layer) {
      void this.edit((sc) => {
        const l = sc.layers.find((x) => x.id === layer.id);
        return l ? muteLayer(sc, l.id, !l.muted) : null;
      });
    } else if ((e.key === 's' || e.key === 'S') && layer) {
      void this.edit((sc) => {
        const l = sc.layers.find((x) => x.id === layer.id);
        return l ? soloLayer(sc, l.id, !l.solo) : null;
      });
    } else if (e.key === '1') {
      this.set({ editorTab: 'steps' });
    } else if (e.key === '2') {
      this.set({ editorTab: 'roll' });
    }
  }
}

/** First unlocked layer with clips, on its first clip; else the first layer. */
function defaultSelection(score: NScore): Selection {
  const layer = score.layers.find((l) => !l.locked && l.clips.length) ?? score.layers[0];
  if (!layer) return { layerId: null, bars: { start: 1, end: Math.min(4, score.bars) } };
  const clip = layer.clips[0];
  return { layerId: layer.id, bars: clip ? { ...clip.bars } : { start: 1, end: Math.min(4, score.bars) } };
}
