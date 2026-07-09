import { useState } from 'react';
import { CredentialGuide } from './CredentialGuide';

interface CredentialData {
  apiKey: string;
  practiceAccountId: string;
  liveAccountId?: string;
}

interface EnterCredentialsProps {
  onComplete: (data: CredentialData) => Promise<void> | void;
  onBack: () => void;
}

export const EnterCredentials = ({ onComplete, onBack }: EnterCredentialsProps) => {
  const [apiKey, setApiKey] = useState('');
  const [practiceAccountId, setPracticeAccountId] = useState('');
  const [liveAccountId, setLiveAccountId] = useState('');
  const [showApiKey, setShowApiKey] = useState(false);
  const [includeLive, setIncludeLive] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const canSubmit = apiKey.trim() && practiceAccountId.trim() &&
    (!includeLive || liveAccountId.trim()) && !isSubmitting;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;

    setIsSubmitting(true);
    try {
      await onComplete({
        apiKey: apiKey.trim(),
        practiceAccountId: practiceAccountId.trim(),
        liveAccountId: includeLive && liveAccountId.trim() ? liveAccountId.trim() : undefined,
      });
    } catch {
      // Error handled by parent (OnboardingFlow)
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div className="max-w-md mx-auto p-6">
      <h2 className="text-2xl font-bold mb-2">Enter OANDA Credentials</h2>
      <p className="text-[var(--color-text-muted)] mb-6">
        Enter your OANDA API key and account ID. These will be encrypted with your master password
        and never sent to our servers.
      </p>

      <form onSubmit={handleSubmit} className="space-y-6">
        {/* API Key (one for all accounts) */}
        <div className="bg-[var(--color-bg-elevated)] rounded-lg p-4">
          <h3 className="font-semibold text-[var(--color-info-text)] mb-3 flex items-center gap-2">
            <span className="w-2 h-2 rounded-full bg-[var(--color-info)]" />
            API Key
          </h3>
          <p className="text-xs text-[var(--color-text-muted)] mb-3">
            Your OANDA API key works for both practice and live accounts.
          </p>

          <div className="relative">
            <input
              type={showApiKey ? 'text' : 'password'}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              className="w-full bg-[var(--color-bg-card)] rounded px-3 py-2 pr-20 focus:outline-none focus:ring-2 focus:ring-[var(--color-info)] text-sm font-mono"
              placeholder="Enter your OANDA API key"
              autoComplete="off"
            />
            <button
              type="button"
              onClick={() => setShowApiKey(!showApiKey)}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] px-2 py-1"
            >
              {showApiKey ? 'Hide' : 'Show'}
            </button>
          </div>
        </div>

        {/* Practice Account (Required) */}
        <div className="bg-[var(--color-bg-elevated)] rounded-lg p-4">
          <h3 className="font-semibold text-[var(--color-info-text)] mb-3 flex items-center gap-2">
            <span className="w-2 h-2 rounded-full bg-[var(--color-info)]" />
            Practice Account (Required)
          </h3>
          <p className="text-xs text-[var(--color-text-muted)] mb-3">
            Start with a practice account to test strategies safely.
          </p>

          <div>
            <label className="block text-sm font-medium mb-1">Account ID</label>
            <input
              type="text"
              value={practiceAccountId}
              onChange={(e) => setPracticeAccountId(e.target.value)}
              className="w-full bg-[var(--color-bg-card)] rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-[var(--color-info)] text-sm font-mono"
              placeholder="e.g., 101-001-12345678-001"
              autoComplete="off"
            />
          </div>
        </div>

        {/* Live Account (Optional) */}
        <div className={`bg-[var(--color-bg-elevated)] rounded-lg p-4 ${!includeLive ? 'opacity-60' : ''}`}>
          <div className="flex items-center justify-between mb-3">
            <h3 className="font-semibold text-[var(--color-warning)] flex items-center gap-2">
              <span className="w-2 h-2 rounded-full bg-[var(--color-warning)]" />
              Live Account (Optional)
            </h3>
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={includeLive}
                onChange={(e) => setIncludeLive(e.target.checked)}
                className="rounded bg-[var(--color-bg-card)] border-[var(--color-border)] text-[var(--color-info)] focus:ring-[var(--color-info)]"
              />
              <span className="text-sm text-[var(--color-text-muted)]">Add now</span>
            </label>
          </div>

          {includeLive ? (
            <>
              <p className="text-xs text-[var(--color-warning)]/70 mb-3">
                Live credentials enable real trading. Uses the same API key as practice.
              </p>
              <div>
                <label className="block text-sm font-medium mb-1">Account ID</label>
                <input
                  type="text"
                  value={liveAccountId}
                  onChange={(e) => setLiveAccountId(e.target.value)}
                  className="w-full bg-[var(--color-bg-card)] rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-[var(--color-warning)] text-sm font-mono"
                  placeholder="e.g., 001-001-12345678-001"
                  autoComplete="off"
                />
              </div>
            </>
          ) : (
            <p className="text-xs text-[var(--color-text-muted)]">
              You can add live credentials later in Settings.
            </p>
          )}
        </div>

        {/* Credential guide */}
        <CredentialGuide />

        {/* Actions */}
        <div className="flex gap-3 pt-4">
          <button
            type="button"
            onClick={onBack}
            className="flex-1 px-4 py-2 bg-[var(--color-bg-card)] rounded hover:bg-[var(--color-bg-elevated)] transition-colors"
          >
            Back
          </button>
          <button
            type="submit"
            disabled={!canSubmit}
            className={`flex-1 px-4 py-2 rounded transition-colors ${
              canSubmit
                ? 'bg-[var(--color-info)] hover:bg-[var(--color-info)]/80'
                : 'bg-[var(--color-bg-card)] text-[var(--color-text-muted)] cursor-not-allowed'
            }`}
          >
            {isSubmitting ? 'Encrypting...' : 'Encrypt & Save'}
          </button>
        </div>
      </form>
    </div>
  );
}
