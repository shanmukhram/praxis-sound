// The wasm-bindgen glue, bundled into the worklet. A module of its own so it is evaluated after
// worklet-polyfill.ts (imports run in order).

import type { PraxisGlue } from './wasm-api';

const mods = import.meta.glob<PraxisGlue>('./pkg/praxis_wasm.js', { eager: true });

export const glue: PraxisGlue | undefined = Object.values(mods)[0];
