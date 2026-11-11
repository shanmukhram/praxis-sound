// UTF-8 TextEncoder / TextDecoder for the AudioWorklet scope, which Chromium does not give them.
// The wasm-bindgen glue reads them when it is evaluated, so worklet.ts imports this first.

type Enc = { encode(s?: string): Uint8Array; encodeInto(s: string, dest: Uint8Array): { read: number; written: number } };

function utf8Encode(s: string): Uint8Array {
  const out: number[] = [];
  for (let i = 0; i < s.length; i++) {
    let c = s.charCodeAt(i);
    if (c >= 0xd800 && c <= 0xdbff && i + 1 < s.length) {
      const d = s.charCodeAt(i + 1);
      if (d >= 0xdc00 && d <= 0xdfff) {
        c = 0x10000 + ((c - 0xd800) << 10) + (d - 0xdc00);
        i++;
      }
    }
    if (c < 0x80) out.push(c);
    else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
    else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
  }
  return new Uint8Array(out);
}

function utf8Decode(b: Uint8Array): string {
  let s = '';
  for (let i = 0; i < b.length; ) {
    const x = b[i++];
    let c: number;
    if (x < 0x80) c = x;
    else if (x < 0xe0) c = ((x & 31) << 6) | (b[i++] & 63);
    else if (x < 0xf0) c = ((x & 15) << 12) | ((b[i++] & 63) << 6) | (b[i++] & 63);
    else c = ((x & 7) << 18) | ((b[i++] & 63) << 12) | ((b[i++] & 63) << 6) | (b[i++] & 63);
    s += String.fromCodePoint(c);
  }
  return s;
}

const g = globalThis as unknown as Record<string, unknown>;

if (typeof g.TextEncoder === 'undefined') {
  g.TextEncoder = class implements Enc {
    readonly encoding = 'utf-8';
    encode(s = ''): Uint8Array {
      return utf8Encode(s);
    }
    encodeInto(s: string, dest: Uint8Array): { read: number; written: number } {
      const b = utf8Encode(s);
      const n = Math.min(b.length, dest.length);
      dest.set(b.subarray(0, n));
      return { read: s.length, written: n };
    }
  };
}

if (typeof g.TextDecoder === 'undefined') {
  g.TextDecoder = class {
    readonly encoding = 'utf-8';
    decode(input?: ArrayBufferView | ArrayBuffer): string {
      if (!input) return '';
      const bytes = input instanceof ArrayBuffer ? new Uint8Array(input) : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      return utf8Decode(bytes);
    }
  };
}

export {};
