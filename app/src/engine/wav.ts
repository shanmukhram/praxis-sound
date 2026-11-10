// Minimal WAV (RIFF) encoder and decoder: PCM 16/24-bit and IEEE float 32-bit.

import type { SampleFormat } from './EngineAdapter';

export interface DecodedAudio {
  sampleRate: number;
  channels: Float32Array[];
}

export function encodeWav(channels: Float32Array[], sampleRate: number, format: SampleFormat = 'wav16'): ArrayBuffer {
  const nch = channels.length;
  const frames = channels[0]?.length ?? 0;
  const bytes = format === 'wav16' ? 2 : format === 'wav24' ? 3 : 4;
  const isFloat = format === 'wav32f';
  const dataLen = frames * nch * bytes;
  const buf = new ArrayBuffer(44 + dataLen);
  const v = new DataView(buf);
  const str = (o: number, s: string) => [...s].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
  str(0, 'RIFF');
  v.setUint32(4, 36 + dataLen, true);
  str(8, 'WAVE');
  str(12, 'fmt ');
  v.setUint32(16, 16, true);
  v.setUint16(20, isFloat ? 3 : 1, true);
  v.setUint16(22, nch, true);
  v.setUint32(24, sampleRate, true);
  v.setUint32(28, sampleRate * nch * bytes, true);
  v.setUint16(32, nch * bytes, true);
  v.setUint16(34, bytes * 8, true);
  str(36, 'data');
  v.setUint32(40, dataLen, true);
  let o = 44;
  for (let i = 0; i < frames; i++) {
    for (let c = 0; c < nch; c++) {
      const x = Math.max(-1, Math.min(1, channels[c][i]));
      if (isFloat) {
        v.setFloat32(o, x, true);
      } else if (bytes === 2) {
        v.setInt16(o, Math.round(x < 0 ? x * 0x8000 : x * 0x7fff), true);
      } else {
        const n = Math.round(x < 0 ? x * 0x800000 : x * 0x7fffff);
        v.setUint8(o, n & 0xff);
        v.setUint8(o + 1, (n >> 8) & 0xff);
        v.setUint8(o + 2, (n >> 16) & 0xff);
      }
      o += bytes;
    }
  }
  return buf;
}

export function decodeWav(buf: ArrayBuffer): DecodedAudio {
  const v = new DataView(buf);
  const tag = (o: number) => String.fromCharCode(v.getUint8(o), v.getUint8(o + 1), v.getUint8(o + 2), v.getUint8(o + 3));
  if (tag(0) !== 'RIFF' || tag(8) !== 'WAVE') throw new Error('not a WAV file');
  let o = 12;
  let fmt = 1;
  let nch = 0;
  let sr = 0;
  let bits = 0;
  while (o + 8 <= buf.byteLength) {
    const id = tag(o);
    const len = v.getUint32(o + 4, true);
    const body = o + 8;
    if (id === 'fmt ') {
      fmt = v.getUint16(body, true);
      nch = v.getUint16(body + 2, true);
      sr = v.getUint32(body + 4, true);
      bits = v.getUint16(body + 14, true);
    } else if (id === 'data') {
      const bytes = bits / 8;
      const frames = Math.floor(Math.min(len, buf.byteLength - body) / (bytes * nch));
      const channels = Array.from({ length: nch }, () => new Float32Array(frames));
      let p = body;
      for (let i = 0; i < frames; i++) {
        for (let c = 0; c < nch; c++) {
          let x: number;
          if (fmt === 3) x = v.getFloat32(p, true);
          else if (bytes === 2) x = v.getInt16(p, true) / 0x8000;
          else {
            let n = v.getUint8(p) | (v.getUint8(p + 1) << 8) | (v.getUint8(p + 2) << 16);
            if (n & 0x800000) n -= 0x1000000;
            x = n / 0x800000;
          }
          channels[c][i] = x;
          p += bytes;
        }
      }
      return { sampleRate: sr, channels };
    }
    o = body + len + (len % 2);
  }
  throw new Error('WAV has no data chunk');
}
