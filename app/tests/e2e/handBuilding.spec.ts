// Building acceptance test 1 by hand in the window (WP-U5), on the mock engine: a blank sound,
// sound settings, layers by hand, notes in Rhythm and Notes without a take per click, one take for
// the batch, seek, a loop range that does not clash with scope dragging, and keyboard play/stop,
// at 1440 and at 390 (the bottom sheet). Screenshots of each control go to app/screenshots/.
//
// No sound: WebAudio is removed before the page loads and Chromium runs muted, so nothing can
// reach the speakers.

import { expect, test, type Page } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const shot = (name: string) => fileURLToPath(new URL(`../../screenshots/${name}`, import.meta.url));

test.use({ launchOptions: { args: ['--mute-audio'] } });

async function silent(page: Page) {
  await page.addInitScript(() => {
    for (const k of ['AudioContext', 'webkitAudioContext']) Object.defineProperty(window, k, { value: undefined, configurable: true, writable: true });
    HTMLMediaElement.prototype.play = function () {
      return Promise.resolve();
    };
  });
}

const st = (page: Page) =>
  page.evaluate(() => {
    const s = (window as any).praxis.engine.state();
    return { count: s.takes.length, current: s.current, draft: !!s.draft, last: s.takes.at(-1) ?? null, score: s.score };
  });

const notesOf = (page: Page, pattern: string) =>
  page.evaluate((id) => (window as any).praxis.engine.state().score.patterns.find((p: any) => p.id === id)?.notes.length ?? -1, pattern);

async function startBlank(page: Page, title = 'Calm piano by hand') {
  await page.goto('/?sound=new');
  await page.getByTestId('start-blank').click();
  await expect(page.getByTestId('blank-form')).toBeVisible();
  await page.getByTestId('blank-title').fill(title);
}

async function drag(page: Page, from: string, to: string) {
  const a = (await page.getByTestId(from).boundingBox())!;
  const b = (await page.getByTestId(to).boundingBox())!;
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 8 });
  await page.mouse.up();
}

async function say(page: Page, words: string, bar?: string) {
  if (bar !== undefined) await page.getByTestId('say-bar-which').selectOption(bar);
  await page.getByTestId('say-bar').fill(words);
  await page.getByTestId('say-bar-go').click();
}

test.describe('desktop 1440: build acceptance test 1 by hand', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('blank sound, layers, notes without a take per click, one take, seek, loop and keys', async ({ page }) => {
    await silent(page);
    await startBlank(page);

    // The frame: a length the solver fits the bars into, or a tempo.
    await expect(page.getByTestId('blank-length-line')).toHaveText('16 bars of 4/4 at 128 BPM last 0:30, as a seamless loop.');
    await page.getByTestId('blank-by-tempo').click();
    await page.getByTestId('blank-bpm').fill('96');
    await expect(page.getByTestId('blank-length-line')).toHaveText('16 bars of 4/4 at 96 BPM last 0:40, as a seamless loop.');
    await page.getByTestId('blank-by-length').click();
    await expect(page.getByTestId('blank-length-line')).toContainText('at 128 BPM last 0:30');
    await page.screenshot({ path: shot('hand-blank-form-1440.png') });
    await page.getByTestId('blank-go').click();

    // A draft: no take, the panel open on Overview with the sound's settings.
    await expect(page.getByTestId('draft-card')).toBeVisible();
    await expect(page.getByTestId('sound-line')).toHaveText('Built by hand, no take yet · A minor · 128 BPM · 16 bars');
    await expect(page.getByTestId('sound-settings')).toBeVisible();
    await expect(page.getByTestId('pending')).toContainText('Add a layer to start');
    expect(await st(page)).toMatchObject({ count: 0, draft: true });
    // Sound settings are live edits: the tempo changes and comes back without a take.
    await page.getByTestId('set-bpm').fill('120');
    await page.getByTestId('set-bpm').press('Tab');
    await expect(page.getByTestId('sound-line')).toContainText('120 BPM');
    await page.getByTestId('set-bpm').fill('128');
    await page.getByTestId('set-bpm').press('Tab');
    await expect(page.getByTestId('sound-line')).toContainText('128 BPM');
    await page.getByTestId('set-oneshot').click();
    await expect(page.getByTestId('set-oneshot')).toHaveAttribute('aria-pressed', 'true');
    await page.getByTestId('set-loop').click();
    await page.getByTestId('panel').screenshot({ path: shot('hand-settings-1440.png') });

    // Layers by hand: the felt piano from the picker.
    await page.getByTestId('add-layer').click();
    await expect(page.getByTestId('picker')).toBeVisible();
    await expect(page.getByTestId('picker').locator('.quick-presets .preset').first()).toHaveText(/Felt piano/);
    await page.getByTestId('panel').screenshot({ path: shot('hand-picker-1440.png') });
    await page.getByTestId('quick-felt_piano').click();
    await expect(page.getByTestId('cell-felt_piano-1')).toHaveClass(/on/);
    await expect(page.getByTestId('make-take')).toHaveText('Make the first take');

    // The motif, a bar at a time in rhythm words: heard live, no take.
    await page.getByTestId('tab-rhythm').click();
    await expect(page.getByTestId('rhythm-view')).toContainText('Felt piano rhythm, bars 1–16');
    await say(page, 'DON din don din TIN din don din', '0');
    await expect(page.getByTestId('step-0')).toHaveClass(/strong/);
    await expect(page.getByTestId('step-4')).toHaveClass(/bright/);
    await say(page, 'DON din don din TIN din - din', '1');
    await expect(page.getByTestId('step-8')).toHaveClass(/strong/);
    await expect(page.getByTestId('step-14')).toHaveClass(/off/);
    await page.getByTestId('step-14').click();
    await expect(page.getByTestId('step-14')).toHaveClass(/normal/);
    await page.getByTestId('say-bar').fill('DON bom');
    await page.getByTestId('say-bar-go').click();
    await expect(page.getByTestId('say-msg')).toContainText('"bom" is not a rhythm word');
    expect((await st(page)).count).toBe(0);
    expect(await notesOf(page, 'felt_piano_p1')).toBe(16);
    await page.getByTestId('panel').screenshot({ path: shot('hand-rhythm-1440.png') });

    // Notes: a click adds a note in key, in every repeat of the motif.
    await page.getByTestId('tab-notes').click();
    const roll = page.getByTestId('roll');
    const box = (await roll.boundingBox())!;
    await page.mouse.click(box.x + box.width * 0.02, box.y + box.height * 0.08);
    await expect.poll(() => notesOf(page, 'felt_piano_p1')).toBe(17);
    const pitches = await page.evaluate(() => (window as any).praxis.engine.state().score.patterns[0].notes.map((n: any) => n.pitch));
    const aMinor = [0, 2, 3, 5, 7, 8, 10];
    for (const p of pitches) expect(aMinor).toContain((((p - 9) % 12) + 12) % 12);
    expect((await st(page)).count).toBe(0);
    await page.getByTestId('panel').screenshot({ path: shot('hand-notes-1440.png') });

    // The soft kick, in bars 9-16, a little lower.
    await page.getByTestId('tab-overview').click();
    await page.getByTestId('add-layer').click();
    await page.getByTestId('quick-soft_kick').click();
    await page.getByTestId('edit-soft_kick').click();
    await expect(page.getByTestId('layer-edit-soft_kick')).toBeVisible();
    await page.getByTestId('layer-range-soft_kick-second').click();
    await expect(page.getByTestId('cell-soft_kick-8')).not.toHaveClass(/on/);
    await expect(page.getByTestId('cell-soft_kick-9')).toHaveClass(/on/);
    await page.getByTestId('layer-gain-soft_kick').fill('-4');
    await expect(page.getByTestId('layer-soft_kick')).toContainText('−4.0 dB');
    await page.getByTestId('layer-pan-soft_kick').fill('0.2');
    await expect(page.getByTestId('layer-edit-soft_kick')).toContainText('20% right');
    await page.getByTestId('layer-name-soft_kick').fill('Soft kick');
    await page.getByTestId('panel').screenshot({ path: shot('hand-layer-edit-1440.png') });
    // Rename and change instrument, then back.
    await page.getByTestId('layer-instrument-soft_kick').selectOption('kick');
    await expect(page.getByTestId('layer-soft_kick')).toContainText('kick · plays bars 9–16');
    await page.getByTestId('layer-instrument-soft_kick').selectOption('soft_kick');
    await expect(page.getByTestId('layer-soft_kick')).toContainText('soft kick · plays bars 9–16');
    await page.getByTestId('edit-soft_kick').click();

    // Its beat in rhythm words.
    await page.getByTestId('tab-rhythm').click();
    await page.getByTestId('view-layer-soft_kick').click();
    await expect(page.getByTestId('rhythm-view')).toContainText('Soft kick rhythm, bars 9–16');
    await say(page, 'DON - - - don - - -');
    await expect(page.getByTestId('step-0')).toHaveClass(/strong/);
    await expect(page.getByTestId('step-4')).toHaveClass(/normal/);
    expect((await st(page)).count).toBe(0);

    // One take for the whole batch.
    await page.getByTestId('make-take').click();
    await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '1');
    await expect(page.getByTestId('take-current')).toContainText('Take 1 · Built by hand');
    await expect(page.getByTestId('draft-card')).toHaveCount(0);
    await expect(page.getByTestId('pending')).toHaveCount(0);
    const t1 = await st(page);
    expect(t1.count).toBe(1);
    expect(t1.last.patch).toBeNull();
    // Settings changed back and forth read once, as they ended.
    expect(t1.last.ask).toMatch(/^By hand: tempo 128 BPM; plays as a loop; added Felt piano; /);
    expect(t1.score.layers.map((l: any) => l.id)).toEqual(['felt_piano', 'soft_kick']);

    // Seek from a bar number; loop bars 13-16 on the loop strip, which leaves the scope alone.
    await page.getByTestId('seek-9').click();
    await expect(page.getByTestId('transport-time')).toHaveText('0:15 / 0:30');
    await drag(page, 'loop-13', 'loop-16');
    await expect(page.getByTestId('loop-status')).toHaveText('Looping bars 13–16');
    await expect(page.getByTestId('scope-chip')).toHaveCount(0);
    await drag(page, 'cell-felt_piano-5', 'cell-felt_piano-8');
    await expect(page.getByTestId('scope-chip')).toHaveText('Bars 5–8 · Felt piano');
    await expect(page.getByTestId('loop-status')).toHaveText('Looping bars 13–16');
    await page.getByTestId('change-map').screenshot({ path: shot('hand-transport-1440.png') });
    await page.getByTestId('loop-clear').click();
    await expect(page.getByTestId('loop-status')).toHaveText('Loops the whole sound');

    // Keys: Space plays and stops, L loops the selected bars, Home goes back to the start.
    await page.getByTestId('thread').click({ position: { x: 5, y: 5 } });
    await page.keyboard.press('Space');
    await expect(page.getByTestId('transport-play')).toHaveAttribute('aria-label', 'Stop (Space)');
    await page.keyboard.press('Space');
    await expect(page.getByTestId('transport-play')).toHaveAttribute('aria-label', 'Play (Space)');
    await page.keyboard.press('l');
    await expect(page.getByTestId('loop-status')).toHaveText('Looping bars 5–8');
    await page.keyboard.press('Escape');
    await expect(page.getByTestId('scope-chip')).toHaveCount(0);
    await page.keyboard.press('l');
    await expect(page.getByTestId('loop-status')).toHaveText('Loops the whole sound');
    await page.keyboard.press('Home');
    await expect(page.getByTestId('transport-time')).toHaveText('0:00 / 0:30');
    // Click the take's waveform to play from there.
    const wave = (await page.getByTestId('wave-current').boundingBox())!;
    await page.mouse.click(wave.x + wave.width * 0.5, wave.y + wave.height / 2);
    await expect(page.getByTestId('transport-time')).toHaveText('0:15 / 0:30');

    // The next batch is the next take.
    await page.getByTestId('tab-overview').click();
    await page.getByTestId('set-bpm').fill('120');
    await page.getByTestId('set-bpm').press('Tab');
    await expect(page.getByTestId('pending-thread')).toContainText('1 hand edit since take 1');
    await expect(page.getByTestId('pending')).toContainText('1 change heard live');
    await page.getByTestId('make-take').click();
    await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '2');
    const t2 = await st(page);
    expect(t2.last.patch.ops).toContainEqual({ op: 'set_tempo', bpm: 120 });
    expect(t2.last.say).toBe('Changed by hand: tempo 120 BPM.');
    await page.screenshot({ path: shot('hand-take-1440.png') });
  });

  test('a request in words waits for the first take of a blank sound; a bad frame is refused', async ({ page }) => {
    await silent(page);
    await startBlank(page, 'Words later');
    await page.getByTestId('blank-bars').fill('0');
    await expect(page.getByTestId('blank-length-line')).toHaveText('Bars must be a whole number from 1 to 2048.');
    await expect(page.getByTestId('blank-go')).toBeDisabled();
    await page.getByTestId('blank-bars').fill('8');
    await page.getByTestId('blank-go').click();
    await page.getByTestId('prompt').fill('make it darker');
    await page.getByTestId('send').click();
    await expect(page.getByTestId('reply').last()).toContainText('This sound was started by hand');
    expect((await st(page)).count).toBe(0);
    // A layer removed by hand comes back with Undo.
    await page.getByTestId('add-layer').click();
    await page.getByTestId('quick-soft_kick').click();
    await page.getByTestId('edit-soft_kick').click();
    await page.getByTestId('remove-soft_kick').click();
    await expect(page.getByTestId('layer-soft_kick')).toHaveCount(0);
    await page.getByTestId('undo').click();
    await expect(page.getByTestId('layer-soft_kick')).toBeVisible();
  });
});

test.describe('phone 390: the controls fit the bottom sheet', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true });

  const noSideScroll = async (page: Page) => {
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
    const body = await page.getByTestId('panel-body').evaluate((e) => [e.scrollWidth, e.clientWidth]);
    expect(body[0]).toBeLessThanOrEqual(body[1] + 1);
  };

  test('blank sound, settings, picker, layer details, rhythm, the batch and the transport', async ({ page }) => {
    await silent(page);
    await startBlank(page);
    await page.screenshot({ path: shot('hand-blank-form-390.png'), fullPage: true });
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
    await page.getByTestId('blank-go').click();

    // The sheet opens on the new sound's settings.
    await expect(page.getByTestId('panel')).toHaveClass(/open/);
    await page.waitForTimeout(350);
    await expect(page.getByTestId('sound-settings')).toBeInViewport();
    await noSideScroll(page);
    await page.screenshot({ path: shot('hand-sheet-390.png') });

    await page.getByTestId('add-layer').click();
    await page.getByTestId('picker').scrollIntoViewIfNeeded();
    await noSideScroll(page);
    await page.screenshot({ path: shot('hand-picker-390.png') });
    await page.getByTestId('quick-felt_piano').click();

    await page.getByTestId('edit-felt_piano').click();
    await page.getByTestId('layer-edit-felt_piano').scrollIntoViewIfNeeded();
    await noSideScroll(page);
    await page.screenshot({ path: shot('hand-layer-edit-390.png') });
    await page.getByTestId('edit-felt_piano').click();

    await page.getByTestId('tab-rhythm').click();
    await say(page, 'DON din don din TIN din don din', '0');
    await expect(page.getByTestId('step-0')).toHaveClass(/strong/);
    await noSideScroll(page);
    await page.screenshot({ path: shot('hand-rhythm-390.png') });

    await page.getByTestId('make-take').scrollIntoViewIfNeeded();
    await expect(page.getByTestId('make-take')).toBeInViewport();
    await page.screenshot({ path: shot('hand-pending-390.png') });
    await page.getByTestId('make-take').click();
    await expect.poll(async () => (await st(page)).count).toBe(1);

    await page.getByTestId('transport').scrollIntoViewIfNeeded();
    await page.getByTestId('seek-9').click();
    await expect(page.getByTestId('transport-time')).toHaveText('0:15 / 0:30');
    await page.getByTestId('loop-13').click();
    await expect(page.getByTestId('loop-status')).toHaveText('Looping bar 13');
    await noSideScroll(page);
    await page.screenshot({ path: shot('hand-transport-390.png') });
  });
});
