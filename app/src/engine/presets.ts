// Instrument presets in plain words, for the layer picker (WP-U5). `STANDARD_PRESETS` mirrors the
// planner catalogue (crates/praxis-planner/src/catalogue.rs) for the mock engine; the desktop
// shell reads the real list, with availability, from `praxis presets`.

import type { GeneratorClass, InstrumentKind } from '../score/types';
import type { PresetInfo } from './EngineAdapter';

type Row = [kind: InstrumentKind, name: string, cls: GeneratorClass, pitched: boolean, description: string];

const ROWS: Row[] = [
  ['sampler', 'felt_piano', 'motif', true, 'sampled upright piano voiced soft with felt: intimate, rhythmic motifs'],
  ['sampler', 'upright_piano', 'melody', true, 'sampled upright piano, natural voicing'],
  ['synth', 'bell', 'motif', true, 'struck FM bell'],
  ['synth', 'soft_keys', 'chords', true, 'soft electric keys for chords'],
  ['synth', 'pluck', 'arp', true, 'plucked string model for arpeggios'],
  ['synth', 'mallet', 'motif', true, 'struck mallet (marimba / vibraphone-like)'],
  ['synth', 'sub_bass', 'bass', true, 'sine sub bass'],
  ['synth', 'warm_bass', 'bass', true, 'warm round synth bass'],
  ['synth', 'fm_keys', 'melody', true, 'FM electric piano for lead lines'],
  ['synth', 'glass', 'motif', true, 'glassy struck tone'],
  ['synth', 'granular_texture', 'pad', true, 'granular sustained pad'],
  ['synth', 'noise_bed', 'texture', false, 'filtered noise bed'],
  ['drum', 'soft_kick', 'drums', false, 'soft, muted kick: short sine sweep plus felt click, low-passed'],
  ['drum', 'kick', 'drums', false, 'synthesised kick'],
  ['drum', 'snare', 'drums', false, 'synthesised snare'],
  ['drum', 'rim', 'drums', false, 'rim click'],
  ['drum', 'hat_closed', 'drums', false, 'closed hi-hat'],
  ['drum', 'hat_open', 'drums', false, 'open hi-hat'],
  ['drum', 'shaker', 'percussion', false, 'shaker'],
  ['drum', 'tom_low', 'drums', false, 'low tom'],
  ['drum', 'tom_mid', 'drums', false, 'mid tom'],
  ['drum', 'tom_high', 'drums', false, 'high tom'],
  ['drum', 'clap', 'drums', false, 'hand clap'],
  ['drum', 'kit', 'drums', false, 'multi-voice kit keyed by the GM drum map (36 kick, 38 snare, 42 hat)'],
  ['sfx', 'whoosh', 'whoosh', true, 'procedural whoosh'],
  ['sfx', 'impact', 'impact', true, 'procedural impact'],
  ['sfx', 'riser', 'riser', true, 'procedural riser'],
  ['sfx', 'drone', 'drone', true, 'tuned drone'],
  ['sfx', 'glitch', 'glitch', true, 'glitch texture'],
  ['sfx', 'tick', 'tick', false, 'clock tick'],
];

export const STANDARD_PRESETS: PresetInfo[] = ROWS.map(([kind, name, cls, pitched, description]) => ({ kind, name, class: cls, pitched, description, available: true }));

const LABEL: Record<string, string> = {
  felt_piano: 'Felt piano',
  upright_piano: 'Upright piano',
  bell: 'Bell',
  soft_keys: 'Soft keys',
  pluck: 'Pluck',
  mallet: 'Mallet',
  sub_bass: 'Sub bass',
  warm_bass: 'Warm bass',
  fm_keys: 'Electric piano',
  glass: 'Glass',
  granular_texture: 'Granular pad',
  noise_bed: 'Noise bed',
  soft_kick: 'Soft kick',
  kick: 'Kick',
  snare: 'Snare',
  rim: 'Rim click',
  hat_closed: 'Closed hi-hat',
  hat_open: 'Open hi-hat',
  shaker: 'Shaker',
  tom_low: 'Low tom',
  tom_mid: 'Mid tom',
  tom_high: 'High tom',
  clap: 'Clap',
  kit: 'Drum kit',
  whoosh: 'Whoosh',
  impact: 'Impact',
  riser: 'Riser',
  drone: 'Drone',
  glitch: 'Glitch',
  tick: 'Clock tick',
};

/** "felt_piano" -> "Felt piano"; an unknown preset reads as its name in words. */
export function presetLabel(name: string): string {
  if (LABEL[name]) return LABEL[name];
  const words = name.replace(/[_-]+/g, ' ').trim();
  return words ? words[0].toUpperCase() + words.slice(1) : name;
}

/** The two the owner reaches for first (acceptance test 1), shown above the groups. */
export const QUICK_PRESETS = ['felt_piano', 'soft_kick'];

export interface PresetGroup {
  name: string;
  presets: PresetInfo[];
}

const BASS = new Set(['bass']);
const DRUMS = new Set(['drums', 'percussion']);
const SFX = new Set(['whoosh', 'impact', 'riser', 'glitch', 'tick', 'boom', 'sweep', 'sfx']);
const TEXTURE = new Set(['pad', 'texture', 'drone', 'ambience']);

/** Plain-word groups in a fixed order: keys and bells, bass, drums, pads and textures, sound effects. */
export function groupPresets(list: PresetInfo[]): PresetGroup[] {
  const groups: PresetGroup[] = [
    { name: 'Keys, bells and plucks', presets: [] },
    { name: 'Bass', presets: [] },
    { name: 'Drums', presets: [] },
    { name: 'Pads and textures', presets: [] },
    { name: 'Sound effects', presets: [] },
  ];
  for (const p of list) {
    const g = DRUMS.has(p.class) || p.kind === 'drum' ? 2 : BASS.has(p.class) ? 1 : SFX.has(p.class) ? 4 : TEXTURE.has(p.class) || !p.pitched ? 3 : 0;
    groups[g].presets.push(p);
  }
  return groups.filter((g) => g.presets.length);
}

/** `praxis presets` payload -> PresetInfo[] (catalogue entries, then engine-only presets). */
export function presetsFromPayload(p: {
  presets?: { name: string; kind: InstrumentKind; class: GeneratorClass; pitched?: boolean; description?: string; available?: boolean; reason?: string | null }[];
  engine_only?: { name: string; kind: InstrumentKind; class: GeneratorClass }[];
}): PresetInfo[] {
  const out: PresetInfo[] = (p.presets ?? []).map((x) => {
    const info: PresetInfo = { name: x.name, kind: x.kind, class: x.class, pitched: x.pitched ?? true, description: x.description ?? '', available: x.available !== false };
    if (x.reason) info.reason = x.reason;
    return info;
  });
  for (const x of p.engine_only ?? []) {
    if (out.some((o) => o.name === x.name)) continue;
    out.push({ name: x.name, kind: x.kind, class: x.class, pitched: x.kind !== 'drum', description: '', available: true });
  }
  return out;
}
