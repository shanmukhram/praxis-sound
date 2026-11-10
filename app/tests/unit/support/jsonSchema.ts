// A small JSON Schema checker for the subset schemars emits, used to prove that fixtures and
// every patch the UI builds match the frozen schemas. Test support only.

type Schema = Record<string, any>;

export function checkSchema(root: Schema, value: unknown): string[] {
  const errors: string[] = [];
  walk(root, root, value, '', errors);
  return errors;
}

function typeOf(v: unknown): string {
  if (v === null) return 'null';
  if (Array.isArray(v)) return 'array';
  if (typeof v === 'number') return Number.isInteger(v) ? 'integer' : 'number';
  return typeof v;
}

function matchesType(t: string, v: unknown): boolean {
  const actual = typeOf(v);
  return t === actual || (t === 'number' && actual === 'integer');
}

function walk(root: Schema, s: Schema, v: unknown, path: string, errors: string[]): void {
  if (s.$ref) {
    const name = String(s.$ref).replace('#/$defs/', '');
    const def = root.$defs?.[name];
    if (!def) {
      errors.push(`${path}: unresolved ${s.$ref}`);
      return;
    }
    walk(root, def, v, path, errors);
    return;
  }
  if ('const' in s && JSON.stringify(s.const) !== JSON.stringify(v)) {
    errors.push(`${path}: expected const ${JSON.stringify(s.const)}`);
    return;
  }
  if (s.enum && !s.enum.some((e: unknown) => JSON.stringify(e) === JSON.stringify(v))) {
    errors.push(`${path}: ${JSON.stringify(v)} not in enum`);
    return;
  }
  if (s.oneOf) {
    const passing = s.oneOf.filter((sub: Schema) => checkSub(root, sub, v));
    if (passing.length !== 1) errors.push(`${path}: matches ${passing.length} oneOf branches (want 1)`);
    return;
  }
  if (s.anyOf) {
    if (!s.anyOf.some((sub: Schema) => checkSub(root, sub, v))) errors.push(`${path}: matches no anyOf branch`);
    return;
  }
  if (s.type) {
    const types: string[] = Array.isArray(s.type) ? s.type : [s.type];
    if (!types.some((t) => matchesType(t, v))) {
      errors.push(`${path}: expected ${types.join('|')}, got ${typeOf(v)}`);
      return;
    }
  }
  if (typeof v === 'number') {
    if (s.minimum !== undefined && v < s.minimum) errors.push(`${path}: below minimum ${s.minimum}`);
    if (s.maximum !== undefined && v > s.maximum) errors.push(`${path}: above maximum ${s.maximum}`);
  }
  if (Array.isArray(v)) {
    if (s.items) v.forEach((x, i) => walk(root, s.items, x, `${path}/${i}`, errors));
    if (s.uniqueItems && new Set(v.map((x) => JSON.stringify(x))).size !== v.length) errors.push(`${path}: items not unique`);
  }
  if (v && typeof v === 'object' && !Array.isArray(v)) {
    const obj = v as Record<string, unknown>;
    for (const r of s.required ?? []) if (!(r in obj)) errors.push(`${path}: missing required "${r}"`);
    for (const [k, x] of Object.entries(obj)) {
      if (x === undefined) continue;
      if (s.properties?.[k]) walk(root, s.properties[k], x, `${path}/${k}`, errors);
      else if (s.additionalProperties === false) errors.push(`${path}: unknown property "${k}"`);
      else if (s.additionalProperties && typeof s.additionalProperties === 'object') {
        walk(root, s.additionalProperties, x, `${path}/${k}`, errors);
      }
    }
  }
}

function checkSub(root: Schema, s: Schema, v: unknown): boolean {
  const e: string[] = [];
  walk(root, s, v, '', e);
  return e.length === 0;
}
