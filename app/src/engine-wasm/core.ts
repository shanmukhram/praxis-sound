// The offline engine over the wasm module, synchronous: planning, patches, renders, checks,
// analysis and pack files. The worker (worker.ts) runs one of these off the page's thread; tests
// run one directly in Node. Every method returns plain data (JSON parsed, channels as
// Float32Array), never a wasm handle, so results cross postMessage unchanged.

import type { PraxisGlue, WasmEngineHandle } from './wasm-api';

/** `{ok: false, message}` from the engine (and `error` for a refused patch). */
export interface Failure {
  ok: false;
  message: string;
  error?: unknown;
}

export type Result<T> = ({ ok: true } & T) | Failure;

export interface PlanPayload {
  score: unknown;
  title: string;
  summary: string;
  provider: string;
  repairs: unknown[];
  lint: unknown[];
  plan: unknown;
}

export interface EditPayload {
  changed: boolean;
  understood: boolean;
  patch: unknown;
  inverse: unknown;
  score: unknown;
  summary: string;
  title: string;
  bars: [number, number];
  refused: string[];
  new_layers: string[];
}

export interface ApplyPayload {
  score: unknown;
  inverse: unknown;
}

export interface RenderPayload {
  report: Record<string, unknown>;
  checks: Record<string, unknown>;
  /** The score as played (a synth stands in for a sampled preset whose pack is not loaded). */
  played: unknown;
  left: Float32Array;
  right: Float32Array;
  /** Wall time of the render in the worker, milliseconds. */
  ms: number;
}

export interface PackNeed {
  pack: string;
  preset: string;
  complete: boolean;
}

export interface RenderSettings {
  sample_rate?: number;
  layers?: string[];
}

function now(): number {
  return typeof performance !== 'undefined' ? performance.now() : Date.now();
}

function parse<T>(json: string): Result<T> {
  try {
    return JSON.parse(json) as Result<T>;
  } catch (e) {
    return { ok: false, message: `the engine answered with something that is not JSON: ${String(e)}` };
  }
}

/** Every operation the offline engine offers, as the worker exposes them. */
export interface CoreApi {
  version(): string;
  validate(score: unknown): Result<{ errors: unknown[]; warnings: unknown[] }>;
  planBrief(brief: string, seed: number): Result<PlanPayload>;
  composePlan(plan: unknown, seed: number): Result<PlanPayload>;
  planEdit(score: unknown, instruction: string, scope: unknown, seed: number): Result<EditPayload>;
  applyPatch(score: unknown, patch: unknown): Result<ApplyPayload>;
  render(score: unknown, settings: RenderSettings): Result<RenderPayload>;
  verify(score: unknown, left: Float32Array, right: Float32Array, sampleRate: number): Result<Record<string, unknown>>;
  analyse(left: Float32Array, right: Float32Array, sampleRate: number, fileName: string, channels: number, beatsPerBar: number): Result<{ profile: Record<string, unknown> }>;
  presets(): Result<{ presets: unknown[]; engine_only: unknown[] }>;
  packNeeds(score: unknown): Result<{ needs: PackNeed[] }>;
  missingPackFiles(pack: string, preset: string): Result<{ missing: string[] }>;
  insertPackFiles(files: [string, ArrayBuffer][]): number;
  packSize(): [number, number];
}

/** The engine in this thread. */
export class PraxisCore implements CoreApi {
  private readonly engine: WasmEngineHandle;

  constructor(private readonly glue: PraxisGlue) {
    this.engine = new glue.Engine();
  }

  version(): string {
    return this.glue.version();
  }

  validate(score: unknown): Result<{ errors: unknown[]; warnings: unknown[] }> {
    return parse(this.glue.validate_score(JSON.stringify(score)));
  }

  planBrief(brief: string, seed: number): Result<PlanPayload> {
    return parse(this.glue.plan_brief(brief, seed));
  }

  composePlan(plan: unknown, seed: number): Result<PlanPayload> {
    return parse(this.glue.compose_plan(JSON.stringify(plan), seed));
  }

  planEdit(score: unknown, instruction: string, scope: unknown, seed: number): Result<EditPayload> {
    return parse(this.glue.plan_edit(JSON.stringify(score), instruction, JSON.stringify(scope ?? null), seed));
  }

  applyPatch(score: unknown, patch: unknown): Result<ApplyPayload> {
    return parse(this.glue.apply_patch(JSON.stringify(score), JSON.stringify(patch)));
  }

  render(score: unknown, settings: RenderSettings): Result<RenderPayload> {
    const t0 = now();
    const r = this.engine.render(JSON.stringify(score), JSON.stringify(settings));
    try {
      const head = parse<Omit<RenderPayload, 'left' | 'right' | 'ms'>>(r.result());
      if (!head.ok) return head;
      const left = r.take_left();
      const right = r.take_right();
      const ms = now() - t0;
      head.report.render_seconds = ms / 1000;
      return { ...head, left, right, ms };
    } finally {
      r.free();
    }
  }

  verify(score: unknown, left: Float32Array, right: Float32Array, sampleRate: number): Result<Record<string, unknown>> {
    return parse(this.glue.verify(JSON.stringify(score), left, right, sampleRate));
  }

  analyse(left: Float32Array, right: Float32Array, sampleRate: number, fileName: string, channels: number, beatsPerBar: number): Result<{ profile: Record<string, unknown> }> {
    return parse(this.glue.analyse(left, right, sampleRate, fileName, channels, beatsPerBar));
  }

  presets(): Result<{ presets: unknown[]; engine_only: unknown[] }> {
    return parse(this.engine.presets());
  }

  packNeeds(score: unknown): Result<{ needs: PackNeed[] }> {
    return parse(this.engine.pack_needs(JSON.stringify(score)));
  }

  missingPackFiles(pack: string, preset: string): Result<{ missing: string[] }> {
    return parse(this.engine.missing_pack_files(pack, preset));
  }

  insertPackFiles(files: [string, ArrayBuffer][]): number {
    for (const [path, bytes] of files) this.engine.insert_pack_file(path, new Uint8Array(bytes));
    return files.length;
  }

  packSize(): [number, number] {
    const s = this.engine.pack_size();
    return [s[0], s[1]];
  }
}

/** The same operations, asynchronous: in a worker (the browser) or in this thread (tests). */
export type CoreClient = { [K in keyof CoreApi]: (...args: Parameters<CoreApi[K]>) => Promise<ReturnType<CoreApi[K]>> };

/** A client running the engine in this thread (Node tests; browsers without module workers). */
export function localClient(core: CoreApi): CoreClient {
  const out: Record<string, unknown> = {};
  for (const name of Object.getOwnPropertyNames(PraxisCore.prototype)) {
    if (name === 'constructor') continue;
    const fn = (core as unknown as Record<string, (...a: unknown[]) => unknown>)[name];
    out[name] = async (...args: unknown[]) => fn.apply(core, args);
  }
  return out as CoreClient;
}
