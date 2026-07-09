import { useState, useCallback } from 'react';
import { check, Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

export type UpdateStatus =
  | 'idle'
  | 'checking'
  | 'available'
  | 'downloading'
  | 'ready'
  | 'error'
  | 'up-to-date';

export type BuildMode = 'development' | 'staging' | 'production';

export interface UpdateState {
  status: UpdateStatus;
  update: Update | null;
  progress: number; // 0-100
  error: string | null;
  currentVersion: string;
  newVersion: string | null;
  buildMode: BuildMode;
}

const initialState: UpdateState = {
  status: 'idle',
  update: null,
  progress: 0,
  error: null,
  currentVersion: __APP_VERSION__,
  newVersion: null,
  buildMode: __BUILD_MODE__,
};

// Declare the globals injected by Vite
declare const __APP_VERSION__: string;
declare const __BUILD_MODE__: 'development' | 'staging' | 'production';

export function useAppUpdater() {
  const [state, setState] = useState<UpdateState>(initialState);

  const checkForUpdates = useCallback(async (): Promise<boolean> => {
    // Skip in development mode
    if (import.meta.env.DEV) {
      setState(prev => ({
        ...prev,
        status: 'error',
        error: 'Updates are disabled in development mode',
      }));
      return false;
    }

    setState(prev => ({ ...prev, status: 'checking', error: null }));

    try {
      const update = await check();

      if (update) {
        setState(prev => ({
          ...prev,
          status: 'available',
          update,
          newVersion: update.version,
        }));
        return true;
      } else {
        setState(prev => ({
          ...prev,
          status: 'up-to-date',
          update: null,
        }));
        return false;
      }
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to check for updates';
      setState(prev => ({
        ...prev,
        status: 'error',
        error: errorMessage,
      }));
      return false;
    }
  }, []);

  const downloadAndInstall = useCallback(async () => {
    if (!state.update) {
      setState(prev => ({
        ...prev,
        status: 'error',
        error: 'No update available to download',
      }));
      return;
    }

    setState(prev => ({ ...prev, status: 'downloading', progress: 0 }));

    try {
      let downloaded = 0;
      let contentLength = 0;

      await state.update.downloadAndInstall((event) => {
        switch (event.event) {
          case 'Started':
            contentLength = event.data.contentLength || 0;
            break;
          case 'Progress':
            downloaded += event.data.chunkLength;
            const progress = contentLength > 0
              ? Math.round((downloaded / contentLength) * 100)
              : 0;
            setState(prev => ({ ...prev, progress }));
            break;
          case 'Finished':
            setState(prev => ({ ...prev, status: 'ready', progress: 100 }));
            break;
        }
      });
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to download update';
      setState(prev => ({
        ...prev,
        status: 'error',
        error: errorMessage,
      }));
    }
  }, [state.update]);

  const restartApp = useCallback(async () => {
    try {
      await relaunch();
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : 'Failed to restart app';
      setState(prev => ({
        ...prev,
        status: 'error',
        error: errorMessage,
      }));
    }
  }, []);

  const reset = useCallback(() => {
    setState(initialState);
  }, []);

  const setAvailable = useCallback((update: Update) => {
    setState(prev => ({
      ...prev,
      status: 'available' as UpdateStatus,
      update,
      newVersion: update.version,
    }));
  }, []);

  return {
    ...state,
    checkForUpdates,
    downloadAndInstall,
    restartApp,
    reset,
    setAvailable,
  };
}
