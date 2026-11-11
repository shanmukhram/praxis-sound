// The desktop playback line under the Sound panel's title: which engine plays (the real-time
// engine on a named device, or the take's own file and why), callback sizes, xruns and device
// errors, and a small left / right level meter fed by the engine's meters.

import type { Meters } from '../EngineAdapter';
import { h } from '../../ui/dom';
import type { PlaybackInfo, TauriEngine } from './TauriEngine';

/** Meter range: -60 dBFS is empty, 0 dBFS full. */
const METER_FLOOR = -60;

/** The words for one playback state (exported for tests). */
export function playbackText(p: PlaybackInfo): { text: string; trouble: boolean } {
  if (p.mode === 'audio') return { text: p.message ?? "Playing the take's own file.", trouble: !!p.message };
  const parts = ['Real-time engine'];
  if (p.device) parts.push(p.device);
  if (p.sampleRate) parts.push(`${p.sampleRate / 1000} kHz`);
  const [lo, hi] = p.callbackFrames;
  if (hi) parts.push(lo === hi ? `${hi}-frame callbacks` : `${lo}–${hi}-frame callbacks`);
  parts.push(`${p.xruns} ${p.xruns === 1 ? 'xrun' : 'xruns'}`);
  if (p.deviceErrors) parts.push(`${p.deviceErrors} device ${p.deviceErrors === 1 ? 'error' : 'errors'}`);
  if (p.stalled) parts.push('the device stopped calling back');
  if (p.preparing) parts.push('building the new version');
  return { text: parts.join(' · '), trouble: p.xruns > 0 || p.deviceErrors > 0 || p.stalled };
}

function fill(db: number): string {
  const f = Number.isFinite(db) ? Math.max(0, Math.min(1, (db - METER_FLOOR) / -METER_FLOOR)) : 0;
  return `${(f * 100).toFixed(1)}%`;
}

/** Adds the playback line to the Sound panel's header of the shell under `root`. */
export function mountPlaybackLine(root: HTMLElement, engine: TauriEngine): () => void {
  const head = root.querySelector('.panel-head > div');
  if (!head) return () => undefined;
  const bar = (label: string) => {
    const level = h('span', { style: 'display:block;height:100%;width:0;background:var(--accent);border-radius:2px;transition:width 60ms linear' });
    const track = h('span', { 'aria-hidden': 'true', title: label, style: 'display:block;width:56px;height:3px;background:var(--raised);border-radius:2px;overflow:hidden' }, level);
    return { track, level };
  };
  const l = bar('Left');
  const r = bar('Right');
  const meter = h('span', { 'data-testid': 'meters', style: 'display:inline-grid;gap:2px;vertical-align:middle;margin-right:8px' }, l.track, r.track);
  const text = h('span', { 'data-testid': 'playback-text' });
  const line = h('p', { class: 'playback-line', 'data-testid': 'playback', role: 'status', style: 'margin:-8px 0 12px;font-size:var(--fs-small);line-height:16px;color:var(--faint)' }, meter, text);
  head.append(line);
  let last = '';
  const offPlayback = engine.onPlayback((p) => {
    const { text: t, trouble } = playbackText(p);
    if (t !== last) {
      text.textContent = t;
      last = t;
    }
    line.style.color = trouble ? 'var(--negative)' : 'var(--faint)';
    meter.style.display = p.mode === 'engine' ? 'inline-grid' : 'none';
  });
  const offMeters = engine.onMeters((m: Meters) => {
    l.level.style.width = fill(m.peakL);
    r.level.style.width = fill(m.peakR);
  });
  const offPos = engine.onPosition((p) => {
    if (!p.playing) {
      l.level.style.width = '0';
      r.level.style.width = '0';
    }
  });
  return () => {
    offPlayback();
    offMeters();
    offPos();
    line.remove();
  };
}
