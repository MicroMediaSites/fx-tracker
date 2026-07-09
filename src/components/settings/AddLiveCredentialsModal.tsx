import { useState, useRef, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useCredentialFormStore } from '../../stores/credentialFormStore';
import { useSettingsStore } from '../../stores/settingsStore';

interface AddLiveCredentialsModalProps {
  isOpen: boolean;
  practiceBlob: string; // Required to verify master password
  onComplete: (liveAccountId: string, liveBlob: string) => Promise<void>;
  onCancel: () => void;
}

export const AddLiveCredentialsModal = ({
  isOpen,
  practiceBlob,
  onComplete,
  onCancel,
}: AddLiveCredentialsModalProps) => {
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
  const [showPassword, setShowPassword] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);
  // Dev-only: allow using demo credentials in the "live" slot for testing
  const [allowDemoAccount, setAllowDemoAccount] = useState(false);
  const isDev = import.meta.env.DEV;

  const passwordInputRef = useRef<HTMLInputElement>(null);

  // Focus password input when modal opens
  useEffect(() => {
    if (isOpen && !masterPassword) {
      setTimeout(() => passwordInputRef.current?.focus(), 100);
    }
  }, [isOpen, masterPassword]);

  // Clear form when user explicitly cancels
  const handleCancel = () => {
    clearForm();
    setError(null);
    setShowPassword(false);
    setShowApiKey(false);
    onCancel();
  };

  const canSubmit = masterPassword.trim() && apiKey.trim() && accountId.trim() && !isSubmitting;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;

    setIsSubmitting(true);
    setError(null);

    try {
      // Call Rust backend to add live credentials
      // This validates the API key + account ID, encrypts the key, and returns the blob
      const liveBlob = await invoke<string>('add_live_credentials', {
        masterPassword: masterPassword.trim(),
        liveApiKey: apiKey.trim(),
        liveAccountId: accountId.trim(),
        practiceBlob,
        // Dev-only: validate against practice instead of live
        validateAsPractice: allowDemoAccount,
      });

      // Clear form after successful submission
      clearForm();
      setShowPassword(false);
      setShowApiKey(false);

      // Update dev setting based on whether this is a demo account in the live slot
      // When adding real live credentials, clear any previous dev setting
      setDevUsePracticeUrlForLive(allowDemoAccount);

      // Call onComplete to store the live account ID and encrypted blob locally
      await onComplete(accountId.trim(), liveBlob);

      // Sync trades from the new live account to the local store (fire-and-forget)
      invoke('sync_trades', {
        count: 500,
        dataSource: 'live',
      }).catch((err) => console.error('[AddLiveCredentials] Trade sync failed:', err));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      // Translate server error codes to user-friendly messages
      let userMessage = message;
      if (message.includes('ACCOUNT_NOT_AVAILABLE')) {
        userMessage = 'Unable to validate account. Please check your credentials and try again.';
      } else if (message.includes('Invalid master password')) {
        userMessage = 'Invalid master password. Please try again.';
      }
      setError(userMessage);
      setIsSubmitting(false);
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
          <div className="w-10 h-10 rounded-full bg-[var(--color-warning)]/10 flex items-center justify-center">
            <svg
              className="w-5 h-5 text-[var(--color-warning-text)]"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
              />
            </svg>
          </div>
          <div>
            <h3 className="text-lg font-semibold text-[var(--color-text-primary)]">Enable Live Trading</h3>
            <p className="text-xs text-[var(--color-warning-text)]">Real money - proceed with caution</p>
          </div>
        </div>

        <p className="text-sm text-[var(--color-text-secondary)] mb-6">
          Add your OANDA live account credentials. Live trading uses a separate API key
          from practice.
        </p>

        {error && (
          <div className="mb-4 p-3 bg-[var(--color-sell-bg)] border border-[var(--color-sell-border)] rounded">
            <p className="text-sm text-[var(--color-sell-text)]">{error}</p>
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-4">
          {/* Master Password */}
          <div>
            <label className="block text-sm font-medium text-[var(--color-text-secondary)] mb-1">
              Master Password
            </label>
            <div className="relative">
              <input
                ref={passwordInputRef}
                type={showPassword ? 'text' : 'password'}
                value={masterPassword}
                onChange={(e) => setMasterPassword(e.target.value)}
                disabled={isSubmitting}
                className="w-full bg-[var(--color-bg-input)] text-[var(--color-text-primary)] placeholder-[var(--color-text-muted)] rounded px-3 py-2 pr-10 focus:outline-none focus:ring-2 focus:ring-[var(--color-warning)] text-sm"
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

          {/* Live Account Credentials */}
          <div className="bg-[var(--color-warning)]/10 border border-[var(--color-warning)]/30 rounded-lg p-4 space-y-3">
            <h4 className="text-sm font-medium text-[var(--color-warning-text)] flex items-center gap-2">
              <span className="w-2 h-2 rounded-full bg-[var(--color-warning)]" />
              Live Account Credentials
            </h4>

            {/* Live API Key */}
            <div>
              <label className="block text-sm font-medium text-[var(--color-text-secondary)] mb-1">
                Live API Key
              </label>
              <div className="relative">
                <input
                  type={showApiKey ? 'text' : 'password'}
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  disabled={isSubmitting}
                  className="w-full bg-[var(--color-bg-input)] text-[var(--color-text-primary)] placeholder-[var(--color-text-muted)] rounded px-3 py-2 pr-10 focus:outline-none focus:ring-2 focus:ring-[var(--color-warning)] font-mono text-sm"
                  placeholder="Your live API key from fxtrade.oanda.com"
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

            {/* Live Account ID */}
            <div>
              <label className="block text-sm font-medium text-[var(--color-text-secondary)] mb-1">
                Live Account ID
              </label>
              <input
                type="text"
                value={accountId}
                onChange={(e) => setAccountId(e.target.value)}
                disabled={isSubmitting}
                className="w-full bg-[var(--color-bg-input)] text-[var(--color-text-primary)] placeholder-[var(--color-text-muted)] rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-[var(--color-warning)] font-mono text-sm"
                placeholder="e.g., 001-001-12345678-001"
                autoComplete="off"
              />
            </div>
          </div>

          {/* Dev toggle */}
          {isDev && (
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
            <strong className="text-[var(--color-text-primary)]">Note:</strong> Generate a live API key at{' '}
            <span className="text-[var(--color-warning-text)]">fxtrade.oanda.com</span> under Manage API Access.
            Live credentials are separate from practice. Credentials are encrypted
            on this device and never leave your computer.
          </div>

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
              disabled={!canSubmit}
              className={`flex-1 px-4 py-2 rounded font-medium transition-colors ${
                canSubmit
                  ? 'bg-[var(--color-warning)] hover:opacity-90 text-white'
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
                  Validating...
                </span>
              ) : (
                'Enable Live Trading'
              )}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
