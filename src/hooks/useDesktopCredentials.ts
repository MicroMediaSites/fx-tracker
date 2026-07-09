/**
 * useDesktopCredentials — device credential management over the local store.
 *
 * AGT-650: the encrypted OANDA credential blobs moved from the cloud
 * `user_credentials` table (Zero + Clerk auth) into the local SQLite store
 * (`credential` table, `src/lib/localStore.ts`). Encryption/decryption still
 * happens entirely in the Rust crypto vault — this hook only shuttles
 * ciphertext blobs between the store and the vault. No sign-in, no network.
 */

import { useCallback, useEffect, useState } from 'react';
import { useVault } from './useVault';
import {
  LocalCredential,
  deleteCredentials as deleteStoredCredentials,
  getCredential,
  saveCredential,
} from '../lib/localStore';

// localStorage key for tracking if credentials have ever been set up
const getCredentialsSetupKey = (deviceId: string) =>
  `credentials_setup_complete_${deviceId}`;

const hasCredentialsSetupFlag = (deviceId: string): boolean => {
  try {
    return localStorage.getItem(getCredentialsSetupKey(deviceId)) === 'true';
  } catch {
    return false;
  }
};

const setCredentialsSetupFlag = (deviceId: string, value: boolean): void => {
  try {
    if (value) {
      localStorage.setItem(getCredentialsSetupKey(deviceId), 'true');
    } else {
      localStorage.removeItem(getCredentialsSetupKey(deviceId));
    }
  } catch {
    // localStorage might be unavailable
  }
};

export const useDesktopCredentials = () => {
  const {
    status,
    deviceId,
    hasPracticeCredentials,
    hasLiveCredentials,
    error,
    initialize,
    unlockVault,
    lockVault,
    setNoCredentials,
    refreshCredentialStatus,
    clearError,
    setError,
  } = useVault();

  const [isInitialized, setIsInitialized] = useState(false);
  const [credentialLoaded, setCredentialLoaded] = useState(false);
  const [deviceCredentials, setDeviceCredentials] = useState<LocalCredential | null>(null);

  // Initialize vault on mount
  useEffect(() => {
    initialize().then(() => setIsInitialized(true));
  }, [initialize]);

  // Load the stored credential row from the local store
  const refreshStoredCredentials = useCallback(async (): Promise<LocalCredential | null> => {
    try {
      const row = await getCredential();
      setDeviceCredentials(row);
      setCredentialLoaded(true);
      return row;
    } catch (err) {
      console.error('[useDesktopCredentials] Failed to read local credential row:', err);
      setDeviceCredentials(null);
      setCredentialLoaded(true);
      return null;
    }
  }, []);

  useEffect(() => {
    refreshStoredCredentials();
  }, [refreshStoredCredentials]);

  // Decide onboarding-vs-unlock once both the vault and the store have loaded
  useEffect(() => {
    if (!isInitialized || !deviceId || !credentialLoaded) return;
    if (status === 'unlocked') return;

    if (deviceCredentials) {
      setCredentialsSetupFlag(deviceId, true);
    } else if (!hasCredentialsSetupFlag(deviceId)) {
      setNoCredentials();
    }
  }, [isInitialized, deviceId, credentialLoaded, deviceCredentials, status, setNoCredentials]);

  // Unlock vault with stored credentials
  const unlock = useCallback(
    async (
      masterPassword: string,
      environment: 'practice' | 'live' = 'practice',
      usePracticeUrl: boolean = false
    ): Promise<boolean> => {
      if (!deviceCredentials?.practice_blob || !deviceCredentials.practice_account_id) {
        setError('No stored credentials found. Please set up your credentials first.');
        return false;
      }

      if (environment === 'live' && (!deviceCredentials.live_blob || !deviceCredentials.live_account_id)) {
        setError('No live credentials found. Please add live credentials first.');
        return false;
      }

      return unlockVault(
        masterPassword,
        deviceCredentials.practice_blob,
        deviceCredentials.practice_account_id,
        deviceCredentials.live_blob || null,
        deviceCredentials.live_account_id || null,
        environment,
        usePracticeUrl
      );
    },
    [deviceCredentials, unlockVault, setError]
  );

  // Store new credentials in the local store
  const storeCredentials = useCallback(
    async (
      apiKeyBlob: string,
      practiceAccountId: string,
      liveAccountId: string | null
    ): Promise<void> => {
      if (!deviceId) {
        throw new Error('Device not initialized');
      }

      const now = Date.now();
      const row: LocalCredential = {
        id: deviceId,
        device_id: deviceId,
        practice_blob: apiKeyBlob,
        practice_account_id: practiceAccountId,
        live_blob: null,
        live_account_id: liveAccountId,
        created_at: deviceCredentials?.created_at ?? now,
        updated_at: now,
      };
      await saveCredential(row);
      setCredentialsSetupFlag(deviceId, true);
      await refreshStoredCredentials();
    },
    [deviceId, deviceCredentials, refreshStoredCredentials]
  );

  // Update existing credentials (partial: only provided fields change)
  const updateCredentials = useCallback(
    async (
      practiceBlob?: string,
      liveBlob?: string | null,
      practiceAccountId?: string,
      liveAccountId?: string | null
    ): Promise<void> => {
      if (!deviceCredentials) {
        throw new Error('No existing credentials to update');
      }

      const next: LocalCredential = {
        ...deviceCredentials,
        practice_blob: practiceBlob !== undefined ? practiceBlob : deviceCredentials.practice_blob,
        live_blob: liveBlob !== undefined ? liveBlob : deviceCredentials.live_blob,
        practice_account_id:
          practiceAccountId !== undefined ? practiceAccountId : deviceCredentials.practice_account_id,
        live_account_id:
          liveAccountId !== undefined ? liveAccountId : deviceCredentials.live_account_id,
        updated_at: Date.now(),
      };
      await saveCredential(next);
      await refreshStoredCredentials();
    },
    [deviceCredentials, refreshStoredCredentials]
  );

  // Delete credentials (the "reset credentials" flow)
  const deleteCredentials = useCallback(async (): Promise<void> => {
    if (!deviceId) return;

    setCredentialsSetupFlag(deviceId, false);
    await lockVault();
    await deleteStoredCredentials();
    await refreshStoredCredentials();
    setNoCredentials();
  }, [deviceId, lockVault, refreshStoredCredentials, setNoCredentials]);

  return {
    status,
    isInitialized: isInitialized && credentialLoaded,
    deviceId,
    hasPracticeCredentials,
    hasLiveCredentials,
    hasStoredCredentials: !!deviceCredentials,
    error,
    isUnlocked: status === 'unlocked',
    isLocked: status === 'locked',
    needsOnboarding: status === 'no_credentials',
    unlock,
    lockVault,
    storeCredentials,
    updateCredentials,
    deleteCredentials,
    refreshCredentialStatus,
    refreshStoredCredentials,
    clearError,
    apiKeyBlob: deviceCredentials?.practice_blob ?? undefined,
    practiceAccountId: deviceCredentials?.practice_account_id ?? undefined,
    liveAccountId: deviceCredentials?.live_account_id ?? undefined,
    practiceBlob: deviceCredentials?.practice_blob ?? undefined,
    liveBlob: deviceCredentials?.live_blob ?? undefined,
  };
};
