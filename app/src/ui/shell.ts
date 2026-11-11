// The conversation-first workstation (ADR-0011): sounds list | conversation about one sound |
// Sound panel. Every request makes a take through the EngineAdapter; the thread and the panel
// are drawn from the adapter's TakeState, so people and agents share one history.

import type { AgentInfo, AskProgress, AskResult, BlankSound, EngineAdapter, NoTake, PresetInfo, Take, TakeScope, TakeState, TransportPosition, ViewName, Who } from '../engine/EngineAdapter';
import { patchErrorMessage } from '../engine/EngineAdapter';
import { changedText, layerLabel } from '../engine/takes';
import type { BarRange, ScorePatch } from '../score/types';
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
import { defaultBlank, renderBlankForm, type BlankDraft } from './views/blankForm';
import { addToBatch, batchAsk, batchKey, batchScope, hasUnsaved, type BatchEdit } from './handEdits';
import { clock } from './views/overview';
import { applyProgress, making as newMaking, type Making } from './agentWork';
import type { ClipFile, LibraryEntry, ReferenceClip } from '../engine/EngineAdapter';
import { type AbLevels, abLevels, clipProblem, libraryMention } from '../engine/reference';
import { type AbSide, referenceChip, renderReference } from './views/reference';
import './hand.css';
import './reference.css';

export interface SoundEntry {
  id: string;
  name: string;
  sub: string;
  /** Opens this sound in the adapter (starts its conversation). */
  open(): Promise<void>;
}

/** The panel's views: the take views agents can open, plus the person's Reference view. */
type PanelView = ViewName | 'reference';

const VIEWS: { id: PanelView; name: string }[] = [
  { id: 'overview', name: 'Overview' },
  { id: 'rhythm', name: 'Rhythm' },
  { id: 'notes', name: 'Notes' },
  { id: 'mix', name: 'Mix' },
  { id: 'export', name: 'Export' },
  { id: 'reference', name: 'Reference' },
];

interface Reply {
  /** Shown after this many takes. */
  after: number;
  who: Who;
  ask: string | null;
  message: string;
  /** Set for the sound assistant's answer to a request that made no take (a question, a refusal, an undo). */
  noTake?: NoTake;
  /** The reference the request carried. */
  reference?: string;
}


const PHONE = '(max-width: 760px)';

const ICON_PLAY = '<svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"><path d="M3 1.5v11l9.5-5.5z" fill="currentColor"/></svg>';
const ICON_PAUSE =
  '<svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true"><rect x="2" y="1" width="3.5" height="12" fill="currentColor"/><rect x="8.5" y="1" width="3.5" height="12" fill="currentColor"/></svg>';
const ICON_PLAY_SMALL = '<svg width="12" height="12" viewBox="0 0 14 14" aria-hidden="true"><path d="M3 1.5v11l9.5-5.5z" fill="currentColor"/></svg>';
const ICON_STOP_SMALL = '<svg width="12" height="12" viewBox="0 0 14 14" aria-hidden="true"><rect x="2" y="2" width="10" height="10" rx="1" fill="currentColor"/></svg>';
const LOGO =
  '<svg width="26" height="26" viewBox="0 0 26 26" aria-hidden="true"><rect width="26" height="26" rx="7" fill="#f5a524"/><path d="M6 17V9M10 19V7M14 16v-6M18 18V8M22 14v-2" stroke="#111" stroke-width="2.2" stroke-linecap="round"/></svg>';
const CLIP =
  '<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M10.5 5 5.8 9.7a1.5 1.5 0 0 0 2.1 2.1l5-5a3 3 0 0 0-4.2-4.2l-5 5a4.5 4.5 0 0 0 6.4 6.4L14 9"/></svg>';

export class Shell {
  private st: TakeState;
  private view: PanelView = 'overview';
  private flash: { view: ViewName; by: string } | null = null;
  private selection: TakeScope | null = null;
  private making: Making | null = null;
  /** The reference the request being made carries (shown under it). */
  private makingRef: string | null = null;
  private replies: Reply[] = [];

  // Reference clips (WP-U6).
  /** The dropped clip (or library style) the Reference view shows and the A/B plays. */
  private reference: ReferenceClip | null = null;
  /** The reference goes with the next request (its chip shows in the composer). */
  private attached = false;
  /** The file being measured. */
  private measuring: string | null = null;
  private refError: string | null = null;
  private refNumbers = false;
  private refSaving = false;
  private libraryList: LibraryEntry[] = [];
  private libraryNote: string | null = null;
  /** The A/B side to hear, and the running comparison (the clip's <audio>, the matched levels). */
  private abSide: AbSide = 'A';
  private abRun: { audio: HTMLAudioElement; levels: AbLevels; looped: boolean } | null = null;
  private knobs: Record<string, number> = { ...MACRO_DEFAULTS };
  private chips: string[] = [];
  private agentList: AgentInfo[] = [];
  private soundId = '';
  /**
   * What is playing: the current take through the engine (`audio` null; the real-time engine on
   * the desktop), or an earlier take's own render in an <audio> element.
   */
  private playing: { n: number; audio: HTMLAudioElement | null; length: number; loop: boolean; started: boolean } | null = null;
  /** The engine's last transport position (drives the playhead while the engine plays). */
  private enginePos: TransportPosition | null = null;
  private lastTakeCount = -1;
  /** True while a sound is being opened: views opened then do not pop the phone sheet up. */
  private opening = false;

  // Building by hand (WP-U5).
  /** The "Start a blank sound" form: open, its values, and whether it is starting. */
  private blank: { draft: BlankDraft; busy: boolean; error: string | null } | null = null;
  /** Hand edits heard live since the last take, in the words the next take's request uses. */
  private batch: BatchEdit[] = [];
  /** Why the last live edit was refused (shown in the panel; cleared by the next edit). */
  private liveError: string | null = null;
  private presetList: PresetInfo[] = [];
  private viewLayer: string | null = null;
  private expanded: string | null = null;
  /** The loop range the transport repeats (null: the whole sound as it plays). */
  private loopRange: BarRange | null = null;

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
      const f = this.fileInput.files?.[0];
      this.fileInput.value = '';
      if (f) void this.takeClip(f);
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
      // The engine plays the live score: a new take, a go-back or an undo while playing is heard
      // at once, so the playhead moves to the take that is now current.
      const cur = s.takes.find((t) => t.n === s.current);
      if (this.playing && !this.playing.audio && cur) Object.assign(this.playing, { n: cur.n, ...takeLength(cur) });
      this.update();
      void engine.suggestions().then((c) => {
        this.chips = c;
        this.renderSuggestions();
      });
      // Agents connect by making takes: re-read them whenever the take state moves.
      this.refreshAgents();
    });
    engine.onPosition((p) => this.onEnginePosition(p));
    engine.onOpenView((e) => {
      this.view = e.view;
      this.flash = { view: e.view, by: e.by };
      if (matchMedia(PHONE).matches && !this.opening) this.toggleSheet(true);
      this.renderPanel();
    });
    this.refreshAgents();
    void engine
      .presets()
      .then((p) => {
        this.presetList = p;
        this.renderPanel();
      })
      .catch(() => undefined);
    this.wireDrop();
    void this.refreshLibrary();
    window.addEventListener('keydown', (e) => this.onKey(e));
    requestAnimationFrame(() => this.tick());
  }

  private refreshAgents(): void {
    void this.engine.agents().then((a) => {
      if (JSON.stringify(a) === JSON.stringify(this.agentList)) return;
      this.agentList = a;
      this.renderSide();
    });
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
    this.blank = null;
    this.batch = [];
    this.liveError = null;
    this.viewLayer = null;
    this.expanded = null;
    if (this.loopRange) {
      this.loopRange = null;
      void this.engine.setLoop(null);
    }
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
    const ref = this.attachedRef();
    if (!text && !ref) return;
    if (this.measuring && !text) return;
    this.prompt.value = '';
    this.prompt.style.height = 'auto';
    void this.ask(text || `Make something like ${ref!.name}`);
  }

  /** The reference attached to the next request, if any. */
  private attachedRef(): ReferenceClip | null {
    return this.attached ? this.reference : null;
  }

  /**
   * A request in words. It carries the attached reference, or a library style it names in quotes
   * ("make something like 'rain loop'"), so its profile reaches the planner and the assistant.
   */
  private async ask(text: string, withRef?: ReferenceClip): Promise<void> {
    if (this.making) return;
    const scope = this.selection ?? undefined;
    const label = scope ? ` (${scopeText(this.st, scope)})` : '';
    // Claim the turn before looking anything up, so a second click waits.
    const m = newMaking(text + label, 'You');
    this.making = m;
    const reference = withRef ?? this.attachedRef() ?? (await this.mentioned(text)) ?? undefined;
    this.makingRef = reference?.name ?? null;
    this.syncSend();
    this.renderThread(true);
    const r = await this.engine.ask({ text, scope, reference, onProgress: (p) => this.progress(m, p) });
    this.makingRef = null;
    this.finish(r, text + label, 'You', reference?.name);
    if (r.ok && r.take) {
      this.selection = null;
      // The reference stays in the Reference view for the A/B; the next request goes without it.
      if (reference) this.attached = false;
      this.renderChips();
      this.syncSend();
      if (this.view === 'reference') this.renderPanel();
    }
  }

  /** A library style the request names in quotes. */
  private async mentioned(text: string): Promise<ReferenceClip | null> {
    const id = libraryMention(text, this.libraryList);
    return id ? this.libraryRef(id) : null;
  }

  /** A library style as a reference, its profile fetched when the listing did not carry it. */
  private async libraryRef(id: string): Promise<ReferenceClip | null> {
    const e = this.libraryList.find((x) => x.id === id);
    if (!e) return null;
    const profile = e.profile ?? (await this.engine.libraryProfile(id));
    if (!profile) return null;
    e.profile = profile;
    return { name: e.name, profile, audioUrl: null, path: null, libraryId: e.id };
  }| null {
    const id = libraryMention(text, this.libraryList);
    const e = id ? this.libraryList.find((x) => x.id === id) : null;
    return e ? { name: e.name, profile: e.profile, audioUrl: null, path: null, libraryId: e.id } : null;
  }

  // ------------------------------------------------------------ reference clips

  /** A dropped or chosen clip: measured, shown in the Reference view, attached to the next request. */
  private async takeClip(file: ClipFile): Promise<void> {
    const problem = clipProblem(file);
    if (problem) {
      this.refError = problem;
      this.replies.push({ after: this.st.takes.length, who: 'You', ask: null, message: problem });
      this.renderThread(true);
      this.showReference();
      return;
    }
    if (this.measuring) return;
    this.stopAb();
    this.measuring = file.name;
    this.refError = null;
    this.refSaving = false;
    this.renderChips();
    this.syncSend();
    this.showReference();
    const r = await this.engine.analyzeClip(file);
    this.measuring = null;
    if (r.ok) {
      this.reference = r.clip;
      this.attached = true;
      this.abSide = 'A';
    } else {
      this.refError = r.message;
      this.replies.push({ after: this.st.takes.length, who: 'You', ask: null, message: r.message });
      this.renderThread(true);
    }
    this.renderChips();
    this.syncSend();
    this.renderPanel();
  }

  /** Opens the Reference view (the phone's sheet too). */
  private showReference(): void {
    this.view = 'reference';
    this.flash = null;
    if (matchMedia(PHONE).matches) this.toggleSheet(true);
    this.renderPanel();
  }

  private clearReference(): void {
    this.stopAb();
    this.reference = null;
    this.attached = false;
    this.refError = null;
    this.refSaving = false;
    this.renderChips();
    this.syncSend();
    this.renderPanel();
  }

  private async refreshLibrary(): Promise<void> {
    try {
      this.libraryList = await this.engine.library();
    } catch {
      this.libraryList = [];
    }
    if (this.view === 'reference') this.renderPanel();
  }

  private async saveReference(name: string, tags: string[]): Promise<void> {
    const ref = this.reference;
    if (!ref) return;
    const r = await this.engine.saveReference(ref, name, tags);
    if (!r.ok) {
      this.libraryNote = r.message;
      this.renderPanel();
      return;
    }
    this.libraryNote = `Saved "${r.entry.name}" to the library: its profile, not its audio.`;
    this.refSaving = false;
    if (this.reference === ref) this.reference = { ...ref, libraryId: r.entry.id };
    await this.refreshLibrary();
    this.renderPanel();
  }

  private async useLibrary(id: string): Promise<void> {
    const ref = await this.libraryRef(id);
    if (!ref) {
      this.libraryNote = 'That reference is no longer in the library.';
      await this.refreshLibrary();
      return;
    }
    this.stopAb();
    this.reference = ref;
    this.attached = true;
    this.refError = null;
    this.libraryNote = null;
    this.renderChips();
    this.syncSend();
    this.renderPanel();
    this.prompt.placeholder = `What should it be? It will be like "${ref.name}".`;
    this.prompt.focus();
  }

  private async removeLibrary(id: string): Promise<void> {
    const e = this.libraryList.find((x) => x.id === id);
    const ok = await this.engine.removeReference(id);
    this.libraryNote = ok ? `Removed "${e?.name ?? id}" from the library.` : 'Could not remove it from the library.';
    if (ok && this.reference?.libraryId === id) this.reference = this.reference.audioUrl ? { ...this.reference, libraryId: null } : this.reference;
    await this.refreshLibrary();
    this.renderPanel();
  }

  private async tagLibrary(id: string, tags: string[]): Promise<void> {
    const r = await this.engine.tagReference(id, tags);
    this.libraryNote = r.ok ? `Tags of "${r.entry.name}": ${r.entry.tags.join(', ') || 'none'}.` : r.message;
    await this.refreshLibrary();
  }

  /** The take's loudness for the A/B: the check report's, else the render's. */
  private abLevelsFor(take: Take | null): AbLevels | null {
    const ref = this.reference;
    if (!take || !ref) return null;
    const takeLufs = take.checks.integrated_lufs ?? take.render.report.loudness_lufs;
    return abLevels(takeLufs, ref.profile.loudness.integrated_lufs);
  }

  /**
   * Plays the loudness-matched A/B: the current take through the engine (trimmed when it is the
   * louder one) and the clip's own file (turned down when it is), together from the start, both
   * looping; only the chosen side is heard.
   */
  private playAb(): void {
    const ref = this.reference;
    const take = this.current();
    if (!ref?.audioUrl || !take) return;
    this.stopPlaying();
    const levels = this.abLevelsFor(take)!;
    const audio = new Audio(ref.audioUrl);
    audio.loop = true;
    audio.volume = levels.clipVolume;
    audio.muted = this.abSide === 'A';
    const looped = take.score.playback.mode !== 'loop' && !this.loopRange;
    this.abRun = { audio, levels, looped };
    void this.engine.setCompare({ gainDb: levels.takeDb, muted: this.abSide === 'B' });
    // A one-shot take repeats while comparing, as the clip does.
    if (looped) void this.engine.setLoop({ start: 1, end: take.score.bars });
    void this.engine.seek(0);
    this.playing = { n: take.n, audio: null, ...takeLength(take), loop: true, started: false };
    void this.engine.play();
    void audio.play().catch(() => undefined);
    this.refreshPlayButtons();
    this.renderPanel();
  }

  private setAbSide(side: AbSide): void {
    this.abSide = side;
    const run = this.abRun;
    if (run) {
      run.audio.muted = side === 'A';
      void this.engine.setCompare({ gainDb: run.levels.takeDb, muted: side === 'B' });
    }
    this.renderPanel();
  }

  /** Ends the comparison: the clip stops and the take plays at its own level again. */
  private stopAb(): void {
    const run = this.abRun;
    if (!run) return;
    this.abRun = null;
    run.audio.pause();
    run.audio.removeAttribute('src');
    void this.engine.setCompare(null);
    if (run.looped) void this.engine.setLoop(this.loopRange);
  }

  /** The assistant's next step, shown under the request while it works. */
  private progress(m: Making, p: AskProgress): void {
    if (this.making !== m) return;
    applyProgress(m, p);
    this.renderThread(true);
  }

  private async edit(patch: ScorePatch, ask: string, scope?: TakeScope): Promise<void> {
    if (this.making) return;
    this.making = newMaking(ask, 'You');
    this.renderThread(true);
    const r = await this.engine.edit(patch, ask, scope);
    this.finish(r, ask, 'You');
  }

  private finish(r: AskResult, ask: string, who: Who, reference?: string): void {
    this.making = null;
    if (!r.ok) this.replies.push({ after: this.st.takes.length, who, ask, message: r.message, reference });
    // A reply without a take is the assistant's answer, shown like any reply (never as an error).
    else if (!r.take) this.replies.push({ after: this.st.takes.length, who, ask, message: r.reply, noTake: r.noTake, reference });
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

  // ------------------------------------------------------------ building by hand

  /** A hand edit heard at once: applied to the live score and added to the batch. */
  private async live(patch: ScorePatch, label: string, at?: TakeScope, group?: { id: string; words: string }): Promise<void> {
    const r = await this.engine.applyPatch(patch);
    if (!r.ok) {
      this.liveError = `Could not do that: ${patchErrorMessage(r.error)}.`;
      this.renderPanel();
      return;
    }
    this.liveError = null;
    this.batch = addToBatch(this.batch, { label, layer: at?.layer, bars: at?.bars, key: batchKey(patch), group });
    // The adapter's state event has already redrawn; draw again for the batch line.
    this.renderPanel();
    this.renderThread(false);
  }

  /** "Make a take": renders, checks and records the live score (the batch) as one take. */
  private async makeTake(): Promise<void> {
    if (this.making) return;
    const first = !this.current();
    const ask = this.batch.length ? batchAsk(this.batch) : first ? 'Built by hand' : 'Hand edits';
    this.making = newMaking(ask, 'You');
    this.syncSend();
    this.renderThread(true);
    this.renderPanel();
    const r = await this.engine.makeTake(ask, { scope: batchScope(this.batch) });
    if (r.ok) this.batch = [];
    this.finish(r, ask, 'You');
  }

  private openBlank(): void {
    this.blank = { draft: this.blank?.draft ?? defaultBlank(), busy: false, error: null };
    this.renderThread(true);
  }

  private async startBlank(b: BlankSound): Promise<void> {
    if (!this.blank) return;
    this.blank.busy = true;
    this.blank.error = null;
    this.renderThread(false);
    const r = await this.engine.startBlank(b);
    if (!r.ok) {
      if (this.blank) {
        this.blank.busy = false;
        this.blank.error = r.message;
      }
      this.renderThread(false);
      return;
    }
    // The mock starts the sound in place; the desktop gets a new folder and reopens it.
    this.blank = null;
    this.batch = [];
    this.view = 'overview';
    this.update();
    if (matchMedia(PHONE).matches) this.toggleSheet(true);
  }

  private hasUnsaved(): boolean {
    return hasUnsaved(this.current(), this.st.score, !!this.st.draft);
  }

  /** The panel's batch line: what is heard but not in a take yet, and "Make a take". */
  private pendingBar(): HTMLElement | null {
    const score = this.st.score;
    if (!score) return null;
    const first = !this.current();
    if (this.liveError) return h('div', { class: 'pending bad', 'data-testid': 'live-error', role: 'alert' }, h('p', {}, this.liveError));
    if (first && !score.layers.length) return null;
    if (!this.hasUnsaved()) return null;
    const n = this.batch.length;
    const what = first
      ? 'Not in a take yet. Make the first take when it sounds right.'
      : `${n ? `${n} ${n === 1 ? 'change' : 'changes'}` : 'Changes'} heard live, not in a take yet.`;
    const last = this.batch.at(-1)?.label;
    return h(
      'div',
      { class: 'pending', 'data-testid': 'pending' },
      h('p', {}, h('b', {}, what), last && !first ? h('small', {}, `Last: ${last}`) : null),
      h('button', { class: 'primary', 'data-testid': 'make-take', disabled: !!this.making, onclick: () => void this.makeTake() }, first ? 'Make the first take' : 'Make a take'),
    );
  }

  /** The thread's card for a blank sound before its first take. */
  private draftCard(): HTMLElement {
    const s = this.st.score!;
    const secs = scoreSeconds(s);
    const on = this.playing?.n === 0;
    const play = h('button', { class: 'play', 'aria-label': on ? 'Stop' : 'Play the sound as it is', 'data-play': 0, onclick: () => this.togglePlayLive() });
    play.innerHTML = on ? ICON_PAUSE : ICON_PLAY;
    const layers = s.layers.length;
    return h(
      'article',
      { class: 'take draft', 'data-testid': 'draft-card', 'aria-label': 'New sound, built by hand' },
      h('div', { class: 'take-head' }, h('h3', {}, `${s.title || 'Untitled sound'} · built by hand`), h('span', {}, 'No take yet')),
      h(
        'p',
        { class: 'say' },
        layers
          ? `${layers} ${layers === 1 ? 'layer' : 'layers'} so far. Write notes in Rhythm or Notes; every change is heard at once. Make the first take when it sounds right.`
          : 'Add a layer in the Sound panel, write its notes in Rhythm or Notes, then make the first take.',
      ),
      h('div', { class: 'player draft-player' }, play, h('span', { class: 'hint', style: 'margin:0' }, layers ? 'Plays the sound as it is now, before any take.' : 'Nothing to play until a layer has notes.')),
      h(
        'div',
        { class: 'facts' },
        h('b', {}, keyLabel(s.key)),
        h('span', {}, h('b', { class: 'num' }, String(round(s.tempo.bpm))), ' BPM'),
        h('span', {}, h('b', { class: 'num' }, String(s.bars)), ' bars'),
        h('span', {}, h('b', { class: 'num' }, clock(secs)), s.playback.mode === 'loop' ? ' loop' : ' one-shot'),
      ),
      h(
        'div',
        { class: 'take-actions' },
        layers ? h('button', { class: 'primary', 'data-testid': 'draft-make', disabled: !!this.making, onclick: () => void this.makeTake() }, 'Make the first take') : null,
        h('button', { class: 'sheet-open', 'data-testid': 'draft-panel', onclick: () => this.openView('overview') }, layers ? 'Open the Sound panel' : 'Add a layer'),
      ),
    );
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

  private openView(view: PanelView): void {
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
    const line =
      take && s
        ? `Take ${take.n} is current · ${keyLabel(s.key)} · ${round(s.tempo.bpm)} BPM · ${s.bars} bars`
        : this.st.draft && s
          ? `Built by hand, no take yet · ${keyLabel(s.key)} · ${round(s.tempo.bpm)} BPM · ${s.bars} bars`
          : 'Describe a sound to make the first take';
    this.head.append(
      h('div', {}, h('h1', { 'data-testid': 'sound-title' }, take || this.st.draft ? this.soundName() || s?.title || 'Untitled sound' : 'New sound'), h('p', { 'data-testid': 'sound-line' }, line)),
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
    const draft = !!this.st.draft && !!this.st.score;
    if (!takes.length && !this.making && !this.replies.length && !draft && !this.blank) {
      this.thread.append(
        h(
          'div',
          { class: 'empty-thread' },
          h('b', {}, 'What should this sound be?'),
          'Say it in your own words: a mood, a length, a key, a rhythm like "DON din don din". Or drop in a clip to start from.',
          h('span', { class: 'or' }, 'Or build it yourself, layer by layer.'),
          h('button', { class: 'ghost start-blank', 'data-testid': 'start-blank', onclick: () => this.openBlank() }, 'Start a blank sound'),
        ),
      );
    }
    if (!takes.length && this.blank && !draft) {
      this.thread.append(
        renderBlankForm(this.blank.draft, {
          busy: this.blank.busy,
          error: this.blank.error,
          start: (b) => void this.startBlank(b),
          cancel: () => {
            this.blank = null;
            this.renderThread(false);
          },
        }),
      );
    }
    if (draft && !takes.length) this.thread.append(this.draftCard());
    repliesAfter(0);
    takes.forEach((t, i) => {
      this.thread.append(userMsg(t.who, t.ask, t.reference));
      this.thread.append(t.n === this.st.current ? this.takeCard(t) : this.pastRow(t));
      repliesAfter(i + 1);
    });
    if (takes.length && !this.making && this.hasUnsaved()) {
      const n = this.batch.length;
      this.thread.append(
        h(
          'div',
          { class: 'pending-row', 'data-testid': 'pending-thread' },
          h('span', {}, `${n ? `${n} hand ${n === 1 ? 'edit' : 'edits'}` : 'Hand edits'} since take ${this.st.current}, heard live.`),
          h('button', { class: 'ghost', onclick: () => void this.makeTake() }, 'Make a take'),
        ),
      );
    }
    if (this.making) {
      const m = this.making;
      this.thread.append(userMsg(m.who, m.ask, this.makingRef));
      if (m.steps.length || m.said) {
        // The sound assistant at work: its steps in plain words, replaced by the take or reply.
        this.thread.append(
          h(
            'div',
            { class: 'take making agent-work', 'data-testid': 'making' },
            h('div', { class: 'agent-now' }, h('i', { 'aria-hidden': 'true' }), 'Working on it…'),
            m.said ? h('p', { class: 'agent-said', 'data-testid': 'agent-said' }, m.said) : null,
            h('ol', { class: 'agent-steps', 'data-testid': 'agent-steps' }, ...m.steps.map((s) => h('li', { class: s.state }, s.words))),
          ),
        );
      } else {
        this.thread.append(
          h('div', { class: 'take making', 'data-testid': 'making' }, h('i', { 'aria-hidden': 'true' }), `Making take ${takes.length + 1}: ${m.ask.startsWith('By hand') || m.ask === 'Built by hand' || m.ask === 'Hand edits' ? 'rendering the live score, checking it' : 'planning the score, rendering, checking it'}…`),
        );
      }
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
      h('div', { class: 'player' }, this.playButton(t, 'play'), this.seekableWave(t)),
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

  /** The current take's waveform: a click plays from that point (seeks the engine). */
  private seekableWave(t: Take): HTMLElement {
    const wave = h('div', { class: 'wave seekable', 'data-testid': 'wave-current', title: 'Click to play from here' }, waveSvg(t, 192, 64));
    wave.addEventListener('click', (e: MouseEvent) => {
      const r = wave.getBoundingClientRect();
      const f = Math.max(0, Math.min(1, (e.clientX - r.left) / Math.max(1, r.width)));
      this.seek(f * takeLength(t).length);
    });
    return wave;
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
    if (this.measuring) {
      this.chipRow.append(h('span', { class: 'clip-chip', 'data-testid': 'clip-chip' }, `Measuring ${this.measuring}…`));
    } else if (this.attached && this.reference) {
      this.chipRow.append(
        h('button', { type: 'button', class: 'ref-chip', 'data-testid': 'clip-chip', title: 'Show its profile', onclick: () => this.showReference() }, referenceChip(this.reference)),
        h(
          'button',
          {
            type: 'button',
            'data-testid': 'clip-detach',
            onclick: () => {
              this.attached = false;
              this.renderChips();
              this.syncSend();
              if (this.view === 'reference') this.renderPanel();
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
    this.send.disabled = !!this.making || (!this.prompt.value.trim() && (!this.attachedRef() || !!this.measuring));
    this.prompt.style.height = 'auto';
    this.prompt.style.height = `${Math.min(160, this.prompt.scrollHeight)}px`;
  }

  private renderPanel(): void {
    const take = this.current();
    this.panelLine.textContent = take ? `What is playing in take ${take.n}${this.hasUnsaved() ? ', with your changes' : ''}` : this.st.draft ? 'Built by hand, no take yet' : 'Nothing yet';
    clear(this.tabs);
    for (const v of VIEWS) {
      this.tabs.append(
        h('button', { role: 'tab', 'aria-selected': String(v.id === this.view), 'data-testid': `tab-${v.id}`, onclick: () => this.openView(v.id) }, v.name),
      );
    }
    const scroll = this.body.scrollTop;
    clear(this.body);
    const score = this.st.score;
    if (this.view === 'reference') {
      // Works before any take: a clip can be the start of a sound.
      this.body.append(this.referenceView(take));
      this.body.scrollTop = scroll;
      return;
    }
    if (!score || (!take && !this.st.draft)) {
      this.body.append(h('p', { class: 'hint', style: 'margin-top:0' }, 'The first take appears here: where each layer plays, its rhythm, notes, mix and export. Or start a blank sound and build it here by hand.'));
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
      live: (p, label, at, group) => void this.live(p, label, at, group),
      presets: this.presetList,
      viewLayer: this.viewLayer,
      setViewLayer: (id) => {
        this.viewLayer = id;
        this.renderPanel();
      },
      expanded: this.expanded,
      setExpanded: (id) => {
        this.expanded = id;
        this.renderPanel();
      },
      transport: { playing: !!this.playing && !this.playing.audio, seconds: this.enginePos?.seconds ?? 0, loop: this.loopRange },
      seek: (sec) => this.seek(sec),
      setLoop: (r) => this.setLoop(r),
      togglePlay: () => this.togglePlayLive(),
    };
    this.body.append(renderChangeMap(ctx));
    const pending = this.pendingBar();
    if (pending) this.body.append(pending);
    const name = VIEWS.find((v) => v.id === this.view)!.name;
    this.body.append(
      h('p', { class: 'flash', 'data-testid': 'flash' }, ...(this.flash && this.flash.view === this.view ? [`${name} opened by `, h('b', {}, this.flash.by)] : [])),
    );
    const view = { overview: renderOverview, rhythm: renderRhythm, notes: renderNotes, mix: renderMix, export: renderExport }[this.view];
    this.body.append(view(ctx));
    this.body.scrollTop = scroll;
  }

  private referenceView(take: Take | null): HTMLElement {
    return renderReference(
      {
        reference: this.reference,
        measuring: this.measuring,
        error: this.refError,
        attached: this.attached,
        take,
        ab: { playing: !!this.abRun, side: this.abSide, levels: this.abRun?.levels ?? this.abLevelsFor(take) },
        library: this.libraryList,
        libraryNote: this.libraryNote,
        numbers: this.refNumbers,
        saving: this.refSaving,
      },
      {
        chooseClip: () => this.fileInput.click(),
        clear: () => this.clearReference(),
        likeThis: () => {
          const ref = this.reference;
          if (!ref) return;
          this.attached = true;
          void this.ask(`Make something like ${ref.name}`, ref);
        },
        toggleNumbers: () => {
          this.refNumbers = !this.refNumbers;
          this.renderPanel();
        },
        playAb: () => this.playAb(),
        stopAb: () => this.stopPlaying(),
        side: (s) => this.setAbSide(s),
        openSave: (open) => {
          this.refSaving = open;
          this.libraryNote = null;
          this.renderPanel();
        },
        save: (name, tags) => void this.saveReference(name, tags),
        use: (id) => void this.useLibrary(id),
        remove: (id) => void this.removeLibrary(id),
        tag: (id, tags) => void this.tagLibrary(id, tags),
      },
    );
  }

  // ------------------------------------------------------------ playback (the take's own render)

  private togglePlay(t: Take): void {
    const was = this.playing?.n;
    this.stopPlaying();
    if (was === t.n) return;
    if (t.n === this.st.current) {
      // The current take plays through the engine: live mute / solo and new takes are heard.
      this.playing = { n: t.n, audio: null, ...takeLength(t), started: false };
      void this.engine.play();
    } else {
      const audio = new Audio(t.render.audioUrl);
      audio.loop = t.render.report.is_loop;
      this.playing = { n: t.n, audio, ...takeLength(t), started: true };
      void audio.play().catch(() => undefined);
    }
    this.refreshPlayButtons();
  }

  /** Plays or stops the live score: the current take through the engine, or a draft. */
  private togglePlayLive(): void {
    const t = this.current();
    if (t) return this.togglePlay(t);
    if (!this.st.draft || !this.st.score) return;
    const was = this.playing?.n;
    this.stopPlaying();
    if (was === 0) return;
    const s = this.st.score;
    this.playing = { n: 0, audio: null, length: Math.max(1e-3, scoreSeconds(s)), loop: s.playback.mode === 'loop', started: false };
    void this.engine.play();
    this.refreshPlayButtons();
    this.renderPanel();
  }

  private seek(seconds: number): void {
    void this.engine.seek(Math.max(0, seconds));
    if (this.playing?.audio) this.playing.audio.currentTime = Math.max(0, seconds);
  }

  private setLoop(range: BarRange | null): void {
    this.loopRange = range;
    void this.engine.setLoop(range);
    // Start the loop where it begins, so it is heard at once.
    if (range && this.st.score) this.seek((range.start - 1) * (scoreSeconds(this.st.score) / this.st.score.bars));
    this.renderPanel();
  }

  private stopPlaying(): void {
    const ab = !!this.abRun;
    this.stopAb();
    if (this.playing) {
      if (this.playing.audio) this.playing.audio.pause();
      else void this.engine.stop();
      this.clearPlaying();
    }
    if (ab) this.renderPanel();
  }

  private clearPlaying(): void {
    if (this.abRun) {
      // The engine stopped (the sound closed, the device went away): the comparison ends too.
      this.stopAb();
      if (this.view === 'reference') this.renderPanel();
    }
    this.playing = null;
    document.querySelectorAll<SVGRectElement>('svg .ph').forEach((p) => p.setAttribute('opacity', '0'));
    this.refreshPlayButtons();
    this.renderTransport();
  }

  /** The engine stopped by itself (a one-shot ended, the sound closed): the buttons follow. */
  private onEnginePosition(p: TransportPosition): void {
    this.enginePos = p;
    const pl = this.playing;
    if (!pl || pl.audio) return;
    if (p.playing && !pl.started) {
      pl.started = true;
      this.renderTransport();
    } else if (!p.playing && pl.started) this.clearPlaying();
  }

  /** The change map's play button follows the transport without redrawing the panel. */
  private renderTransport(): void {
    const b = this.body.querySelector<HTMLButtonElement>('[data-testid=transport-play]');
    if (!b) return;
    const on = !!this.playing && !this.playing.audio;
    b.innerHTML = on ? ICON_STOP_SMALL : ICON_PLAY_SMALL;
    b.setAttribute('aria-label', on ? 'Stop (Space)' : 'Play (Space)');
    b.title = on ? 'Stop (Space)' : 'Play (Space)';
  }

  private refreshPlayButtons(): void {
    document.querySelectorAll<HTMLButtonElement>('[data-play]').forEach((b) => {
      const n = Number(b.dataset.play);
      const on = this.playing?.n === n;
      b.innerHTML = on ? ICON_PAUSE : ICON_PLAY;
      b.setAttribute('aria-label', n === 0 ? (on ? 'Stop' : 'Play the sound as it is') : `${on ? 'Stop' : 'Play'} take ${n}`);
    });
    this.renderTransport();
  }

  private tick(): void {
    const p = this.playing;
    const at = p ? (p.audio ? p.audio.currentTime : (this.enginePos?.seconds ?? 0)) : 0;
    if (p) {
      if (p.audio?.ended) this.stopPlaying();
      else {
        const f = p.loop ? (at % p.length) / p.length : Math.min(at, p.length) / p.length;
        document.querySelectorAll<SVGRectElement>(`svg[data-take="${p.n}"] .ph`).forEach((r) => {
          r.setAttribute('x', String(f * 1000));
          r.setAttribute('opacity', '1');
        });
      }
    }
    this.tickMap(p && !p.audio ? at : null);
    requestAnimationFrame(() => this.tick());
  }

  /** The change map's playhead and time while the engine plays the live score. */
  private tickMap(at: number | null): void {
    const map = this.body.querySelector<HTMLElement>('[data-testid=change-map]');
    const ph = map?.querySelector<HTMLElement>('.map-ph');
    const s = this.st.score;
    if (!map || !ph || !s) return;
    const total = scoreSeconds(s);
    const time = map.querySelector<HTMLElement>('[data-testid=transport-time]');
    const shown = at ?? this.enginePos?.seconds ?? 0;
    if (time) time.textContent = `${clock(s.playback.mode === 'loop' && total > 0 ? shown % total : Math.min(shown, total))} / ${clock(total)}`;
    const lanes = map.querySelectorAll<HTMLElement>('.lane:not(.loop-lane) .cells');
    if (at === null || !lanes.length || total <= 0) {
      ph.style.opacity = '0';
      return;
    }
    const first = lanes[0].getBoundingClientRect();
    const last = lanes[lanes.length - 1].getBoundingClientRect();
    const box = map.getBoundingClientRect();
    const f = s.playback.mode === 'loop' ? (at % total) / total : Math.min(at, total) / total;
    ph.style.left = `${first.left - box.left + f * first.width}px`;
    ph.style.top = `${first.top - box.top - 2}px`;
    ph.style.height = `${last.bottom - first.top + 4}px`;
    ph.style.opacity = '1';
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
      if (f) void this.takeClip(f);
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
    } else if (e.key === ' ' && !mod && !e.repeat) {
      // Space plays and stops, unless a control reached by keyboard wants it (a focused button
      // keeps Space for itself when the focus ring shows, so keyboard users can still press it).
      const t = e.target as HTMLElement | null;
      if (t instanceof HTMLButtonElement && t.matches(':focus-visible')) return;
      if (!this.current() && !this.st.draft) return;
      e.preventDefault();
      this.togglePlayLive();
    } else if (e.key === 'Home' && !mod) {
      e.preventDefault();
      this.seek(this.loopRange && this.st.score ? (this.loopRange.start - 1) * (scoreSeconds(this.st.score) / this.st.score.bars) : 0);
    } else if ((e.key === 'l' || e.key === 'L') && !mod) {
      if (this.selection) this.setLoop({ ...this.selection.bars });
      else if (this.loopRange) this.setLoop(null);
    }
  }
}

/** A request as the thread shows it: who, the words, and the reference it carried. */
function userMsg(who: string, ask: string, reference?: string | null): HTMLElement {
  return h('div', { class: 'msg-user' }, h('small', {}, who), ask, reference ? h('span', { class: 'ref-tag', 'data-testid': 'ref-tag' }, `Reference: ${reference}`) : null);
}

function replyEls(r: Reply): HTMLElement[] {
  const out: HTMLElement[] = [];
  if (r.ask) out.push(userMsg(r.who, r.ask, r.reference));
  const attrs: Record<string, string> = { class: r.noTake ? 'msg-reply assistant' : 'msg-reply', 'data-testid': 'reply' };
  if (r.noTake) attrs['data-no-take'] = r.noTake;
  out.push(h('div', attrs, h('small', {}, r.noTake ? (r.noTake === 'live' ? 'Praxis' : 'Assistant') : 'Praxis'), r.message));
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

/** A take's musical length in seconds and whether it loops (the playhead's range). */
function takeLength(t: Take): { length: number; loop: boolean } {
  const r = t.render.report;
  return { length: Math.max(1e-3, r.length_samples / (r.sample_rate || 48000)), loop: r.is_loop };
}
