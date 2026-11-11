// TauriEngine's reference-clip calls over a fake bridge that answers like crates/praxis-app:
// `clip_analyze` (the clip's bytes as the body, its name in a header), the library commands,
// `transport_compare` for the A/B, and `ask` carrying the reference's profile in words.

import { describe, expect, it } from 'vitest';
import { TauriEngine } from '../../src/engine/tauri/TauriEngine';
import type { Bridge } from '../../src/engine/tauri/bridge';
import type { AskPayload, LibraryEntryPayload, TakeRecord } from '../../src/engine/tauri/protocol';
import type { ClipFile } from '../../src/engine/EngineAdapter';
import { fixture } from './fixtures';
import { profile } from './support/profiles';

const PATH = 'C:\\Praxis\\sounds\\like-rain';

class RefBridge implements Bridge {
  calls: { cmd: string; args?: unknown; headers?: Record<string, string>; bytes?: number }[] = [];
  lib: LibraryEntryPayload[] = [];
  takes: TakeRecord[] = [];
  /** What `clip_analyze` fails with, when set. */
  clipError: { code: string; message: string } | null = null;

  fileUrl(p: string): string {
    return `http://asset.localhost/${encodeURIComponent(p)}`;
  }

  async listen<T>(_e: string, _cb: (p: T) => void): Promise<() => void> {
    return () => undefined;
  }

  async invokeBytes<T>(cmd: string, body: Uint8Array, headers: Record<string, string>): Promise<T> {
    this.calls.push({ cmd, headers, bytes: body.byteLength });
    if (cmd !== 'clip_analyze') throw { code: 'error', message: `unknown command ${cmd}` };
    if (this.clipError) throw this.clipError;
    const name = decodeURIComponent(headers['x-clip-name']);
    return { path: `C:\\Praxis\\clips\\0123-${name}`, name, profile: { ...profile(), complete: true } } as T;
  }

  async invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    this.calls.push({ cmd, args: JSON.parse(JSON.stringify(args ?? null)) });
    const status = { available: true, error: null, playing: false, seconds: 0, peak_db: [-120, -120], rms_db: [-120, -120] };
    switch (cmd) {
      case 'transport_sync':
      case 'transport_play':
      case 'transport_status':
        return status as T;
      case 'transport_compare':
      case 'transport_stop':
        return undefined as T;
      case 'new_sound_path':
        return PATH as T;
      case 'list_sounds':
        return [{ id: 'like-rain', name: 'Like rain', path: PATH, sub: '', takes: this.takes.length, modified_ms: 1 }] as T;
      case 'watch_sound':
        return this.takes.length as T;
      case 'ask': {
        const req = args!.req as { text: string };
        const rec: TakeRecord = {
          n: this.takes.length + 1,
          who: 'You',
          ask: req.text,
          title: 'Like rain',
          say: 'A soft bed.',
          patch: null,
          changed: {},
          untouched: '',
          render: { path: `${PATH}\\takes\\1\\mix.wav`, report: { loudness_lufs: -16, is_loop: true, sample_rate: 48000, length_samples: 48000 } },
          checks: { checks: [], integrated_lufs: -16, passed: true },
          score: fixture(),
        };
        this.takes.push(rec);
        const out: AskPayload = { sound: PATH, revision: this.takes.length, current: rec.n, take: rec, can_undo: true, can_redo: false };
        return out as T;
      }
      case 'takes':
        return { sound: PATH, revision: this.takes.length, current: this.takes.length, takes: this.takes, score: fixture(), can_undo: true, can_redo: false, views: [] } as T;
      case 'library_list':
        return this.lib as T;
      case 'library_add': {
        const a = args as { name: string; tags: string[]; profile: unknown };
        if (this.lib.some((e) => e.name === a.name)) throw { code: 'error', message: `there is already a reference called ${a.name}` };
        const e: LibraryEntryPayload = { id: a.name.toLowerCase().replace(/\s+/g, '-'), name: a.name, tags: a.tags, profile: a.profile };
        this.lib.unshift(e);
        return e as T;
      }
      case 'library_tag': {
        const e = this.lib.find((x) => x.id === args!.id)!;
        e.tags = args!.tags as string[];
        return e as T;
      }
      case 'library_remove':
        this.lib = this.lib.filter((x) => x.id !== args!.id);
        return { removed: true } as T;
      default:
        throw { code: 'error', message: `unknown command ${cmd}` };
    }
  }
}

const file = (name: string, size = 64): ClipFile => ({ name, size, arrayBuffer: async () => new ArrayBuffer(size) });

describe('TauriEngine reference clips', () => {
  it('sends a dropped clip as bytes with its name, and reads the profile back', async () => {
    const b = new RefBridge();
    const e = new TauriEngine(b, { makeAudio: () => null });
    const r = await e.analyzeClip(file('rain é.wav', 128));
    expect(b.calls[0]).toEqual({ cmd: 'clip_analyze', headers: { 'x-clip-name': 'rain%20%C3%A9.wav' }, bytes: 128 });
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.clip).toMatchObject({ name: 'rain é.wav', path: 'C:\\Praxis\\clips\\0123-rain é.wav', libraryId: null, audioUrl: null });
    expect(r.clip.profile.tempo?.bpm).toBeCloseTo(127.99, 6);
  });

  it('refuses a file that is not a clip before sending it, and words the shell\'s errors', async () => {
    const b = new RefBridge();
    const e = new TauriEngine(b, { makeAudio: () => null });
    expect(await e.analyzeClip(file('notes.txt'))).toMatchObject({ ok: false });
    expect(b.calls).toEqual([]);
    b.clipError = { code: 'ffmpeg_missing', message: 'Mp4 files need ffmpeg' };
    const r = await e.analyzeClip(file('scene.mp4'));
    expect(r).toEqual({ ok: false, message: expect.stringContaining('scene.mp4 is a video') });
  });

  it('a request with a reference carries its profile in words; the take reads back as typed', async () => {
    const b = new RefBridge();
    const e = new TauriEngine(b, { makeAudio: () => null });
    await e.openSound(null);
    const c = await e.analyzeClip(file('rain.wav'));
    if (!c.ok) throw new Error(c.message);
    const r = await e.ask({ text: 'Make something like this', reference: c.clip });
    const sent = b.calls.find((x) => x.cmd === 'ask')!.args as { req: { text: string } };
    expect(sent.req.text).toMatch(/^Make something like this\n\nReference clip "rain\.wav": keep its character, never its melody\./);
    expect(sent.req.text).toContain('analyze_clip: C:\\Praxis\\clips\\0123-rain.wav');
    expect(r.ok && r.take?.ask).toBe('Make something like this');
    expect(r.ok && r.take?.reference).toBe('rain.wav');
    e.dispose();
  });

  it('saves, tags, lists and removes library profiles (no audio, no file path)', async () => {
    const b = new RefBridge();
    const e = new TauriEngine(b, { makeAudio: () => null });
    const c = await e.analyzeClip(file('rain.wav'));
    if (!c.ok) throw new Error(c.message);
    const s = await e.saveReference(c.clip, '  Rain loop ', ['rain', 'night']);
    expect(s).toMatchObject({ ok: true, entry: { id: 'rain-loop', name: 'Rain loop', tags: ['rain', 'night'], kind: 'music' } });
    const add = b.calls.find((x) => x.cmd === 'library_add')!.args as Record<string, unknown>;
    expect(Object.keys(add).sort()).toEqual(['name', 'profile', 'tags']);
    expect(JSON.stringify(add)).not.toContain('clips');
    expect((await e.saveReference(c.clip, 'Rain loop', [])).ok).toBe(false);
    expect((await e.library()).map((x) => x.name)).toEqual(['Rain loop']);
    expect(await e.tagReference('rain-loop', ['bed'])).toMatchObject({ ok: true, entry: { tags: ['bed'] } });
    expect(await e.removeReference('rain-loop')).toBe(true);
    expect(await e.library()).toEqual([]);
  });

  it('the A/B trim goes to the audio service, and null puts the take back', async () => {
    const b = new RefBridge();
    const e = new TauriEngine(b, { makeAudio: () => null });
    await e.openSound(null);
    await e.setCompare({ gainDb: -4.5, muted: false });
    await e.setCompare({ gainDb: -4.5, muted: true });
    await e.setCompare(null);
    const sent = b.calls.filter((x) => x.cmd === 'transport_compare').map((x) => x.args);
    expect(sent).toEqual([{ level: { gain_db: -4.5, muted: false } }, { level: { gain_db: -4.5, muted: true } }, { level: null }]);
  });
});
