// FFT and a log-frequency spectrogram image for the Mix view (FR-30).

import type { DecodedAudio } from '../engine/wav';

/** In-place radix-2 FFT on real/imag arrays (length a power of two). */
export function fft(re: Float32Array, im: Float32Array): void {
  const n = re.length;
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      [re[i], re[j]] = [re[j], re[i]];
      [im[i], im[j]] = [im[j], im[i]];
    }
  }
  for (let len = 2; len <= n; len <<= 1) {
    const ang = (-2 * Math.PI) / len;
    const wr = Math.cos(ang);
    const wi = Math.sin(ang);
    for (let i = 0; i < n; i += len) {
      let cr = 1;
      let ci = 0;
      for (let k = 0; k < len / 2; k++) {
        const a = i + k;
        const b = a + len / 2;
        const tr = re[b] * cr - im[b] * ci;
        const ti = re[b] * ci + im[b] * cr;
        re[b] = re[a] - tr;
        im[b] = im[a] - ti;
        re[a] += tr;
        im[a] += ti;
        const ncr = cr * wr - ci * wi;
        ci = cr * wi + ci * wr;
        cr = ncr;
      }
    }
  }
}

/** Magnitude spectrogram in dB: columns of `size/2` bins, hop `hop`. Mono mix of all channels. */
export function spectrogram(audio: DecodedAudio, size = 2048, hop = 1024): { cols: Float32Array[]; bins: number } {
  const ch = audio.channels;
  const n = ch[0]?.length ?? 0;
  const win = new Float32Array(size).map((_, i) => 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / (size - 1)));
  const cols: Float32Array[] = [];
  const re = new Float32Array(size);
  const im = new Float32Array(size);
  for (let start = 0; start + size <= n; start += hop) {
    for (let i = 0; i < size; i++) {
      let x = 0;
      for (const c of ch) x += c[start + i];
      re[i] = (x / ch.length) * win[i];
      im[i] = 0;
    }
    fft(re, im);
    const col = new Float32Array(size / 2);
    for (let k = 0; k < size / 2; k++) col[k] = 10 * Math.log10(re[k] * re[k] + im[k] * im[k] + 1e-12);
    cols.push(col);
  }
  return { cols, bins: size / 2 };
}


/** Renders a spectrogram of `audio` into `canvas` (log frequency, dark-to-light ramp). */
export function drawSpectrogram(canvas: HTMLCanvasElement, audio: DecodedAudio): void {
  const { cols, bins } = spectrogram(audio);
  const rows = 120;
  canvas.width = Math.max(1, cols.length);
  canvas.height = rows;
  const g = canvas.getContext('2d')!;
  const img = g.createImageData(canvas.width, rows);
  const nyq = audio.sampleRate / 2;
  const fmin = 30;
  let max = -Infinity;
  for (const col of cols) for (const v of col) max = Math.max(max, v);
  for (let x = 0; x < cols.length; x++) {
    for (let y = 0; y < rows; y++) {
      const f = fmin * Math.pow(nyq / fmin, 1 - y / (rows - 1));
      const k = Math.min(bins - 1, Math.round((f / nyq) * bins));
      const t = Math.pow(Math.max(0, Math.min(1, (cols[x][k] - (max - 90)) / 90)), 1.6);
      const [r, gg, b] = ramp(t);
      const o = (y * canvas.width + x) * 4;
      img.data[o] = r;
      img.data[o + 1] = gg;
      img.data[o + 2] = b;
      img.data[o + 3] = 255;
    }
  }
  g.putImageData(img, 0, 0);
}

/** Dark-to-light ramp: near-black -> blue -> amber -> white. */
function ramp(t: number): [number, number, number] {
  const stops: [number, number, number, number][] = [
    [0, 17, 17, 17],
    [0.4, 74, 74, 74],
    [0.75, 245, 165, 36],
    [1, 255, 240, 210],
  ];
  for (let i = 1; i < stops.length; i++) {
    if (t <= stops[i][0]) {
      const [t0, r0, g0, b0] = stops[i - 1];
      const [t1, r1, g1, b1] = stops[i];
      const u = (t - t0) / (t1 - t0);
      return [r0 + (r1 - r0) * u, g0 + (g1 - g0) * u, b0 + (b1 - b0) * u];
    }
  }
  return [250, 245, 230];
}
