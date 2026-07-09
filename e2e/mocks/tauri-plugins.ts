/**
 * E2E mock for @tauri-apps/plugin-*
 *
 * Covers: plugin-updater, plugin-process, plugin-shell, plugin-notification
 */

declare global {
  interface Window {
    __E2E_UPDATER_CHECK_RESPONSE__?: unknown;
  }
}

// plugin-updater
export async function check() {
  // Allow per-test override via addInitScript setting window.__E2E_UPDATER_CHECK_RESPONSE__
  if (typeof window !== 'undefined' && window.__E2E_UPDATER_CHECK_RESPONSE__ !== undefined) {
    return window.__E2E_UPDATER_CHECK_RESPONSE__;
  }
  return null; // No update available
}

export async function installUpdate() {}

// plugin-process
export async function relaunch() {}
export async function exit() {}

// plugin-shell
export async function open(_url: string) {}

// plugin-notification
export function isPermissionGranted() {
  return Promise.resolve(true);
}

export function requestPermission() {
  return Promise.resolve('granted');
}

export function sendNotification(_options: unknown) {}

// Default export for modules that use default import
export default {
  check,
  installUpdate,
  relaunch,
  exit,
  open,
  isPermissionGranted,
  requestPermission,
  sendNotification,
};
