// The desktop playback line's words for each state of the real-time engine.

import { describe, expect, it } from 'vitest';
import { playbackText } from '../../src/engine/tauri/playbackLine';
import type { PlaybackInfo } from '../../src/engine/tauri/TauriEngine';

const ok: PlaybackInfo = {
  mode: 'engine',
  message: null,
  device: 'Speakers (Realtek(R) Audio)',
  sampleRate: 48000,
  xruns: 0,
  lateWakeups: 0,
  deviceErrors: 0,
  callbackFrames: [480, 480],
  preparing: false,
  stalled: false,
};

describe('playback line', () => {
  it('names the engine, device, rate, callback size and xruns', () => {
    expect(playbackText(ok)).toEqual({ text: 'Real-time engine · Speakers (Realtek(R) Audio) · 48 kHz · callbacks 480 frames · 0 xruns while playing', trouble: false });
    expect(playbackText({ ...ok, callbackFrames: [441, 480], preparing: true }).text).toBe(
      'Real-time engine · Speakers (Realtek(R) Audio) · 48 kHz · callbacks 441–480 frames · 0 xruns while playing · building the new version',
    );
  });

  it('flags xruns, device errors and a stalled device', () => {
    expect(playbackText({ ...ok, xruns: 1, deviceErrors: 2, stalled: true })).toEqual({
      text: 'Real-time engine · Speakers (Realtek(R) Audio) · 48 kHz · callbacks 480 frames · 1 xrun while playing · 2 device errors · the device stopped calling back',
      trouble: true,
    });
  });

  it('says why the take file plays instead', () => {
    const msg = "No audio output: no default output device. Playing the take's own file instead.";
    expect(playbackText({ ...ok, mode: 'audio', message: msg })).toEqual({ text: msg, trouble: true });
    expect(playbackText({ ...ok, mode: 'audio', message: null })).toEqual({ text: "Playing the take's own file.", trouble: false });
  });
});
