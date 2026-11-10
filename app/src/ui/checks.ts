// Plain-language summary of a take's measured checks ("8 of 8 checks passed: in key ...").
// Counts come from the report itself, never from separate state.

import type { VerifyCheck, VerifyReport } from '../engine/EngineAdapter';

export interface ChecksSummary {
  passed: number;
  total: number;
  allPassed: boolean;
  /** "8 of 8 checks passed" */
  label: string;
  /** One short phrase per check, in report order: "in key (a minor)". */
  items: { text: string; passed: boolean }[];
}

const PHRASE: Record<string, string> = {
  tempo: 'on tempo',
  key: 'in key',
  grid: 'on the beat',
  loudness: 'loudness',
  true_peak: 'peak',
  clipping: 'no clipping',
  loop_seam: 'seamless loop',
  silence: 'not silent',
};

function phrase(c: VerifyCheck, report: VerifyReport): string {
  const name = PHRASE[c.name] ?? c.name.replace(/_/g, ' ');
  switch (c.name) {
    case 'key':
      return c.passed && report.key ? `${name} (${titleKey(report.key)})` : `${name}: ${c.measured}`;
    case 'grid':
      return report.onsets_on_grid !== null ? `${name} (${Math.round(report.onsets_on_grid * 100)}% of notes within 10 ms)` : name;
    case 'loudness':
      return report.integrated_lufs !== null ? `${name} (${report.integrated_lufs.toFixed(1)} LUFS)` : name;
    case 'true_peak':
      return report.true_peak_dbtp !== null ? `${name} (${report.true_peak_dbtp.toFixed(1)} dB)` : name;
    case 'loop_seam':
      return `${name} (${c.measured})`;
    case 'tempo':
      return report.tempo_bpm !== null ? `${name} (${Math.round(report.tempo_bpm)} BPM)` : name;
    default:
      return c.passed ? name : `${name}: ${c.measured}`;
  }
}

function titleKey(k: string): string {
  return k.charAt(0).toUpperCase() + k.slice(1);
}

export function summarizeChecks(report: VerifyReport): ChecksSummary {
  const total = report.checks.length;
  const passed = report.checks.filter((c) => c.passed).length;
  return {
    passed,
    total,
    allPassed: passed === total,
    label: passed === total ? `${passed} of ${total} checks passed` : `${total - passed} of ${total} checks failed`,
    items: report.checks.map((c) => ({ text: phrase(c, report), passed: c.passed })),
  };
}
