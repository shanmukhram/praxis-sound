import { expect, test, type Page } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const shot = (name: string) => fileURLToPath(new URL(`../../screenshots/${name}`, import.meta.url));

async function setRange(page: Page, layer: string, from: number, to: number) {
  await page.getByTestId('scope-layer').selectOption(layer);
  // Tab commits the field (a native change event), as a user would.
  await page.getByTestId('scope-from').fill(String(from));
  await page.getByTestId('scope-from').press('Tab');
  await page.getByTestId('scope-to').fill(String(to));
  await page.getByTestId('scope-to').press('Tab');
}

const kickClips = (page: Page) =>
  page.evaluate(() => (window as any).praxis.app.state.score.layers.find((l: any) => l.id === 'kick').clips.map((c: any) => `${c.bars.start}-${c.bars.end}:${c.pattern}`));

test('acceptance 1: timeline, mute/solo/lock, scoped step edit, undo', async ({ page }) => {
  await page.goto('/?fixture=acceptance1');

  // 16 bars, 2 layers, 2 sections.
  await expect(page.getByTestId('bars-count')).toHaveText('16 bars');
  await expect(page.locator('.layer-head')).toHaveCount(2);
  await expect(page.getByTestId('layer-piano')).toContainText('Felt piano');
  await expect(page.getByTestId('layer-kick')).toContainText('Soft kick');
  const sectionOptions = page.locator('.scopebar select').nth(1).locator('option');
  await expect(sectionOptions).toContainText(['Section…', 'All bars (1-16)', 'Piano alone (1-8)', 'Piano and kick (9-16)']);
  await expect(page.getByTestId('timeline-canvas')).toBeVisible();

  // Mute and solo the kick: each is an update_layer patch scoped to that layer only.
  await page.getByTestId('mute-kick').click();
  await expect(page.getByTestId('mute-kick')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('patch-scope')).toHaveText('all bars · layers kick');
  await page.getByTestId('solo-kick').click();
  await expect(page.getByTestId('solo-kick')).toHaveAttribute('aria-pressed', 'true');
  await page.getByTestId('solo-kick').click();
  await page.getByTestId('mute-kick').click();
  await expect(page.getByTestId('mute-kick')).toHaveAttribute('aria-pressed', 'false');
  await expect(page.getByTestId('solo-kick')).toHaveAttribute('aria-pressed', 'false');

  // The piano is locked: muting it is rejected by the engine's lock rule.
  await expect(page.getByTestId('lock-piano')).toHaveAttribute('aria-pressed', 'true');
  await page.getByTestId('mute-piano').click();
  await expect(page.getByTestId('patch-status')).toContainText('rejected');
  await expect(page.getByTestId('status')).toContainText('layer `piano` is locked');
  await expect(page.getByTestId('mute-piano')).toHaveAttribute('aria-pressed', 'false');
  // Unlock (a user may), then lock again.
  await page.getByTestId('lock-piano').click();
  await expect(page.getByTestId('lock-piano')).toHaveAttribute('aria-pressed', 'false');
  await page.getByTestId('lock-piano').click();
  await expect(page.getByTestId('lock-piano')).toHaveAttribute('aria-pressed', 'true');

  // Edit a step in bars 13-16 of the kick: the patch scope is exactly those bars and that layer.
  await setRange(page, 'kick', 13, 16);
  await expect(page.getByTestId('edit-scope')).toHaveText('next edit: bars 13-16 · kick');
  await expect(page.getByTestId('editor-info')).toContainText('bars 13-16 of Soft kick');
  await expect(page.getByTestId('step-36-6')).toHaveAttribute('aria-pressed', 'false');
  await page.getByTestId('step-36-6').click();
  await expect(page.getByTestId('step-36-6')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('patch-scope')).toHaveText('bars 13-16 · layers kick');
  await expect(page.getByTestId('patch-panel-scope')).toHaveText('scope: bars 13-16 · layers kick');
  const json = await page.getByTestId('patch-json').textContent();
  const patch = JSON.parse(json!);
  expect(patch.scope).toEqual({ bars: { start: 13, end: 16 }, layers: ['kick'] });
  expect(patch.ops).toHaveLength(1);
  expect(patch.ops[0].op).toBe('write_bars');
  expect(patch.ops[0].bars).toEqual({ start: 13, end: 16 });
  expect(patch.ops[0].pattern.id).toBe('kick_b13-16');
  expect(await kickClips(page)).toEqual(['9-12:kick_soft', '13-16:kick_b13-16']);
  // Accent and length on the new note through the inspector.
  await page.getByTestId('inspector-accent').selectOption('bright');
  await expect(page.getByTestId('patch-scope')).toHaveText('bars 13-16 · layers kick');
  await expect(page.getByTestId('inspector-accent')).toHaveValue('bright');
  await page.getByTestId('inspector-length').fill('180');
  await page.getByTestId('inspector-length').press('Tab');
  await expect(page.getByTestId('note-inspector')).toContainText('Overlaps 0.80 steps');
  await expect(page.getByTestId('render-report')).toContainText('Notes');
  await page.mouse.move(0, 0);
  await page.screenshot({ path: shot('01-step-edit-bars-13-16.png') });

  const descriptions = await page.evaluate(() => (window as any).praxis.app.history.undo.map((e: any) => e.patch.description));
  expect(descriptions.slice(-3)).toEqual([
    'Toggle step in bars 13-16 of Soft kick',
    'Set accent in bars 13-16 of Soft kick',
    'Set length in bars 13-16 of Soft kick',
  ]);
  // Undo three times: back to the original kick clip; redo restores the write.
  await page.getByTestId('undo').click();
  await page.getByTestId('undo').click();
  await page.getByTestId('undo').click();
  await expect(page.getByTestId('patch-status')).toContainText('undo');
  await expect(page.getByTestId('step-36-6')).toHaveAttribute('aria-pressed', 'false');
  expect(await kickClips(page)).toEqual(['9-16:kick_soft']);
  await page.locator('body').click({ position: { x: 5, y: 890 } });
  await page.keyboard.press('Control+Shift+Z');
  await expect(page.getByTestId('step-36-6')).toHaveAttribute('aria-pressed', 'true');
  expect(await kickClips(page)).toEqual(['9-12:kick_soft', '13-16:kick_b13-16']);
  await page.keyboard.press('Control+Z');
  await expect(page.getByTestId('step-36-6')).toHaveAttribute('aria-pressed', 'false');

  // Named version and restore (undoable).
  await page.getByTestId('version-name').fill('original');
  await page.getByTestId('save-version').click();
  await page.getByTestId('step-36-2').click();
  await expect(page.getByTestId('step-36-2')).toHaveAttribute('aria-pressed', 'true');
  await page.getByTestId('restore-original').click();
  await expect(page.getByTestId('step-36-2')).toHaveAttribute('aria-pressed', 'false');
  expect(await kickClips(page)).toEqual(['9-16:kick_soft']);
});

test('piano roll shows the key and flags off-scale notes', async ({ page }) => {
  await page.goto('/?fixture=acceptance1');
  await setRange(page, 'piano', 1, 8);
  await page.getByTestId('tab-roll').click();
  await expect(page.getByTestId('roll-info')).toContainText('Key A minor');
  await expect(page.getByTestId('roll-info')).toContainText('all notes in key');
  await expect(page.getByTestId('roll-info')).toContainText('layer is locked');
  await expect(page.getByTestId('editor-info')).toContainText('piano_motif (repeats in range)');
  await page.mouse.move(0, 0);
  await page.screenshot({ path: shot('02-piano-roll-a-minor.png') });

  // A user unlocks the piano; adding an off-scale note is rejected by validation (off_scale).
  await page.getByTestId('lock-piano').click();
  const before = await page.evaluate(() => JSON.stringify((window as any).praxis.app.state.score.patterns));
  const roll = page.getByTestId('piano-roll');
  const box = (await roll.boundingBox())!;
  // Bb3 (58) row: rows run from hi down; find it through the drawn geometry exposed on the view.
  const y = await page.evaluate(() => {
    const view = (window as any).praxis.app.roll;
    return (view.geo.hi - 58 + 0.5) * view.geo.rowH;
  });
  await page.mouse.click(box.x + 64 + 3, box.y + y);
  await expect(page.getByTestId('status')).toContainText('off_scale');
  expect(await page.evaluate(() => JSON.stringify((window as any).praxis.app.state.score.patterns))).toBe(before);
});

test('a loaded score with an off-scale note is reported and the note flagged', async ({ page }) => {
  await page.goto('/?fixture=acceptance1');
  await page.evaluate(async () => {
    const praxis = (window as any).praxis;
    const score = structuredClone(praxis.app.state.score);
    score.patterns[0].notes[1].pitch = 63; // Eb4 is not in A minor
    await praxis.app.loadScore(score, 'off-scale test');
  });
  await expect(page.getByTestId('status')).toContainText('off_scale');
  await setRange(page, 'piano', 1, 2);
  await page.getByTestId('tab-roll').click();
  await expect(page.getByTestId('roll-info')).toContainText('1 off-scale note flagged (Eb4@2)');
  await expect(page.getByTestId('roll-info')).toHaveAttribute('data-offscale', '1');
});

test('export dialog lists every format and records the request', async ({ page }) => {
  await page.goto('/?fixture=acceptance1');
  await page.getByTestId('export').click();
  const dialog = page.getByTestId('export-dialog');
  await expect(dialog).toBeVisible();
  for (const k of ['wav16', 'wav24', 'wav32f', 'flac', 'stems', 'midi', 'project']) {
    await page.getByTestId(`export-${k}`).check();
  }
  await page.getByTestId('export-go').click();
  await expect(page.getByTestId('export-result')).toContainText('request recorded');
  await expect(page.getByTestId('export-result')).toContainText('exports/acceptance-test-1.stem-kick.wav');
  await expect(page.getByTestId('export-result')).toContainText('exports/acceptance-test-1.praxis');
  const recorded = await page.evaluate(() => (window as any).praxis.engine.exports);
  expect(recorded).toHaveLength(1);
  expect(recorded[0].kinds).toEqual(['wav16', 'wav24', 'wav32f', 'flac', 'stems', 'midi', 'project']);
  await page.screenshot({ path: shot('03-export-dialog.png') });
});

test('transport: play moves the playhead, loop follows the selection', async ({ page }) => {
  await page.goto('/?fixture=acceptance1');
  await setRange(page, 'kick', 13, 16);
  await page.locator('body').click({ position: { x: 5, y: 890 } });
  await page.keyboard.press('l');
  await expect(page.getByTestId('loop')).toHaveText('Loop 13-16');
  await page.getByTestId('play').click();
  await expect(page.getByTestId('play')).toHaveAttribute('aria-pressed', 'true');
  await page.waitForTimeout(600);
  await page.keyboard.press(' ');
  await expect(page.getByTestId('play')).toHaveAttribute('aria-pressed', 'false');
  const bar = await page.evaluate(() => (window as any).praxis.app.state.position.bar);
  expect(bar).toBeGreaterThan(1);
});
