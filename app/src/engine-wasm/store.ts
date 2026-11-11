// Takes kept in the browser: each sound's history (takes, the current take, the live score) and
// every take's rendered audio, in IndexedDB, so a reload or a later visit opens the same sounds.
// The reference library (named, tagged Sound Profiles, never audio) lives here too.
// Audio is stored as 32-bit float WAV, so a reopened take compares bit-exactly with the next one.
// Where IndexedDB is missing (private modes, tests) the store lives in memory for the session.

import type { NScore } from '../score/types';
import type { LibraryEntry, Take } from '../engine/EngineAdapter';

/** A take without its audio URL (URLs do not outlive the page). */
export type StoredTake = Omit<Take, 'render'> & { render: { report: Take['render']['report'] } };

export interface StoredSound {
  id: string;
  name: string;
  /** "A minor · 0:30 loop". */
  sub: string;
  /** Milliseconds since the epoch, for ordering. */
  updated: number;
  takes: StoredTake[];
  current: number;
  score: NScore | null;
  draft: boolean;
}

export type SoundSummary = Pick<StoredSound, 'id' | 'name' | 'sub' | 'updated'>;

export interface SoundStore {
  readonly persistent: boolean;
  list(): Promise<SoundSummary[]>;
  load(id: string): Promise<StoredSound | null>;
  save(sound: StoredSound): Promise<void>;
  saveAudio(id: string, n: number, wav: ArrayBuffer): Promise<void>;
  loadAudio(id: string, n: number): Promise<ArrayBuffer | null>;
  /** Reference library entries with their profiles, newest first. */
  references(): Promise<LibraryEntry[]>;
  saveReferenceEntry(entry: LibraryEntry & { added: number }): Promise<void>;
  removeReferenceEntry(id: string): Promise<void>;
}

function summary(s: StoredSound): SoundSummary {
  return { id: s.id, name: s.name, sub: s.sub, updated: s.updated };
}

export class MemoryStore implements SoundStore {
  readonly persistent = false;
  private sounds = new Map<string, StoredSound>();
  private audio = new Map<string, ArrayBuffer>();
  private refs = new Map<string, LibraryEntry & { added: number }>();

  async list(): Promise<SoundSummary[]> {
    return [...this.sounds.values()].map(summary).sort((a, b) => b.updated - a.updated);
  }

  async load(id: string): Promise<StoredSound | null> {
    const s = this.sounds.get(id);
    return s ? structuredClone(s) : null;
  }

  async save(sound: StoredSound): Promise<void> {
    this.sounds.set(sound.id, structuredClone(sound));
  }

  async saveAudio(id: string, n: number, wav: ArrayBuffer): Promise<void> {
    this.audio.set(`${id}/${n}`, wav);
  }

  async loadAudio(id: string, n: number): Promise<ArrayBuffer | null> {
    return this.audio.get(`${id}/${n}`) ?? null;
  }

  async references(): Promise<LibraryEntry[]> {
    return byNewest([...this.refs.values()].map((e) => structuredClone(e)));
  }

  async saveReferenceEntry(entry: LibraryEntry & { added: number }): Promise<void> {
    this.refs.set(entry.id, structuredClone(entry));
  }

  async removeReferenceEntry(id: string): Promise<void> {
    this.refs.delete(id);
  }
}

function byNewest(list: (LibraryEntry & { added?: number })[]): LibraryEntry[] {
  return list.sort((a, b) => (b.added ?? 0) - (a.added ?? 0)).map(({ added: _added, ...e }) => e);
}

const DB = 'praxis-sound';
const SOUNDS = 'sounds';
const AUDIO = 'audio';
const REFERENCES = 'references';

function req<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error ?? new Error('IndexedDB request failed'));
  });
}

export class IdbStore implements SoundStore {
  readonly persistent = true;
  private constructor(private readonly db: IDBDatabase) {}

  static async open(name = DB): Promise<IdbStore> {
    const open = indexedDB.open(name, 2);
    open.onupgradeneeded = () => {
      const db = open.result;
      if (!db.objectStoreNames.contains(SOUNDS)) db.createObjectStore(SOUNDS, { keyPath: 'id' });
      if (!db.objectStoreNames.contains(AUDIO)) db.createObjectStore(AUDIO);
      if (!db.objectStoreNames.contains(REFERENCES)) db.createObjectStore(REFERENCES, { keyPath: 'id' });
    };
    return new IdbStore(await req(open));
  }

  private store(name: string, mode: IDBTransactionMode): IDBObjectStore {
    return this.db.transaction(name, mode).objectStore(name);
  }

  async list(): Promise<SoundSummary[]> {
    const all = (await req(this.store(SOUNDS, 'readonly').getAll())) as StoredSound[];
    return all.map(summary).sort((a, b) => b.updated - a.updated);
  }

  async load(id: string): Promise<StoredSound | null> {
    return ((await req(this.store(SOUNDS, 'readonly').get(id))) as StoredSound | undefined) ?? null;
  }

  async save(sound: StoredSound): Promise<void> {
    await req(this.store(SOUNDS, 'readwrite').put(sound));
  }

  async saveAudio(id: string, n: number, wav: ArrayBuffer): Promise<void> {
    await req(this.store(AUDIO, 'readwrite').put(wav, `${id}/${n}`));
  }

  async loadAudio(id: string, n: number): Promise<ArrayBuffer | null> {
    return ((await req(this.store(AUDIO, 'readonly').get(`${id}/${n}`))) as ArrayBuffer | undefined) ?? null;
  }

  async references(): Promise<LibraryEntry[]> {
    return byNewest((await req(this.store(REFERENCES, 'readonly').getAll())) as (LibraryEntry & { added: number })[]);
  }

  async saveReferenceEntry(entry: LibraryEntry & { added: number }): Promise<void> {
    await req(this.store(REFERENCES, 'readwrite').put(entry));
  }

  async removeReferenceEntry(id: string): Promise<void> {
    await req(this.store(REFERENCES, 'readwrite').delete(id));
  }
}

/** IndexedDB when the browser has it, else memory. */
export async function openStore(): Promise<SoundStore> {
  if (typeof indexedDB === 'undefined') return new MemoryStore();
  try {
    return await IdbStore.open();
  } catch {
    return new MemoryStore();
  }
}
