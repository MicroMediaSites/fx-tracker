import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useSettingsStore, type DataSource } from '../stores/settingsStore';

/**
 * Hook that listens for `environment-changed` Tauri events (emitted by SettingsModal
 * when the user switches between demo/live accounts) and updates the local Zustand
 * store's dataSource.
 *
 * In Tauri, each window runs in a separate webview with its own localStorage and
 * Zustand store instance. When the user switches accounts in one window, other windows
 * won't see the Zustand update because localStorage is per-webview. The
 * `environment-changed` event is the cross-window communication mechanism.
 *
 * This hook centralizes the listener so every window can react to account switches.
 * It optionally accepts a callback that fires after the dataSource is updated,
 * allowing each window to trigger its own refresh logic (e.g., re-fetching account
 * data, reloading candles, etc.).
 *
 * @param onEnvironmentChanged Optional callback invoked with the new DataSource
 *   after the Zustand store has been updated. Use this to trigger window-specific
 *   refresh logic.
 *
 * BUG-024: Switching account IDs does not update data source across windows.
 */
export const useEnvironmentSync = (
  onEnvironmentChanged?: (newSource: DataSource) => void
) => {
  const setDataSource = useSettingsStore((state) => state.setDataSource);

  useEffect(() => {
    const unlisten = listen<{ source: DataSource; environment: string }>(
      'environment-changed',
      (event) => {
        const newSource = event.payload.source;
        setDataSource(newSource);
        onEnvironmentChanged?.(newSource);
      }
    );

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [setDataSource, onEnvironmentChanged]);
};
