// The instrument pack downloader (desktop only): lists the packs in packs/index.json, installs
// one through `packs_install` (the same checks as `praxis packs install`: exact size and blake3
// before anything is unpacked) and shows its progress from `praxis://packs` events. Opened from
// the sounds list ("Instruments") and offered once at start when the felt piano is missing.

import type { Bridge } from '../engine/tauri/bridge';
import { isCmdError } from '../engine/tauri/bridge';
import { clear, h } from './dom';
import './packs.css';

/** One pack as `packs_list` returns it (crates/praxis-cli/src/ops/packs.rs). */
export interface PackInfo {
  name: string;
  title: string;
  version: string;
  license: string;
  size: number;
  presets: string[];
  attribution: string;
  source_page: string;
  installed: boolean;
  installed_version: string | null;
}

/** `praxis://packs` (crates/praxis-app/src/packs.rs `PackProgress`). */
export interface PackProgress {
  pack: string;
  phase: 'downloading' | 'unpacking' | 'done' | 'failed';
  bytes: number;
  total: number;
  message?: string;
}

export const PACKS_EVENT = 'praxis://packs';
/** Window event that opens the dialog (the sounds list's "Instruments" button sends it). */
export const OPEN_PACKS = 'praxis:packs';

export function megabytes(bytes: number): string {
  return `${(bytes / 1e6).toFixed(1)} MB`;
}

/** The status words for a pack row, given its install progress if any. */
export function packStatus(p: PackInfo, progress?: PackProgress): { text: string; busy: boolean; failed: boolean } {
  if (progress) {
    switch (progress.phase) {
      case 'downloading': {
        const pct = progress.total ? Math.min(100, Math.floor((progress.bytes / progress.total) * 100)) : 0;
        return { text: `Downloading ${megabytes(progress.bytes)} of ${megabytes(progress.total)} (${pct}%)`, busy: true, failed: false };
      }
      case 'unpacking':
        return { text: 'Checking the download and unpacking', busy: true, failed: false };
      case 'failed':
        return { text: `Not installed: ${progress.message ?? 'the install failed'}`, busy: false, failed: true };
      case 'done':
        break;
    }
  }
  if (p.installed) return { text: `Installed (version ${p.installed_version ?? p.version})`, busy: false, failed: false };
  return { text: `Not installed · ${megabytes(p.size)} download`, busy: false, failed: false };
}

/** True when a preset the planner reaches for first (the felt piano) needs a pack that is missing. */
export function feltPianoMissing(packs: PackInfo[]): boolean {
  const p = packs.find((x) => x.presets.includes('felt_piano'));
  return !!p && !p.installed;
}

function errorText(e: unknown): string {
  if (isCmdError(e)) return e.message;
  return e instanceof Error ? e.message : String(e);
}

/** Adds the Instruments dialog (and the start-up offer when the felt piano is missing). */
export function mountPacks(root: HTMLElement, bridge: Bridge): () => void {
  const list = h('div', { class: 'packs-list', 'data-testid': 'packs-list' });
  const note = h('p', { class: 'packs-note' });
  const dialog = h(
    'dialog',
    { class: 'packs', 'aria-labelledby': 'packs-title', 'data-testid': 'packs-dialog' },
    h('div', { class: 'packs-head' }, h('h2', { id: 'packs-title' }, 'Instruments'), h('button', { class: 'ghost', onclick: () => dialog.close() }, 'Close')),
    h(
      'p',
      { class: 'packs-intro' },
      'Sampled instruments come as free packs. Each download is checked against its published size and blake3 fingerprint before anything is unpacked.',
    ),
    list,
    note,
  );
  root.append(dialog);
  let packs: PackInfo[] = [];
  const progress = new Map<string, PackProgress>();

  const render = () => {
    clear(list);
    if (!packs.length) {
      list.append(h('p', { class: 'packs-empty' }, 'No packs are listed here.'));
      return;
    }
    for (const p of packs) {
      const pr = progress.get(p.name);
      const st = packStatus(p, pr);
      const pct = pr && pr.phase === 'downloading' && pr.total ? (pr.bytes / pr.total) * 100 : pr?.phase === 'unpacking' ? 100 : 0;
      list.append(
        h(
          'div',
          { class: 'pack', 'data-testid': `pack-${p.name}` },
          h('div', {}, h('b', {}, p.title), h('small', {}, `${p.license} · ${p.attribution}`), h('small', {}, `Plays: ${p.presets.join(', ')}`)),
          h('p', { class: st.failed ? 'pack-status bad' : 'pack-status', role: 'status', 'data-testid': `pack-status-${p.name}` }, st.text),
          st.busy ? h('div', { class: 'pack-bar', 'aria-hidden': 'true' }, h('span', { style: `width:${pct.toFixed(1)}%` })) : null,
          p.installed && !st.busy
            ? null
            : h('button', { class: 'primary', disabled: st.busy, 'data-testid': `pack-install-${p.name}`, onclick: () => void install(p.name) }, st.failed ? 'Try again' : 'Install'),
        ),
      );
    }
  };

  const load = async (): Promise<void> => {
    try {
      const v = await bridge.invoke<{ root: string; packs: PackInfo[] }>('packs_list');
      packs = v.packs;
      note.textContent = `Packs folder: ${v.root}`;
    } catch (e) {
      packs = [];
      note.textContent = errorText(e);
    }
    render();
  };

  const install = async (name: string): Promise<void> => {
    progress.set(name, { pack: name, phase: 'downloading', bytes: 0, total: packs.find((p) => p.name === name)?.size ?? 0 });
    render();
    try {
      await bridge.invoke('packs_install', { name });
      progress.delete(name);
      note.textContent = 'Installed. New takes use it at once; restart Praxis Sound to hear it while you edit live.';
    } catch (e) {
      progress.set(name, { pack: name, phase: 'failed', bytes: 0, total: 0, message: errorText(e) });
    }
    await load();
    offer.remove();
  };

  const open = () => {
    void load();
    if (!dialog.open) dialog.showModal();
  };
  window.addEventListener(OPEN_PACKS, open);

  const offer = h(
    'div',
    { class: 'packs-offer', role: 'status', 'data-testid': 'packs-offer' },
    h('span', {}, 'The felt piano is not installed yet, so piano parts cannot be rendered.'),
    h('button', { class: 'primary', onclick: open }, 'Install instruments'),
    h('button', { class: 'ghost', onclick: () => offer.remove() }, 'Not now'),
  );
  let off: (() => void) | undefined;
  void bridge
    .listen<PackProgress>(PACKS_EVENT, (p) => {
      if (p.phase === 'done') progress.delete(p.pack);
      else if (progress.get(p.pack)?.phase !== 'failed') progress.set(p.pack, p);
      render();
    })
    .then((u) => (off = u));
  void bridge
    .invoke<{ packs: PackInfo[] }>('packs_list')
    .then((v) => {
      if (feltPianoMissing(v.packs)) root.append(offer);
    })
    .catch(() => undefined);

  return () => {
    window.removeEventListener(OPEN_PACKS, open);
    off?.();
    dialog.remove();
    offer.remove();
  };
}
