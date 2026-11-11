// Shown at start when Praxis Sound closed on a crash it has not told the user about: where the
// local report is and how to send it. Reports are written by the panic hook
// (crates/praxis-cli/src/ops/crash.rs) and never uploaded.

import type { Bridge } from '../engine/tauri/bridge';
import { h } from './dom';
import './packs.css';

/** `crash_reports` (crates/praxis-cli/src/ops/crash.rs `CrashReports`). */
export interface CrashReports {
  dir: string;
  reports: { path: string; file: string; program: string; time: string; message: string; unseen: boolean }[];
  unseen: number;
  how_to_send: string;
}

/** The notice's words, or null when there is nothing new to tell. */
export function crashNoticeText(r: CrashReports): { title: string; detail: string; path: string } | null {
  const newest = r.reports.find((x) => x.unseen);
  if (!newest) return null;
  const more = r.unseen > 1 ? ` (${r.unseen} reports since you last looked)` : '';
  return {
    title: `Praxis Sound closed unexpectedly${more}.`,
    detail: `A report was saved on this computer: ${newest.path}. ${r.how_to_send}`,
    path: newest.path,
  };
}

/** Shows the crash notice when there is an unseen report; dismissing marks it seen. */
export async function mountCrashNotice(root: HTMLElement, bridge: Bridge): Promise<void> {
  let reports: CrashReports;
  try {
    reports = await bridge.invoke<CrashReports>('crash_reports', { seen: false });
  } catch {
    return;
  }
  const t = crashNoticeText(reports);
  if (!t) return;
  const copy = h('button', { class: 'ghost' }, 'Copy the file path');
  const notice = h(
    'div',
    { class: 'crash-notice', role: 'alert', 'data-testid': 'crash-notice' },
    h('b', {}, t.title),
    h('p', {}, t.detail),
    h(
      'div',
      { class: 'crash-actions' },
      copy,
      h(
        'button',
        {
          class: 'primary',
          onclick: () => {
            notice.remove();
            void bridge.invoke('crash_reports', { seen: true }).catch(() => undefined);
          },
        },
        'OK',
      ),
    ),
  );
  copy.addEventListener('click', () => {
    void navigator.clipboard
      ?.writeText(t.path)
      .then(() => (copy.textContent = 'Copied'))
      .catch(() => (copy.textContent = 'Copy failed: select the path above'));
  });
  root.append(notice);
}
