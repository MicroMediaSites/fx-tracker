import { getCurrentWebview } from '@tauri-apps/api/webview';

/**
 * App-wide webview zoom with discrete, keyboard-driven steps.
 *
 * Replaces the native webview zoom (zoom_hotkeys_enabled / ctrl+scroll
 * magnification), which was continuous and far too sensitive on trackpads
 * and could leave windows stuck at odd zoom levels. Instead:
 *
 *   Cmd/Ctrl + '='/'+'  -> zoom in one step
 *   Cmd/Ctrl + '-'      -> zoom out one step
 *   Cmd/Ctrl + '0'      -> reset to 100%
 *
 * Ctrl/Cmd + scroll never zooms the app (blocked in index.tsx / the
 * DISABLE_ZOOM_SCRIPT init script); trackpad pinch remains reserved for the
 * chart's own time-scale zoom, which handles it well.
 *
 * The level persists per window label so e.g. the chart window can hold a
 * different zoom than the watcher window across restarts.
 */

// Browser-style discrete zoom ladder
const ZOOM_LEVELS = [0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];
const DEFAULT_INDEX = ZOOM_LEVELS.indexOf(1.0);

const storageKey = (): string => `wickd-webview-zoom:${getCurrentWebview().label}`;

let currentIndex = DEFAULT_INDEX;

const applyZoom = (): void => {
  getCurrentWebview()
    .setZoom(ZOOM_LEVELS[currentIndex])
    .catch((err) => console.error('[webviewZoom] Failed to set zoom:', err));
  try {
    localStorage.setItem(storageKey(), String(ZOOM_LEVELS[currentIndex]));
  } catch {
    // localStorage unavailable — zoom still applies, just not persisted
  }
};

export const zoomIn = (): void => {
  if (currentIndex < ZOOM_LEVELS.length - 1) {
    currentIndex += 1;
    applyZoom();
  }
};

export const zoomOut = (): void => {
  if (currentIndex > 0) {
    currentIndex -= 1;
    applyZoom();
  }
};

export const zoomReset = (): void => {
  currentIndex = DEFAULT_INDEX;
  applyZoom();
};

/**
 * Restore the persisted zoom level and install the keyboard shortcuts.
 * Call once at app boot (index.tsx).
 */
export const initWebviewZoom = (): void => {
  // Restore persisted level (snap to the nearest defined step)
  try {
    const saved = parseFloat(localStorage.getItem(storageKey()) ?? '');
    if (!Number.isNaN(saved)) {
      let nearest = DEFAULT_INDEX;
      let minDiff = Infinity;
      ZOOM_LEVELS.forEach((level, i) => {
        const diff = Math.abs(level - saved);
        if (diff < minDiff) {
          minDiff = diff;
          nearest = i;
        }
      });
      currentIndex = nearest;
      if (currentIndex !== DEFAULT_INDEX) applyZoom();
    }
  } catch {
    // Ignore — boot at 100%
  }

  window.addEventListener('keydown', (e) => {
    if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
    if (e.key === '=' || e.key === '+') {
      e.preventDefault();
      zoomIn();
    } else if (e.key === '-') {
      e.preventDefault();
      zoomOut();
    } else if (e.key === '0') {
      e.preventDefault();
      zoomReset();
    }
  });
};
