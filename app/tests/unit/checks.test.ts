// "Can't tell" checks (WP-D1c): the take line counts only decided checks and says in plain words
// what the material could not decide.

import { describe, expect, it } from 'vitest';
import type { VerifyCheck, VerifyReport } from '../../src/engine/EngineAdapter';
import { toVerifyReport } from '../../src/engine/tauri/protocol';
import { verifyMock } from '../../src/engine/mock/verify';
import { summarizeChecks, isInconclusive } from '../../src/ui/checks';
import { fixture } from './fixtures';

const ok = (name: string, measured = 'm'): VerifyCheck => ({ name, passed: true, measured, expected: 'e', inconclusive: false });

/** The owner's hand-built loop as `praxis render` reports it (take 1 of "storyloop"). */
function ownerReport(): VerifyReport {
  return {
    checks: [
      {
        name: 'tempo',
        passed: false,
        inconclusive: true,
        measured: "85.34 bpm (confidence 0.42); can't tell: the rhythm doesn't clearly show 128 BPM",
        expected: '128.00 +- 2.00 bpm',
      },
      {
        name: 'key',
        passed: false,
        inconclusive: true,
        measured: "a major (confidence 0.58); can't tell: no third, so A minor vs A major can't be heard",
        expected: 'a minor',
      },
      ok('grid'),
      ok('loudness'),
      ok('true_peak'),
      ok('clipping'),
      ok('loop_seam', 'jump 0.00013'),
      ok('silence'),
    ],
    tempo_bpm: 85.339,
    key: 'a major',
    grid_deviation_ms: 0.08,
    onsets_on_grid: 1,
    integrated_lufs: -16.03,
    true_peak_dbtp: -1.02,
    seam_jump_ratio: 0.02,
    seam_excess_db: -15.1,
    passed: true,
  };
}

const LINE =
  "6 of 6 checks passed · 2 can't tell from these notes: tempo (the rhythm doesn't clearly show 128 BPM), key (no third, so A minor vs A major can't be heard)";

describe("checks the material can't decide", () => {
  it("say can't tell instead of failed, and count only decided checks", () => {
    const s = summarizeChecks(ownerReport());
    expect([s.passed, s.total, s.unsure, s.allPassed]).toEqual([6, 6, 2, true]);
    expect(s.label).toBe('6 of 6 checks passed');
    expect(s.line).toBe(LINE);
    // The can't-tell sentence comes first and is not shown as a failure; tempo and key get no
    // pass/fail phrase of their own.
    expect(s.items[0]).toEqual({ text: s.cantTell, passed: true, unsure: true });
    expect(s.items.slice(1).map((i) => i.text)).not.toContain('on tempo (85 BPM)');
    expect(s.items.filter((i) => !i.passed)).toEqual([]);
  });

  it('still report real failures next to them', () => {
    const r = ownerReport();
    r.checks[3] = { ...r.checks[3], passed: false, measured: '-19.95 LUFS' };
    const s = summarizeChecks(r);
    expect([s.passed, s.total, s.allPassed]).toEqual([5, 6, false]);
    expect(s.line.startsWith("1 of 6 checks failed · 2 can't tell from these notes: tempo")).toBe(true);
    expect(s.items.find((i) => !i.passed)?.text).toBe('loudness (-16.0 LUFS)');
  });

  it('carry through the desktop protocol', () => {
    const r = toVerifyReport(ownerReport());
    expect(summarizeChecks(r).line).toBe(LINE);
    // A payload without the field (an older app build) is read from the reason in `measured`.
    const old = { ...ownerReport(), checks: ownerReport().checks.map(({ inconclusive: _drop, ...c }) => c) };
    expect(old.checks.filter(isInconclusive).map((c) => c.name)).toEqual(['tempo', 'key']);
    expect(summarizeChecks(old).line).toBe(LINE);
  });

  it('a plain failure with no reason stays a failure', () => {
    const r = ownerReport();
    r.checks[1] = { name: 'key', passed: false, measured: 'b minor (confidence 0.55)', expected: 'a minor' };
    expect(isInconclusive(r.checks[1])).toBe(false);
    expect(summarizeChecks(r).label).toBe('1 of 7 checks failed');
  });

  it("the mock says can't tell for a key with no third", () => {
    const s = fixture();
    const piano = s.patterns.find((p) => s.layers.find((l) => l.id === 'piano')?.clips[0].pattern === p.id)!;
    // Only A and E: no third over A.
    piano.notes = piano.notes.map((n) => ({ ...n, pitch: n.pitch % 12 === 9 ? n.pitch : 64 }));
    const sr = 8000;
    const n = sr * 2;
    const r = verifyMock(s, [new Float32Array(n).fill(0.1), new Float32Array(n).fill(0.1)], sr, {
      loudness_lufs: -16,
      true_peak_dbtp: -1.1,
    } as never);
    const key = r.checks.find((c) => c.name === 'key')!;
    expect(key.inconclusive).toBe(true);
    expect(summarizeChecks(r).cantTell).toBe("1 can't tell from these notes: key (no third, so A minor vs A major can't be heard)");
  });
});
