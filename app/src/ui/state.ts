// Application state shared by the views, and the controller API they call.

import type { DecodedAudio } from '../engine/wav';
import type { EngineAdapter, Issue, Meters, RenderResult, TransportPosition } from '../engine/EngineAdapter';
import type { History } from '../score/history';
import type { BarRange, NScore, ScorePatch } from '../score/types';

export interface Selection {
  layerId: string | null;
  bars: BarRange;
}

export interface LastPatch {
  patch: ScorePatch;
  ok: boolean;
  message: string;
  kind: 'edit' | 'undo' | 'redo';
}

export interface AppState {
  score: NScore | null;
  selection: Selection;
  editorTab: 'steps' | 'roll';
  /** Note selected in the editors, by its (step, pitch) in the derived pattern. */
  selectedNote: { step: number; pitch: number } | null;
  /** Step sequencer lane whose velocity/accent rows are shown. */
  focusPitch: number | null;
  lastPatch: LastPatch | null;
  position: TransportPosition;
  meters: Meters;
  render: { result: RenderResult; audio: DecodedAudio } | null;
  rendering: boolean;
  status: { text: string; kind: 'ok' | 'bad' | 'info' };
  issues: { errors: Issue[]; warnings: Issue[] };
}

export interface Controller {
  readonly state: AppState;
  readonly history: History;
  readonly engine: EngineAdapter;
  /** Sends a patch through history + engine. Resolves true when applied. */
  commit(patch: ScorePatch | null): Promise<boolean>;
  /**
   * Builds a patch from the score as it is once every earlier edit has landed, then commits it.
   * Use this from event handlers so quick successive events never build on a stale score.
   */
  edit(build: (score: NScore, state: AppState) => ScorePatch | null): Promise<boolean>;
  select(sel: Partial<Selection>): void;
  set(partial: Partial<AppState>): void;
  undo(): Promise<void>;
  redo(): Promise<void>;
  seekBar(bar: number): void;
  togglePlay(): void;
  toggleLoop(): void;
  render(): Promise<void>;
  saveVersion(name: string): void;
  restoreVersion(name: string): Promise<void>;
  openExport(): void;
  loadScore(raw: unknown, label: string): Promise<void>;
}
