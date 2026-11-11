// Entry: inside the Tauri shell the workstation runs on the real engine (TauriEngine, sound
// folders on disk); in a plain browser it runs against the mock engine for UI development.

import './styles.css';
import { MockEngine } from './engine/MockEngine';
import { inTauri } from './engine/tauri/bridge';
import { normalizeScore, type Score } from './score/types';
import { keyLabel } from './score/theory';
import { scoreSeconds } from './score/timeline';
import { Shell, type SoundEntry } from './ui/shell';
import acceptance1 from '../../fixtures/acceptance1.score.json';
import bell from '../../fixtures/acceptance1_bell.score.json';
import empty from '../../fixtures/empty.score.json';

function sub(raw: unknown): string {
  const s = normalizeScore(raw as Score);
  const secs = scoreSeconds(s);
  return `${keyLabel(s.key)} · ${Math.floor(secs / 60)}:${String(Math.round(secs % 60)).padStart(2, '0')}${s.playback.mode === 'loop' ? ' loop' : ''}`;
}

function startMock(root: HTMLElement): { shell: Shell; engine: MockEngine } {
  const engine = new MockEngine();
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
  const shell = new Shell(root, engine, sounds);
  const wanted = new URLSearchParams(location.search).get('sound') ?? 'calm';
  void shell.openSound(sounds.some((s) => s.id === wanted) ? wanted : 'calm');
  return { shell, engine };
}

async function start(): Promise<void> {
  const root = document.getElementById('app')!;
  const app = inTauri() ? await (await import('./engine/tauri/desktop')).startDesktop(root) : startMock(root);
  // Handy in the devtools console and for UI tests (e.g. praxis.engine.agentAsk(...)); not an API.
  (window as unknown as { praxis: unknown }).praxis = app;
}

void start();
