/**
 * E2E mock for @tauri-apps/api/core
 *
 * Routes invoke() calls through window.__E2E_TAURI_INVOKE__
 * which is set up by the tauri-bridge.ts init script.
 */

declare global {
  interface Window {
    __E2E_TAURI_INVOKE__?: (cmd: string, args?: Record<string, unknown>) => unknown;
  }
}

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (window.__E2E_TAURI_INVOKE__) {
    return window.__E2E_TAURI_INVOKE__(cmd, args) as T;
  }
  throw new Error(
    `[e2e-mock] invoke('${cmd}') called but no bridge handler registered. ` +
    `Ensure the Tauri bridge init script runs before the app loads.`
  );
}
