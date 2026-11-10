// Browser dev entry: the workstation against the mock engine. The desktop (Tauri IPC) and
// browser (wasm) builds construct App with their own EngineAdapter instead.

import './styles.css';
import { App, type FixtureEntry } from './ui/app';
import { MockEngine } from './engine/MockEngine';

const modules = import.meta.glob('../../fixtures/*.score.json', { eager: true, import: 'default' });
const fixtures: FixtureEntry[] = Object.entries(modules)
  .map(([path, score]) => ({ name: path.split('/').pop()!.replace('.score.json', ''), score }))
  .sort((a, b) => a.name.localeCompare(b.name));

const params = new URLSearchParams(location.search);
const theme = params.get('theme');
if (theme === 'light' || theme === 'dark') document.documentElement.dataset.theme = theme;

const engine = new MockEngine();
const app = new App(document.getElementById('app')!, engine, fixtures);
const wanted = params.get('fixture') ?? 'acceptance1';
const first = fixtures.find((f) => f.name === wanted) ?? fixtures[0];
if (first) void app.loadScore(first.score, first.name);

// Handy in the browser console and for UI tests; not an API.
(window as unknown as { praxis: unknown }).praxis = { app, engine };
