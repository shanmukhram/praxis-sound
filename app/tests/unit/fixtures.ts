import acceptance1 from '../../../fixtures/acceptance1.score.json';
import acceptance1Bell from '../../../fixtures/acceptance1_bell.score.json';
import empty from '../../../fixtures/empty.score.json';
import scoreSchema from '../../../schema/score.schema.json';
import patchSchema from '../../../schema/patch.schema.json';
import type { NScore, Score } from '../../src/score/types';
import { normalizeScore } from '../../src/score/types';

export const rawFixtures: Record<string, unknown> = { acceptance1, acceptance1_bell: acceptance1Bell, empty };
export { scoreSchema, patchSchema };

export function fixture(name = 'acceptance1'): NScore {
  return normalizeScore(rawFixtures[name] as Score);
}

/** acceptance1 with the piano unlocked, for tests that edit it. */
export function unlocked(): NScore {
  const s = fixture();
  s.layers.forEach((l) => (l.locked = false));
  return s;
}
