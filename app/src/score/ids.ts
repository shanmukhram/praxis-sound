// Deterministic pattern ids, matching crates/praxis-score/src/apply/ids.rs.

/** `stem + tail`, plus `_<n>` for n >= 2, with the stem shortened so the id fits 64 chars. */
export function composeId(stem: string, tail: string, n: number): string {
  const suffix = n >= 2 ? `_${n}` : '';
  const room = Math.max(1, 64 - tail.length - suffix.length);
  return `${stem.slice(0, room)}${tail}${suffix}`;
}
