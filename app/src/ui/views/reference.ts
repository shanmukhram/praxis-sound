// The Reference view of the Sound panel (FR-11, FR-12, FR-19, FR-30): a dropped clip's Sound
// Profile in plain words with small visuals, a loudness-matched A/B against the current take, and
// the reference library (profiles only, never audio). It works before the first take, so a clip
// can be the start of a sound.

import type { LibraryEntry, ReferenceClip, Take } from '../../engine/EngineAdapter';
import { type AbLevels, kindWord, lengthWords, profileLines } from '../../engine/reference';
import { h } from '../dom';
import { bandsViz, envelopeSvg, grooveViz } from '../profileViz';

export type AbSide = 'A' | 'B';

export interface ReferenceViewState {
  reference: ReferenceClip | null;
  /** The file being measured right now. */
  measuring: string | null;
  /** Why the last clip could not be measured. */
  error: string | null;
  /** The reference goes with the next request. */
  attached: boolean;
  take: Take | null;
  ab: { playing: boolean; side: AbSide; levels: AbLevels | null };
  library: LibraryEntry[];
  libraryNote: string | null;
  numbers: boolean;
  saving: boolean;
}

export interface ReferenceActions {
  chooseClip(): void;
  clear(): void;
  likeThis(): void;
  toggleNumbers(): void;
  playAb(): void;
  stopAb(): void;
  side(s: AbSide): void;
  openSave(open: boolean): void;
  save(name: string, tags: string[]): void;
  use(id: string): void;
  remove(id: string): void;
  tag(id: string, tags: string[]): void;
}

/** "rain, loop , Night" -> ["rain", "loop", "night"]. */
export function parseTags(text: string): string[] {
  return [...new Set(text.split(/[,;]/).map((t) => t.trim().toLowerCase()).filter(Boolean))];
}

/** The library name a clip starts with: its file name without the extension, in words. */
export function defaultRefName(fileName: string): string {
  return fileName.replace(/\.[a-z0-9]+$/i, '').replace(/[_-]+/g, ' ').replace(/\s+/g, ' ').trim() || 'Reference';
}

export function renderReference(st: ReferenceViewState, act: ReferenceActions): HTMLElement {
  const box = h('div', { class: 'refview', 'data-testid': 'reference-view' });
  box.append(clipSection(st, act));
  if (st.reference) box.append(abSection(st, act));
  box.append(librarySection(st, act));
  return box;
}

function clipSection(st: ReferenceViewState, act: ReferenceActions): HTMLElement {
  const ref = st.reference;
  if (st.measuring) {
    return h('div', { class: 'ref-card measuring', 'data-testid': 'ref-measuring', role: 'status' }, h('i', { 'aria-hidden': 'true' }), `Measuring ${st.measuring}: tempo, key, loudness, brightness, space and groove…`);
  }
  const error = st.error ? h('p', { class: 'ref-error', role: 'alert', 'data-testid': 'ref-error' }, st.error) : null;
  if (!ref) {
    return h(
      'div',
      { class: 'ref-card ref-empty', 'data-testid': 'ref-empty' },
      h('b', {}, 'Start from a clip'),
      h('p', {}, 'Drop an audio or video clip anywhere on the window, or choose one. Praxis measures it: what kind of sound it is, tempo and key, loudness, brightness, space and groove. It keeps the character, never the melody.'),
      error,
      h('button', { class: 'ghost', 'data-testid': 'ref-choose', onclick: () => act.chooseClip() }, 'Choose a clip'),
    );
  }
  const p = ref.profile;
  const lines = profileLines(p);
  const list = h('dl', { class: 'ref-lines', 'data-testid': 'profile-lines' });
  for (const l of lines) {
    if (l.id === 'kind') continue;
    const viz = l.id === 'brightness' ? bandsViz(p) : l.id === 'groove' ? grooveViz(p) : null;
    list.append(
      h('dt', {}, l.label),
      h(
        'dd',
        { class: l.cantTell ? 'cant' : '', 'data-line': l.id },
        h('span', {}, l.words),
        viz,
        st.numbers ? h('small', { class: 'ref-num' }, l.detail) : null,
      ),
    );
  }
  const kind = lines[0];
  const source = ref.audioUrl ? 'Dropped clip' : 'From your library';
  return h(
    'div',
    { class: 'ref-card', 'data-testid': 'profile-card' },
    h(
      'div',
      { class: 'ref-head' },
      h('div', {}, h('h3', { 'data-testid': 'ref-name' }, ref.name), h('small', {}, `${source} · ${lengthWords(p.source.duration_s)}${p.mock ? ' · mock measurements' : ''}`)),
      h('button', { class: 'linkish', 'data-testid': 'ref-clear', onclick: () => act.clear() }, 'Remove'),
    ),
    h('p', { class: 'ref-kind', 'data-testid': 'ref-kind' }, h('b', {}, kind.words), st.numbers ? h('small', { class: 'ref-num' }, kind.detail) : null),
    envelopeSvg(p),
    h('div', { class: 'env-legend', 'aria-hidden': 'true' }, h('span', {}, 'Loudness over time'), h('span', {}, `${p.onsets_s.length} ${p.onsets_s.length === 1 ? 'onset' : 'onsets'} below`)),
    list,
    h('button', { class: 'linkish', 'data-testid': 'ref-numbers', 'aria-pressed': String(st.numbers), onclick: () => act.toggleNumbers() }, st.numbers ? 'Hide the numbers' : 'Show the numbers'),
    error,
    h(
      'div',
      { class: 'take-actions' },
      h('button', { class: 'primary', 'data-testid': 'ref-like', onclick: () => act.likeThis() }, 'Make something like this'),
      ref.libraryId ? null : h('button', { 'data-testid': 'ref-save-open', onclick: () => act.openSave(!st.saving) }, 'Save to library'),
    ),
    st.saving && !ref.libraryId ? saveForm(ref, act) : null,
    h('p', { class: 'hint' }, st.attached ? 'Goes with your next request: its profile in words, never its audio.' : 'Attach it to a request with "Make something like this".'),
  );
}

function saveForm(ref: ReferenceClip, act: ReferenceActions): HTMLElement {
  const name = h('input', { type: 'text', value: defaultRefName(ref.name), 'data-testid': 'ref-save-name', 'aria-label': 'Name in the library', maxlength: 80 });
  const tags = h('input', { type: 'text', value: '', placeholder: 'rain, night, loop', 'data-testid': 'ref-save-tags', 'aria-label': 'Tags, separated by commas', maxlength: 200 });
  const form = h(
    'form',
    { class: 'ref-save', 'data-testid': 'ref-save' },
    h('label', {}, h('span', {}, 'Name'), name),
    h('label', {}, h('span', {}, 'Tags'), tags),
    h('div', { class: 'row' }, h('button', { type: 'submit', class: 'primary', 'data-testid': 'ref-save-go' }, 'Save'), h('button', { type: 'button', class: 'ghost', onclick: () => act.openSave(false) }, 'Cancel')),
    h('p', { class: 'hint' }, 'The library keeps the measurements only, never the audio.'),
  );
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    act.save(name.value, parseTags(tags.value));
  });
  return form;
}

function abSection(st: ReferenceViewState, act: ReferenceActions): HTMLElement {
  const ref = st.reference!;
  const take = st.take;
  const head = h('div', { class: 'section-h' }, h('h3', {}, 'Compare by ear'), h('span', {}, 'Loudness-matched A/B'));
  if (!ref.audioUrl) {
    return h('div', { class: 'ref-ab', 'data-testid': 'ab' }, head, h('p', { class: 'hint', style: 'margin-top:0' }, 'A library style keeps no audio, so there is nothing to compare by ear. Drop the clip again to hear it against a take.'));
  }
  if (!take) {
    return h('div', { class: 'ref-ab', 'data-testid': 'ab' }, head, h('p', { class: 'hint', style: 'margin-top:0' }, 'Make a take, then compare it with the clip here, at matched loudness.'));
  }
  const seg = (s: AbSide, label: string) =>
    h('button', { role: 'radio', 'aria-checked': String(st.ab.side === s), 'data-testid': `ab-${s.toLowerCase()}`, onclick: () => act.side(s) }, h('b', {}, s), ' ', label);
  const play = h(
    'button',
    { class: st.ab.playing ? 'ab-play on' : 'ab-play', 'data-testid': 'ab-play', 'aria-pressed': String(st.ab.playing), onclick: () => (st.ab.playing ? act.stopAb() : act.playAb()) },
    st.ab.playing ? 'Stop' : 'Play A/B',
  );
  return h(
    'div',
    { class: 'ref-ab', 'data-testid': 'ab' },
    head,
    h('div', { class: 'ab-row' }, h('div', { class: 'ab-seg', role: 'radiogroup', 'aria-label': 'What you hear' }, seg('A', `Take ${take.n}`), seg('B', ref.name)), play),
    h('p', { class: 'hint', 'data-testid': 'ab-levels' }, st.ab.levels?.words ?? ''),
    h('p', { class: 'hint' }, 'Both play together from the start; switching A and B swaps which one you hear. B is only heard here, never in a render or an export.'),
  );
}

function librarySection(st: ReferenceViewState, act: ReferenceActions): HTMLElement {
  const box = h('div', { class: 'ref-lib', 'data-testid': 'library' }, h('div', { class: 'section-h' }, h('h3', {}, 'Reference library'), h('span', {}, `${st.library.length} saved`)));
  if (st.libraryNote) box.append(h('p', { class: 'hint', 'data-testid': 'library-note', style: 'margin-top:0' }, st.libraryNote));
  if (!st.library.length) {
    box.append(h('p', { class: 'hint', style: 'margin-top:0' }, 'Saved references appear here as named styles: profiles only, never audio. Use one later with "Make something like \'its name\'".'));
    return box;
  }
  const list = h('ul', { class: 'lib-list' });
  for (const e of st.library) {
    const tagInput = h('input', { type: 'text', value: e.tags.join(', '), class: 'lib-tags-input', 'aria-label': `Tags of ${e.name}`, 'data-testid': `lib-tags-${e.id}` });
    tagInput.addEventListener('keydown', (ev: KeyboardEvent) => {
      if (ev.key === 'Enter') {
        ev.preventDefault();
        act.tag(e.id, parseTags(tagInput.value));
      }
    });
    tagInput.addEventListener('change', () => act.tag(e.id, parseTags(tagInput.value)));
    const using = st.reference?.libraryId === e.id;
    list.append(
      h(
        'li',
        { 'data-testid': `lib-${e.id}`, class: using ? 'using' : '' },
        h('div', { class: 'lib-main' }, h('b', {}, e.name), h('small', {}, e.summary), tagInput),
        h(
          'div',
          { class: 'lib-actions' },
          h('button', { class: 'ghost', 'data-testid': `lib-use-${e.id}`, disabled: using, onclick: () => act.use(e.id) }, using ? 'In use' : 'Use'),
          h('button', { class: 'linkish', 'data-testid': `lib-remove-${e.id}`, 'aria-label': `Remove ${e.name} from the library`, onclick: () => act.remove(e.id) }, 'Remove'),
        ),
      ),
    );
  }
  box.append(list);
  return box;
}

/** The composer chip's words for an attached reference. */
export function referenceChip(ref: ReferenceClip): string {
  return `Reference: ${ref.name} · ${kindWord(ref.profile.class.kind)}, ${lengthWords(ref.profile.source.duration_s)}`;
}

