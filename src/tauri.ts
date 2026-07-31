/**
 * Tiny helpers for talking to the Tauri backend that also work in a plain
 * browser (`npm run dev`) where there is no backend. This lets the same
 * React code power both the desktop app and the bundled-sample web demo.
 */
import { invoke } from "@tauri-apps/api/core";

/** True when running inside the Tauri webview (v2 injects `__TAURI_INTERNALS__`). */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * Invoke a Tauri command, returning `null` (instead of throwing) when there is
 * no backend. Callers use this for best-effort features — e.g. reading-progress
 * persistence — that should silently no-op in the browser demo.
 */
export async function tryInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    console.warn(`invoke(${cmd}) failed:`, e);
    return null;
  }
}
