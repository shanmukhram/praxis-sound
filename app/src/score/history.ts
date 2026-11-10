// Local undo/redo and named versions, mirroring the project contract (docs/contracts/patch.md):
// commit pushes {seq, patch, inverse} and clears redo; undo applies the top inverse and moves the
// entry to redo; redo reverses that; restoring a version is itself an undoable patch.

import type { ApplyResult, EngineAdapter } from '../engine/EngineAdapter';
import type { HistoryEntry, NScore, Project, ScorePatch, Version } from './types';
import { clone } from './types';
import { restorePatch } from './patchBuilder';

/** The part of the adapter history needs (so tests can pass a plain function). */
export type Applier = Pick<EngineAdapter, 'applyPatch'>;

export class History {
  undo: HistoryEntry[] = [];
  redo: HistoryEntry[] = [];
  nextSeq = 1;
  versions: Version[] = [];
  /** The score after the last successful operation (set by reset() on load). */
  current: NScore | null = null;

  /** Operations run one at a time, so a fast second undo never reads a stale stack top. */
  private chain: Promise<unknown> = Promise.resolve();

  constructor(private readonly engine: Applier) {}

  private exclusive<T>(fn: () => Promise<T>): Promise<T> {
    const run = this.chain.then(fn, fn);
    this.chain = run.catch(() => undefined);
    return run;
  }

  get canUndo(): boolean {
    return this.undo.length > 0;
  }

  get canRedo(): boolean {
    return this.redo.length > 0;
  }

  commit(patch: ScorePatch): Promise<ApplyResult> {
    return this.exclusive(() => this.commitNow(patch));
  }

  private async commitNow(patch: ScorePatch): Promise<ApplyResult> {
    const r = await this.engine.applyPatch(patch);
    if (r.ok) {
      this.current = r.score;
      this.undo.push({ seq: this.nextSeq++, patch: clone(patch), inverse: r.inverse });
      this.redo = [];
    }
    return r;
  }

  /** Undoes the newest edit; the result carries the entry that was undone. */
  undoLast(): Promise<(ApplyResult & { entry: HistoryEntry }) | null> {
    return this.exclusive(() => this.undoNow());
  }

  private async undoNow(): Promise<(ApplyResult & { entry: HistoryEntry }) | null> {
    const entry = this.undo.at(-1);
    if (!entry) return null;
    const r = await this.engine.applyPatch(entry.inverse);
    if (r.ok) {
      this.current = r.score;
      this.undo.pop();
      // The inverse of the inverse re-applies the edit; keep the original patch for display.
      this.redo.push({ seq: entry.seq, patch: entry.patch, inverse: entry.inverse });
    }
    return { ...r, entry };
  }

  /** Re-applies the newest undone edit; the result carries its entry. */
  redoLast(): Promise<(ApplyResult & { entry: HistoryEntry }) | null> {
    return this.exclusive(() => this.redoNow());
  }

  private async redoNow(): Promise<(ApplyResult & { entry: HistoryEntry }) | null> {
    const entry = this.redo.at(-1);
    if (!entry) return null;
    const r = await this.engine.applyPatch(entry.patch);
    if (r.ok) {
      this.current = r.score;
      this.redo.pop();
      this.undo.push({ seq: entry.seq, patch: entry.patch, inverse: r.inverse });
    }
    return { ...r, entry };
  }

  saveVersion(name: string, score: NScore): Version {
    const v: Version = { name, seq: this.nextSeq - 1, score: clone(score) };
    const i = this.versions.findIndex((x) => x.name === name);
    if (i >= 0) this.versions[i] = v;
    else this.versions.push(v);
    return v;
  }

  /** Restores a named version through a patch (so it can be undone), from the latest score. */
  restoreVersion(name: string): Promise<ApplyResult | null> {
    return this.exclusive(async () => {
      const v = this.versions.find((x) => x.name === name);
      const current = this.current;
      if (!v || !current) return null;
      const patch = restorePatch(current, v.score, name);
      if (patch.ops.length === 0) return { ok: true, score: clone(current), inverse: { ...patch, ops: [] } };
      return this.commitNow(patch);
    });
  }

  reset(score: NScore | null = null): void {
    this.current = score ? clone(score) : null;
    this.undo = [];
    this.redo = [];
    this.nextSeq = 1;
    this.versions = [];
  }

  toProject(score: NScore): Project {
    return {
      format: 'praxis-project',
      version: 1,
      score: clone(score),
      undo: clone(this.undo),
      redo: clone(this.redo),
      next_seq: this.nextSeq,
      versions: clone(this.versions),
      references: [],
    };
  }
}
