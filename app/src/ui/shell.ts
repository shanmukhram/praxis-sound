// The conversation-first workstation (ADR-0011): sounds list | conversation about one sound |
// Sound panel. Every request makes a take through the EngineAdapter; the thread and the panel
// are drawn from the adapter's TakeState, so people and agents share one history.

import type { AgentInfo, AskResult, EngineAdapter, Take, TakeScope, TakeState, ViewName, Who } from '../engine/EngineAdapter';
import { patchErrorMessage } from '../engine/EngineAdapter';
import { changedText, layerLabel } from '../engine/takes';
import type { ScorePatch } from '../score/types';
import { keyLabel } from '../score/theory';
import { scoreSeconds } from '../score/timeline';
import type { ViewContext } from './context';
import { summarizeChecks } from './checks';
import { clear, h, isTyping } from './dom';
import { waveSvg } from './waveform';
import { renderChangeMap } from './views/changeMap';
import { MACRO_DEFAULTS, renderOverview } from './views/overview';
import { renderRhythm } from './views/rhythm';
import { renderNotes } from './views/notes';
import { renderExport, renderMix } from './views/mixExport';

export interface SoundEntry {
  id: string;
  name: string;
  sub: string;
  /** Opens this sound in the adapter (starts its conversation). */
  open(): Promise<void>;
}

const VIEWS: { id: ViewName; name: string }[] = [
  { id: 'overview', name: 'Overview' },
  { id: 'rhythm', name: 'Rhythm' },
  { id: 'notes', name: 'Notes' },
  { id: 'mix', name: 'Mix' },
  { id: 'export', name: 'Export' },
];

interface Reply {
  /** Shown after this many takes. */
  after: number;
  who: Who;
  ask: string | null;
  message: string;
}

const PHONE = '(max-width: 760px)';

const ICON_PLAY = '<svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"><path d="M3 1.5v11l9.5-5.5z" fill="currentColor"/></svg>';
const ICON_PAUSE =
  '<svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"><rect x="2" y="1" width="3.5" height="12" fill="currentColor"/><rect x="8.5" y="1" width="3.5" height="12" fill="currentColor"/></svg>';
const LOGO =
  '<svg width="26" height="26" viewBox="0 0 26 26" aria-hidden="true"><rect width="26" height="26" rx="7" fill="#f5a524"/><path d="M6 17V9M10 19V7M14 16v-6M18 18V8M22 14v-2" stroke="#111" stroke-width="2.2" stroke-linecap="round"/></svg>';
const CLIP =
  '<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M10.5 5 5.8 9.7a1.5 1.5 0 0 0 2.1 2.1l5-5a3 3 0 0 0-4.2-4.2l-5 5a4.5 4.5 0 0 0 6.4 6.4L14 9"/></svg>';

export class Shell {
  private st: TakeState;
  private view: ViewName = 'overview';
  private flash: { view: ViewName; by: string } | null = null;
  private selection: TakeScope | null = null;
  private making: { ask: string; who: Who } | null = null;
  private replies: Reply[] = [];
  private clip: File | null = null;
  private knobs: Record<string, number> = { ...MACRO_DEFAULTS };
  private chips: string[] = [];
  private agentList: AgentInfo[] = [];
  private soundId = '';
  private playing: { n: number; audio: HTMLAudioElement; length: number } | null = null;
  private lastTakeCount = -1;
  /** True while a sound is being opened: views opened then do not pop the phone sheet up. */
  private opening = false;

  private side = h('nav', { class: 'side', 'aria-label': 'Your sounds' });
  private head = h('div', { class: 'chat-head' });
  private thread = h('div', { class: 'thread', 'aria-live': 'polite', 'data-testid': 'thread' });
  private suggest = h('div', { class: 'suggest', 'aria-label': 'Suggestions', 'data-testid': 'suggestions' });
  private chipRow = h('div', { class: 'chips' });
  private prompt = h('textarea', { id: 'prompt', rows: 1, placeholder: 'Describe a sound, or drop in a clip to start from', 'data-testid': 'prompt' });
  private send = h('button', { class: 'send', type: 'submit', disabled: true, 'data-testid': 'send' }, 'Make it');
  private fileInput = h('input', { type: 'file', accept: 'audio/*,video/*', class: 'sr', 'data-testid': 'clip-input', 'aria-label': 'Choose a clip' });
  private panel = h('aside', { class: 'panel', id: 'panel', 'aria-label': 'Sound panel', 'data-testid': 'panel' });
  private panelLine = h('p', {});
  private tabs = h('div', { class: 'tabs', role: 'tablist' });
  private body = h('div', { class: 'panel-body', role: 'tabpanel', 'data-testid': 'panel-body' });
  private drop = h('div', { class: 'drop' }, 'Drop an audio or video clip. Praxis keeps its character, never its melody.');

  constructor(
    root: HTMLElement,
    readonly engine: EngineAdapter,
    private readonly sounds: SoundEntry[],
  ) {
    this.st = engine.state();
    const composer = h(
      'form',
      { class: 'composer', 'data-testid': 'composer' },
      this.chipRow,
      h('label', { for: 'prompt', class: 'sr' }, 'Describe what you want'),
      this.prompt,
      h(
        'div',
        { class: 'composer-row' },
        h('button', { type: 'button', class: 'attach', onclick: () => this.fileInput.click() }, svg(CLIP), 'Add a clip'),
        this.send,
      ),
      this.fileInput,
    );
    composer.addEventListener('submit', (e) => {
      e.preventDefault();
      this.submit();
    });
    this.prompt.addEventListener('input', () => this.syncSend());
    this.prompt.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        this.submit();
      }
    });
    this.fileInput.addEventListener('change', () => {
      this.clip = this.fileInput.files?.[0] ?? null;
      this.renderChips();
      this.syncSend();
    });

    const sheetToggle = h('button', { class: 'ghost sheet-toggle', 'aria-controls': 'panel', 'data-testid': 'sheet-toggle', onclick: () => this.toggleSheet() }, 'Sound');
    const chat = h(
      'main',
      { class: 'chat' },
      h('div', { class: 'phone-bar' }, h('b', {}, 'Praxis Sound'), sheetToggle),
      this.head,
      this.thread,
      h('div', { class: 'composer-wrap' }, this.suggest, composer),
    );
    this.panel.append(
      h(
        'div',
        { class: 'panel-head' },
        h('div', {}, h('h2', {}, 'Sound'), this.panelLine),
        h('button', { class: 'ghost sheet-close', onclick: () => this.toggleSheet(false), 'aria-label': 'Close the Sound panel' }, 'Close'),
      ),
      this.tabs,
      this.body,
    );
    root.append(h('div', { class: 'app' }, this.side, chat, this.panel), this.drop);

    engine.onState((s) => {
      this.st = s;
      this.update();
      void engine.suggestions().then((c) => {
        this.chips = c;
        this.renderSuggestions();
      });
    });
    engine.onOpenView((e) => {
      this.view = e.view;
      this.flash = { view: e.view, by: e.by };
      if (matchMedia(PHONE).matches && !this.opening) this.toggleSheet(true);
      this.renderPanel();
    });
    void engine.agents().then((a) => {
      this.agentList = a;
      this.renderSide();
    });
    this.wireDrop();
    window.addEventListener('keydown', (e) => this.onKey(e));
    requestAnimationFrame(() => this.tick());
  }

  async openSound(id: string): Promise<void> {
    const s = this.sounds.find((x) => x.id === id);
    if (!s) return;
    this.stopPlaying();
    this.soundId = id;
    this.replies = [];
    this.selection = null;
    this.flash = null;
    this.view = 'overview';
    this.knobs = { ...MACRO_DEFAULTS };
    this.lastTakeCount = -1;
    this.renderSide();
    this.toggleSheet(false);
    this.opening = true;
    try {
      await s.open();
    } finally {
      this.opening = false;
    }
    this.update();
  }

  // ------------------------------------------------------------ actions

  /** The sound's name as listed (a new sound has none until its first take). */
  private soundName(): string {
    const e = this.sounds.find((x) => x.id === this.soundId);
    return e && e.id !== 'new' ? e.name : '';
  }

  private current(): Take | null {
    return this.st.takes.find((t) => t.n === this.st.current) ?? null;
  }

  private submit(): void {
    const text = this.prompt.value.trim();
    if (!text && !this.clip) return;
    this.prompt.value = '';
    this.prompt.style.height = 'auto';
    void this.ask(text || `Start from ${this.clip!.name}`);
  }

  private async ask(text: string): Promise<void> {
    if (this.making) return;
    const scope = this.selection ?? undefined;
    const label = scope ? ` (${scopeText(this.st, scope)})` : '';
    const clip = this.clip;
    this.making = { ask: text + label, who: 'You' };
    this.syncSend();
    this.renderThread(true);
    const r = await this.engine.ask({ text, scope, clipFile: clip ?? undefined });
    this.finish(r, text + label, 'You');
    if (r.ok) {
      this.selection = null;
      this.clip = null;
      this.renderChips();
    }
  }

  private async edit(patch: ScorePatch, ask: string, scope?: TakeScope): Promise<void> {
    if (this.making) return;
    this.making = { ask, who: 'You' };
    this.renderThread(true);
    const r = await this.engine.edit(patch, ask, scope);
    this.finish(r, ask, 'You');
  }

  private finish(r: AskResult, ask: string, who: Who): void {
    this.making = null;
    if (!r.ok) this.replies.push({ after: this.st.takes.length, who, ask, message: r.message });
    this.syncSend();
    this.update();
  }

  private async apply(patch: ScorePatch): Promise<void> {
    const r = await this.engine.applyPatch(patch);
    if (!r.ok) {
      this.replies.push({ after: this.st.takes.length, who: 'You', ask: null, message: `Could not ${patch.description?.toLowerCase() ?? 'do that'}: ${patchErrorMessage(r.error)}.` });
      this.update();
    }
  }

  private async restore(n: number): Promise<void> {
    const r = await this.engine.restore(n);
    if (!r.ok) {
      this.replies.push({ after: this.st.takes.length, who: 'You', ask: null, message: r.message });
      this.update();
    }
  }

  private select(scope: TakeScope | null): void {
    this.selection = scope;
    this.renderChips();
    this.renderPanel();
    this.prompt.placeholder = scope
      ? `What should change in ${scope.bars.start === scope.bars.end ? `bar ${scope.bars.start}` : `bars ${scope.bars.start}–${scope.bars.end}`}?`
      : 'Describe a sound, or drop in a clip to start from';
  }

  private openView(view: ViewName): void {
    this.view = view;
    this.flash = null;
    if (matchMedia(PHONE).matches) this.toggleSheet(true);
    this.renderPanel();
  }

  private toggleSheet(open?: boolean): void {
    this.panel.classList.toggle('open', open);
  }

  // ------------------------------------------------------------ rendering

  update(): void {
    this.renderSide();
    this.renderHead();
    this.renderThread(this.st.takes.length !== this.lastTakeCount);
    this.lastTakeCount = this.st.takes.length;
    this.renderChips();
    this.renderPanel();
  }

  private renderSide(): void {
    clear(this.side);
    const list = h('ul', { class: 'sounds' });
    for (const s of this.sounds) {
      if (s.id === 'new' && this.soundId !== 'new') continue;
      list.append(
        h(
          'li',
          {},
          h('button', { 'aria-current': String(s.id === this.soundId), 'data-testid': `sound-${s.id}`, onclick: () => void this.openSound(s.id) }, h('span', {}, s.name), h('small', {}, s.sub)),
        ),
      );
    }
    const connected = this.agentList.filter((a) => a.connected);
    this.side.append(
      h('div', { class: 'brand' }, svg(LOGO), h('b', {}, 'Praxis Sound')),
      h('button', { class: 'new-sound', 'data-testid': 'new-sound', onclick: () => void this.openSound('new') }, h('span', { 'aria-hidden': 'true' }, '+'), ' New sound'),
      h('h2', {}, 'Sounds'),
      list,
      h(
        'div',
        { class: 'agents', 'data-testid': 'agents' },
        h('b', {}, h('span', { class: 'dot', 'aria-hidden': 'true' }), `${connected.length} ${connected.length === 1 ? 'agent' : 'agents'} connected`),
        connected.length ? `${connected.map((a) => a.name).join(', ')}. They see this sound and can make takes here too.` : 'Connect an agent through the Praxis MCP server.',
      ),
      this.engine.kind === 'mock' ? h('p', { class: 'mock' }, 'Mock engine: placeholder tones and a keyword planner, for building the UI.') : '',
    );
  }

  private renderHead(): void {
    clear(this.head);
    const take = this.current();
    const s = this.st.score;
    const line = take && s ? `Take ${take.n} is current · ${keyLabel(s.key)} · ${round(s.tempo.bpm)} BPM · ${s.bars} bars` : 'Describe a sound to make the first take';
    this.head.append(
      h('div', {}, h('h1', { 'data-testid': 'sound-title' }, take ? this.soundName() || s?.title || 'Untitled sound' : 'New sound'), h('p', { 'data-testid': 'sound-line' }, line)),
      h(
        'div',
        { class: 'head-actions' },
        h('button', { class: 'ghost', disabled: !this.st.canUndo, 'data-testid': 'undo', title: 'Undo (Ctrl+Z)', onclick: () => void this.engine.undo() }, 'Undo'),
        h('button', { class: 'ghost redo', disabled: !this.st.canRedo, 'data-testid': 'redo', title: 'Redo (Ctrl+Shift+Z)', onclick: () => void this.engine.redo() }, 'Redo'),
        h('button', { class: 'ghost export-top', disabled: !take, onclick: () => this.openView('export') }, 'Export'),
      ),
    );
  }

  private renderThread(scrollToEnd: boolean): void {
    clear(this.thread);
    const takes = this.st.takes;
    const repliesAfter = (k: number) => this.replies.filter((r) => r.after === k).forEach((r) => this.thread.append(...replyEls(r)));
    if (!takes.length && !this.making && !this.replies.length) {
      this.thread.append(
        h('div', { class: 'empty-thread' }, h('b', {}, 'What should this sound be?'), 'Say it in your own words: a mood, a length, a key, a rhythm like "DON din don din". Or drop in a clip to start from.'),
      );
    }
    repliesAfter(0);
    takes.forEach((t, i) => {
      this.thread.append(h('div', { class: 'msg-user' }, h('small', {}, t.who), t.ask));
      this.thread.append(t.n === this.st.current ? this.takeCard(t) : this.pastRow(t));
      repliesAfter(i + 1);
    });
    if (this.making) {
      this.thread.append(
        h('div', { class: 'msg-user' }, h('small', {}, this.making.who), this.making.ask),
        h('div', { class: 'take making', 'data-testid': 'making' }, h('i', { 'aria-hidden': 'true' }), `Making take ${takes.length + 1}: planning the score, rendering, checking it…`),
      );
    }
    if (scrollToEnd) {
      const go = () => {
        this.thread.scrollTop = this.thread.scrollHeight;
        if (matchMedia(PHONE).matches) window.scrollTo(0, document.body.scrollHeight);
      };
      go();
      requestAnimationFrame(go);
    }
  }

  private takeCard(t: Take): HTMLElement {
    const s = t.score;
    const sum = summarizeChecks(t.checks);
    const by = t.who === 'You' ? 'Current' : `Current · made by ${t.who.replace('Agent · ', '')}`;
    const secs = scoreSeconds(s);
    const step = s.bars > 24 ? 4 : 1;
    const ruler = h(
      'div',
      { class: 'ruler', style: `grid-template-columns: repeat(${s.bars}, minmax(0, 1fr))`, 'aria-hidden': 'true' },
      ...Array.from({ length: s.bars }, (_, i) => h('span', { class: i % 4 === 0 ? 'major' : '' }, (i + 1) % step === 0 || i === 0 ? String(i + 1) : '')),
    );
    const items: (Node | string)[] = [];
    sum.items.forEach((it, i) => {
      if (i) items.push(' · ');
      items.push(it.passed ? it.text : h('span', { class: 'bad' }, it.text));
    });
    return h(
      'article',
      { class: 'take', 'data-take': t.n, 'data-testid': 'take-current', 'aria-label': `Take ${t.n}, current` },
      h('div', { class: 'take-head' }, h('h3', {}, `Take ${t.n} · ${t.title}`), h('span', {}, by)),
      h('p', { class: 'say' }, t.say),
      h('div', { class: 'player' }, this.playButton(t, 'play'), h('div', { class: 'wave' }, waveSvg(t, 192, 64))),
      ruler,
      h(
        'div',
        { class: 'facts' },
        h('b', {}, keyLabel(s.key)),
        h('span', {}, h('b', { class: 'num' }, String(round(s.tempo.bpm))), ' BPM'),
        h('span', {}, h('b', { class: 'num' }, String(s.bars)), ' bars'),
        h('span', {}, h('b', { class: 'num' }, clock(secs)), s.playback.mode === 'loop' ? ' loop' : ' one-shot'),
        t.render.report.mock ? h('span', { class: 'mocknote' }, 'mock preview: placeholder tones') : '',
      ),
      h('p', { class: 'checkline', 'data-testid': 'checks' }, h('span', { class: `check${sum.allPassed ? '' : ' warn'}` }, sum.label), ...items),
      t.patch ? h('p', { class: 'changed', 'data-testid': 'changed' }, h('b', {}, changedText(t)), ` ${t.untouched}`) : '',
      h(
        'div',
        { class: 'take-actions' },
        h('button', { class: 'primary', onclick: () => void this.ask('Try 3 variations of this take') }, 'Try 3 variations'),
        h('button', { onclick: () => this.openView('export') }, 'Export'),
      ),
    );
  }

  private pastRow(t: Take): HTMLElement {
    const by = t.who === 'You' ? '' : ` · by ${t.who.replace('Agent · ', '')}`;
    const label = t.n < this.st.current ? 'Go back' : 'Go to take';
    return h(
      'div',
      { class: 'past', 'data-take': t.n, 'data-testid': `take-${t.n}` },
      this.playButton(t, 'mini-play'),
      h('div', {}, h('b', {}, `Take ${t.n} · ${t.title}`), h('small', {}, `${changedText(t)}${by}`)),
      h('div', { class: 'wave mini' }, waveSvg(t, 96, 28)),
      h('button', { class: 'ghost', 'data-testid': `go-back-${t.n}`, onclick: () => void this.restore(t.n) }, label),
    );
  }

  private playButton(t: Take, cls: string): HTMLButtonElement {
    const on = this.playing?.n === t.n;
    const b = h('button', { class: cls, 'aria-label': `${on ? 'Stop' : 'Play'} take ${t.n}`, 'data-play': t.n, onclick: () => this.togglePlay(t) });
    b.innerHTML = on ? ICON_PAUSE : ICON_PLAY;
    return b;
  }

  private renderChips(): void {
    clear(this.chipRow);
    if (this.selection) {
      const locked = this.st.score?.layers.find((l) => l.id === this.selection!.layer)?.locked;
      this.chipRow.append(
        h('span', { class: 'scope-chip', 'data-testid': 'scope-chip' }, scopeText(this.st, this.selection) + (locked ? ' (locked)' : '')),
        h('button', { type: 'button', onclick: () => this.select(null) }, 'Clear'),
      );
    }
    if (this.clip) {
      this.chipRow.append(
        h('span', { class: 'clip-chip', 'data-testid': 'clip-chip' }, `Clip: ${this.clip.name}`),
        h(
          'button',
          {
            type: 'button',
            onclick: () => {
              this.clip = null;
              this.fileInput.value = '';
              this.renderChips();
              this.syncSend();
            },
          },
          'Remove',
        ),
      );
    }
  }

  private renderSuggestions(): void {
    clear(this.suggest);
    for (const c of this.chips) this.suggest.append(h('button', { type: 'button', onclick: () => void this.ask(c) }, c));
  }

  private syncSend(): void {
    this.send.disabled = !!this.making || (!this.prompt.value.trim() && !this.clip);
    this.prompt.style.height = 'auto';
    this.prompt.style.height = `${Math.min(160, this.prompt.scrollHeight)}px`;
  }

  private renderPanel(): void {
    const take = this.current();
    this.panelLine.textContent = take ? `What is playing in take ${take.n}` : 'Nothing yet';
    clear(this.tabs);
    for (const v of VIEWS) {
      this.tabs.append(
        h('button', { role: 'tab', 'aria-selected': String(v.id === this.view), 'data-testid': `tab-${v.id}`, onclick: () => this.openView(v.id) }, v.name),
      );
    }
    const scroll = this.body.scrollTop;
    clear(this.body);
    const score = this.st.score;
    if (!score || !take) {
      this.body.append(h('p', { class: 'hint', style: 'margin-top:0' }, 'The first take appears here: where each layer plays, its rhythm, notes, mix and export.'));
      return;
    }
    const ctx: ViewContext = {
      engine: this.engine,
      state: this.st,
      take,
      score,
      selection: this.selection,
      select: (s) => this.select(s),
      ask: (t) => void this.ask(t),
      edit: (p, a, sc) => void this.edit(p, a, sc),
      apply: (p) => void this.apply(p),
      openView: (v) => this.openView(v),
      knobs: this.knobs,
      note: (m) => {
        this.replies.push({ after: this.st.takes.length, who: 'You', ask: null, message: m });
        this.renderThread(true);
      },
    };
    this.body.append(renderChangeMap(ctx));
    const name = VIEWS.find((v) => v.id === this.view)!.name;
    this.body.append(
      h('p', { class: 'flash', 'data-testid': 'flash' }, ...(this.flash && this.flash.view === this.view ? [`${name} opened by `, h('b', {}, this.flash.by)] : [])),
    );
    const view = { overview: renderOverview, rhythm: renderRhythm, notes: renderNotes, mix: renderMix, export: renderExport }[this.view];
    this.body.append(view(ctx));
    this.body.scrollTop = scroll;
  }

  // ------------------------------------------------------------ playback (the take's own render)

  private togglePlay(t: Take): void {
    const was = this.playing?.n;
    this.stopPlaying();
    if (was === t.n) return;
    const audio = new Audio(t.render.audioUrl);
    audio.loop = t.render.report.is_loop;
    this.playing = { n: t.n, audio, length: t.render.report.length_samples / t.render.report.sample_rate };
    void audio.play().catch(() => undefined);
    this.refreshPlayButtons();
  }

  private stopPlaying(): void {
    if (!this.playing) return;
    this.playing.audio.pause();
    this.playing = null;
    document.querySelectorAll<SVGRectElement>('svg .ph').forEach((p) => p.setAttribute('opacity', '0'));
    this.refreshPlayButtons();
  }

  private refreshPlayButtons(): void {
    document.querySelectorAll<HTMLButtonElement>('[data-play]').forEach((b) => {
      const on = this.playing?.n === Number(b.dataset.play);
      b.innerHTML = on ? ICON_PAUSE : ICON_PLAY;
      b.setAttribute('aria-label', `${on ? 'Stop' : 'Play'} take ${b.dataset.play}`);
    });
  }

  private tick(): void {
    const p = this.playing;
    if (p) {
      if (p.audio.ended) this.stopPlaying();
      else {
        const f = (p.audio.currentTime % p.length) / p.length;
        document.querySelectorAll<SVGRectElement>(`svg[data-take="${p.n}"] .ph`).forEach((r) => {
          r.setAttribute('x', String(f * 1000));
          r.setAttribute('opacity', '1');
        });
      }
    }
    requestAnimationFrame(() => this.tick());
  }

  // ------------------------------------------------------------ input

  private wireDrop(): void {
    let depth = 0;
    window.addEventListener('dragenter', (e) => {
      if (!e.dataTransfer?.types.includes('Files')) return;
      depth++;
      this.drop.classList.add('on');
    });
    window.addEventListener('dragleave', () => {
      depth = Math.max(0, depth - 1);
      if (!depth) this.drop.classList.remove('on');
    });
    window.addEventListener('dragover', (e) => e.preventDefault());
    window.addEventListener('drop', (e) => {
      e.preventDefault();
      depth = 0;
      this.drop.classList.remove('on');
      const f = e.dataTransfer?.files?.[0];
      if (f) {
        this.clip = f;
        this.renderChips();
        this.syncSend();
      }
    });
  }

  private onKey(e: KeyboardEvent): void {
    if (isTyping(e)) return;
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === 'z') {
      e.preventDefault();
      void (e.shiftKey ? this.engine.redo() : this.engine.undo());
    } else if (mod && e.key.toLowerCase() === 'y') {
      e.preventDefault();
      void this.engine.redo();
    } else if (e.key === 'Escape') {
      if (this.panel.classList.contains('open')) this.toggleSheet(false);
      else if (this.selection) this.select(null);
    } else if (e.key === ' ' && !mod && !(e.target instanceof HTMLButtonElement)) {
      const t = this.current();
      if (t) {
        e.preventDefault();
        this.togglePlay(t);
      }
    }
  }
}

function replyEls(r: Reply): HTMLElement[] {
  const out: HTMLElement[] = [];
  if (r.ask) out.push(h('div', { class: 'msg-user' }, h('small', {}, r.who), r.ask));
  out.push(h('div', { class: 'msg-reply', 'data-testid': 'reply' }, h('small', {}, 'Praxis'), r.message));
  return out;
}

function scopeText(st: TakeState, scope: TakeScope): string {
  const { start, end } = scope.bars;
  const name = st.score ? layerLabel(st.score, scope.layer) : scope.layer;
  return `${start === end ? `Bar ${start}` : `Bars ${start}–${end}`} · ${name}`;
}

function svg(markup: string): HTMLElement {
  const span = h('span', { style: 'display:inline-flex' });
  span.innerHTML = markup;
  return span;
}

function round(n: number): number {
  return Math.round(n * 100) / 100;
}

function clock(s: number): string {
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.round(s - m * 60)).padStart(2, '0')}`;
}
