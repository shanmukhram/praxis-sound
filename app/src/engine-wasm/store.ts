// Takes kept in the browser: each sound's history (takes, the current take, the live score) and
// every take's rendered audio, in IndexedDB, so a reload or a later visit opens the same sounds.
// Audio is stored as 32-bit float WAV, so a reopened take compares bit-exactly with the next one.
// Where IndexedDB is missing (private modes, tests) the store lives in memory for the session.

import type { NScore } from '../score/types';
import type { Take } from '../engine/EngineAdapter';

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
}

function summary(s: StoredSound): SoundSummary {
  return { id: s.id, name: s.name, sub: s.sub, updated: s.updated };
}

export class MemoryStore implements SoundStore {
  readonly persistent = false;
  private sounds = new Map<string, StoredSound>();
  private audio = new Map<string, ArrayBuffer>();

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
}

const DB = 'praxis-sound';
const SOUNDS = 'sounds';
const AUDIO = 'audio';

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
    const open = indexedDB.open(name, 1);
    open.onupgradeneeded = () => {
      const db = open.result;
      if (!db.objectStoreNames.contains(SOUNDS)) db.createObjectStore(SOUNDS, { keyPath: 'id' });
      if (!db.objectStoreNames.contains(AUDIO)) db.createObjectStore(AUDIO);
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
