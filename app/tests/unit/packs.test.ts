// The pack downloader's and the crash notice's words (app/src/ui/packs.ts, crashNotice.ts),
// fed payloads shaped like crates/praxis-app's `packs_list`, `praxis://packs` and
// `crash_reports`.

import { describe, expect, it } from 'vitest';
import { feltPianoMissing, megabytes, packStatus, type PackInfo } from '../../src/ui/packs';
import { crashNoticeText, type CrashReports } from '../../src/ui/crashNotice';

const piano: PackInfo = {
  name: 'upright-piano-kw',
  title: 'Upright Piano KW (FreePats), SFZ+FLAC',
  version: '2022-02-21',
  license: 'CC0-1.0',
  size: 33294287,
  presets: ['felt_piano', 'upright_piano'],
  attribution: 'Upright Piano KW by FreePats (freepats.zenvoid.org), released under CC0-1.0',
  source_page: 'https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html',
  installed: false,
  installed_version: null,
};

describe('pack downloader', () => {
  it('says what a missing pack costs to download', () => {
    expect(megabytes(33294287)).toBe('33.3 MB');
    expect(packStatus(piano)).toEqual({ text: 'Not installed · 33.3 MB download', busy: false, failed: false });
  });

  it('shows download progress, then the check and unpack', () => {
    const half = packStatus(piano, { pack: piano.name, phase: 'downloading', bytes: 16647143, total: 33294287 });
    expect(half).toEqual({ text: 'Downloading 16.6 MB of 33.3 MB (49%)', busy: true, failed: false });
    expect(packStatus(piano, { pack: piano.name, phase: 'unpacking', bytes: 1, total: 1 }).text).toBe('Checking the download and unpacking');
  });

  it('says why an install failed, in the command layer words', () => {
    const st = packStatus(piano, { pack: piano.name, phase: 'failed', bytes: 0, total: 0, message: 'blake3 mismatch' });
    expect(st).toEqual({ text: 'Not installed: blake3 mismatch', busy: false, failed: true });
  });

  it('shows the installed version', () => {
    expect(packStatus({ ...piano, installed: true, installed_version: '2022-02-21' }).text).toBe('Installed (version 2022-02-21)');
  });

  it('offers the felt piano only when its pack is missing', () => {
    expect(feltPianoMissing([piano])).toBe(true);
    expect(feltPianoMissing([{ ...piano, installed: true }])).toBe(false);
    expect(feltPianoMissing([])).toBe(false);
  });
});

describe('crash notice', () => {
  const base: CrashReports = { dir: 'C:\\x\\logs', reports: [], unseen: 0, how_to_send: 'Nothing is uploaded.' };
  const report = (file: string, unseen: boolean) => ({
    path: `C:\\x\\logs\\${file}`,
    file,
    program: 'praxis-app 0.1.0',
    time: '2026-11-12T10:00:00Z',
    message: 'boom',
    unseen,
  });

  it('says nothing when every report was shown', () => {
    expect(crashNoticeText(base)).toBeNull();
    expect(crashNoticeText({ ...base, reports: [report('crash-20261112-100000-1.txt', false)] })).toBeNull();
  });

  it('names the newest unseen report and how to send it', () => {
    const t = crashNoticeText({
      ...base,
      unseen: 2,
      reports: [report('crash-20261112-100000-1.txt', true), report('crash-20261111-100000-1.txt', true)],
    });
    expect(t?.title).toBe('Praxis Sound closed unexpectedly (2 reports since you last looked).');
    expect(t?.path).toBe('C:\\x\\logs\\crash-20261112-100000-1.txt');
    expect(t?.detail).toContain('Nothing is uploaded.');
  });
});
