// What a panel view gets: the current take, the live score, the person's selection, and the
// actions it may take. Views never mutate a score; they ask, edit (a patch -> a take) or apply a
// mute/solo/lock patch through the adapter.

import type { EngineAdapter, Take, TakeScope, TakeState, ViewName } from '../engine/EngineAdapter';
import type { NScore, ScorePatch } from '../score/types';

export interface ViewContext {
  engine: EngineAdapter;
  state: TakeState;
  /** The current take (null before the first take). */
  take: Take | null;
  /** The live score (the current take plus mute/solo/lock since). */
  score: NScore;
  selection: TakeScope | null;
  select(scope: TakeScope | null): void;
  ask(text: string): void;
  edit(patch: ScorePatch, ask: string, scope?: TakeScope): void;
  apply(patch: ScorePatch): void;
  openView(view: ViewName): void;
  /** Feel macro positions, kept per sound by the shell. */
  knobs: Record<string, number>;
  /** Say something in the thread without making a take (a short reply). */
  note(text: string): void;
}
