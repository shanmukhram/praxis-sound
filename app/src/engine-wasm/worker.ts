// The offline engine's thread: planning, patches, renders and checks run here so the page stays
// responsive while a take renders. The page sends the compiled wasm module first ({type:
// 'init'}), then calls ({id, method, args}); each answer is {id, result} or {id, error}.
// Rendered channels are transferred, not copied.

import { loadGlue } from './glue';
import { PraxisCore } from './core';

type Call = { id: number; method: string; args: unknown[] };
type Init = { type: 'init'; module: WebAssembly.Module };

let core: PraxisCore | null = null;
const scope = self as unknown as {
  onmessage: ((e: MessageEvent) => void) | null;
  postMessage(msg: unknown, transfer?: Transferable[]): void;
};

function transferables(v: unknown): Transferable[] {
  const out: Transferable[] = [];
  if (v && typeof v === 'object') {
    for (const x of Object.values(v as Record<string, unknown>)) {
      if (x instanceof Float32Array && x.buffer instanceof ArrayBuffer) out.push(x.buffer);
    }
  }
  return out;
}

scope.onmessage = async (e: MessageEvent) => {
  const msg = e.data as Init | Call;
  if ('type' in msg && msg.type === 'init') {
    try {
      const glue = await loadGlue();
      glue.initSync({ module: msg.module });
      core = new PraxisCore(glue);
      scope.postMessage({ id: 0, result: core.version() });
    } catch (err) {
      scope.postMessage({ id: 0, error: err instanceof Error ? err.message : String(err) });
    }
    return;
  }
  const call = msg as Call;
  try {
    if (!core) throw new Error('the engine is not started');
    const fn = (core as unknown as Record<string, (...a: unknown[]) => unknown>)[call.method];
    if (typeof fn !== 'function') throw new Error(`no engine method \`${call.method}\``);
    const result = fn.apply(core, call.args);
    scope.postMessage({ id: call.id, result }, transferables(result));
  } catch (err) {
    scope.postMessage({ id: call.id, error: err instanceof Error ? err.message : String(err) });
  }
};
