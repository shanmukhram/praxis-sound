// The sound assistant at work, as the thread shows it under a request: its steps in plain words
// ("Reading the sound", "Changing the felt piano's tone", "Rendering", "Checking: 8 of 8
// passed") and its latest words between steps. Replaced by the take or the reply when done.

import type { AskProgress, Who } from '../engine/EngineAdapter';

/** One line under the request. */
export interface StepLine {
  words: string;
  state: 'running' | 'done' | 'failed';
}

/** A request being carried out, with the assistant's steps so far. */
export interface Making {
  ask: string;
  who: Who;
  steps: StepLine[];
  /** The assistant's latest words between steps. */
  said: string | null;
}

export function making(ask: string, who: Who): Making {
  return { ask, who, steps: [], said: null };
}

/**
 * Adds one step of progress. A step starts a line; when it finishes the line is done (or failed),
 * and the checks' result or a failure gets a line of its own.
 */
export function applyProgress(m: Making, p: AskProgress): void {
  if (p.kind === 'step') {
    m.steps.push({ words: p.words, state: 'running' });
  } else if (p.kind === 'step_done') {
    const running = [...m.steps].reverse().find((s) => s.state === 'running');
    if (running) running.state = p.ok ? 'done' : 'failed';
    if (!p.ok || p.words.startsWith('Checking')) m.steps.push({ words: p.words, state: p.ok ? 'done' : 'failed' });
  } else {
    m.said = p.text;
  }
}
