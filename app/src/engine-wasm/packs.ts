// Sample packs over HTTP. The browser has no pack folder: the page fetches the files a sampled
// preset needs from the served `packs/` folder (the same layout as the desktop's, e.g.
// packs/upright-piano-kw/pack.json) and hands them to the engine. The engine says which files
// are still missing (crates/praxis-wasm/src/library.rs), so only those are fetched: the
// manifest, then the SFZ, then its samples. A pack that is not served is reported once, in plain
// words, and its layers play a synth preset instead (the render says so too).

import type { CoreClient, PackNeed } from './core';

/** Fetches one pack file by pack-relative path; null when the server does not have it. */
export type PackFetcher = (path: string) => Promise<ArrayBuffer | null>;

/** Files under `base` (default: `packs/` next to the page). */
export function httpFetcher(base: string = new URL('packs/', document.baseURI).href): PackFetcher {
  const root = base.endsWith('/') ? base : `${base}/`;
  return async (path) => {
    const url = root + path.split('/').map(encodeURIComponent).join('/');
    try {
      const res = await fetch(url);
      if (!res.ok) return null;
      // A dev server answers unknown paths with the app's index.html: that is not a pack file.
      const type = res.headers.get('content-type') ?? '';
      if (type.includes('text/html')) return null;
      return await res.arrayBuffer();
    } catch {
      return null;
    }
  };
}

/** What happened to one pack. */
export interface PackState {
  pack: string;
  ready: boolean;
  /** Plain words when it could not be loaded. */
  message?: string;
}

/** Most fetch rounds per preset (manifest, entry, includes, samples). */
const MAX_ROUNDS = 8;
/** Files fetched at once. */
const CONCURRENCY = 8;

export class PackLoader {
  /** Every file fetched so far (the worklet gets the same files). */
  readonly files = new Map<string, ArrayBuffer>();
  private unavailable = new Map<string, string>();
  private ready = new Set<string>();
  private sent = new Set<string>();

  constructor(
    private readonly core: CoreClient,
    private readonly fetcher: PackFetcher,
  ) {}

  /** Loads the packs `score`'s sampled layers need. Resolves to each pack's state. */
  async ensure(score: unknown, onFetch?: (pack: string, files: number) => void): Promise<PackState[]> {
    const needs = await this.core.packNeeds(score);
    if (!needs.ok) return [];
    const out: PackState[] = [];
    for (const need of needs.needs) {
      out.push(await this.ensureOne(need, onFetch));
    }
    return out;
  }

  private async ensureOne(need: PackNeed, onFetch?: (pack: string, files: number) => void): Promise<PackState> {
    const key = `${need.pack}/${need.preset}`;
    if (need.complete || this.ready.has(key)) {
      this.ready.add(key);
      return { pack: need.pack, ready: true };
    }
    const known = this.unavailable.get(need.pack);
    if (known) return { pack: need.pack, ready: false, message: known };
    for (let round = 0; round < MAX_ROUNDS; round++) {
      const m = await this.core.missingPackFiles(need.pack, need.preset);
      if (!m.ok) return this.fail(need.pack, `The ${need.pack} pack on this server is not usable: ${m.message}.`);
      if (!m.missing.length) {
        this.ready.add(key);
        return { pack: need.pack, ready: true };
      }
      onFetch?.(need.pack, m.missing.length);
      const got = await fetchAll(m.missing, this.fetcher);
      const absent = m.missing.filter((_, i) => !got[i]);
      if (absent.length) {
        const what = round === 0 ? 'is not on this server' : `is incomplete on this server (${absent.length} missing, e.g. ${absent[0]})`;
        return this.fail(need.pack, `The ${need.pack} pack ${what}, so its instruments play a synth stand-in.`);
      }
      const files: [string, ArrayBuffer][] = m.missing.map((p, i) => [p, got[i]!]);
      for (const [p, b] of files) this.files.set(p, b);
      await this.core.insertPackFiles(files);
    }
    return this.fail(need.pack, `The ${need.pack} pack references more files than expected.`);
  }

  private fail(pack: string, message: string): PackState {
    this.unavailable.set(pack, message);
    return { pack, ready: false, message };
  }

  /** Files fetched since the last call (to forward to the worklet). */
  takeUnsent(): [string, ArrayBuffer][] {
    const out: [string, ArrayBuffer][] = [];
    for (const [p, b] of this.files) {
      if (this.sent.has(p)) continue;
      this.sent.add(p);
      out.push([p, b]);
    }
    return out;
  }

  /** Every file fetched (for a new worklet). */
  all(): [string, ArrayBuffer][] {
    return [...this.files.entries()];
  }
}

async function fetchAll(paths: string[], fetcher: PackFetcher): Promise<(ArrayBuffer | null)[]> {
  const out: (ArrayBuffer | null)[] = new Array(paths.length).fill(null);
  let i = 0;
  const worker = async () => {
    while (i < paths.length) {
      const k = i++;
      out[k] = await fetcher(paths[k]);
    }
  };
  await Promise.all(Array.from({ length: Math.min(CONCURRENCY, paths.length) }, worker));
  return out;
}
