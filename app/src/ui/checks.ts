// Plain-language summary of a take's measured checks ("8 of 8 checks passed: in key ...").
// Counts come from the report itself, never from separate state. Checks the material cannot
// decide ("can't tell from these notes") count neither as passed nor as failed.

import type { VerifyCheck, VerifyReport } from '../engine/EngineAdapter';

export interface ChecksSummary {
  /** Decided checks that passed. */
  passed: number;
  /** Decided checks (passed or failed). */
  total: number;
  /** Checks the material cannot decide. */
  unsure: number;
  /** Every decided check passed. */
  allPassed: boolean;
  /** "8 of 8 checks passed", "2 of 8 checks failed" (decided checks only). */
  label: string;
  /**
   * "2 can't tell from these notes: tempo (the rhythm doesn't clearly show 128 BPM), key (no
   * third, so A minor vs A major can't be heard)"; empty when every check was decided.
   */
  cantTell: string;
  /** The whole line: label, then cantTell after " · " when there is one. */
  line: string;
  /**
   * One short phrase per item, in report order: the can't-tell sentence first (when there is
   * one, `unsure: true`), then one per decided check ("in key (a minor)").
   */
  items: { text: string; passed: boolean; unsure?: boolean }[];
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

/** Separates an inconclusive check's measurement from its reason (verify.rs `CANT_TELL`). */
const CANT_TELL = "; can't tell: ";

/**
 * Whether the material could not decide `c`. Falls back on the reason in `measured` for payloads
 * that do not carry the `inconclusive` field through.
 */
export function isInconclusive(c: VerifyCheck): boolean {
  return c.inconclusive === true || (c.inconclusive === undefined && !c.passed && c.measured.includes(CANT_TELL));
}

/** Why an inconclusive check can't be told, in plain words. */
export function cantTellReason(c: VerifyCheck): string {
  const i = c.measured.indexOf(CANT_TELL);
  return i >= 0 ? c.measured.slice(i + CANT_TELL.length) : "the material doesn't show it clearly";
}

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
  const decided = report.checks.filter((c) => !isInconclusive(c));
  const unsure = report.checks.filter(isInconclusive);
  const total = decided.length;
  const passed = decided.filter((c) => c.passed).length;
  const label = passed === total ? `${passed} of ${total} checks passed` : `${total - passed} of ${total} checks failed`;
  const cantTell = unsure.length
    ? `${unsure.length} can't tell from these notes: ${unsure.map((c) => `${c.name.replace(/_/g, ' ')} (${cantTellReason(c)})`).join(', ')}`
    : '';
  const items: ChecksSummary['items'] = decided.map((c) => ({ text: phrase(c, report), passed: c.passed }));
  if (cantTell) items.unshift({ text: cantTell, passed: true, unsure: true });
  return {
    passed,
    total,
    unsure: unsure.length,
    allPassed: passed === total,
    label,
    cantTell,
    line: cantTell ? `${label} · ${cantTell}` : label,
    items,
  };
}
