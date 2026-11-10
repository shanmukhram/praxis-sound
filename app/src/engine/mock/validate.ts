// A subset of crates/praxis-score/src/validate.rs, enough for the mock engine to reject the
// same broken results the real `apply` would. Codes use the Rust IssueCode names in snake_case.
// Not covered here (registry-dependent or not needed by the UI): effect params, automation,
// markers, loudness ranges, sidechains, preset checks.

import type { Issue } from '../EngineAdapter';
import type { BarRange, NScore } from '../../score/types';
import { SCHEMA_VERSION } from '../../score/types';
import { inKey, isUnpitchedClass, midiName, keyLabel, stepsPerBar } from '../../score/theory';

export interface Report {
  errors: Issue[];
  warnings: Issue[];
}

const ID_RE = /^[a-z0-9_-]{1,64}$/;

export function validate(score: NScore): Report {
  const errors: Issue[] = [];
  const warnings: Issue[] = [];
  const err = (code: string, path: string, message: string) => errors.push({ code, path, message });

  if (score.schema_version !== SCHEMA_VERSION) {
    err('schema_version', '/schema_version', `expected ${SCHEMA_VERSION}, got ${score.schema_version}`);
  }
  const bpm = score.tempo.bpm;
  if (!(Number.isFinite(bpm) && bpm >= 20 && bpm <= 400)) err('tempo_range', '/tempo/bpm', 'tempo must be 20..=400 BPM');
  const ts = score.time_signature;
  if (ts.numerator === 0 || ts.numerator > 32 || ![2, 4, 8, 16].includes(ts.denominator)) {
    err('time_signature', '/time_signature', 'numerator 1..=32, denominator 2, 4, 8 or 16');
  }
  if (score.bars === 0 || score.bars > 2048) err('bars_range', '/bars', 'bars must be 1..=2048');

  const checkRange = (br: BarRange, path: string) => {
    if (br.start === 0 || br.end < br.start || br.end > score.bars) {
      err('bar_range', path, `bar range ${br.start}-${br.end} outside 1-${score.bars}`);
    }
  };
  const idCheck = (seen: Set<string>, id: string, path: string) => {
    if (!ID_RE.test(id)) err('bad_id', path, `bad id \`${id}\`: use [a-z0-9_-], max 64`);
    if (seen.has(id)) err('duplicate_id', path, `duplicate id \`${id}\``);
    seen.add(id);
  };

  const sids = new Set<string>();
  score.sections.forEach((s, i) => {
    idCheck(sids, s.id, `/sections/${i}/id`);
    checkRange(s.bars, `/sections/${i}/bars`);
  });
  const secs = [...score.sections].sort((a, b) => a.bars.start - b.bars.start);
  for (let i = 1; i < secs.length; i++) {
    if (secs[i].bars.start <= secs[i - 1].bars.end) {
      err('section_overlap', '/sections', `sections \`${secs[i - 1].id}\` and \`${secs[i].id}\` overlap`);
    }
  }

  const pids = new Set<string>();
  score.patterns.forEach((p, i) => {
    const path = `/patterns/${i}`;
    idCheck(pids, p.id, `${path}/id`);
    const spb = stepsPerBar(ts, p.grid);
    if (spb === null) err('grid_mismatch', `${path}/grid`, 'grid does not divide the bar');
    else if (p.length_steps === 0 || p.length_steps % spb !== 0) {
      err('grid_mismatch', `${path}/length_steps`, `length_steps must be a positive multiple of ${spb} (one bar)`);
    }
    p.notes.forEach((n, j) => {
      const np = `${path}/notes/${j}`;
      if (n.step >= p.length_steps) err('step_out_of_range', `${np}/step`, 'step beyond pattern');
      if (n.pitch > 127 || n.pitch < 0) err('pitch_range', `${np}/pitch`, 'pitch 0..=127');
      if (!(n.velocity > 0 && n.velocity <= 1)) err('velocity_range', `${np}/velocity`, 'velocity in (0, 1]');
      if (!(n.length > 0 && n.length <= 64)) err('length_range', `${np}/length`, 'length in (0, 64] steps');
      const off = n.offset ?? 0;
      if (!(off >= -0.5 && off <= 0.5)) err('offset_range', `${np}/offset`, 'offset -0.5..=0.5');
    });
  });

  const lids = new Set<string>();
  score.layers.forEach((l, i) => {
    const path = `/layers/${i}`;
    idCheck(lids, l.id, `${path}/id`);
    if ((score.constraints.avoid ?? []).includes(l.class)) {
      err('avoided_class', `${path}/class`, `class ${l.class} is on the avoid-list`);
    }
    if (!(l.pan >= -1 && l.pan <= 1)) err('pan_range', `${path}/pan`, 'pan -1..=1');
    if (!(l.gain_db >= -60 && l.gain_db <= 24)) err('param_range', `${path}/gain_db`, 'gain_db -60..=24');
    const pitched = !isUnpitchedClass(l.class) && score.key.mode !== 'chromatic';
    const spans: [number, number, number][] = [];
    l.clips.forEach((clip, c) => {
      const cp = `${path}/clips/${c}`;
      checkRange(clip.bars, `${cp}/bars`);
      spans.push([clip.bars.start, clip.bars.end, c]);
      const pi = score.patterns.findIndex((p) => p.id === clip.pattern);
      if (pi < 0) {
        err('unknown_pattern', `${cp}/pattern`, `unknown pattern \`${clip.pattern}\``);
        return;
      }
      const p = score.patterns[pi];
      if (pitched) {
        p.notes.forEach((n, j) => {
          const pitch = n.pitch + (clip.transpose ?? 0);
          if (pitch < 0 || pitch > 127) err('pitch_range', `${cp}/transpose`, 'transposed pitch out of range');
          else if (!inKey(score.key, pitch)) {
            err('off_scale', `/patterns/${pi}/notes/${j}/pitch`, `${midiName(pitch)} is not in ${keyLabel(score.key)} (layer \`${l.id}\`)`);
          }
        });
      }
      if ((clip.pattern_offset_steps ?? 0) >= Math.max(1, p.length_steps)) {
        err('step_out_of_range', `${cp}/pattern_offset_steps`, 'pattern_offset_steps must be below the pattern length');
      }
      const vs = clip.velocity_scale ?? 1;
      if (!(vs > 0 && vs <= 2)) err('velocity_range', `${cp}/velocity_scale`, '0 < scale <= 2');
    });
    spans.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    for (let k = 1; k < spans.length; k++) {
      if (spans[k][0] <= spans[k - 1][1]) err('overlapping_clips', `${path}/clips/${spans[k][2]}`, 'clips on one layer overlap');
    }
  });
  return { errors, warnings };
}
