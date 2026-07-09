import { useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emit, listen } from '@tauri-apps/api/event';
import { useVaultStore, PasswordStrength } from '../stores/vaultStore';

/**
 * Hook for managing the credential vault
 *
 * Provides methods for:
 * - Checking password strength
 * - Creating master password and encrypting credentials
 * - Unlocking the vault
 * - Checking vault status
 */
export const useVault = () => {
  const {
    status,
    deviceId,
    hasPracticeCredentials,
    hasLiveCredentials,
    rateLimitMessage,
    rateLimitSeconds,
    error,
    setStatus,
    setDeviceId,
    setCredentialStatus,
    setUnlockedAt,
    setRateLimitMessage,
    setRateLimitSeconds,
    setError,
  } = useVaultStore();

  // Initialize - get device ID and check vault status
  const initialize = useCallback(async () => {
    try {
      // Get device ID
      const id = await invoke<string>('get_device_id');
      setDeviceId(id);

      // Check if vault is unlocked
      const unlocked = await invoke<boolean>('is_vault_unlocked');
      if (unlocked) {
        const practice = await invoke<boolean>('has_practice_credentials');
        const live = await invoke<boolean>('has_live_credentials');
        setCredentialStatus(practice, live);
        setStatus('unlocked');
      } else {
        // Vault is locked - frontend needs to check if credentials exist in Zero
        // This will be determined by the component that uses this hook
        setStatus('locked');
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setStatus('locked');
    }
  }, [setDeviceId, setStatus, setCredentialStatus, setError]);

  // Listen for vault-unlocked events from other windows
  useEffect(() => {
    const unlistenUnlock = listen<{ practice: boolean; live: boolean }>('vault-unlocked', (event) => {
      // Another window unlocked the vault - update our status
      setCredentialStatus(event.payload.practice, event.payload.live);
      setStatus('unlocked');
    });

    const unlistenLock = listen('vault-locked', () => {
      // Another window locked the vault - update our status
      setCredentialStatus(false, false);
      setStatus('locked');
    });

    return () => {
      unlistenUnlock.then((fn) => fn());
      unlistenLock.then((fn) => fn());
    };
  }, [setCredentialStatus, setStatus]);

  // Check password strength (local only - fast)
  const checkPasswordStrengthLocal = useCallback(async (password: string): Promise<PasswordStrength> => {
    return invoke<PasswordStrength>('check_password_strength_local', { password });
  }, []);

  // Check password strength (includes HIBP - slower).
  //
  // AGT-669 (privacy): verified k-anonymous. The backend
  // (`check_password_strength` -> wickd-core `check_hibp`) hashes the password
  // with SHA-1 locally and sends ONLY the first 5 hex chars of that hash as a
  // range query to api.pwnedpasswords.com; the remaining suffix is matched
  // locally against the returned list. The full hash, and the password itself,
  // never leave the device. No fix needed — the check is kept.
  const checkPasswordStrength = useCallback(async (password: string): Promise<PasswordStrength> => {
    return invoke<PasswordStrength>('check_password_strength', { password });
  }, []);

  // Encrypt API key with master password
  // Account ID is stored in Zero (not encrypted)
  // Returns encrypted API key blob for storage
  const encryptCredentials = useCallback(async (
    masterPassword: string,
    apiKey: string,
    accountId: string,
    environment?: 'practice' | 'live',
  ): Promise<string> => {
    return invoke<string>('encrypt_credentials', {
      masterPassword,
      apiKey,
      accountId,
      environment: environment || 'practice',
    });
  }, []);

  // Check rate limit before unlock attempt
  const checkRateLimit = useCallback(async (): Promise<number | null> => {
    try {
      const seconds = await invoke<number | null>('check_unlock_rate_limit');
      setRateLimitSeconds(seconds);

      if (seconds !== null) {
        const message = await invoke<string | null>('get_rate_limit_status');
        setRateLimitMessage(message);
      } else {
        setRateLimitMessage(null);
      }

      return seconds;
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      return null;
    }
  }, [setRateLimitSeconds, setRateLimitMessage, setError]);

  // Unlock vault with master password
  // Decrypts both practice and live blobs (if available)
  // Account IDs come from Zero
  const unlockVault = useCallback(async (
    masterPassword: string,
    practiceBlob: string,
    practiceAccountId: string,
    liveBlob: string | null,
    liveAccountId: string | null,
    environment: 'practice' | 'live' = 'practice',
    usePracticeUrl: boolean = false, // Dev-only: use practice URL when environment is 'live'
  ): Promise<boolean> => {
    try {
      setError(null);

      // Check rate limit first
      const waitSeconds = await checkRateLimit();
      if (waitSeconds !== null && waitSeconds > 0) {
        throw new Error(`Rate limited. Please wait ${waitSeconds} seconds.`);
      }

      await invoke('unlock_vault', {
        masterPassword,
        practiceBlob,
        practiceAccountId,
        liveBlob: liveBlob || undefined,
        liveAccountId: liveAccountId || undefined,
        environment,
        usePracticeUrl,
      });

      // Update status
      const practice = await invoke<boolean>('has_practice_credentials');
      const live = await invoke<boolean>('has_live_credentials');
      setCredentialStatus(practice, live);
      setStatus('unlocked');
      setUnlockedAt(Date.now());
      // Store timestamp in localStorage (shared across windows) for animation timing
      localStorage.setItem('vault-unlocked-at', Date.now().toString());
      setRateLimitMessage(null);
      setRateLimitSeconds(null);

      // Notify other windows that vault was unlocked
      emit('vault-unlocked', { practice, live });

      return true;
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      // Refresh rate limit status after failed attempt
      await checkRateLimit();
      return false;
    }
  }, [setError, setCredentialStatus, setStatus, setUnlockedAt, setRateLimitMessage, setRateLimitSeconds, checkRateLimit]);

  // Lock the vault (clear credentials from memory)
  const lockVault = useCallback(async (): Promise<void> => {
    try {
      await invoke('lock_vault');
      setCredentialStatus(false, false);
      setStatus('locked');
      // Notify other windows that vault was locked
      emit('vault-locked', {});
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [setCredentialStatus, setStatus, setError]);

  // Set status to no_credentials (for when Zero has no stored blobs)
  const setNoCredentials = useCallback(() => {
    setStatus('no_credentials');
  }, [setStatus]);

  // Refresh credential status from Rust backend (call after adding/updating credentials)
  const refreshCredentialStatus = useCallback(async () => {
    try {
      const practice = await invoke<boolean>('has_practice_credentials');
      const live = await invoke<boolean>('has_live_credentials');
      setCredentialStatus(practice, live);
    } catch (err) {
      // Silently fail - status will be refreshed on next unlock
    }
  }, [setCredentialStatus]);

  return {
    // State
    status,
    deviceId,
    hasPracticeCredentials,
    hasLiveCredentials,
    rateLimitMessage,
    rateLimitSeconds,
    error,
    isUnlocked: status === 'unlocked',
    isLocked: status === 'locked',
    needsOnboarding: status === 'no_credentials',

    // Actions
    initialize,
    checkPasswordStrengthLocal,
    checkPasswordStrength,
    encryptCredentials,
    checkRateLimit,
    unlockVault,
    lockVault,
    setNoCredentials,
    refreshCredentialStatus,
    clearError: () => setError(null),
    setError,
  };
}
