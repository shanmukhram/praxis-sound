// Browser dev entry: the conversation-first workstation against the mock engine. The desktop
// (Tauri IPC) and browser (wasm) builds construct the Shell with their own EngineAdapter.

import './styles.css';
import { MockEngine } from './engine/MockEngine';
import { normalizeScore, type Score } from './score/types';
import { keyLabel } from './score/theory';
import { scoreSeconds } from './score/timeline';
import { Shell, type SoundEntry } from './ui/shell';
import acceptance1 from '../../fixtures/acceptance1.score.json';
import bell from '../../fixtures/acceptance1_bell.score.json';
import empty from '../../fixtures/empty.score.json';

const engine = new MockEngine();

function sub(raw: unknown): string {
  const s = normalizeScore(raw as Score);
  const secs = scoreSeconds(s);
  return `${keyLabel(s.key)} · ${Math.floor(secs / 60)}:${String(Math.round(secs % 60)).padStart(2, '0')}${s.playback.mode === 'loop' ? ' loop' : ''}`;
}

const sounds: SoundEntry[] = [
  { id: 'calm', name: 'Calm piano loop', sub: sub(acceptance1), open: () => engine.seedDemo(acceptance1 as Score) },
  {
    id: 'bell',
    name: 'Bell loop',
    sub: sub(bell),
    open: async () => {
      await engine.startSound(bell as Score, {
        ask: 'The same calm loop, with a bell instead of the felt piano.',
        title: 'Bell loop',
        say: 'A bell playing the two-bar motif with a soft kick from bar 9, 16 bars at 128 BPM.',
      });
    },
  },
  {
    id: 'new',
    name: 'Untitled sound',
    sub: 'New',
    open: async () => {
      await engine.load(empty as Score);
    },
  },
];

const shell = new Shell(document.getElementById('app')!, engine, sounds);
const wanted = new URLSearchParams(location.search).get('sound') ?? 'calm';
void shell.openSound(sounds.some((s) => s.id === wanted) ? wanted : 'calm');

// Handy in the browser console and for UI tests (e.g. praxis.engine.agentAsk(...)); not an API.
(window as unknown as { praxis: unknown }).praxis = { shell, engine };
