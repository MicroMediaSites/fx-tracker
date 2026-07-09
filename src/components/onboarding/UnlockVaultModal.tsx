import { useState, useEffect, useRef } from 'react';
import { useVault } from '../../hooks/useVault';
import logoText from '../../assets/logo_text.png';

interface UnlockVaultModalProps {
  isOpen: boolean;
  // unlock function that handles getting credentials from Zero
  unlock: (masterPassword: string) => Promise<boolean>;
  onUnlock: () => void;
  onReset: () => void; // Called when user wants to reset credentials
  onBack?: () => void; // Called when user wants to go back to onboarding
  onSignOut?: () => void; // Called when user wants to sign out
  userName?: string; // User's display name for greeting
}

export const UnlockVaultModal = ({
  isOpen,
  unlock,
  onUnlock,
  onReset,
  onBack,
  onSignOut,
  userName,
}: UnlockVaultModalProps) => {
  const {
    rateLimitMessage,
    rateLimitSeconds,
    error,
    clearError,
    checkRateLimit,
  } = useVault();

  const [password, setPassword] = useState('');
  const [isUnlocking, setIsUnlocking] = useState(false);
  const [showPassword, setShowPassword] = useState(false);
  const [countdown, setCountdown] = useState<number | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Focus input when modal opens
  useEffect(() => {
    if (isOpen) {
      setTimeout(() => inputRef.current?.focus(), 100);
      checkRateLimit();
    }
  }, [isOpen, checkRateLimit]);

  // Handle countdown
  useEffect(() => {
    if (rateLimitSeconds && rateLimitSeconds > 0) {
      setCountdown(rateLimitSeconds);
      const interval = setInterval(() => {
        setCountdown((prev) => {
          if (prev === null || prev <= 1) {
            clearInterval(interval);
            checkRateLimit();
            return null;
          }
          return prev - 1;
        });
      }, 1000);
      return () => clearInterval(interval);
    }
  }, [rateLimitSeconds, checkRateLimit]);

  // Handle escape key
  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen) {
        // Don't close on escape - user must unlock or reset
      }
    };
    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [isOpen]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!password || isUnlocking || countdown !== null) return;

    setIsUnlocking(true);
    clearError();

    const success = await unlock(password);

    setIsUnlocking(false);

    if (success) {
      setPassword('');
      onUnlock();
    }
  };

  if (!isOpen) return null;

  const isRateLimited = countdown !== null && countdown > 0;

  return (
    <div className="fixed inset-0 z-[150] flex flex-col items-center justify-center bg-[var(--color-bg-page)] p-8">
      {/* Logo text */}
      <img
        src={logoText}
        alt="wickd"
        className="w-56 h-auto invert mb-2"
      />

      {/* Welcome message */}
      <p className="text-[var(--color-text-muted)] mb-6 text-center max-w-xs text-sm">
        Welcome back{userName ? `, ${userName}` : ''}
      </p>

      <form onSubmit={handleSubmit} className="w-full max-w-xs space-y-4">
        <div>
          <div className="relative">
            <input
              ref={inputRef}
              type={showPassword ? 'text' : 'password'}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              disabled={isUnlocking || isRateLimited}
              className={`w-full bg-transparent text-[var(--color-text-primary)] placeholder-[var(--color-text-muted)] rounded-lg px-4 py-3 pr-10 focus:outline-none border ${
                error ? 'border-[var(--color-sell)]' : 'border-[var(--color-border)] focus:border-[var(--color-text-muted)]'
              } ${isRateLimited ? 'opacity-50' : ''}`}
              placeholder="Master password"
              autoComplete="current-password"
            />
            <button
              type="button"
              onClick={() => setShowPassword(!showPassword)}
              className="absolute right-3 top-1/2 -translate-y-1/2 text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)]"
            >
              {showPassword ? '👁️' : '👁️‍🗨️'}
            </button>
          </div>

          {/* Error message */}
          {error && (
            <p className="mt-2 text-sm text-[var(--color-sell)] text-center">{error}</p>
          )}

          {/* Rate limit message */}
          {isRateLimited && (
            <p className="mt-2 text-sm text-[var(--color-warning)] text-center">
              {rateLimitMessage || `Please wait ${countdown} seconds`}
            </p>
          )}
        </div>

        <button
          type="submit"
          disabled={!password || isUnlocking || isRateLimited}
          className={`w-full flex items-center justify-center gap-2 px-4 py-3 rounded-lg font-medium transition-colors border ${
            password && !isUnlocking && !isRateLimited
              ? 'border-[var(--color-border)] text-[var(--color-text-primary)] hover:bg-[var(--color-bg-elevated)]'
              : 'border-[var(--color-border)]/50 text-[var(--color-text-muted)] cursor-not-allowed'
          }`}
        >
          {isUnlocking ? (
            <>
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
              Unlocking...
            </>
          ) : isRateLimited ? (
            `Wait ${countdown}s`
          ) : (
            'Unlock the secure vault'
          )}
        </button>
      </form>

      {/* Forgot password link */}
      <button
        onClick={onReset}
        className="mt-4 text-sm text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
      >
        Forgot password? Reset credentials
      </button>

      {/* Back to setup link (when coming from onboarding) */}
      {onBack && (
        <button
          onClick={onBack}
          className="mt-2 text-sm text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
        >
          Back to credential setup
        </button>
      )}

      {/* Sign out link */}
      {onSignOut && (
        <button
          onClick={onSignOut}
          className="mt-4 text-sm text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
        >
          Sign in with a different account
        </button>
      )}
    </div>
  );
}
