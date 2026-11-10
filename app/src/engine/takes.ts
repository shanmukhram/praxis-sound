// Pure helpers for takes, shared by every adapter: what a patch changed (per layer, in bars), and
// the plain-language lines a take card shows. The change map and the waveform highlight read
// `Take.changed` only, so they always agree with the patch that made the take.

import type { NScore, PatchOp, ScorePatch } from '../score/types';
import type { Take } from './EngineAdapter';

type Changed = Record<string, [number, number]>;

function extent(score: NScore, layerId: string): [number, number] | null {
  const l = score.layers.find((x) => x.id === layerId);
  if (!l || !l.clips.length) return null;
  return [Math.min(...l.clips.map((c) => c.bars.start)), Math.max(...l.clips.map((c) => c.bars.end))];
}

function merge(out: Changed, layer: string, r: [number, number] | null): void {
  if (!r) return;
  const cur = out[layer];
  out[layer] = cur ? [Math.min(cur[0], r[0]), Math.max(cur[1], r[1])] : [r[0], r[1]];
}

const SOUND_FIELDS = ['class', 'instrument', 'muted', 'solo', 'gain_db', 'pan', 'effects', 'automation', 'seed_offset'];

/**
 * Bars each layer was changed in by `patch` (applied to `before`, giving `after`). Bar ops count
 * their range; layer-wide changes count the layer's whole extent; global ops count every layer.
 */
export function changedFromPatch(before: NScore, after: NScore, patch: ScorePatch): Changed {
  const out: Changed = {};
  const all = () => after.layers.forEach((l) => merge(out, l.id, extent(after, l.id)));
  for (const op of patch.ops as PatchOp[]) {
    switch (op.op) {
      case 'write_bars':
      case 'clear_bars':
        merge(out, op.layer, [op.bars.start, op.bars.end]);
        break;
      case 'place_clip':
        merge(out, op.layer, [op.clip.bars.start, op.clip.bars.end]);
        break;
      case 'set_clips': {
        const b = patch.scope?.bars;
        merge(out, op.layer, b ? [b.start, b.end] : extent(after, op.layer) ?? extent(before, op.layer));
        break;
      }
      case 'replace_pattern':
        for (const l of after.layers) {
          for (const c of l.clips) {
            if (c.pattern !== op.pattern.id) continue;
            // The clip's bars, limited to the patch scope when there is one.
            const b = patch.scope?.bars;
            merge(out, l.id, b ? [Math.max(b.start, c.bars.start), Math.min(b.end, c.bars.end)] : [c.bars.start, c.bars.end]);
          }
        }
        break;
      case 'update_layer':
        if (Object.entries(op.update).some(([k, v]) => v !== null && v !== undefined && SOUND_FIELDS.includes(k))) {
          merge(out, op.id, extent(after, op.id));
        }
        break;
      case 'add_layer':
        merge(out, op.layer.id, extent(after, op.layer.id));
        break;
      case 'remove_layer':
        merge(out, op.id, extent(before, op.id));
        break;
      case 'set_title':
      case 'set_meta':
      case 'set_lock':
      case 'add_pattern':
      case 'remove_pattern':
      case 'add_section':
      case 'update_section':
      case 'remove_section':
      case 'add_marker':
      case 'remove_marker':
      case 'set_chords':
        break;
      default:
        // set_tempo, set_seed, set_key, set_bars, set_master, ...: every bar of every layer.
        all();
    }
  }
  return out;
}

export function layerLabel(score: NScore, id: string): string {
  return score.layers.find((l) => l.id === id)?.name || id;
}

function dash(a: number, b: number): string {
  return a === b ? `${a}` : `${a}–${b}`;
}

/** "Changed felt piano in bars 13–16." (or "A new sound."). */
export function changedText(take: Pick<Take, 'patch' | 'changed' | 'score'>): string {
  if (!take.patch) return 'A new sound.';
  const parts = Object.entries(take.changed).map(([l, [a, b]]) => `${layerLabel(take.score, l).toLowerCase()} in ${a === b ? 'bar' : 'bars'} ${dash(a, b)}`);
  return parts.length ? `Changed ${parts.join(', ')}.` : 'No audible change.';
}

/**
 * What stayed the same, in words: the bars outside the change (sample-identical when the render
 * comparison proved it, else note for note) and each unchanged layer.
 */
export function untouchedText(prev: NScore, next: NScore, changed: Changed, prevN: number, sampleIdentical: boolean): string {
  const ids = Object.keys(changed);
  const out: string[] = [];
  if (ids.length) {
    const a = Math.min(...ids.map((l) => changed[l][0]));
    const b = Math.max(...ids.map((l) => changed[l][1]));
    const spans: string[] = [];
    if (a > 1) spans.push(dash(1, a - 1));
    if (b < next.bars) spans.push(dash(b + 1, next.bars));
    if (spans.length) {
      const many = spans.length > 1 || spans[0].includes('–');
      out.push(`${many ? 'Bars' : 'Bar'} ${spans.join(' and ')} ${sampleIdentical ? 'sample-identical' : 'identical note for note'} to take ${prevN}.`);
    }
  }
  for (const l of next.layers) {
    if (!changed[l.id] && prev.layers.some((p) => p.id === l.id)) out.push(`${l.name || l.id} unchanged.`);
  }
  return out.join(' ');
}
