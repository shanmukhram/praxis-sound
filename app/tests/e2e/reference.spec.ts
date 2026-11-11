// Reference clips in the workstation (WP-U6), on the mock engine in a muted Chromium: drop a
// clip (or choose one) and its profile shows in the Sound panel's Reference view in plain words;
// "Make something like this" carries it with the request; the loudness-matched A/B trims the
// louder side and swaps what is heard; the library keeps named, tagged profiles and a request
// that names one in quotes uses it. At 1440 and on a 390 phone.

import { expect, test, type Page } from '@playwright/test';
import { fileURLToPath } from 'node:url';

// Never play sound on the owner's speakers.
test.use({ launchOptions: { args: ['--mute-audio'] } });

const shot = (name: string) => fileURLToPath(new URL(`../../screenshots/${name}`, import.meta.url));

const SR = 22050;

/** A 16-bit stereo WAV. */
function wav(l: Float32Array, r: Float32Array): Buffer {
  const n = l.length;
  const b = Buffer.alloc(44 + n * 4);
  b.write('RIFF', 0);
  b.writeUInt32LE(36 + n * 4, 4);
  b.write('WAVEfmt ', 8);
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20);
  b.writeUInt16LE(2, 22);
  b.writeUInt32LE(SR, 24);
  b.writeUInt32LE(SR * 4, 28);
  b.writeUInt16LE(4, 32);
  b.writeUInt16LE(16, 34);
  b.write('data', 36);
  b.writeUInt32LE(n * 4, 40);
  for (let i = 0; i < n; i++) {
    b.writeInt16LE(Math.round(Math.max(-1, Math.min(1, l[i])) * 32767), 44 + i * 4);
    b.writeInt16LE(Math.round(Math.max(-1, Math.min(1, r[i])) * 32767), 46 + i * 4);
  }
  return b;
}

function noise(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    s >>>= 0;
    return (s / 4294967296) * 2 - 1;
  };
}

/** A soft rain-like bed: 10 s of decorrelated noise, no beat. */
function rainBed(): Buffer {
  const a = noise(11);
  const b = noise(29);
  const n = SR * 10;
  return wav(new Float32Array(n).map(() => 0.08 * a()), new Float32Array(n).map(() => 0.08 * b()));
}

/** A click every half second (120 BPM), 8 s: music to the mock's measurements. */
function clickLoop(): Buffer {
  const n = SR * 8;
  const x = new Float32Array(n);
  for (let t = 0; t < n; t += SR / 2) for (let i = 0; i < 400 && t + i < n; i++) x[t + i] = 0.8 * Math.exp(-i / 60) * Math.sin(i * 0.3);
  return wav(x, x);
}

/** Drops a file on the window the way a person drags one in. */
async function dropFile(page: Page, name: string, data: Buffer): Promise<void> {
  await page.evaluate(
    ({ name, b64 }) => {
      const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
      const dt = new DataTransfer();
      dt.items.add(new File([bytes], name, { type: 'audio/wav' }));
      window.dispatchEvent(new DragEvent('dragenter', { dataTransfer: dt, bubbles: true }));
      window.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }));
    },
    { name, b64: data.toString('base64') },
  );
}

const compareLevel = (page: Page) => page.evaluate(() => (window as any).praxis.engine.compareLevel());

test.describe('desktop 1440', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('a dropped clip shows its profile in plain words, and "Make something like this" carries it', async ({ page }) => {
    await page.goto('/?sound=new');
    await expect(page.getByTestId('sound-title')).toHaveText('New sound');
    await dropFile(page, 'rain bed.wav', rainBed());
    const view = page.getByTestId('reference-view');
    await expect(view.getByTestId('ref-name')).toHaveText('rain bed.wav');
    await expect(page.getByTestId('tab-reference')).toHaveAttribute('aria-selected', 'true');
    await expect(view.getByTestId('ref-kind')).toHaveText('Ambience');
    const lines = view.getByTestId('profile-lines');
    await expect(lines.locator('[data-line=tempo]')).toHaveText("Can't tell: no steady beat");
    await expect(lines.locator('[data-line=tempo]')).toHaveClass(/cant/);
    await expect(lines.locator('[data-line=key]')).toContainText("Can't tell");
    await expect(lines.locator('[data-line=loudness]')).toContainText('LUFS');
    await expect(lines.locator('[data-line=space]')).toContainText('very wide');
    await expect(view.getByTestId('profile-envelope')).toBeVisible();
    await expect(view.getByTestId('profile-bands').locator('i')).toHaveCount(10);
    await expect(page.getByTestId('clip-chip')).toHaveText('Reference: rain bed.wav · Ambience, 10 s');
    // Depth on demand: the numbers behind each line.
    await expect(view.locator('.ref-num')).toHaveCount(0);
    await view.getByTestId('ref-numbers').click();
    await expect(view.locator('[data-line=loudness] .ref-num')).toContainText('LUFS, range');
    await view.getByTestId('ref-numbers').click();
    // No take yet: nothing to compare by ear.
    await expect(view.getByTestId('ab')).toContainText('Make a take, then compare it with the clip here');
    await page.waitForTimeout(150);
    await page.screenshot({ path: shot('reference-profile-1440.png') });

    await view.getByTestId('ref-like').click();
    await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '1');
    const asked = page.locator('.msg-user').last();
    await expect(asked).toContainText('Make something like rain bed.wav');
    await expect(asked.getByTestId('ref-tag')).toHaveText('Reference: rain bed.wav');
    const planned = await page.evaluate(() => (window as any).praxis.engine.lastRequest as string);
    expect(planned).toContain('Reference clip "rain bed.wav": keep its character, never its melody.');
    // Sent once: the next request goes without it, the Reference view keeps it for the A/B.
    await expect(page.getByTestId('clip-chip')).toHaveCount(0);
    await expect(view.getByTestId('ref-name')).toHaveText('rain bed.wav');
  });

  test('loudness-matched A/B against the current take: the louder one is turned down, A and B swap', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '3');
    await dropFile(page, 'click loop.wav', clickLoop());
    const view = page.getByTestId('reference-view');
    await expect(view.getByTestId('ref-kind')).toHaveText('Music');
    await expect(view.locator('[data-line=tempo]')).toContainText('120 BPM');
    const ab = view.getByTestId('ab');
    await expect(ab.getByTestId('ab-a')).toContainText('Take 3');
    await expect(ab.getByTestId('ab-b')).toContainText('click loop.wav');
    await expect(ab.getByTestId('ab-a')).toHaveAttribute('aria-checked', 'true');
    const words = (await ab.getByTestId('ab-levels').textContent()) ?? '';
    expect(words).toMatch(/^Levels matched at -?\d+\.\d LUFS: the (take|reference) plays \d+\.\d dB quieter so the louder one does not win\.$/);

    await ab.getByTestId('ab-play').click();
    await expect(ab.getByTestId('ab-play')).toHaveText('Stop');
    const a = await compareLevel(page);
    expect(a.muted).toBe(false);
    expect(a.gainDb).toBeLessThanOrEqual(0);
    // The clip side is heard: the take is silenced, the clip unmuted at its matched volume.
    await ab.getByTestId('ab-b').click();
    await expect(ab.getByTestId('ab-b')).toHaveAttribute('aria-checked', 'true');
    expect(await compareLevel(page)).toEqual({ gainDb: a.gainDb, muted: true });
    await page.waitForTimeout(150);
    await page.screenshot({ path: shot('reference-ab-1440.png') });
    await ab.getByTestId('ab-a').click();
    expect((await compareLevel(page)).muted).toBe(false);
    await ab.getByTestId('ab-play').click();
    await expect(ab.getByTestId('ab-play')).toHaveText('Play A/B');
    expect(await compareLevel(page)).toBeNull();
    // Playing a take the usual way ends any comparison.
    await ab.getByTestId('ab-play').click();
    await page.getByTestId('take-current').locator('[data-play="3"]').click();
    await expect(ab.getByTestId('ab-play')).toHaveText('Play A/B');
    expect(await compareLevel(page)).toBeNull();
  });

  test('the library keeps named, tagged profiles; a request naming one in quotes uses it', async ({ page }) => {
    await page.goto('/?sound=new');
    await dropFile(page, 'rain bed.wav', rainBed());
    const view = page.getByTestId('reference-view');
    await expect(view.getByTestId('ref-kind')).toHaveText('Ambience');
    await view.getByTestId('ref-save-open').click();
    await expect(view.getByTestId('ref-save-name')).toHaveValue('rain bed');
    await view.getByTestId('ref-save-name').fill('Night rain');
    await view.getByTestId('ref-save-tags').fill('rain, Night, bed');
    await view.getByTestId('ref-save-go').click();
    await expect(view.getByTestId('library-note')).toHaveText('Saved "Night rain" to the library: its profile, not its audio.');
    const row = view.getByTestId('library').locator('li');
    await expect(row).toHaveCount(1);
    await expect(row).toContainText('Night rain');
    await expect(row).toContainText('Ambience · 10 s');
    await expect(row.locator('input')).toHaveValue('rain, night, bed');
    // Tags change in place.
    await row.locator('input').fill('rain, loop');
    await row.locator('input').press('Enter');
    await expect(view.getByTestId('library-note')).toHaveText('Tags of "Night rain": rain, loop.');
    await page.waitForTimeout(150);
    await page.screenshot({ path: shot('reference-library-1440.png') });

    // Without attaching anything, a request that names the style in quotes carries its profile.
    await page.getByTestId('clip-detach').click();
    await expect(page.getByTestId('clip-chip')).toHaveCount(0);
    await page.getByTestId('prompt').fill("A calm 20-second loop, like 'night rain'");
    await page.getByTestId('send').click();
    await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '1');
    await expect(page.locator('.msg-user').last().getByTestId('ref-tag')).toHaveText('Reference: Night rain');
    const planned = await page.evaluate(() => (window as any).praxis.engine.lastRequest as string);
    expect(planned).toContain('Reference style "Night rain" (from the reference library)');

    // The saved clip is the style "in use"; once removed from view, "Use" shows the style (no
    // audio: nothing to A/B) and attaches it; Remove takes it out of the library.
    await expect(view.getByTestId(/^lib-use-/)).toHaveText('In use');
    await view.getByTestId('ref-clear').click();
    await expect(view.getByTestId('ref-empty')).toBeVisible();
    await view.getByTestId(/^lib-use-/).click();
    await expect(page.getByTestId('clip-chip')).toHaveText('Reference: Night rain · Ambience, 10 s');
    await expect(view.getByTestId('ab')).toContainText('A library style keeps no audio');
    await view.getByTestId(/^lib-remove-/).click();
    await expect(view.getByTestId('library').locator('li')).toHaveCount(0);
  });

  test('a file that is not a clip is refused in plain words', async ({ page }) => {
    await page.goto('/?sound=new');
    await page.getByTestId('clip-input').setInputFiles({ name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('hello') });
    await expect(page.getByTestId('ref-error')).toHaveText('notes.txt is not a clip Praxis can read. Drop audio (WAV, MP3, FLAC, OGG, M4A) or video (MP4, MOV, WebM).');
    await expect(page.getByTestId('reply').last()).toContainText('notes.txt is not a clip Praxis can read');
  });
});

test.describe('phone 390', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true });

  test('"Add a clip" opens the Sound sheet on the profile; the A/B and the library fit the width', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '3');
    await page.getByTestId('clip-input').setInputFiles({ name: 'click loop.wav', mimeType: 'audio/wav', buffer: clickLoop() });
    await expect(page.getByTestId('panel')).toHaveClass(/open/);
    const view = page.getByTestId('reference-view');
    await expect(view.getByTestId('ref-kind')).toHaveText('Music');
    await expect(view.getByTestId('ab').getByTestId('ab-play')).toBeVisible();
    // Nothing runs off the side of the phone.
    const overflow = await page.evaluate(() => {
      const panel = document.querySelector('[data-testid=panel-body]')!;
      return panel.scrollWidth - panel.clientWidth;
    });
    expect(overflow).toBeLessThanOrEqual(1);
    await page.waitForTimeout(150);
    await page.screenshot({ path: shot('reference-390.png') });
    await view.getByTestId('ref-save-open').click();
    await view.getByTestId('ref-save-go').click();
    await expect(view.getByTestId('library').locator('li')).toHaveCount(1);
    await view.getByTestId('library').scrollIntoViewIfNeeded();
    await page.screenshot({ path: shot('reference-library-390.png') });
    // Back in the conversation the reference waits in the composer; its chip opens the profile.
    await page.getByRole('button', { name: 'Close the Sound panel' }).click();
    await expect(page.getByTestId('panel')).not.toHaveClass(/open/);
    await expect(page.getByTestId('clip-chip')).toHaveText('Reference: click loop.wav · Music, 8.0 s');
    await page.getByTestId('clip-chip').click();
    await expect(page.getByTestId('panel')).toHaveClass(/open/);
  });
});
