// The AudioWorklet processor: crates/praxis-wasm's real-time engine (`Realtime`, the same
// praxis_engine::RealtimeEngine the desktop plays through cpal) renders every block.
//
// - The page passes the compiled wasm module in `processorOptions.module` (or its bytes in
//   `processorOptions.bytes` where a module cannot be cloned) and talks over the port: pack files,
//   `load` (prepare a score and swap it in), transport and layer commands.
// - `process` asks the engine for the block's frames and copies them out of the engine's own
//   buffers through views on the wasm memory: no allocation per block on either side.
// - Every STATUS_EVERY blocks it posts the engine status (position, playing, peaks, RMS).

import './worklet-polyfill';
import { glue } from './worklet-glue';
import { STATUS_LEN } from './wasm-api';
import type { WasmRealtimeHandle } from './wasm-api';

/** Blocks between two status messages (8 x 128 frames: about 21 ms at 48 kHz). */
const STATUS_EVERY = 8;

interface WorkletScope {
  AudioWorkletProcessor: new () => { readonly port: MessagePort };
  registerProcessor(name: string, ctor: unknown): void;
  sampleRate: number;
}

const scope = globalThis as unknown as WorkletScope;

export type WorkletMessage =
  | { type: 'packs'; id: number; files: [string, ArrayBuffer][] }
  | { type: 'load'; id: number; score: string; gain: number }
  | { type: 'play' | 'stop' }
  | { type: 'seek'; position: number }
  | { type: 'loop'; start: number; end: number }
  | { type: 'mute' | 'solo'; layer: string; on: boolean }
  | { type: 'gain'; layer: string; db: number }
  | { type: 'master'; db: number };

export type WorkletEvent =
  | { type: 'ready'; version: string }
  | { type: 'error'; message: string }
  | { type: 'reply'; id: number; result: unknown }
  | { type: 'status'; s: number[] };

class PraxisProcessor extends scope.AudioWorkletProcessor {
  private rt: WasmRealtimeHandle | null = null;
  private memory: WebAssembly.Memory | null = null;
  private buffer: ArrayBufferLike | null = null;
  private left = new Float32Array(0);
  private right = new Float32Array(0);
  private status = new Float64Array(0);
  private blocks = 0;

  constructor(options?: { processorOptions?: { module?: WebAssembly.Module; bytes?: ArrayBuffer } }) {
    super();
    this.port.onmessage = (e: MessageEvent<WorkletMessage>) => this.onMessage(e.data);
    try {
      if (!glue) throw new Error('the wasm engine is not in this build');
      const input = options?.processorOptions?.module ?? options?.processorOptions?.bytes;
      if (!input) throw new Error('no wasm module was passed to the worklet');
      const out = glue.initSync({ module: input });
      this.memory = out.memory;
      this.rt = new glue.Realtime(scope.sampleRate);
      this.post({ type: 'ready', version: glue.version() });
    } catch (err) {
      this.post({ type: 'error', message: err instanceof Error ? err.message : String(err) });
    }
  }

  private post(e: WorkletEvent): void {
    this.port.postMessage(e);
  }

  private onMessage(m: WorkletMessage): void {
    const rt = this.rt;
    if (!rt) {
      if ('id' in m) this.post({ type: 'reply', id: m.id, result: { ok: false, message: 'the engine did not start in the worklet' } });
      return;
    }
    switch (m.type) {
      case 'packs':
        for (const [path, bytes] of m.files) rt.insert_pack_file(path, new Uint8Array(bytes));
        this.post({ type: 'reply', id: m.id, result: m.files.length });
        break;
      case 'load':
        this.post({ type: 'reply', id: m.id, result: rt.load(m.score, m.gain) });
        break;
      case 'play':
        rt.play();
        break;
      case 'stop':
        rt.stop();
        break;
      case 'seek':
        rt.seek(m.position);
        break;
      case 'loop':
        rt.set_loop(m.start, m.end);
        break;
      case 'mute':
        rt.set_layer_mute(m.layer, m.on);
        break;
      case 'solo':
        rt.set_layer_solo(m.layer, m.on);
        break;
      case 'gain':
        rt.set_layer_gain(m.layer, m.db);
        break;
      case 'master':
        rt.set_master_gain(m.db);
        break;
    }
  }

  /** (Re)makes the views when the wasm memory grew (a grown memory has a new buffer). */
  private views(rt: WasmRealtimeHandle, memory: WebAssembly.Memory): void {
    if (this.buffer === memory.buffer) return;
    this.buffer = memory.buffer;
    const max = 4096;
    this.left = new Float32Array(memory.buffer, rt.left_ptr(), max);
    this.right = new Float32Array(memory.buffer, rt.right_ptr(), max);
    this.status = new Float64Array(memory.buffer, rt.status_ptr(), STATUS_LEN);
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const rt = this.rt;
    const memory = this.memory;
    const out = outputs[0];
    if (!rt || !memory || !out || !out.length) return true;
    const frames = out[0].length;
    const n = rt.render(frames);
    this.views(rt, memory);
    out[0].set(this.left.subarray(0, n));
    if (out.length > 1) out[1].set(this.right.subarray(0, n));
    if (++this.blocks % STATUS_EVERY === 0) {
      this.post({ type: 'status', s: Array.from(this.status) });
    }
    return true;
  }
}

scope.registerProcessor('praxis-engine', PraxisProcessor);
