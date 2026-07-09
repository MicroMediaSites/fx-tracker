import { ReactNode, useState, useCallback } from 'react';
import { useDesktopCredentials } from '../hooks/useDesktopCredentials';
import { useSettingsStore } from '../stores/settingsStore';
import { useUIStore } from '../stores/uiStore';
import { OnboardingFlow, UnlockVaultModal } from './onboarding';
import { ConfirmModal } from './ui/ConfirmModal';

// Selector for hydration status - stable reference to prevent re-renders
const selectHasHydrated = (s: { _hasHydrated: boolean }) => s._hasHydrated;

interface CredentialGateProps {
  children: ReactNode;
}

/**
 * Gate component that ensures credentials are set up and vault is unlocked
 * before rendering the main app content.
 *
 * Flow:
 * 1. Loading - initializing vault and reading the local store for stored credentials
 * 2. No credentials - show onboarding flow to create master password and enter creds
 * 3. Locked - show unlock modal to enter master password
 * 4. Unlocked - render children (the actual app)
 */
export const CredentialGate = ({ children }: CredentialGateProps) => {
  const {
    status,
    isInitialized,
    liveAccountId, // Available from the local store before unlock
    unlock,
    storeCredentials,
    deleteCredentials,
    refreshStoredCredentials,
    error,
  } = useDesktopCredentials();

  // Get the saved data source preference to restore environment on unlock
  const dataSource = useSettingsStore((s) => s.dataSource);
  // Dev-only: use practice URL when in "live" mode (for testing with demo account)
  const devUsePracticeUrlForLive = useSettingsStore((s) => s.devUsePracticeUrlForLive);
  // Wait for settings to hydrate from localStorage before using persisted values
  const settingsHydrated = useSettingsStore(selectHasHydrated);

  const [showResetConfirm, setShowResetConfirm] = useState(false);
  const [isResetting, setIsResetting] = useState(false);
  const [forceUnlockMode, setForceUnlockMode] = useState(false);
  const [isCheckingCredentials, setIsCheckingCredentials] = useState(false);
  const [checkError, setCheckError] = useState<string | null>(null);

  // Wrap unlock to restore the user's preferred environment
  // Only restore 'live' if they have live credentials stored and were using live
  // Use liveAccountId (from the store) instead of hasLiveCredentials (from vault, only available after unlock)
  const unlockWithEnvironment = useCallback(
    async (masterPassword: string) => {
      const hasStoredLive = !!liveAccountId;
      const targetEnv =
        dataSource === 'live' && hasStoredLive ? 'live' : 'practice';
      // Pass devUsePracticeUrlForLive so the backend uses practice URL even in "live" mode
      const usePracticeUrl = targetEnv === 'live' && devUsePracticeUrlForLive;

      return unlock(masterPassword, targetEnv, usePracticeUrl);
    },
    [unlock, dataSource, liveAccountId, devUsePracticeUrlForLive, settingsHydrated]
  );

  // Handle onboarding completion - store API key blob and account IDs locally
  const handleOnboardingComplete = useCallback(
    async (
      apiKeyBlob: string,
      practiceAccountId: string,
      liveAccountId: string | null
    ) => {
      await storeCredentials(apiKeyBlob, practiceAccountId, liveAccountId);
    },
    [storeCredentials]
  );

  // Handle unlock from modal
  const handleUnlock = useCallback(() => {
    // The unlock is handled by the modal calling useVault.unlockVault
    // Trigger the welcome rays animation on first window after unlock
    useUIStore.getState().triggerWelcomeRays();
  }, []);

  // Handle "I already have a master password" - re-read the local store
  const handleCheckForCredentials = useCallback(async () => {
    setCheckError(null);
    setIsCheckingCredentials(true);

    const row = await refreshStoredCredentials();

    if (row) {
      setForceUnlockMode(true);
      setIsCheckingCredentials(false);
    } else {
      setCheckError('No credentials found on this device. Set up new credentials to continue.');
      setIsCheckingCredentials(false);
    }
  }, [refreshStoredCredentials]);

  // Handle credential reset (forgot password)
  const handleResetRequest = useCallback(() => {
    setShowResetConfirm(true);
  }, []);

  const handleResetConfirm = useCallback(async () => {
    setIsResetting(true);
    try {
      await deleteCredentials();
    } finally {
      setIsResetting(false);
      setShowResetConfirm(false);
    }
  }, [deleteCredentials]);

  // Loading state - vault initializing, reading the store, or waiting for settings hydration
  // Must wait for settings hydration to ensure devUsePracticeUrlForLive is loaded from localStorage
  if (!isInitialized || status === 'loading' || !settingsHydrated) {
    return (
      <div className="min-h-screen bg-[var(--color-bg-page)] flex items-center justify-center">
        <div className="text-center">
          <div className="flex justify-center mb-4">
            <svg
              className="animate-spin h-8 w-8 text-[var(--color-info-text)]"
              viewBox="0 0 24 24"
            >
              <circle
                className="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                strokeWidth="4"
                fill="none"
              />
              <path
                className="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
              />
            </svg>
          </div>
          <p className="text-[var(--color-text-muted)]">Initializing secure vault...</p>
        </div>
      </div>
    );
  }

  // No credentials - show onboarding (unless user clicked "I already have a master password" and credentials were found)
  // Only trust the explicit 'no_credentials' status set after the local store read completes
  if (status === 'no_credentials' && !forceUnlockMode) {
    return (
      <OnboardingFlow
        onComplete={handleOnboardingComplete}
        onCheckForCredentials={handleCheckForCredentials}
        isCheckingCredentials={isCheckingCredentials}
        checkError={checkError}
      />
    );
  }

  // Locked - show unlock modal (always visible until unlocked)
  // Also show if user clicked "I already have a master password" from onboarding
  if (status === 'locked' || forceUnlockMode) {
    return (
      <>
        {/* Dimmed background placeholder */}
        <div className="min-h-screen bg-[var(--color-bg-page)]" />

        {/* Unlock modal */}
        <UnlockVaultModal
          isOpen={true}
          unlock={unlockWithEnvironment}
          onUnlock={handleUnlock}
          onReset={handleResetRequest}
          onBack={forceUnlockMode ? () => setForceUnlockMode(false) : undefined}
        />

        {/* Reset confirmation modal */}
        <ConfirmModal
          isOpen={showResetConfirm}
          onCancel={() => setShowResetConfirm(false)}
          onConfirm={handleResetConfirm}
          title="Reset Credentials"
          message="This will permanently delete your stored OANDA credentials from this device. You'll need to enter them again. Your OANDA account will not be affected."
          confirmLabel={isResetting ? 'Resetting...' : 'Reset Credentials'}
          variant="danger"
        />
      </>
    );
  }

  // Unlocked - render children
  if (status === 'unlocked') {
    return <>{children}</>;
  }

  // Fallback for any unexpected state
  return (
    <div className="min-h-screen bg-[var(--color-bg-page)] flex items-center justify-center">
      <div className="text-center max-w-md p-6">
        <p className="text-[var(--color-sell)] mb-4">
          {error || 'An unexpected error occurred with the credential vault.'}
        </p>
        <button
          onClick={() => window.location.reload()}
          className="px-4 py-2 bg-[var(--color-info)] hover:bg-[var(--color-info)]/80 rounded transition-colors"
        >
          Reload
        </button>
      </div>
    </div>
  );
}
