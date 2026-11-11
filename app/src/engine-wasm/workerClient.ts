// The page's side of worker.ts: the offline engine's calls as promises.

import type { CoreApi, CoreClient } from './core';
import { PraxisCore } from './core';

type Answer = { id: number; result?: unknown; error?: string };

/** Starts the engine worker with an already compiled module; resolves once it answers. */
export async function workerClient(module: WebAssembly.Module): Promise<{ client: CoreClient; worker: Worker; version: string }> {
  const worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module', name: 'praxis-engine' });
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  let next = 1;
  const ready = new Promise<string>((resolve, reject) => {
    pending.set(0, { resolve: (v) => resolve(String(v)), reject });
  });
  worker.onmessage = (e: MessageEvent<Answer>) => {
    const p = pending.get(e.data.id);
    if (!p) return;
    pending.delete(e.data.id);
    if (e.data.error !== undefined) p.reject(new Error(e.data.error));
    else p.resolve(e.data.result);
  };
  worker.onerror = (e) => {
    const err = new Error(`the engine worker stopped: ${e.message || 'unknown error'}`);
    for (const p of pending.values()) p.reject(err);
    pending.clear();
  };
  worker.postMessage({ type: 'init', module });
  const version = await ready;
  const call = (method: string, args: unknown[]) =>
    new Promise<unknown>((resolve, reject) => {
      const id = next++;
      pending.set(id, { resolve, reject });
      worker.postMessage({ id, method, args });
    });
  const client: Record<string, unknown> = {};
  for (const name of Object.getOwnPropertyNames(PraxisCore.prototype) as (keyof CoreApi | 'constructor')[]) {
    if (name === 'constructor') continue;
    client[name] = (...args: unknown[]) => call(name, args);
  }
  return { client: client as CoreClient, worker, version };
}
