// What a panel view gets: the current take, the live score, the person's selection, and the
// actions it may take. Views never mutate a score; they ask, edit (a patch -> a take), apply a
// mute/solo/lock patch, or make a live hand edit (a patch heard at once, batched until "Make a
// take") through the adapter.

import type { EngineAdapter, PresetInfo, Take, TakeScope, TakeState, ViewName } from '../engine/EngineAdapter';
import type { BarRange, NScore, ScorePatch } from '../score/types';

/** Where the transport is, for the change map's playhead, seek and loop controls. */
export interface TransportView {
  playing: boolean;
  /** Seconds from the start of bar 1. */
  seconds: number;
  loop: BarRange | null;
}

export interface ViewContext {
  engine: EngineAdapter;
  state: TakeState;
  /** The current take (null before the first take). */
  take: Take | null;
  /** The live score (the current take plus mute/solo/lock changes and hand edits since). */
  score: NScore;
  selection: TakeScope | null;
  select(scope: TakeScope | null): void;
  ask(text: string): void;
  edit(patch: ScorePatch, ask: string, scope?: TakeScope): void;
  apply(patch: ScorePatch): void;
  /**
   * A hand edit heard at once (no take yet): applied to the live score and added to the batch the
   * next "Make a take" records. `label` is how it reads in that take's request.
   */
  live(patch: ScorePatch, label: string, at?: TakeScope): void;
  openView(view: ViewName): void;
  /** Feel macro positions, kept per sound by the shell. */
  knobs: Record<string, number>;
  /** Say something in the thread without making a take (a short reply). */
  note(text: string): void;
  /** Instrument presets for the layer picker (empty until loaded). */
  presets: PresetInfo[];
  /** The layer the Rhythm and Notes views show when nothing is selected (null: their default). */
  viewLayer: string | null;
  setViewLayer(id: string | null): void;
  /** The layer whose details are open in the Overview (null: none). */
  expanded: string | null;
  setExpanded(id: string | null): void;
  transport: TransportView;
  seek(seconds: number): void;
  setLoop(range: BarRange | null): void;
  togglePlay(): void;
}
