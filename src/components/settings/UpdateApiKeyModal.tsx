import { useState, useRef, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useCredentialFormStore } from '../../stores/credentialFormStore';

interface UpdateApiKeyModalProps {
  isOpen: boolean;
  practiceAccountId: string;
  currentBlob: string;
  onComplete: (newBlob: string) => Promise<void>;
  onCancel: () => void;
}

export const UpdateApiKeyModal = ({
  isOpen,
  practiceAccountId,
  currentBlob,
  onComplete,
  onCancel,
}: UpdateApiKeyModalProps) => {
  // Use zustand store for form values - persists across window focus changes
  const {
    masterPassword,
    setMasterPassword,
    apiKey,
    setApiKey,
    clearForm,
  } = useCredentialFormStore();

  const [error, setError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [showPassword, setShowPassword] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);

  const passwordInputRef = useRef<HTMLInputElement>(null);

  // Focus password input when modal opens (only if password is empty)
  useEffect(() => {
    if (isOpen && !masterPassword) {
      setTimeout(() => passwordInputRef.current?.focus(), 100);
    }
  }, [isOpen, masterPassword]);

  const handleCancel = () => {
    clearForm();
    setError(null);
    setShowPassword(false);
    setShowApiKey(false);
    onCancel();
  };

  const canSubmit = masterPassword.trim() && apiKey.trim() && !isSubmitting;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;

    setIsSubmitting(true);
    setError(null);

    try {
      // Validate and encrypt the new API key
      const newBlob = await invoke<string>('update_api_key_with_vault', {
        masterPassword: masterPassword.trim(),
        newApiKey: apiKey.trim(),
        practiceAccountId,
        currentBlob,
      });

      clearForm();
      setShowPassword(false);
      setShowApiKey(false);
      await onComplete(newBlob);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      setIsSubmitting(false);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-[150] flex items-center justify-center">
      <div className="absolute inset-0 bg-black/70" onClick={handleCancel} />
      <div
        className="relative bg-gray-800 rounded-lg shadow-xl max-w-md w-full mx-4 p-6"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center gap-3 mb-4">
          <div className="w-10 h-10 rounded-full bg-blue-900/50 flex items-center justify-center">
            <svg
              className="w-5 h-5 text-blue-400"
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
            <h3 className="text-lg font-semibold text-white">Update API Key</h3>
            <p className="text-xs text-gray-400">Replace your OANDA API key</p>
          </div>
        </div>

        <p className="text-sm text-gray-300 mb-6">
          Enter your master password and new OANDA API key. The new key will be validated
          against your existing practice account before saving.
        </p>

        {error && (
          <div className="mb-4 p-3 bg-red-900/30 border border-red-600/50 rounded">
            <p className="text-sm text-red-400">{error}</p>
          </div>
        )}

        <form onSubmit={handleSubmit} className="space-y-4">
          {/* Master Password */}
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-1">
              Master Password
            </label>
            <div className="relative">
              <input
                ref={passwordInputRef}
                type={showPassword ? 'text' : 'password'}
                value={masterPassword}
                onChange={(e) => setMasterPassword(e.target.value)}
                disabled={isSubmitting}
                className="w-full bg-gray-700 text-white placeholder-gray-400 rounded px-3 py-2 pr-10 focus:outline-none focus:ring-2 focus:ring-blue-500 text-sm"
                placeholder="Enter your master password"
                autoComplete="current-password"
              />
              <button
                type="button"
                onClick={() => setShowPassword(!showPassword)}
                className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-300 p-1"
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
            <label className="block text-sm font-medium text-gray-300 mb-1">
              New API Key
            </label>
            <div className="relative">
              <input
                type={showApiKey ? 'text' : 'password'}
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                disabled={isSubmitting}
                className="w-full bg-gray-700 text-white placeholder-gray-400 rounded px-3 py-2 pr-10 focus:outline-none focus:ring-2 focus:ring-blue-500 font-mono text-sm"
                placeholder="Enter your new API key"
                autoComplete="off"
              />
              <button
                type="button"
                onClick={() => setShowApiKey(!showApiKey)}
                className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-300 p-1"
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

          {/* Info box */}
          <div className="bg-gray-700/50 rounded p-3 text-xs text-gray-400">
            <strong className="text-gray-300">Note:</strong> Generate a new API key at{' '}
            <span className="text-blue-400">fxpractice.oanda.com</span> or{' '}
            <span className="text-blue-400">fxtrade.oanda.com</span> under
            Manage API Access. The same key works for both practice and live accounts.
          </div>

          {/* Actions */}
          <div className="flex gap-3 pt-2">
            <button
              type="button"
              onClick={handleCancel}
              disabled={isSubmitting}
              className="flex-1 px-4 py-2 bg-gray-600 text-white rounded hover:bg-gray-500 transition-colors disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={!canSubmit}
              className={`flex-1 px-4 py-2 rounded font-medium transition-colors ${
                canSubmit
                  ? 'bg-blue-600 hover:bg-blue-500 text-white'
                  : 'bg-gray-600 text-gray-400 cursor-not-allowed'
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
                'Update API Key'
              )}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
