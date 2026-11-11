// Reference clips (FR-11, FR-12, FR-19, FR-30): the Sound Profile the analyzer returns
// (crates/praxis-analyze/src/profile.rs, docs/contracts/sound-profile.md), its reading in plain
// words, loudness-matched A/B levels, and how a reference travels with a request.
//
// A profile holds measurements only, never audio. A dropped clip also keeps a URL the WebView can
// play it from (for A/B) and the shell's copy of the file (so the assistant can measure it); a
// library style is the profile alone.

import type { Grid, Key } from '../score/types';
import { keyLabel } from '../score/theory';

// ---------------------------------------------------------------- the profile (frozen contract)

export type ClipKind = 'music' | 'sfx' | 'ambience';

export interface SoundProfile {
  version: number;
  source: { file_name: string; duration_s: number; sample_rate: number; channels: number; pcm_hash: string };
  class: { kind: ClipKind; confidence: number; /** music, sfx, ambience; sum 1 */ scores: [number, number, number] };
  tempo: { bpm: number; confidence: number; beats_s: number[]; downbeat_s: number; beats_per_bar: number } | null;
  key: { key: Key; confidence: number; chroma: number[] } | null;
  loudness: { integrated_lufs: number; range_lu: number; true_peak_dbtp: number; sample_peak_dbfs: number; rms_dbfs: number; crest_db: number };
  envelope: { attack_s: number; decay_s: number; sustain: number; release_s: number; frames_db: number[]; hop_s: number };
  spectral: { centroid_hz: number; rolloff_hz: number; flatness: number; bands_db: number[]; tilt_db_per_oct: number; centroid_track_hz: number[] };
  groove: { grid: Grid; steps_per_bar: number; step_weights: number[]; swing: number; onsets_per_bar: number } | null;
  onsets_s: number[];
  pitch: { hop_s: number; midi: (number | null)[] };
  tonality: number;
  stereo: { width: number; correlation: number };
  reverb: { rt60_s: number; confidence: number } | null;
  sections: { start_s: number; end_s: number; label: string }[];
  features: number[];
  tags?: string[];
  style_name?: string | null;
  /** Set by the mock engine: rough browser measurements, not the analyzer's. */
  mock?: boolean;
  /** The analyzer's one line in plain words (`praxis analyze`'s `summary`), when it sent one. */
  summary?: string;
}

/** A reference the next request (and the A/B) uses: a dropped clip, or a style from the library. */
export interface ReferenceClip {
  /** The file name, or the library style's name. */
  name: string;
  profile: SoundProfile;
  /** Where the WebView plays the clip from (a blob: URL); null for a library style (no audio). */
  audioUrl: string | null;
  /** The shell's copy of the clip, which the assistant can measure; null for a library style. */
  path: string | null;
  /** Set when it came from (or was saved to) the library. */
  libraryId: string | null;
}

/** A reference library entry: a named, tagged profile (never audio). */
export interface LibraryEntry {
  id: string;
  name: string;
  tags: string[];
  kind: ClipKind;
  /** One line in plain words ("ambience (82% sure) · 12.0 s · -23.5 LUFS · noisy"). */
  summary: string;
  /** The full profile; null in a listing until it is fetched (`libraryProfile`). */
  profile: SoundProfile | null;
}

export type AnalyzeResult = { ok: true; clip: ReferenceClip } | { ok: false; message: string };
export type LibraryResult = { ok: true; entry: LibraryEntry } | { ok: false; message: string };

/** The A/B's take side: a trim on the take while comparing (null: back to normal). */
export interface CompareLevel {
  gainDb: number;
  muted: boolean;
}

// ---------------------------------------------------------------- what can be dropped

export const AUDIO_EXTS = ['wav', 'mp3', 'flac', 'ogg', 'm4a'] as const;
export const VIDEO_EXTS = ['mp4', 'mov', 'webm'] as const;
/** Larger clips are refused before they are read (a long video: trim it first). */
export const MAX_CLIP_BYTES = 512 * 1024 * 1024;

export function extOf(name: string): string {
  const m = /\.([a-z0-9]+)$/i.exec(name.trim());
  return m ? m[1].toLowerCase() : '';
}

export function isVideoName(name: string): boolean {
  return (VIDEO_EXTS as readonly string[]).includes(extOf(name));
}

/** Why a file cannot be a reference clip, in plain words; null when it can. */
export function clipProblem(file: { name: string; size?: number }): string | null {
  const ext = extOf(file.name);
  if (![...AUDIO_EXTS, ...VIDEO_EXTS].includes(ext as never)) {
    return `${file.name} is not a clip Praxis can read. Drop audio (WAV, MP3, FLAC, OGG, M4A) or video (MP4, MOV, WebM).`;
  }
  if (file.size !== undefined && file.size === 0) return `${file.name} is empty.`;
  if (file.size !== undefined && file.size > MAX_CLIP_BYTES) return `${file.name} is larger than 512 MB. Trim it to the part you want to match and drop that.`;
  return null;
}

/** A failed analysis (the shell's error code and message) in plain words. */
export function clipErrorWords(name: string, code: string, message: string): string {
  switch (code) {
    case 'ffmpeg_missing':
      return isVideoName(name)
        ? `${name} is a video. Praxis reads a video's sound with ffmpeg, which is not installed here. Install ffmpeg (or set PRAXIS_FFMPEG to it) and drop the clip again, or drop the audio on its own.`
        : `${name} needs ffmpeg to read, which is not installed here. Install ffmpeg (or set PRAXIS_FFMPEG to it), or drop a WAV, MP3, FLAC or OGG instead.`;
    case 'ffmpeg_failed':
      return `ffmpeg could not read the sound in ${name}. It may have no audio track, or be damaged.`;
    case 'decode_failed':
    case 'bad_format':
    case 'unsupported_format':
      return `Praxis could not read ${name}: ${message.replace(/\.?$/, '.')}`;
    default:
      return `Could not measure ${name}: ${message.replace(/\.?$/, '.')}`;
  }
}

// ---------------------------------------------------------------- reading a profile in words

/** Below these the measurement says nothing (contracts/sound-profile.md, "Can't tell"). */
export const TEMPO_SURE = 0.25;
export const KEY_SURE = 0.4;

const KIND_WORD: Record<ClipKind, string> = { music: 'Music', sfx: 'Sound effect', ambience: 'Ambience' };
const KINDS: ClipKind[] = ['music', 'sfx', 'ambience'];

export function kindWord(kind: ClipKind): string {
  return KIND_WORD[kind] ?? kind;
}

const one = (v: number) => (Math.round(v * 10) / 10).toFixed(1);
const fin = (v: number | null | undefined): v is number => typeof v === 'number' && Number.isFinite(v);

/** "0:07", "1:32". */
export function clockS(s: number): string {
  const t = Math.max(0, Math.round(s));
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`;
}

export function lengthWords(s: number): string {
  if (!fin(s)) return '';
  if (s < 10) return `${one(s)} s`;
  if (s < 60) return `${Math.round(s)} s`;
  return clockS(s);
}

/** One line of the profile: a label, the reading in words, and the numbers behind it. */
export interface ProfileLine {
  id: 'kind' | 'tempo' | 'key' | 'loudness' | 'brightness' | 'space' | 'groove' | 'shape' | 'tone' | 'sections';
  label: string;
  words: string;
  /** The measured numbers, for "Show the numbers". */
  detail: string;
  /** True when the material cannot decide it. */
  cantTell?: boolean;
}

export function kindLine(p: SoundProfile): ProfileLine {
  const c = p.class;
  const order = KINDS.map((k, i) => ({ k, s: c.scores?.[i] ?? 0 })).sort((a, b) => b.s - a.s);
  const second = order.find((o) => o.k !== c.kind);
  const name = kindWord(c.kind);
  let words: string;
  if (c.confidence >= 0.7) words = name;
  else if (c.confidence >= 0.45 || !second) words = `Probably ${name.toLowerCase()}`;
  else words = `${name}, or maybe ${kindWord(second.k).toLowerCase()}`;
  const detail = KINDS.map((k, i) => `${k} ${Math.round((c.scores?.[i] ?? 0) * 100)}%`).join(', ');
  return { id: 'kind', label: 'What it is', words, detail };
}

/**
 * A beat worth naming: music with a confident tempo, or anything else whose onsets fall on a
 * regular grid (the analyzer finds a spurious pulse in noise beds; WP-D2's hand-over).
 */
export function hasBeat(p: SoundProfile): boolean {
  const t = p.tempo;
  if (!t || !fin(t.bpm) || t.confidence < TEMPO_SURE) return false;
  return p.class.kind === 'music' || !!p.groove;
}

export function tempoLine(p: SoundProfile): ProfileLine {
  const t = p.tempo;
  if (!t || !hasBeat(p)) {
    return {
      id: 'tempo',
      label: 'Tempo',
      words: p.class.kind === 'music' ? "Can't tell: the beat is not clear" : "Can't tell: no steady beat",
      detail: t ? `${one(t.bpm)} BPM at confidence ${t.confidence.toFixed(2)}` : 'no periodic onsets',
      cantTell: true,
    };
  }
  // A metre in numbers ("in 3/4"), never "bar": the planner reads "bars" as the sound's length.
  const meter = t.beats_per_bar && t.beats_per_bar !== 4 ? `, in ${t.beats_per_bar}/4` : '';
  return {
    id: 'tempo',
    label: 'Tempo',
    words: `${Math.round(t.bpm)} BPM${meter}${t.confidence < 0.5 ? ' (not sure)' : ''}`,
    detail: `${one(t.bpm)} BPM, confidence ${t.confidence.toFixed(2)}, ${t.beats_s.length} beats, first downbeat at ${t.downbeat_s.toFixed(2)} s`,
  };
}

export function keyLine(p: SoundProfile): ProfileLine {
  const k = p.key;
  if (!k || p.tonality < 0.2) {
    const words = p.tonality < 0.2 ? "Can't tell: mostly noise, no clear notes" : "Can't tell: the notes do not settle on a key";
    return { id: 'key', label: 'Key', words, detail: k ? `${keyLabel(k.key)} at confidence ${k.confidence.toFixed(2)}` : 'no key found', cantTell: true };
  }
  if (k.confidence < KEY_SURE) {
    return { id: 'key', label: 'Key', words: "Can't tell: the notes do not settle on a key", detail: `${keyLabel(k.key)} at confidence ${k.confidence.toFixed(2)}`, cantTell: true };
  }
  return { id: 'key', label: 'Key', words: `${keyLabel(k.key)}${k.confidence < 0.6 ? ' (not sure)' : ''}`, detail: `${keyLabel(k.key)}, confidence ${k.confidence.toFixed(2)}` };
}

export function loudnessLine(p: SoundProfile): ProfileLine {
  const l = p.loudness;
  const i = l.integrated_lufs;
  const level = !fin(i) ? 'Silent' : i < -30 ? 'Very quiet' : i < -20 ? 'Quiet' : i < -12 ? 'Moderate' : i < -8 ? 'Loud' : 'Very loud, heavily limited';
  const range = !fin(l.range_lu) ? '' : l.range_lu < 3 ? ', a steady level' : l.range_lu < 8 ? ', some light and shade' : ', wide swings in level';
  return {
    id: 'loudness',
    label: 'Loudness',
    words: `${level}${range}${fin(i) ? ` (${one(i)} LUFS)` : ''}`,
    detail: `${fin(i) ? one(i) : '–'} LUFS, range ${fin(l.range_lu) ? one(l.range_lu) : '–'} LU, true peak ${fin(l.true_peak_dbtp) ? one(l.true_peak_dbtp) : '–'} dBTP, RMS ${fin(l.rms_dbfs) ? one(l.rms_dbfs) : '–'} dBFS, crest ${fin(l.crest_db) ? one(l.crest_db) : '–'} dB`,
  };
}

export function brightnessWord(centroidHz: number): string {
  if (!fin(centroidHz) || centroidHz <= 0) return 'no clear colour';
  if (centroidHz < 700) return 'dark';
  if (centroidHz < 1500) return 'warm';
  if (centroidHz < 3000) return 'balanced';
  if (centroidHz < 5000) return 'bright';
  return 'very bright';
}

export function brightnessLine(p: SoundProfile): ProfileLine {
  const s = p.spectral;
  const word = brightnessWord(s.centroid_hz);
  const tilt = !fin(s.tilt_db_per_oct) ? '' : s.tilt_db_per_oct < -6 ? ', the highs fall away fast' : s.tilt_db_per_oct > -1.5 ? ', plenty of top end' : '';
  return {
    id: 'brightness',
    label: 'Brightness',
    words: `${word[0].toUpperCase()}${word.slice(1)}${tilt}`,
    detail: `centroid ${Math.round(s.centroid_hz)} Hz, 85% roll-off ${Math.round(s.rolloff_hz)} Hz, tilt ${fin(s.tilt_db_per_oct) ? one(s.tilt_db_per_oct) : '–'} dB/oct, flatness ${fin(s.flatness) ? s.flatness.toFixed(2) : '–'}`,
  };
}

export function roomWord(rt60: number): string {
  if (rt60 < 0.3) return 'dry, close up';
  if (rt60 < 0.8) return 'a small room';
  if (rt60 < 1.8) return 'a room';
  if (rt60 < 3.5) return 'a large hall';
  return 'a huge space';
}

export function widthWord(width: number): string {
  if (!fin(width) || width < 0.03) return 'mono';
  if (width < 0.25) return 'narrow';
  if (width < 0.6) return 'wide';
  return 'very wide';
}

export function spaceLine(p: SoundProfile): ProfileLine {
  const r = p.reverb;
  const w = p.stereo;
  const room = r && fin(r.rt60_s) && r.confidence >= 0.3 ? `${roomWord(r.rt60_s)}${r.rt60_s >= 0.3 ? ` (about ${one(r.rt60_s)} s of reverb)` : ''}` : "room can't tell";
  const width = widthWord(w.width);
  const phase = fin(w.correlation) && w.correlation < 0 ? ', some left and right cancel out' : '';
  const roomCap = room[0].toUpperCase() + room.slice(1);
  return {
    id: 'space',
    label: 'Space',
    words: `${roomCap}, ${width}${phase}`,
    detail: `${r ? `reverb ${one(r.rt60_s)} s at confidence ${r.confidence.toFixed(2)}` : 'no reverb decay found'}, width ${fin(w.width) ? w.width.toFixed(2) : '–'}, L/R correlation ${fin(w.correlation) ? w.correlation.toFixed(2) : '–'}`,
  };
}

const GRID_WORD: Record<string, string> = { quarter: 'quarter notes', eighth: 'eighths', sixteenth: 'sixteenths', eighth_triplet: 'eighth triplets', sixteenth_triplet: 'sixteenth triplets' };

/** Which beats carry the accents ("beats 1 and 3", "off the beat"). */
export function accentWords(stepWeights: number[], stepsPerBar: number, beatsPerBar: number): string {
  const n = stepWeights.length;
  if (!n || !stepsPerBar || !beatsPerBar) return '';
  const per = stepsPerBar / beatsPerBar;
  const strong = stepWeights.map((w, i) => ({ w, i })).filter((s) => s.w >= 0.7);
  if (!strong.length) return '';
  const on = strong.filter((s) => Number.isInteger(s.i / per)).map((s) => s.i / per + 1);
  const off = strong.length - on.length;
  if (!on.length) return 'accents off the beat';
  const list = on.length === 1 ? `beat ${on[0]}` : `beats ${on.slice(0, -1).join(', ')} and ${on.at(-1)}`;
  return off ? `accents on ${list} and off the beat` : `accents on ${list}`;
}

export function grooveLine(p: SoundProfile): ProfileLine {
  const g = p.groove;
  const tempoSure = !!p.tempo && p.tempo.confidence >= TEMPO_SURE;
  if (!g || !tempoSure) {
    const n = p.onsets_s.length;
    const per = p.source.duration_s > 0 ? n / p.source.duration_s : 0;
    const words = n === 0 ? 'No hits, it flows' : per < 0.5 ? `A few separate events (${n})` : `Events without a steady grid (${n})`;
    return { id: 'groove', label: 'Groove', words, detail: `${n} onsets in ${one(p.source.duration_s)} s`, cantTell: !g && n > 0 };
  }
  const feel = g.swing < 0.15 ? 'Straight' : g.swing < 0.45 ? 'A light swing' : 'Swung';
  const density = g.onsets_per_bar < 3 ? 'sparse' : g.onsets_per_bar <= 10 ? 'steady' : 'busy';
  const accents = accentWords(g.step_weights, g.steps_per_bar, p.tempo?.beats_per_bar ?? 4);
  return {
    id: 'groove',
    label: 'Groove',
    words: `${feel} ${GRID_WORD[g.grid] ?? g.grid}, ${density}${accents ? `, ${accents}` : ''}`,
    detail: `grid ${g.grid}, ${g.steps_per_bar} steps a bar, swing ${g.swing.toFixed(2)}, ${one(g.onsets_per_bar)} onsets a bar`,
  };
}

export function shapeLine(p: SoundProfile): ProfileLine {
  const e = p.envelope;
  const attack = e.attack_s < 0.01 ? 'A sharp attack' : e.attack_s < 0.08 ? 'A quick attack' : e.attack_s < 0.5 ? 'A soft attack' : 'A slow swell';
  const body = e.sustain > 0.6 ? 'holds steady' : e.sustain > 0.25 ? 'settles' : 'falls away';
  const tail = e.release_s > 1.5 ? 'a long tail' : e.release_s > 0.4 ? 'a short tail' : 'stops cleanly';
  return {
    id: 'shape',
    label: 'Shape',
    words: `${attack}, ${body}, ${tail}`,
    detail: `attack ${(e.attack_s * 1000).toFixed(0)} ms, decay ${(e.decay_s * 1000).toFixed(0)} ms, sustain ${Math.round(e.sustain * 100)}%, release ${(e.release_s * 1000).toFixed(0)} ms`,
  };
}

export function toneLine(p: SoundProfile): ProfileLine {
  const t = p.tonality;
  const voiced = p.pitch.midi.filter((m) => m !== null).length;
  const share = p.pitch.midi.length ? voiced / p.pitch.midi.length : 0;
  const words = t > 0.7 ? 'Clear notes' : t > 0.35 ? 'Notes and noise together' : 'Mostly noise, few clear notes';
  return { id: 'tone', label: 'Tone', words, detail: `tonality ${t.toFixed(2)}, a clear pitch in ${Math.round(share * 100)}% of frames` };
}

export function sectionsLine(p: SoundProfile): ProfileLine {
  const s = p.sections;
  if (s.length <= 1) return { id: 'sections', label: 'Sections', words: 'One section throughout', detail: s.length ? `${s[0].label} ${clockS(s[0].start_s)}–${clockS(s[0].end_s)}` : 'no boundaries found' };
  return {
    id: 'sections',
    label: 'Sections',
    words: `${s.length} sections, changing at ${s
      .slice(1)
      .map((x) => clockS(x.start_s))
      .join(', ')}`,
    detail: s.map((x) => `${x.label} ${clockS(x.start_s)}–${clockS(x.end_s)}`).join(', '),
  };
}

/** Every line of a profile, in reading order. */
export function profileLines(p: SoundProfile): ProfileLine[] {
  return [kindLine(p), tempoLine(p), keyLine(p), loudnessLine(p), brightnessLine(p), spaceLine(p), grooveLine(p), shapeLine(p), toneLine(p), sectionsLine(p)];
}

/** One line for a chip or a library row: "Ambience · 12 s · dark · a large hall". */
export function profileSummary(p: SoundProfile): string {
  const parts: string[] = [kindWord(p.class.kind), lengthWords(p.source.duration_s)];
  if (p.tempo && hasBeat(p)) parts.push(`${Math.round(p.tempo.bpm)} BPM`);
  if (p.key && p.key.confidence >= KEY_SURE && p.tonality >= 0.2) parts.push(keyLabel(p.key.key));
  parts.push(brightnessWord(p.spectral.centroid_hz));
  if (p.reverb && p.reverb.confidence >= 0.3) parts.push(roomWord(p.reverb.rt60_s));
  return parts.filter(Boolean).join(' · ');
}

// ---------------------------------------------------------------- loudness-matched A/B

export interface AbLevels {
  /** Trim on the take, dB (0 or below). */
  takeDb: number;
  /** Trim on the reference clip, dB (0 or below). */
  clipDb: number;
  /** The reference clip's <audio> volume (0..1). */
  clipVolume: number;
  /** True when both loudnesses were measured and matched. */
  matched: boolean;
  /** Plain words: which one was turned down and by how much. */
  words: string;
}

/** Below this the two already sound equally loud; nothing is turned down. */
const MATCH_EPS_DB = 0.25;

/**
 * Matches the take and the reference by integrated loudness (FR-30): the louder one is turned
 * down by the difference, so neither wins by being louder. Never turns anything up (no clipping).
 */
export function abLevels(takeLufs: number | null | undefined, clipLufs: number | null | undefined): AbLevels {
  if (!fin(takeLufs) || !fin(clipLufs)) {
    return { takeDb: 0, clipDb: 0, clipVolume: 1, matched: false, words: `Levels not matched: ${!fin(clipLufs) ? 'the reference' : 'the take'} has no measured loudness.` };
  }
  const d = takeLufs - clipLufs;
  if (Math.abs(d) < MATCH_EPS_DB) {
    return { takeDb: 0, clipDb: 0, clipVolume: 1, matched: true, words: `Levels matched: both play at ${one(takeLufs)} LUFS.` };
  }
  const takeDb = d > 0 ? -d : 0;
  const clipDb = d < 0 ? d : 0;
  const which = d > 0 ? 'the take' : 'the reference';
  return {
    takeDb,
    clipDb,
    clipVolume: Math.min(1, Math.max(0, 10 ** (clipDb / 20))),
    matched: true,
    words: `Levels matched at ${one(Math.min(takeLufs, clipLufs))} LUFS: ${which} plays ${one(Math.abs(d))} dB quieter so the louder one does not win.`,
  };
}

// ---------------------------------------------------------------- a reference with a request

const MARK = /\n\nReference (clip|style) "([^"\n]*)"/;

/** The profile's key numbers in one short line (never the tracks: a full profile is ~120 kB). */
export function keyNumbers(p: SoundProfile): string {
  const out: string[] = [];
  const t = p.tempo;
  if (t && fin(t.bpm)) out.push(`tempo ${one(t.bpm)} BPM at confidence ${t.confidence.toFixed(2)}`);
  if (p.key) out.push(`key ${keyLabel(p.key.key)} at confidence ${p.key.confidence.toFixed(2)}`);
  const l = p.loudness;
  if (fin(l.integrated_lufs)) out.push(`${one(l.integrated_lufs)} LUFS, range ${fin(l.range_lu) ? one(l.range_lu) : '–'} LU, true peak ${fin(l.true_peak_dbtp) ? one(l.true_peak_dbtp) : '–'} dBTP`);
  out.push(`centroid ${Math.round(p.spectral.centroid_hz)} Hz, tilt ${fin(p.spectral.tilt_db_per_oct) ? one(p.spectral.tilt_db_per_oct) : '–'} dB/oct`);
  out.push(`tonality ${p.tonality.toFixed(2)}`);
  if (p.reverb) out.push(`reverb ${one(p.reverb.rt60_s)} s at confidence ${p.reverb.confidence.toFixed(2)}`);
  out.push(`width ${fin(p.stereo.width) ? p.stereo.width.toFixed(2) : '–'}`);
  if (p.groove) out.push(`swing ${p.groove.swing.toFixed(2)}`);
  out.push(`attack ${Math.round(p.envelope.attack_s * 1000)} ms, release ${Math.round(p.envelope.release_s * 1000)} ms`);
  return out.join('; ');
}

/**
 * The words a request carries when it has a reference: a short version of its profile (the
 * analyzer's summary, each line in plain words, the key numbers), which the planner reads for
 * tempo, key and character and the assistant reads as the reference; for a dropped clip also the
 * kept file (for matching). Never the whole profile. Appended after a blank line, so the thread
 * can show the request as typed (see [`splitReference`]).
 */
export function referenceRequestText(ref: ReferenceClip): string {
  const p = ref.profile;
  const name = ref.name.replace(/["\n]/g, "'");
  const lines = profileLines(p).filter((l) => l.id !== 'sections');
  const said = lines.map((l) => `${l.label.toLowerCase()}: ${l.words.charAt(0).toLowerCase()}${l.words.slice(1)}`).join('; ');
  const head = ref.path ? `Reference clip "${name}"` : `Reference style "${name}" (from the reference library)`;
  const summary = p.summary ? ` (${p.summary.replace(/\n/g, ' ')})` : '';
  const tail = ref.path ? ` The clip's file, for matching: ${ref.path}` : '';
  return `\n\n${head}${summary}: keep its character, never its melody. ${lengthWords(p.source.duration_s)} long; ${said}. Measured: ${keyNumbers(p)}.${tail}`;
}

/** A request as typed, plus its reference's words. */
export function withReference(text: string, ref: ReferenceClip | null | undefined): string {
  return ref ? `${text}${referenceRequestText(ref)}` : text;
}

/** Splits a take's request into what was typed and the reference it carried (by name). */
export function splitReference(ask: string): { ask: string; reference: string | null } {
  const m = MARK.exec(ask);
  if (!m) return { ask, reference: null };
  return { ask: ask.slice(0, m.index), reference: m[2] };
}

/** A library style a request names in quotes ("make something like 'rain loop'"), if any. */
export function libraryMention(text: string, entries: { id: string; name: string }[]): string | null {
  const quoted = [...text.matchAll(/["'“‘]([^"'”’]{1,80})["'”’]/g)].map((m) => m[1].trim().toLowerCase());
  for (const q of quoted) {
    const e = entries.find((x) => x.name.trim().toLowerCase() === q);
    if (e) return e.id;
  }
  return null;
}
