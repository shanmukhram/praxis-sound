import { describe, expect, it } from 'vitest';
import generatedNow from '../../src/score/generated.ts?raw';
import { generate } from '../../scripts/gen-types.mjs';
import { checkSchema } from './support/jsonSchema';
import { patchSchema, rawFixtures, scoreSchema } from './fixtures';
import { fixture } from './fixtures';
import { MockEngine } from '../../src/engine/MockEngine';
import { muteLayer } from '../../src/score/patchBuilder';

describe('generated types', () => {
  it('src/score/generated.ts matches the frozen schemas (run `npm run gen:types` if not)', () => {
    expect(generatedNow.replace(/\r\n/g, '\n')).toBe(generate());
  });
});

describe('fixtures and schemas', () => {
  for (const [name, value] of Object.entries(rawFixtures)) {
    it(`${name} matches score.schema.json`, () => {
      expect(checkSchema(scoreSchema, value)).toEqual([]);
    });
  }

  it('normalised scores still match the schema', () => {
    expect(checkSchema(scoreSchema, fixture())).toEqual([]);
  });

  it('the checker catches a bad patch', () => {
    expect(checkSchema(patchSchema, { ops: [{ op: 'set_tempo' }] }).length).toBeGreaterThan(0);
    expect(checkSchema(patchSchema, { ops: [], scope: { bars: { start: 1 } } }).length).toBeGreaterThan(0);
  });

  it('inverses the mock returns match patch.schema.json', async () => {
    const e = new MockEngine();
    const { score } = await e.load(fixture());
    const r = await e.applyPatch(muteLayer(score, 'kick', true));
    expect(r.ok).toBe(true);
    if (r.ok) expect(checkSchema(patchSchema, r.inverse)).toEqual([]);
  });
});
