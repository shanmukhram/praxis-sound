// MOCK PLANNER. A handful of keyword rules standing in for the language-model planner (WP-P/P2),
// so the take flow can be built and tested now. Every rule builds a ScorePatch through the same
// patch builder the UI uses; the mock `apply` then enforces the lock and scope rules, so a
// request on a locked layer is refused here exactly as the real engine would refuse it.

import type { Effect, GeneratorClass, Key, Mode, NLayer, NScore, Note, Origin, Pattern, PatchOp, PitchClass, Score, ScorePatch } from '../../score/types';
import { formatRange, normalizeScore } from '../../score/types';
import { derivePattern, editPatternInRange, freshPatternId, layerName } from '../../score/patchBuilder';
import { inKey, isUnpitchedClass, modeIntervals, semitone, stepsPerBar } from '../../score/theory';
import type { TakeScope, ViewName } from '../EngineAdapter';

export type Plan =
  | { ok: true; patch: ScorePatch; title: string; say: string; opens?: ViewName }
  | { ok: false; message: string };

const UNDERSTOOD =
  'The mock engine understands: a request on dragged bars (lift, soften, accent), "change the last four bars", "darker" / "brighter", "more room" / "drier", "more energy" / "calmer", "kick quieter" / "louder", "add a quiet glass melody from bar 9", and "try variations". The real planner understands plain language.';

const bars = (a: number, b: number) => (a === b ? `bar ${a}` : `bars ${a} to ${b}`);

/** First pitched layer (the one most requests mean), else the first layer. */
function mainLayer(score: NScore): NLayer | undefined {
  return score.layers.find((l) => !isUnpitchedClass(l.class)) ?? score.layers[0];
}

function extent(layer: NLayer): [number, number] | null {
  if (!layer.clips.length) return null;
  return [Math.min(...layer.clips.map((c) => c.bars.start)), Math.max(...layer.clips.map((c) => c.bars.end))];
}

/** Next pitch above `p` in the key (chromatic: a whole tone). */
function stepUp(key: Key, p: number): number {
  for (let q = p + 1; q <= Math.min(127, p + 4); q++) if (key.mode === 'chromatic' ? q === p + 2 : inKey(key, q)) return q;
  return p;
}

/** The note transforms a scoped request can mean. */
type Transform = 'lift' | 'soften' | 'accent' | 'energy_up' | 'energy_down';

function transformNotes(score: NScore, layer: NLayer, notes: Note[], t: Transform, spb: number): Note[] {
  const drum = isUnpitchedClass(layer.class);
  switch (t) {
    case 'lift': {
      if (drum) {
        // A lift on drums: a soft extra hit in the second half of each bar.
        const pitch = notes[0]?.pitch ?? 36;
        const extra = new Set(notes.map((n) => n.step));
        const out = [...notes];
        const len = Math.max(...notes.map((n) => n.step + 1), spb);
        for (let s = Math.floor(spb * 0.75); s < len; s += spb) {
          if (!extra.has(s)) out.push({ step: s, pitch, velocity: 0.45, accent: 'soft', length: 1 });
        }
        return out;
      }
      // Raise the inner (soft) notes one scale step so the line climbs.
      return notes.map((n) => (n.accent === 'soft' ? { ...n, pitch: stepUp(score.key, n.pitch) } : n));
    }
    case 'soften':
    case 'energy_down':
      return notes.map((n) => ({ ...n, velocity: Math.max(0.05, Math.round(n.velocity * 0.75 * 100) / 100) }));
    case 'energy_up':
      return notes.map((n) => ({ ...n, velocity: Math.min(1, Math.round(n.velocity * 1.15 * 100) / 100) }));
    case 'accent':
      return notes.map((n, i) => (i % 4 === 0 ? { ...n, accent: 'strong' } : n));
  }
}

const TRANSFORM_WORDS: Record<Transform, string> = {
  lift: 'Raised the inner notes so they climb toward the next bar',
  soften: 'Played the notes more softly',
  accent: 'Put a strong accent on the first note of each group',
  energy_up: 'Played every note a little harder',
  energy_down: 'Played every note a little softer',
};

/** Patch descriptions; the builder appends "in bars X of <layer>". */
const VERB: Record<Transform, string> = { lift: 'Lift', soften: 'Soften', accent: 'Accent', energy_up: 'More energy', energy_down: 'Calmer' };

function pickTransform(text: string): Transform {
  if (/lift|climb|raise|rise|higher/.test(text)) return 'lift';
  if (/soft|quiet|gentle|darker/.test(text)) return 'soften';
  if (/accent|punch|strong/.test(text)) return 'accent';
  return 'lift';
}

function noteEdit(score: NScore, layer: NLayer, range: { start: number; end: number }, t: Transform, origin: Origin, opens: ViewName, title: string): Plan {
  const d = derivePattern(score, layer.id, range);
  if (!d) return { ok: false, message: `There is no layer called ${layer.id}.` };
  if (!d.pattern.notes.length) return { ok: false, message: `${layerName(score, layer.id)} plays nothing in ${bars(range.start, range.end)}, so there is nothing to change there.` };
  const notes = transformNotes(score, layer, d.pattern.notes, t, d.stepsPerBar);
  const patch = editPatternInRange(score, layer.id, range, notes, VERB[t]);
  if (!patch) return { ok: false, message: `That would not change anything in ${bars(range.start, range.end)}.` };
  return {
    ok: true,
    patch: { ...patch, origin },
    title,
    say: `${TRANSFORM_WORDS[t]} in ${layerName(score, layer.id).toLowerCase()}, ${bars(range.start, range.end)}. Opened ${opens === 'notes' ? 'Notes' : 'Rhythm'} so you can see what moved.`,
    opens,
  };
}

function layerPatch(layer: NLayer, update: Record<string, unknown>, origin: Origin, description: string): ScorePatch {
  return { description, origin, scope: { layers: [layer.id] }, ops: [{ op: 'update_layer', id: layer.id, update }] };
}

function withEffect(layer: NLayer, kind: Effect['kind'], id: string, change: (params: Record<string, number>) => Record<string, number>): Effect[] {
  const fx = layer.effects.map((e) => ({ ...e, params: { ...(e.params ?? {}) } }));
  const i = fx.findIndex((e) => e.kind === kind);
  if (i >= 0) fx[i].params = change(fx[i].params!);
  else fx.push({ id, kind, params: change({}) });
  return fx;
}

/** Plans an edit of an existing sound. */
export function planEdit(score: NScore, text: string, scope: TakeScope | undefined, origin: Origin): Plan {
  const t = text.toLowerCase();

  if (scope) {
    const layer = score.layers.find((l) => l.id === scope.layer);
    if (!layer) return { ok: false, message: `There is no layer called ${scope.layer}.` };
    const tr = pickTransform(t);
    const opens: ViewName = isUnpitchedClass(layer.class) ? 'rhythm' : 'notes';
    const title = `${tr === 'lift' ? 'Lift' : tr === 'soften' ? 'Softer' : 'Accents'} in ${formatRange(scope.bars)}`;
    return noteEdit(score, layer, scope.bars, tr, origin, opens, title);
  }

  if (/variation/.test(t)) {
    return {
      ok: true,
      patch: { description: 'New seed', origin, scope: { layers: [] }, ops: [{ op: 'set_seed', seed: score.seed + 1 }] },
      title: 'Variation',
      say: `Same notes with a new seed (${score.seed + 1}) for every random draw: humanise, noise, round-robin. In the mock preview it sounds the same; the real engine varies the timing and tone.`,
    };
  }

  if (/last (four|4) bars/.test(t)) {
    const layer = mainLayer(score);
    if (!layer || score.bars < 4) return { ok: false, message: 'This sound is too short to change its last four bars.' };
    const range = { start: score.bars - 3, end: score.bars };
    return noteEdit(score, layer, range, 'lift', origin, isUnpitchedClass(layer.class) ? 'rhythm' : 'notes', `New last four bars`);
  }

  if (/melody/.test(t)) {
    if (score.layers.some((l) => l.id === 'melody')) return { ok: false, message: 'This sound already has a melody layer.' };
    return addMelody(score, t, origin);
  }

  if (/kick|drum/.test(t) && /quiet|soft|louder|loud|strong/.test(t)) {
    const kick = score.layers.find((l) => isUnpitchedClass(l.class));
    if (!kick) return { ok: false, message: 'There is no drum layer in this sound.' };
    const up = /louder|loud|strong/.test(t) && !/quiet|soft/.test(t);
    const gain = Math.max(-60, Math.min(24, kick.gain_db + (up ? 3 : -3)));
    return {
      ok: true,
      patch: layerPatch(kick, { gain_db: gain }, origin, up ? 'Louder kick' : 'Quieter kick'),
      title: up ? 'Louder kick' : 'Quieter kick',
      say: `Turned ${layerName(score, kick.id).toLowerCase()} ${up ? 'up' : 'down'} 3 dB, to ${gain.toFixed(1)} dB. Same hits, same rhythm. Opened Mix so you can see the level.`,
      opens: 'mix',
    };
  }

  if (/dark|bright/.test(t)) {
    const layer = mainLayer(score);
    if (!layer) return { ok: false, message: 'There is nothing to make darker yet.' };
    const darker = /dark/.test(t);
    const effects = withEffect(layer, 'eq', 'tone', (p) => {
      const cur = p.lp_hz ?? 12000;
      return { ...p, lp_hz: Math.round(Math.max(600, Math.min(16000, darker ? cur * 0.6 : cur * 1.6))) };
    });
    const hz = effects.find((e) => e.kind === 'eq')!.params!.lp_hz;
    return {
      ok: true,
      patch: layerPatch(layer, { effects }, origin, darker ? 'A little darker' : 'A little brighter'),
      title: darker ? 'A little darker' : 'A little brighter',
      say: `${darker ? 'Closed' : 'Opened'} the tone of ${layerName(score, layer.id).toLowerCase()} to ${hz} Hz. Same notes, same rhythm. Opened Mix so you can see the level held at its target.`,
      opens: 'mix',
    };
  }

  if (/room|space|reverb|dry|drier/.test(t)) {
    const layer = mainLayer(score);
    if (!layer) return { ok: false, message: 'There is nothing here yet.' };
    const more = !/dry|drier|less/.test(t);
    const effects = withEffect(layer, 'reverb', 'room', (p) => ({ ...p, decay_s: p.decay_s ?? 2.4, mix: Math.round(Math.max(0, Math.min(1, (p.mix ?? 0.2) + (more ? 0.1 : -0.1))) * 100) / 100 }));
    return {
      ok: true,
      patch: layerPatch(layer, { effects }, origin, more ? 'More room' : 'Drier'),
      title: more ? 'More room' : 'Drier',
      say: `${more ? 'More' : 'Less'} reverb on ${layerName(score, layer.id).toLowerCase()}. The mock preview does not render reverb; the real engine does.`,
    };
  }

  if (/energy|harder|driving|calmer|still/.test(t)) {
    const layer = mainLayer(score);
    const ext = layer && extent(layer);
    if (!layer || !ext) return { ok: false, message: 'There is nothing playing to change.' };
    const up = !/calmer|still|less/.test(t);
    return noteEdit(score, layer, { start: ext[0], end: ext[1] }, up ? 'energy_up' : 'energy_down', origin, 'rhythm', up ? 'More energy' : 'Calmer');
  }

  if (/mood|tense|calm/.test(t)) return { ok: false, message: 'Changing the mood needs the real planner; the mock engine cannot do it yet.' };

  return { ok: false, message: UNDERSTOOD };
}

function addMelody(score: NScore, t: string, origin: Origin): Plan {
  const m = t.match(/from bar (\d+)/);
  const from = Math.max(1, Math.min(score.bars, m ? Number(m[1]) : Math.min(9, score.bars)));
  const grid = stepsPerBar(score.time_signature, 'quarter') !== null ? 'quarter' : 'eighth';
  const spb = stepsPerBar(score.time_signature, grid)!;
  const base = 72 + semitone(score.key.tonic) - 12 * (semitone(score.key.tonic) > 6 ? 1 : 0);
  const deg = (d: number) => degreeToMidi(score.key, base, d);
  const id = score.patterns.some((p) => p.id === 'glass_melody') ? freshPatternId(score, 'melody', { start: from, end: score.bars }) : 'glass_melody';
  const notes: Note[] = [
    { step: 0, pitch: deg(4), velocity: 0.35, accent: 'normal', length: spb * 0.9 },
    { step: spb, pitch: deg(2), velocity: 0.3, accent: 'soft', length: spb * 0.9 },
  ];
  const pattern: Pattern = { id, grid, length_steps: spb * 2, notes };
  const layer = {
    id: 'melody',
    name: 'Glass melody',
    class: 'melody' as GeneratorClass,
    instrument: { kind: 'synth' as const, preset: 'glass', params: {} },
    clips: [{ bars: { start: from, end: score.bars }, pattern: id }],
    gain_db: -8,
    pan: 0,
  };
  const ops: PatchOp[] = [
    { op: 'add_pattern', pattern },
    { op: 'add_layer', layer },
  ];
  return {
    ok: true,
    patch: { description: 'Add a quiet glass melody', origin, scope: { layers: ['melody'] }, ops },
    title: 'Quiet glass melody',
    say: `Added a quiet glass melody in ${bars(from, score.bars)}, two long notes per two bars, in key and below the loudest notes. Nothing else changed.`,
  };
}

/** MIDI note of scale degree `d` above `base` (degree 0 = base). */
export function degreeToMidi(key: Key, base: number, d: number): number {
  const iv = key.mode === 'chromatic' ? modeIntervals('minor') : modeIntervals(key.mode);
  const n = iv.length;
  const oct = Math.floor(d / n);
  const i = ((d % n) + n) % n;
  return base + oct * 12 + iv[i];
}

// ---------------------------------------------------------------- new sounds

const CLASS_WORDS: [RegExp, GeneratorClass][] = [
  [/risers?/, 'riser'],
  [/sweeps?/, 'sweep'],
  [/impacts?/, 'impact'],
  [/booms?/, 'boom'],
  [/pads?/, 'pad'],
  [/whoosh(es)?/, 'whoosh'],
  [/fills?/, 'fill'],
];

/** A new sound from words: key, tempo and length if given, a felt piano motif on an eighth grid. */
export function planFresh(text: string): { score: NScore; title: string; say: string } {
  const t = text.toLowerCase();
  const km = text.match(/\b([A-G])(#|b)?\s*(major|minor)\b/i);
  const flats: Record<string, PitchClass> = { 'C#': 'C#', Db: 'C#', 'D#': 'Eb', Eb: 'Eb', 'F#': 'F#', Gb: 'F#', 'G#': 'Ab', Ab: 'Ab', 'A#': 'Bb', Bb: 'Bb' };
  let tonic: PitchClass = 'A';
  let mode: Mode = 'minor';
  if (km) {
    const name = km[1].toUpperCase() + (km[2] ?? '');
    tonic = (flats[name] ?? name) as PitchClass;
    mode = km[3].toLowerCase() as Mode;
  }
  const key: Key = { tonic, mode };
  const secM = t.match(/(\d+)[- ]?(second|sec|s\b)/);
  const bpmM = t.match(/(\d{2,3})\s*bpm/);
  let bars = 16;
  let bpm = bpmM ? Number(bpmM[1]) : 128;
  if (secM && !bpmM) bpm = Math.round(((bars * 4 * 60) / Number(secM[1])) * 100) / 100;
  if (secM && bpmM) bars = Math.max(1, Math.round((Number(secM[1]) * bpm) / 240));
  bpm = Math.max(20, Math.min(400, bpm));
  const base = 48 + semitone(tonic);
  const deg = (d: number) => degreeToMidi(key, base, d);
  const shape = [0, 4, 7, 4, 9, 4, 7, 4, -2, 2, 5, 2, 7, 2, 6, 4];
  const acc: Note['accent'][] = ['strong', 'soft', 'normal', 'soft', 'bright', 'soft', 'normal', 'soft'];
  const notes: Note[] = shape.map((d, i) => ({ step: i, pitch: deg(d), velocity: [0.8, 0.55, 0.65, 0.55, 0.7, 0.55, 0.62, 0.52][i % 8], accent: acc[i % 8], length: 1.6 }));
  notes.push({ step: 0, pitch: deg(-7), velocity: 0.6, accent: 'normal', length: 8 }, { step: 8, pitch: deg(-9), velocity: 0.6, accent: 'normal', length: 8 });
  const avoid = CLASS_WORDS.filter(([re]) => new RegExp(`no ${re.source}|${re.source}`).test(t) && /\bno\b/.test(t)).map(([, c]) => c);
  const raw: Score = {
    schema_version: 1,
    title: 'Felt piano loop',
    seed: 1,
    meta: { brief: text, provenance: { source: 'mock planner' }, reference_profiles: [], tags: [] },
    tempo: { bpm },
    time_signature: { numerator: 4, denominator: 4 },
    key,
    bars,
    playback: /one[- ]shot|sting|hit/.test(t) ? { mode: 'one_shot', tail_seconds: 4 } : { mode: 'loop' },
    constraints: { avoid },
    sections: [],
    patterns: [{ id: 'piano_motif', grid: 'eighth', length_steps: 16, notes }],
    layers: [
      {
        id: 'piano',
        name: 'Felt piano',
        class: 'motif',
        instrument: { kind: 'sampler', preset: 'felt_piano', params: {} },
        clips: [{ bars: { start: 1, end: bars }, pattern: 'piano_motif' }],
        effects: [{ id: 'room', kind: 'reverb', params: { decay_s: 2.4, mix: 0.2 } }],
      },
    ],
    master: { effects: [], loudness: { target_lufs: -16, true_peak_dbtp: -1, normalize: true } },
    markers: [],
    chords: [],
  };
  const score = normalizeScore(raw);
  const secs = (bars * 4 * 60) / bpm;
  return {
    score,
    title: 'Felt piano loop',
    say: `One felt piano playing a two-bar motif on an eighth-note grid in ${tonic} ${mode}, strong on the first beat and bright on beat 3, each note ringing into the next. ${bars} bars at ${bpm} BPM, ${secs.toFixed(0)} seconds. (Mock planner: it always starts from this motif.)`,
  };
}
