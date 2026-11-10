// Tiny DOM and canvas helpers (no framework, per ADR-0002).

type Attrs = Record<string, string | number | boolean | undefined | null | ((e: any) => void)>;

/** Creates an element: h('button', {class: 'x', onclick: fn}, 'Label'). */
export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  ...children: (Node | string | null | undefined | false)[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === null || v === false) continue;
    if (k.startsWith('on') && typeof v === 'function') {
      el.addEventListener(k.slice(2), v as EventListener);
    } else if (k === 'class') {
      el.className = String(v);
    } else if (k === 'style') {
      el.setAttribute('style', String(v));
    } else if (v === true) {
      el.setAttribute(k, '');
    } else {
      el.setAttribute(k, String(v));
    }
  }
  for (const c of children) {
    if (c === null || c === undefined || c === false) continue;
    el.append(typeof c === 'string' ? document.createTextNode(c) : c);
  }
  return el;
}

export function clear(el: Element): void {
  while (el.firstChild) el.firstChild.remove();
}

/** Reads a CSS custom property from :root. */
export function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/**
 * Sizes a canvas for its CSS box and the device pixel ratio and returns a context scaled so
 * drawing uses CSS pixels.
 */
export function fitCanvas(canvas: HTMLCanvasElement, cssHeight?: number): { ctx: CanvasRenderingContext2D; w: number; h: number } {
  const dpr = window.devicePixelRatio || 1;
  const w = Math.max(1, canvas.clientWidth);
  const hgt = Math.max(1, cssHeight ?? canvas.clientHeight);
  if (cssHeight !== undefined) canvas.style.height = `${cssHeight}px`;
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(hgt * dpr)) {
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(hgt * dpr);
  }
  const ctx = canvas.getContext('2d')!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, hgt);
  return { ctx, w, h: hgt };
}

/** Stable, restrained colour per layer index. */
export function layerColor(i: number): string {
  const palette = ['#7fb4ff', '#e3a86b', '#7ccf9a', '#c9a2ff', '#e88a9a', '#6fd0d0', '#d6cf72'];
  return palette[i % palette.length];
}

export function fmtTime(s: number): string {
  const m = Math.floor(s / 60);
  const sec = s - m * 60;
  return `${String(m).padStart(2, '0')}:${sec.toFixed(1).padStart(4, '0')}`;
}

export function fmtDb(db: number, digits = 1): string {
  return Number.isFinite(db) ? `${db.toFixed(digits)}` : '-inf';
}

/** True when a keyboard event comes from a text field (shortcuts should not fire). */
export function isTyping(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null;
  if (!t) return false;
  const tag = t.tagName;
  if (tag === 'TEXTAREA' || t.isContentEditable) return true;
  if (tag === 'SELECT') return true;
  if (tag === 'INPUT') {
    const type = (t as HTMLInputElement).type;
    return !['checkbox', 'radio', 'button', 'range'].includes(type);
  }
  return false;
}
