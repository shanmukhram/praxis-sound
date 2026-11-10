import { expect, test, type Page } from '@playwright/test';
import { fileURLToPath } from 'node:url';

const shot = (name: string) => fileURLToPath(new URL(`../../screenshots/${name}`, import.meta.url));

const currentTakeN = (page: Page) => page.getByTestId('take-current').getAttribute('data-take');
const engineState = (page: Page) =>
  page.evaluate(() => {
    const st = (window as any).praxis.engine.state();
    return { current: st.current, count: st.takes.length, last: st.takes.at(-1) };
  });

async function ready(page: Page) {
  await page.goto('/');
  await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '3');
}

test.describe('desktop 1440', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('the acceptance-1 sound opens on take 3, the current take the largest block', async ({ page }) => {
    await ready(page);
    await expect(page.getByTestId('sound-title')).toHaveText('Calm piano loop');
    await expect(page.getByTestId('sound-line')).toHaveText('Take 3 is current · A minor · 128 BPM · 16 bars');
    const blocks = page.locator('[data-testid=thread] > *');
    const sizes = await blocks.evaluateAll((els) => els.map((e) => ({ take: e.getAttribute('data-take'), area: e.getBoundingClientRect().width * e.getBoundingClientRect().height })));
    const current = sizes.find((s) => s.take === '3')!;
    expect(Math.max(...sizes.map((s) => s.area))).toBe(current.area);
    await expect(page.getByTestId('take-1')).toContainText('Go back');
    await expect(page.getByTestId('take-2')).toContainText('Changed soft kick in bars 9–16.');
    await expect(page.getByTestId('checks')).toContainText(/\d of \d checks passed/);
    await expect(page.getByTestId('changed')).toContainText('Changed felt piano in bars 13–16. Bars 1–12 sample-identical to take 2.');
    await expect(page.getByTestId('agents')).toContainText('2 agents connected');
    // The change map: amber cells are exactly the take's changed bars.
    const lit = await page.locator('[data-testid=change-map] .cell.chg').evaluateAll((els) => els.map((e) => `${e.getAttribute('data-layer')}:${e.getAttribute('data-bar')}`));
    expect(lit).toEqual(['piano:13', 'piano:14', 'piano:15', 'piano:16']);
    await expect(page.getByTestId('flash')).toHaveText('Notes opened by cut-assistant');
    await page.waitForTimeout(400); // let the waveforms decode
    await page.screenshot({ path: shot('desktop-1440.png') });
  });

  test('a suggestion chip makes take 4', async ({ page }) => {
    await ready(page);
    await page.getByTestId('suggestions').getByRole('button', { name: 'Make the kick quieter' }).click();
    await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '4');
    await expect(page.getByTestId('take-current')).toContainText('Take 4 · Quieter kick');
    await expect(page.getByTestId('take-3')).toContainText('Go back');
    await expect(page.getByTestId('flash')).toHaveText('Mix opened by Praxis');
  });

  test('drag bars 13-16 on the piano lane, ask, and the patch is scoped to those bars', async ({ page }) => {
    await ready(page);
    await page.getByTestId('tab-overview').click();
    const a = (await page.getByTestId('cell-piano-13').boundingBox())!;
    const b = (await page.getByTestId('cell-piano-16').boundingBox())!;
    await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
    await page.mouse.down();
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 8 });
    await page.mouse.up();
    await expect(page.getByTestId('scope-chip')).toHaveText('Bars 13–16 · Felt piano');
    await expect(page.locator('[data-testid=change-map] .cell.sel')).toHaveCount(4);
    await page.getByTestId('prompt').fill('Make these softer');
    await page.getByTestId('send').click();
    await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '4');
    await expect(page.locator('.msg-user').last()).toContainText('Make these softer (Bars 13–16 · Felt piano)');
    const { last } = await engineState(page);
    expect(last.patch.scope).toEqual({ bars: { start: 13, end: 16 }, layers: ['piano'] });
    expect(last.changed).toEqual({ piano: [13, 16] });
    await expect(page.getByTestId('scope-chip')).toHaveCount(0);
  });

  test('an agent take appears under its name and the view it opens says "opened by"', async ({ page }) => {
    await ready(page);
    await page.evaluate(() => (window as any).praxis.engine.agentAsk('trailer-bot', 'Make the kick quieter'));
    await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '4');
    await expect(page.getByTestId('take-current')).toContainText('Current · made by trailer-bot');
    await expect(page.locator('.msg-user').last()).toContainText('Agent · trailer-bot');
    await expect(page.getByTestId('flash')).toHaveText('Mix opened by trailer-bot');
    await page.evaluate(() => (window as any).praxis.engine.openView('rhythm', 'cut-assistant'));
    await expect(page.getByTestId('flash')).toHaveText('Rhythm opened by cut-assistant');
    await expect(page.getByTestId('tab-rhythm')).toHaveAttribute('aria-selected', 'true');
  });

  test('Go back to take 1, then Undo', async ({ page }) => {
    await ready(page);
    await page.getByTestId('go-back-1').click();
    await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '1');
    await expect(page.getByTestId('sound-line')).toHaveText('Take 1 is current · A minor · 128 BPM · 16 bars');
    await expect(page.locator('[data-testid=change-map] .lane')).toHaveCount(1); // take 1 is the piano alone
    await page.getByTestId('undo').click();
    await expect(page.getByTestId('take-current')).toHaveAttribute('data-take', '3');
    await expect(page.locator('[data-testid=change-map] .lane')).toHaveCount(2);
    expect((await engineState(page)).count).toBe(3);
  });

  test('a request on a locked layer is refused in words, with no take', async ({ page }) => {
    await ready(page);
    await page.getByTestId('tab-overview').click();
    await page.getByTestId('lock-piano').click();
    await expect(page.getByTestId('lock-piano')).toHaveText('Locked');
    await page.getByTestId('cell-piano-5').click();
    await page.getByTestId('prompt').fill('lift');
    await page.getByTestId('prompt').press('Enter');
    await expect(page.getByTestId('reply').last()).toContainText('layer `piano` is locked');
    expect((await engineState(page)).count).toBe(3);
  });

  test('scrollbars are themed, never the default', async ({ page }) => {
    await ready(page);
    for (const sel of ['html', '.thread', '.panel-body']) {
      const color = await page.locator(sel).first().evaluate((e) => getComputedStyle(e).scrollbarColor);
      expect(color, sel).not.toBe('auto');
      expect(color, sel).toBe('rgb(42, 42, 42) rgba(0, 0, 0, 0)');
    }
    expect(await page.evaluate(() => getComputedStyle(document.documentElement).colorScheme)).toBe('dark');
  });
});

test.describe('phone 390', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true });

  test('full-width thread, sticky composer, and the Sound panel as a bottom sheet', async ({ page }) => {
    await ready(page);
    const panel = page.getByTestId('panel');
    await expect(panel).not.toHaveClass(/open/);
    await page.waitForTimeout(400);
    await page.screenshot({ path: shot('phone-390.png') });
    const composer = (await page.getByTestId('composer').boundingBox())!;
    expect(composer.y + composer.height).toBeLessThanOrEqual(844);
    await page.getByTestId('sheet-toggle').click();
    await expect(panel).toHaveClass(/open/);
    await expect(page.getByTestId('change-map')).toBeInViewport();
    await page.waitForTimeout(350); // the sheet's slide
    const box = (await panel.boundingBox())!;
    expect(box.y).toBeGreaterThan(100);
    expect(box.y).toBeLessThan(844 * 0.3);
    await page.screenshot({ path: shot('phone-390-sheet.png') });
    await page.getByRole('button', { name: 'Close the Sound panel' }).click();
    await expect(panel).not.toHaveClass(/open/);
  });
});
