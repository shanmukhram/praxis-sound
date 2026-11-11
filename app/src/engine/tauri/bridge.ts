// The narrow surface of Tauri the adapter uses. The desktop build passes the real one
// (@tauri-apps/api); tests pass a fake that answers like crates/praxis-app.

export interface Bridge {
  /** Calls a `#[tauri::command]` in crates/praxis-app. Rejects with the command's error. */
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  /**
   * Calls a command with a raw byte body (a dropped clip) and string headers (ASCII; encode
   * anything else). Absent in bridges that cannot send bytes.
   */
  invokeBytes?<T>(cmd: string, body: Uint8Array, headers: Record<string, string>): Promise<T>;
  /** Subscribes to an app event; resolves to the unsubscribe function. */
  listen<T>(event: string, cb: (payload: T) => void): Promise<() => void>;
  /** A URL the WebView can load a local file from (`<audio>`, `fetch`). */
  fileUrl(path: string): string;
}

/** The error a praxis-app command rejects with (`CmdError` in crates/praxis-app/src/commands.rs). */
export interface CmdError {
  code: string;
  message: string;
  issues?: unknown[];
  patch_error?: unknown;
}

export function isCmdError(e: unknown): e is CmdError {
  return typeof e === 'object' && e !== null && typeof (e as CmdError).code === 'string' && typeof (e as CmdError).message === 'string';
}

/** True inside the Tauri shell (its IPC globals are present). */
export function inTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** The real bridge. Imported lazily so the browser build never loads Tauri's IPC code. */
export async function tauriBridge(): Promise<Bridge> {
  const core = await import('@tauri-apps/api/core');
  const event = await import('@tauri-apps/api/event');
  return {
    invoke: (cmd, args) => core.invoke(cmd, args),
    invokeBytes: (cmd, body, headers) => core.invoke(cmd, body, { headers }),
    listen: (name, cb) => event.listen(name, (e) => cb(e.payload as never)),
    fileUrl: (path) => core.convertFileSrc(path),
  };
}
