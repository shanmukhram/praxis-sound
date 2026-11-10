// The change map's model ("Where each layer plays"): a lane per layer over the bars, the sections
// above, and the bars the take changed. Pure, so it can be tested without a DOM. The changed
// cells come from `take.changed` only.

import type { Take, TakeScope } from '../engine/EngineAdapter';
import type { NScore } from '../score/types';
import { changedText } from '../engine/takes';

export interface MapCell {
  bar: number;
  /** The layer plays in this bar (a clip covers it). */
  on: boolean;
  /** The take changed this bar of this layer. */
  chg: boolean;
  /** Inside the scope the person dragged. */
  sel: boolean;
}

export interface MapLane {
  layer: string;
  name: string;
  locked: boolean;
  cells: MapCell[];
}

export interface ChangeMap {
  bars: number;
  sections: { name: string; from: number; to: number }[];
  lanes: MapLane[];
  /** True when the take is an edit (a new sound lights nothing). */
  isEdit: boolean;
  legend: string;
}

/**
 * Lanes come from `score` (the live score: the current take plus any mute/solo/lock since);
 * highlights come from `take.changed`.
 */
export function changeMapModel(take: Take | null, score: NScore, selection: TakeScope | null): ChangeMap {
  const isEdit = !!take?.patch;
  const lanes = score.layers.map((l) => {
    const ch = isEdit ? take!.changed[l.id] : undefined;
    const cells: MapCell[] = [];
    for (let bar = 1; bar <= score.bars; bar++) {
      const on = l.clips.some((c) => bar >= c.bars.start && bar <= c.bars.end);
      cells.push({
        bar,
        on,
        chg: !!ch && bar >= ch[0] && bar <= ch[1],
        sel: !!selection && selection.layer === l.id && bar >= selection.bars.start && bar <= selection.bars.end,
      });
    }
    return { layer: l.id, name: l.name || l.id, locked: l.locked, cells };
  });
  const legend = !take
    ? 'Make a take and the bars it changes light up here.'
    : isEdit
      ? `${changedText(take)} ${take.untouched}`.trim()
      : `Take ${take.n} is a new sound. After an edit, the bars it changed light up here.`;
  return {
    bars: score.bars,
    sections: score.sections.map((s) => ({ name: s.name || s.id, from: s.bars.start, to: s.bars.end })),
    lanes,
    isEdit,
    legend,
  };
}
