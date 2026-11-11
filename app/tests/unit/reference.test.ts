// Reference clips (WP-U6): the profile read in plain words ("can't tell" when unsure), the
// loudness-matched A/B levels, how a reference travels with a request and comes back out of a
// take record, the library mention, and the mock engine's measurements, library and A/B trim.

import { describe, expect, it } from 'vitest';
import {
  abLevels,
  accentWords,
  clipErrorWords,
  clipProblem,
  keyLine,
  libraryMention,
  profileLines,
  profileSummary,
  referenceRequestText,
  splitReference,
  spaceLine,
  tempoLine,
  withReference,
  type ReferenceClip,
  type SoundProfile,
} from '../../src/engine/reference';
import { toProfile } from '../../src/engine/tauri/protocol';
import { mockProfile } from '../../src/engine/mock/profile';
import { encodeWav } from '../../src/engine/wav';
import { MockEngine } from '../../src/engine/MockEngine';
import { envelopeHeights, bandHeights } from '../../src/ui/profileViz';
import { defaultRefName, parseTags } from '../../src/ui/views/reference';
import type { ClipFile } from '../../src/engine/EngineAdapter';
import { profile } from './support/profiles';


const rain = (): SoundProfile =>
  profile({
    source: { file_name: 'rain.wav', duration_s: 12, sample_rate: 48000, channels: 2, pcm_hash: 'r' },
    class: { kind: 'ambience', confidence: 0.42, scores: [0.05, 0.4, 0.55] },
    tempo: { bpm: 201.2, confidence: 0.12, beats_s: [], downbeat_s: 0, beats_per_bar: 4 },
    key: { key: { tonic: 'C#', mode: 'minor' }, confidence: 0.2, chroma: [] },
    tonality: 0.1,
    groove: null,
    spectral: { centroid_hz: 640, rolloff_hz: 2000, flatness: 0.6, bands_db: [], tilt_db_per_oct: -7, centroid_track_hz: [] },
    loudness: { integrated_lufs: -23.5, range_lu: 1.2, true_peak_dbtp: -9, sample_peak_dbfs: -9, rms_dbfs: -24, crest_db: 15 },
    reverb: { rt60_s: 2.6, confidence: 0.5 },
    stereo: { width: 0.9, correlation: 0.1 },
    sections: [{ start_s: 0, end_s: 12, label: 'A' }],
    onsets_s: [],
  });

describe('a profile in plain words', () => {
  it('reads a music clip: kind, tempo, key, loudness, brightness, space, groove, shape, tone, sections', () => {
    const lines = Object.fromEntries(profileLines(profile()).map((l) => [l.id, l.words]));
    expect(lines).toEqual({
      kind: 'Music',
      tempo: '128 BPM',
      key: 'A minor',
      loudness: 'Moderate, some light and shade (-16.0 LUFS)',
      brightness: 'Warm',
      space: 'A room (about 1.4 s of reverb), wide',
      groove: 'Straight sixteenths, steady, accents on beats 1 and 3',
      shape: 'A sharp attack, settles, a long tail',
      tone: 'Clear notes',
      sections: '2 sections, changing at 0:15',
    });
    expect(profileSummary(profile())).toBe('Music · 30 s · 128 BPM · A minor · warm · a room');
  });

  it("says can't tell for a beat or key the material does not show, with the reason", () => {
    const p = rain();
    expect(tempoLine(p)).toMatchObject({ words: "Can't tell: no steady beat", cantTell: true });
    expect(keyLine(p)).toMatchObject({ words: "Can't tell: mostly noise, no clear notes", cantTell: true });
    // A tonal clip with a key below the calibrated confidence (0.4).
    expect(keyLine(profile({ key: { key: { tonic: 'E', mode: 'minor' }, confidence: 0.16, chroma: [] } }))).toMatchObject({
      words: "Can't tell: the notes do not settle on a key",
      cantTell: true,
    });
    // Between the threshold and 0.6 it is said, with "not sure".
    expect(keyLine(profile({ key: { key: { tonic: 'A', mode: 'major' }, confidence: 0.58, chroma: [] } })).words).toBe('A major (not sure)');
    expect(tempoLine(profile({ tempo: null })).words).toBe("Can't tell: the beat is not clear");
    // No key found in a tonal clip is not "noise".
    expect(keyLine(profile({ key: null })).words).toBe("Can't tell: the notes do not settle on a key");
    // A noise bed can carry a confident but spurious pulse (WP-D2): no beat without a groove.
    const bed = { ...rain(), tempo: { bpm: 96, confidence: 0.75, beats_s: [], downbeat_s: 0, beats_per_bar: 4 } };
    expect(tempoLine(bed)).toMatchObject({ words: "Can't tell: no steady beat", cantTell: true });
    expect(profileSummary(bed)).not.toContain('BPM');
    // An effect whose hits fall on a grid has a beat.
    const ticks = { ...bed, class: { kind: 'sfx' as const, confidence: 0.8, scores: [0.1, 0.8, 0.1] as [number, number, number] }, groove: profile().groove };
    expect(tempoLine(ticks).words).toBe('96 BPM');
    const words = Object.fromEntries(profileLines(p).map((l) => [l.id, l.words]));
    expect(words.kind).toBe('Ambience, or maybe sound effect');
    expect(words.groove).toBe('No hits, it flows');
    expect(words.brightness).toBe('Dark, the highs fall away fast');
    expect(words.space).toBe('A large hall (about 2.6 s of reverb), very wide');
    expect(profileSummary(p)).toBe('Ambience · 12 s · dark · a large hall');
  });

  it('a dry, mono effect with a room it cannot measure', () => {
    const p = profile({ reverb: null, stereo: { width: 0, correlation: 1 }, class: { kind: 'sfx', confidence: 0.9, scores: [0.05, 0.9, 0.05] } });
    expect(spaceLine(p).words).toBe("Room can't tell, mono");
    expect(profileLines(p)[0].words).toBe('Sound effect');
  });

  it('names the accented beats of a groove', () => {
    const w = new Array(16).fill(0.1);
    w[4] = 1;
    w[12] = 0.9;
    expect(accentWords(w, 16, 4)).toBe('accents on beats 2 and 4');
    w[6] = 0.8;
    expect(accentWords(w, 16, 4)).toBe('accents on beats 2 and 4 and off the beat');
    expect(accentWords([0.1, 0.9, 0.1, 0.8], 4, 2)).toBe('accents off the beat');
    expect(accentWords([], 16, 4)).toBe('');
  });

  it('scales the small visuals: envelope over 60 dB, bands over 36 dB', () => {
    expect(envelopeHeights([-60, -30, 0])).toEqual([0, 0.5, 1]);
    const low = envelopeHeights([-120, -100]);
    expect(low[0]).toBeCloseTo(2 / 3, 9);
    expect(low[1]).toBe(1);
    expect(bandHeights([-36, -18, 0, Number.NaN])).toEqual([0, 0.5, 1, 0]);
  });
});

describe('loudness-matched A/B', () => {
  it('turns the louder one down by the difference, never anything up', () => {
    const louderClip = abLevels(-16, -11.8);
    expect(louderClip.takeDb).toBe(0);
    expect(louderClip.clipDb).toBeCloseTo(-4.2, 6);
    expect(louderClip.clipVolume).toBeCloseTo(10 ** (-4.2 / 20), 6);
    expect(louderClip.words).toBe('Levels matched at -16.0 LUFS: the reference plays 4.2 dB quieter so the louder one does not win.');
    const louderTake = abLevels(-14, -23.5);
    expect(louderTake).toMatchObject({ takeDb: -9.5, clipDb: 0, clipVolume: 1, matched: true });
    expect(louderTake.words).toContain('the take plays 9.5 dB quieter');
    expect(abLevels(-16, -16.1)).toMatchObject({ takeDb: 0, clipDb: 0, matched: true, words: 'Levels matched: both play at -16.0 LUFS.' });
  });

  it('says so when a loudness is missing (a silent clip)', () => {
    expect(abLevels(-16, -Infinity)).toMatchObject({ matched: false, takeDb: 0, clipVolume: 1, words: 'Levels not matched: the reference has no measured loudness.' });
    expect(abLevels(null, -20).words).toBe('Levels not matched: the take has no measured loudness.');
  });
});

describe('a reference with a request', () => {
  const clip: ReferenceClip = { name: 'rain.wav', profile: rain(), audioUrl: 'blob:x', path: 'C:\\Praxis\\clips\\0f-rain.wav', libraryId: null };

  it('adds the profile in words and the clip file for the assistant, after the typed words', () => {
    const t = withReference('Make something like this, 30 seconds', clip);
    expect(t.startsWith('Make something like this, 30 seconds\n\nReference clip "rain.wav": keep its character, never its melody. 12 s long; ')).toBe(true);
    expect(t).toContain("tempo: can't tell: no steady beat");
    expect(t).toContain('brightness: dark');
    expect(t).toContain("The clip's file, for matching: C:\\Praxis\\clips\\0f-rain.wav");
    // A short version: the words and the key numbers, never the tracks or an invitation to fetch them all.
    expect(t).toContain('Measured: tempo 201.2 BPM at confidence 0.12; key C# minor at confidence 0.20; -23.5 LUFS, range 1.2 LU');
    expect(t).not.toContain('analyze_clip');
    expect(t.length).toBeLessThan(1500);
    // The analyzer's own summary leads when it sent one.
    expect(referenceRequestText({ ...clip, profile: { ...rain(), summary: 'ambience (82% sure) · 12.0 s · -23.5 LUFS · noisy' } })).toContain(
      'Reference clip "rain.wav" (ambience (82% sure) · 12.0 s · -23.5 LUFS · noisy): keep its character',
    );
    // The words never name a length in seconds or bars the planner would take as the request's.
    expect(referenceRequestText(clip)).not.toMatch(/\bseconds?\b|\bbars?\b/);
    expect(withReference('plain', undefined)).toBe('plain');
  });

  it('a library style has no file and says where it came from', () => {
    const t = referenceRequestText({ ...clip, name: 'rain "loop"', audioUrl: null, path: null, libraryId: 'ref-1' });
    expect(t).toContain(`Reference style "rain 'loop'" (from the reference library)`);
    expect(t).not.toContain('analyze_clip');
  });

  it('a take record splits back into the typed request and the reference name', () => {
    expect(splitReference(withReference('Make something like this', clip))).toEqual({ ask: 'Make something like this', reference: 'rain.wav' });
    expect(splitReference('A calm loop\n\nwith a note')).toEqual({ ask: 'A calm loop\n\nwith a note', reference: null });
  });

  it("finds a library style a request names in quotes", () => {
    const lib = [
      { id: 'ref-1', name: 'Rain loop' },
      { id: 'ref-2', name: 'door slam' },
    ];
    expect(libraryMention("make something like 'rain loop' but brighter", lib)).toBe('ref-1');
    expect(libraryMention('a hit like "Door Slam"', lib)).toBe('ref-2');
    expect(libraryMention('make a rain loop', lib)).toBeNull();
    expect(libraryMention("don't make it 'drums'", lib)).toBeNull();
  });
});

describe('what can be dropped, and why not', () => {
  it('takes audio and video, refuses anything else, empty files and huge ones', () => {
    expect(clipProblem({ name: 'rain.WAV', size: 10 })).toBeNull();
    expect(clipProblem({ name: 'scene.mov', size: 10 })).toBeNull();
    expect(clipProblem({ name: 'notes.txt', size: 10 })).toBe('notes.txt is not a clip Praxis can read. Drop audio (WAV, MP3, FLAC, OGG, M4A) or video (MP4, MOV, WebM).');
    expect(clipProblem({ name: 'a.wav', size: 0 })).toBe('a.wav is empty.');
    expect(clipProblem({ name: 'a.mp4', size: 600 * 1024 * 1024 })).toContain('larger than 512 MB');
  });

  it('a video without ffmpeg says what to do', () => {
    expect(clipErrorWords('scene.mp4', 'ffmpeg_missing', 'x')).toBe(
      "scene.mp4 is a video. Praxis reads a video's sound with ffmpeg, which is not installed here. Install ffmpeg (or set PRAXIS_FFMPEG to it) and drop the clip again, or drop the audio on its own.",
    );
    expect(clipErrorWords('a.m4a', 'ffmpeg_missing', 'x')).toContain('needs ffmpeg');
    expect(clipErrorWords('scene.mp4', 'ffmpeg_failed', 'x')).toContain('no audio track');
    expect(clipErrorWords('a.wav', 'decode_failed', 'not a RIFF file')).toBe('Praxis could not read a.wav: not a RIFF file.');
  });

  it('library names and tags from what the person typed', () => {
    expect(defaultRefName('rain_loop-02.wav')).toBe('rain loop 02');
    expect(parseTags(' Rain, night;loop, rain ,')).toEqual(['rain', 'night', 'loop']);
  });
});

describe('the profile as the shell sends it', () => {
  it('accepts the profile or a payload wrapping it, and fills what is missing', () => {
    const p = profile();
    expect(toProfile(JSON.parse(JSON.stringify(p)))).toMatchObject({ class: p.class, tempo: { bpm: 127.99 }, key: { key: { tonic: 'A', mode: 'minor' } } });
    const wrapped = toProfile({ complete: true, profile: { source: p.source, class: p.class, loudness: p.loudness } });
    expect(wrapped.envelope.frames_db).toEqual([]);
    expect(wrapped.tempo).toBeNull();
    expect(wrapped.groove).toBeNull();
    expect(wrapped.reverb).toBeNull();
    // Extra fields the CLI adds (complete, output) are dropped.
    expect(Object.keys(toProfile({ ...p, complete: true, output: 'x' }))).not.toContain('complete');
    expect(() => toProfile({ tempo: 1 })).toThrow('did not return a Sound Profile');
  });
});

// ---------------------------------------------------------------- the mock engine

const SR = 22050;

function clipFile(name: string, channels: Float32Array[]): ClipFile & { bytes: ArrayBuffer } {
  const bytes = encodeWav(channels, SR, 'wav16');
  return { name, size: bytes.byteLength, bytes, arrayBuffer: async () => bytes };
}

/** Clicks every 0.5 s (120 BPM) for 8 s. */
function clicks(): Float32Array[] {
  const n = SR * 8;
  const x = new Float32Array(n);
  for (let t = 0; t < n; t += SR / 2) for (let i = 0; i < 400 && t + i < n; i++) x[t + i] = 0.8 * Math.exp(-i / 60) * Math.sin(i * 0.3);
  return [x, x];
}

/** Steady noise, 10 s, decorrelated channels. */
function bed(): Float32Array[] {
  let s = 0x9e3779b9;
  const r = () => {
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    s >>>= 0;
    return (s / 4294967296) * 2 - 1;
  };
  const n = SR * 10;
  const l = new Float32Array(n).map(() => 0.1 * r());
  const rr = new Float32Array(n).map(() => 0.1 * r());
  return [l, rr];
}

/** One hit and silence, 1 s. */
function hit(): Float32Array[] {
  const n = SR;
  const x = new Float32Array(n);
  for (let i = 0; i < 3000; i++) x[i + 2000] = 0.9 * Math.exp(-i / 500) * Math.sin(i * 0.9);
  return [x, x];
}

describe('the mock engine measures roughly and says so', () => {
  it('a steady click track is music near 120 BPM, a noise bed ambience, a single hit an effect', () => {
    const m = mockProfile('take.wav', { sampleRate: SR, channels: clicks() }, 0);
    expect(m.mock).toBe(true);
    expect(m.class.kind).toBe('music');
    expect(Math.abs((m.tempo?.bpm ?? 0) - 120)).toBeLessThan(3);
    expect(m.source.duration_s).toBeCloseTo(8, 3);
    const a = mockProfile('x.wav', { sampleRate: SR, channels: bed() }, 0);
    expect(a.class.kind).toBe('ambience');
    expect(a.tempo).toBeNull();
    expect(a.stereo.width).toBeGreaterThan(0.6);
    const s = mockProfile('x.wav', { sampleRate: SR, channels: hit() }, 0);
    expect(s.class.kind).toBe('sfx');
    expect(s.onsets_s.length).toBe(1);
    expect(s.envelope.frames_db.length).toBeLessThanOrEqual(2000);
  });

  it('analyses a dropped clip, keeps a library of profiles only, and holds the A/B trim', async () => {
    const e = new MockEngine();
    const bad = await e.analyzeClip({ name: 'notes.txt', size: 3, arrayBuffer: async () => new ArrayBuffer(3) });
    expect(bad).toEqual({ ok: false, message: expect.stringContaining('not a clip Praxis can read') });
    const r = await e.analyzeClip(clipFile('rain bed.wav', bed()));
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.clip.profile.class.kind).toBe('ambience');
    expect(r.clip.path).toBe('mock://clips/rain bed.wav');

    const saved = await e.saveReference(r.clip, 'Rain bed', [' Night', 'rain', 'night']);
    expect(saved).toMatchObject({ ok: true, entry: { name: 'Rain bed', tags: ['night', 'rain'], kind: 'ambience' } });
    expect(await e.saveReference(r.clip, 'rain BED', [])).toEqual({ ok: false, message: 'There is already a reference called "rain BED".' });
    const list = await e.library();
    expect(list).toHaveLength(1);
    expect(JSON.stringify(list)).not.toContain('blob:');
    expect(list[0].profile).toBeNull();
    expect((await e.libraryProfile(list[0].id))?.style_name).toBe('Rain bed');
    if (!saved.ok) return;
    expect((await e.tagReference(saved.entry.id, ['loop'])).ok).toBe(true);
    expect((await e.library())[0].tags).toEqual(['loop']);
    expect(await e.removeReference(saved.entry.id)).toBe(true);
    expect(await e.library()).toEqual([]);

    await e.setCompare({ gainDb: -4, muted: false });
    expect(e.compareLevel()).toEqual({ gainDb: -4, muted: false });
    await e.setCompare(null);
    expect(e.compareLevel()).toBeNull();
    e.dispose();
  });

  it('a request with a reference gives the planner its profile in words; the take shows the typed words', async () => {
    const e = new MockEngine();
    const r = await e.analyzeClip(clipFile('clicks.wav', clicks()));
    if (!r.ok) throw new Error(r.message);
    const t = await e.ask({ text: 'A calm 20-second loop like this', reference: r.clip });
    expect(t.ok).toBe(true);
    if (!t.ok || !t.take) return;
    expect(t.take.ask).toBe('A calm 20-second loop like this');
    expect(t.take.reference).toBe('clicks.wav');
    expect(e.lastRequest).toContain('Reference clip "clicks.wav"');
    e.dispose();
  });
});
