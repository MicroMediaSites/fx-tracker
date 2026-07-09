import { useState, useRef, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useCredentialFormStore } from '../../stores/credentialFormStore';
import { useSettingsStore } from '../../stores/settingsStore';

type Environment = 'practice' | 'live';

interface UpdateCredentialsModalProps {
  isOpen: boolean;
  environment: Environment;
  currentBlob: string | undefined; // The encrypted blob for this environment
  onComplete: (updatedAccountId: string, environment: Environment) => Promise<void>;
  onApiKeyUpdate: (newBlob: string, environment: Environment, newAccountId?: string) => Promise<void>;
  onDelete: (environment: Environment) => Promise<void>;
  onCancel: () => void;
}

export const UpdateCredentialsModal = ({
  isOpen,
  environment,
  currentBlob,
  onComplete,
  onApiKeyUpdate,
  onDelete,
  onCancel,
}: UpdateCredentialsModalProps) => {
  // Use zustand store for form values - persists across component unmounts
  const {
    masterPassword,
    setMasterPassword,
    apiKey,
    setApiKey,
    accountId,
    setAccountId,
    clearForm,
  } = useCredentialFormStore();

  // Dev setting to use practice URL when in "live" mode
  const setDevUsePracticeUrlForLive = useSettingsStore((s) => s.setDevUsePracticeUrlForLive);

  // Local state for UI-only concerns
  const [error, setError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [isDeleting, setIsDeleting] = useState(false);
  // Expandable API key update section
  const [isUpdatingApiKey, setIsUpdatingApiKey] = useState(false);
  const [showPassword, setShowPassword] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);
  // Dev-only: allow using demo credentials in the "live" slot for testing
  const [allowDemoAccount, setAllowDemoAccount] = useState(false);
  const isDev = import.meta.env.DEV;

  const accountInputRef = useRef<HTMLInputElement>(null);
  const passwordInputRef = useRef<HTMLInputElement>(null);

  const isLive = environment === 'live';

  // Focus appropriate input when modal opens or API key mode changes
  useEffect(() => {
    if (isOpen) {
      setTimeout(() => {
        if (isUpdatingApiKey) {
          passwordInputRef.current?.focus();
        } else {
          accountInputRef.current?.focus();
        }
      }, 100);
    }
  }, [isOpen, isUpdatingApiKey]);

  // Clear form when user explicitly cancels
  const handleCancel = () => {
    clearForm();
    setError(null);
    setShowDeleteConfirm(false);
    setIsUpdatingApiKey(false);
    setShowPassword(false);
    setShowApiKey(false);
    onCancel();
  };

  // Collapse API key update section
  const handleCancelApiKeyUpdate = () => {
    setMasterPassword('');
    setApiKey('');
    setIsUpdatingApiKey(false);
    setShowPassword(false);
    setShowApiKey(false);
    setError(null);
  };

  const handleDelete = async () => {
    setIsDeleting(true);
    setError(null);
    try {
      await onDelete(environment);
      clearForm();
      setShowDeleteConfirm(false);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      setIsDeleting(false);
    }
  };

  // Different validation based on mode
  const canSubmitAccountId = accountId.trim() && !isSubmitting && !isUpdatingApiKey;
  const canSubmitApiKey = masterPassword.trim() && apiKey.trim() && !isSubmitting && isUpdatingApiKey && currentBlob;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    if (isUpdatingApiKey) {
      // API key update mode
      if (!canSubmitApiKey || !currentBlob) return;

      setIsSubmitting(true);
      setError(null);

      try {
        // Update API key via Rust backend
        const newBlob = await invoke<string>('update_api_key_with_vault', {
          masterPassword: masterPassword.trim(),
          newApiKey: apiKey.trim(),
          accountId: accountId.trim() || undefined, // Use existing if not changing
          currentBlob,
          environment,
          // Dev-only: validate against practice instead of live
          validateAsPractice: allowDemoAccount,
        });

        // Clear form after successful update
        clearForm();
        setIsUpdatingApiKey(false);
        setShowPassword(false);
        setShowApiKey(false);

        // Update dev setting if this is a live environment update
        // This persists whether we're using demo credentials in the live slot
        if (isLive) {
          setDevUsePracticeUrlForLive(allowDemoAccount);
        }

        // Call onApiKeyUpdate to update Zero with the new blob AND account ID if provided
        const newAccountId = accountId.trim() || undefined;
        await onApiKeyUpdate(newBlob, environment, newAccountId);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        let userMessage = message;
        if (message.includes('Invalid master password')) {
          userMessage = 'Invalid master password. Please try again.';
        } else if (message.includes('ACCOUNT_NOT_AVAILABLE')) {
          userMessage = 'Unable to validate account. Please check your credentials and try again.';
        }
        setError(userMessage);
        setIsSubmitting(false);
      }
    } else {
      // Account ID update mode
      if (!canSubmitAccountId) return;

      setIsSubmitting(true);
      setError(null);

      try {
        // Validate the account ID with the existing API key
        await invoke('validate_account_id', {
          accountId: accountId.trim(),
          environment,
          // Dev-only: validate against practice instead of live
          validateAsPractice: allowDemoAccount,
        });

        // Clear form after successful validation
        clearForm();

        // Update dev setting if this is a live environment update
        // This persists whether we're using demo credentials in the live slot
        if (isLive) {
          setDevUsePracticeUrlForLive(allowDemoAccount);
        }

        // Call onComplete to update Zero with the new account ID
        await onComplete(accountId.trim(), environment);
      } catch (err) {
        console.error('[UpdateCredentialsModal] Validation/update failed:', err);
        const message = err instanceof Error ? err.message : String(err);
        // Translate server error codes to user-friendly messages
        const userMessage = message.includes('ACCOUNT_NOT_AVAILABLE')
          ? 'Unable to validate account. Please check your credentials and try again.'
          : message;
        setError(userMessage);
        setIsSubmitting(false);
      }
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-[150] flex items-center justify-center">
      <div className="absolute inset-0 bg-black/70" onClick={handleCancel} />
      <div
        className="relative bg-[var(--color-bg-card)] rounded-lg shadow-xl max-w-md w-full mx-4 p-6"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center gap-3 mb-4">
          <div className={`w-10 h-10 rounded-full flex items-center justify-center ${
            isLive ? 'bg-[var(--color-buy)]/10' : 'bg-[var(--color-warning)]/10'
          }`}>
            <svg
              className={`w-5 h-5 ${isLive ? 'text-[var(--color-buy-text)]' : 'text-[var(--color-warning-text)]'}`}
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"
              />
            </svg>
          </div>
          <div>
            <h3 className="text-lg font-semibold text-[var(--color-text-primary)]">
              Update {isLive ? 'Live' : 'Demo'} Account
            </h3>
            <p className={`text-xs ${isLive ? 'text-[var(--color-buy-text)]' : 'text-[var(--color-warning-text)]'}`}>
              {isLive ? 'Real money account' : 'Practice account'}
            </p>
          </div>
        </div>

        <p className="text-sm text-[var(--color-text-secondary)] mb-6">
          Enter the new OANDA account ID for your {isLive ? 'live' : 'practice'} account.
          Your existing API key will be used for validation.
        </p>

        {error && (
          <div className="mb-4 p-3 bg-[var(--color-sell-bg)] border border-[var(--color-sell-border)] rounded">
            <p className="text-sm text-[var(--color-sell-text)]">{error}</p>
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-4">
          {/* Account ID */}
          <div className={`rounded-lg p-4 ${
            isLive
              ? 'bg-[var(--color-buy)]/10 border border-[var(--color-buy-border)]/30'
              : 'bg-[var(--color-warning)]/10 border border-[var(--color-warning)]/30'
          }`}>
            <h4 className={`text-sm font-medium mb-3 flex items-center gap-2 ${
              isLive ? 'text-[var(--color-buy-text)]' : 'text-[var(--color-warning-text)]'
            }`}>
              <span className={`w-2 h-2 rounded-full ${isLive ? 'bg-[var(--color-buy)]' : 'bg-[var(--color-warning)]'}`} />
              {isLive ? 'Live' : 'Practice'} Account ID
            </h4>

            <div>
              <input
                ref={accountInputRef}
                type="text"
                value={accountId}
                onChange={(e) => setAccountId(e.target.value)}
                disabled={isSubmitting}
                className={`w-full bg-[var(--color-bg-input)] text-[var(--color-text-primary)] placeholder-[var(--color-text-muted)] rounded px-3 py-2 focus:outline-none focus:ring-2 font-mono text-sm ${
                  isLive ? 'focus:ring-[var(--color-buy)]' : 'focus:ring-[var(--color-warning)]'
                }`}
                placeholder="e.g., 001-001-12345678-001"
                autoComplete="off"
              />
            </div>

            {/* Update API Key link/section */}
            {!isUpdatingApiKey ? (
              <button
                type="button"
                onClick={() => setIsUpdatingApiKey(true)}
                disabled={isSubmitting || !currentBlob}
                className="mt-3 text-xs text-[var(--color-info-text)] hover:opacity-80 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                Update API Key
              </button>
            ) : (
              <div className="mt-4 pt-4 border-t border-[var(--color-border)]/50 space-y-3">
                <div className="flex items-center justify-between">
                  <h5 className="text-xs font-medium text-[var(--color-info-text)]">Update API Key</h5>
                  <button
                    type="button"
                    onClick={handleCancelApiKeyUpdate}
                    className="text-xs text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors"
                  >
                    Cancel
                  </button>
                </div>

                {/* Master Password */}
                <div>
                  <label className="block text-xs text-[var(--color-text-secondary)] mb-1">Master Password</label>
                  <div className="relative">
                    <input
                      ref={passwordInputRef}
                      type={showPassword ? 'text' : 'password'}
                      value={masterPassword}
                      onChange={(e) => setMasterPassword(e.target.value)}
                      disabled={isSubmitting}
                      className="w-full bg-[var(--color-bg-input)] text-[var(--color-text-primary)] placeholder-[var(--color-text-muted)] rounded px-3 py-2 pr-10 focus:outline-none focus:ring-2 focus:ring-[var(--color-info)] text-sm"
                      placeholder="Enter your master password"
                      autoComplete="current-password"
                    />
                    <button
                      type="button"
                      onClick={() => setShowPassword(!showPassword)}
                      className="absolute right-2 top-1/2 -translate-y-1/2 text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] p-1"
                    >
                      {showPassword ? (
                        <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                        </svg>
                      ) : (
                        <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                        </svg>
                      )}
                    </button>
                  </div>
                </div>

                {/* New API Key */}
                <div>
                  <label className="block text-xs text-[var(--color-text-secondary)] mb-1">New API Key</label>
                  <div className="relative">
                    <input
                      type={showApiKey ? 'text' : 'password'}
                      value={apiKey}
                      onChange={(e) => setApiKey(e.target.value)}
                      disabled={isSubmitting}
                      className="w-full bg-[var(--color-bg-input)] text-[var(--color-text-primary)] placeholder-[var(--color-text-muted)] rounded px-3 py-2 pr-10 focus:outline-none focus:ring-2 focus:ring-[var(--color-info)] font-mono text-sm"
                      placeholder="Your new API key"
                      autoComplete="off"
                    />
                    <button
                      type="button"
                      onClick={() => setShowApiKey(!showApiKey)}
                      className="absolute right-2 top-1/2 -translate-y-1/2 text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] p-1"
                    >
                      {showApiKey ? (
                        <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                        </svg>
                      ) : (
                        <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                        </svg>
                      )}
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>

          {/* Dev toggle - show for both account ID and API key updates */}
          {isDev && isLive && (
            <label className="flex items-center gap-2 text-xs text-[var(--color-text-muted)] cursor-pointer">
              <input
                type="checkbox"
                checked={allowDemoAccount}
                onChange={(e) => setAllowDemoAccount(e.target.checked)}
                className="rounded bg-[var(--color-bg-input)] border-[var(--color-border)]"
              />
              [Dev] Allow demo account (validate against practice)
            </label>
          )}

          {/* Info box */}
          <div className="bg-[var(--color-bg-elevated)] rounded p-3 text-xs text-[var(--color-text-secondary)]">
            <strong className="text-[var(--color-text-primary)]">Note:</strong>{' '}
            {isUpdatingApiKey ? (
              <>
                Generate a new API key at{' '}
                <span className={isLive ? 'text-[var(--color-buy-text)]' : 'text-[var(--color-warning-text)]'}>
                  {isLive ? 'fxtrade.oanda.com' : 'fxpractice.oanda.com'}
                </span>{' '}
                under Manage API Access.
              </>
            ) : (
              <>
                Find your account ID at{' '}
                <span className={isLive ? 'text-[var(--color-buy-text)]' : 'text-[var(--color-warning-text)]'}>
                  {isLive ? 'fxtrade.oanda.com' : 'fxpractice.oanda.com'}
                </span>{' '}
                under Account Settings.
              </>
            )}
          </div>

          {/* Delete Confirmation */}
          {showDeleteConfirm ? (
            <div className="bg-[var(--color-sell-bg)] border border-[var(--color-sell-border)] rounded-lg p-4 space-y-3">
              <div className="flex items-start gap-3">
                <div className="w-8 h-8 rounded-full bg-[var(--color-sell)]/20 flex items-center justify-center flex-shrink-0">
                  <svg className="w-4 h-4 text-[var(--color-sell-text)]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                  </svg>
                </div>
                <div>
                  <h4 className="text-sm font-medium text-[var(--color-sell-text)]">
                    Delete {isLive ? 'Live' : 'Practice'} Credentials?
                  </h4>
                  <p className="text-xs text-[var(--color-text-secondary)] mt-1">
                    {isLive
                      ? 'This will remove your live trading credentials. You can add them again later from Settings.'
                      : 'This will reset all your credentials and require you to set up the vault again.'}
                  </p>
                </div>
              </div>
              <div className="flex gap-2 justify-end">
                <button
                  type="button"
                  onClick={() => setShowDeleteConfirm(false)}
                  disabled={isDeleting}
                  className="px-3 py-1.5 text-sm bg-[var(--color-bg-hover)] text-[var(--color-text-primary)] rounded hover:bg-[var(--color-bg-active)] transition-colors disabled:opacity-50"
                >
                  Cancel
                </button>
                <button
                  type="button"
                  onClick={handleDelete}
                  disabled={isDeleting}
                  className="px-3 py-1.5 text-sm bg-[var(--color-sell)] text-white rounded hover:opacity-90 transition-colors disabled:opacity-50"
                >
                  {isDeleting ? 'Deleting...' : 'Delete'}
                </button>
              </div>
            </div>
          ) : (
            <>
              {/* Actions */}
              <div className="flex gap-3 pt-2">
                <button
                  type="button"
                  onClick={handleCancel}
                  disabled={isSubmitting}
                  className="flex-1 px-4 py-2 bg-[var(--color-bg-hover)] text-[var(--color-text-primary)] rounded hover:bg-[var(--color-bg-active)] transition-colors disabled:opacity-50"
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  disabled={isUpdatingApiKey ? !canSubmitApiKey : !canSubmitAccountId}
                  className={`flex-1 px-4 py-2 rounded font-medium transition-colors ${
                    (isUpdatingApiKey ? canSubmitApiKey : canSubmitAccountId)
                      ? isUpdatingApiKey
                        ? 'bg-[var(--color-info)] hover:opacity-90 text-white'
                        : isLive
                          ? 'bg-[var(--color-buy)] hover:opacity-90 text-white'
                          : 'bg-[var(--color-warning)] hover:opacity-90 text-white'
                      : 'bg-[var(--color-bg-hover)] text-[var(--color-text-muted)] cursor-not-allowed'
                  }`}
                >
                  {isSubmitting ? (
                    <span className="flex items-center justify-center gap-2">
                      <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
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
                      {isUpdatingApiKey ? 'Updating Key...' : 'Validating...'}
                    </span>
                  ) : isUpdatingApiKey ? (
                    'Update API Key'
                  ) : (
                    'Update Account'
                  )}
                </button>
              </div>

              {/* Delete link */}
              <div className="pt-2 border-t border-[var(--color-border)]">
                <button
                  type="button"
                  onClick={() => setShowDeleteConfirm(true)}
                  disabled={isSubmitting}
                  className="text-xs text-[var(--color-sell-text)] hover:opacity-80 transition-colors disabled:opacity-50"
                >
                  Delete {isLive ? 'live' : 'practice'} credentials...
                </button>
              </div>
            </>
          )}
        </form>
      </div>
    </div>
  );
}
