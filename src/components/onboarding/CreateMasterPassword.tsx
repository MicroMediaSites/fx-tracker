import { useState, useCallback, useEffect } from 'react';
import { useVault } from '../../hooks/useVault';
import { PasswordStrength } from '../../stores/vaultStore';

interface CreateMasterPasswordProps {
  onComplete: (password: string) => void;
  onBack?: () => void;
  onCheckForCredentials?: () => Promise<void>;
  isCheckingCredentials?: boolean;
  checkError?: string | null;
}

const MIN_PASSWORD_LENGTH = 16;

export const CreateMasterPassword = ({ onComplete, onBack, onCheckForCredentials, isCheckingCredentials, checkError }: CreateMasterPasswordProps) => {
  const { checkPasswordStrengthLocal, checkPasswordStrength } = useVault();

  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [strength, setStrength] = useState<PasswordStrength | null>(null);
  const [isCheckingHIBP, setIsCheckingHIBP] = useState(false);
  const [hibpChecked, setHibpChecked] = useState(false);
  const [showPassword, setShowPassword] = useState(false);
  const [acknowledged, setAcknowledged] = useState(false);

  // Check password strength locally on change (debounced)
  useEffect(() => {
    if (!password) {
      setStrength(null);
      return;
    }

    const timer = setTimeout(async () => {
      const result = await checkPasswordStrengthLocal(password);
      setStrength(result);
      setHibpChecked(false); // Reset HIBP check when password changes
    }, 300);

    return () => clearTimeout(timer);
  }, [password, checkPasswordStrengthLocal]);

  // Check HIBP on blur
  const handlePasswordBlur = useCallback(async () => {
    if (!password || password.length < MIN_PASSWORD_LENGTH || hibpChecked) return;

    setIsCheckingHIBP(true);
    try {
      const result = await checkPasswordStrength(password);
      setStrength(result);
      setHibpChecked(true);
    } catch (err) {
      console.error('Failed to check HIBP:', err);
    } finally {
      setIsCheckingHIBP(false);
    }
  }, [password, hibpChecked, checkPasswordStrength]);

  const passwordsMatch = password === confirmPassword;
  const canSubmit =
    strength?.meetsRequirements &&
    !strength?.isCompromised &&
    passwordsMatch &&
    acknowledged &&
    !isCheckingHIBP;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (canSubmit) {
      onComplete(password);
    }
  };

  const getStrengthColor = (score: number) => {
    // Keep semantic colors for password strength - these are well-understood
    const colors = ['bg-red-500', 'bg-orange-500', 'bg-yellow-500', 'bg-lime-500', 'bg-green-500'];
    return colors[score] || colors[0];
  };

  const getStrengthLabel = (score: number) => {
    const labels = ['Very Weak', 'Weak', 'Fair', 'Strong', 'Very Strong'];
    return labels[score] || labels[0];
  };

  return (
    <div className="max-w-md mx-auto p-6">
      <h2 className="text-2xl font-bold mb-2">Create Master Password</h2>
      <p className="text-[var(--color-text-muted)] mb-6">
        This password protects your OANDA API credentials. Choose a strong password
        that you won't forget - there's no recovery option.
      </p>

      <form onSubmit={handleSubmit} className="space-y-4">
        {/* Password input */}
        <div>
          <label className="block text-sm font-medium mb-1">Master Password</label>
          <div className="relative">
            <input
              type={showPassword ? 'text' : 'password'}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              onBlur={handlePasswordBlur}
              className="w-full bg-[var(--color-bg-card)] rounded px-3 py-2 pr-10 focus:outline-none focus:ring-2 focus:ring-[var(--color-info)]"
              placeholder="Enter master password"
              autoComplete="new-password"
            />
            <button
              type="button"
              onClick={() => setShowPassword(!showPassword)}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)]"
            >
              {showPassword ? '👁️' : '👁️‍🗨️'}
            </button>
          </div>

          {/* Character count */}
          <div className="mt-1 text-xs text-[var(--color-text-muted)]">
            {password.length} / {MIN_PASSWORD_LENGTH} characters minimum
          </div>

          {/* Strength meter */}
          {strength && (
            <div className="mt-2">
              <div className="flex gap-1 mb-1">
                {[0, 1, 2, 3, 4].map((i) => (
                  <div
                    key={i}
                    className={`h-1 flex-1 rounded ${
                      i <= strength.score ? getStrengthColor(strength.score) : 'bg-[var(--color-bg-elevated)]'
                    }`}
                  />
                ))}
              </div>
              <div className="text-xs flex justify-between">
                <span className={strength.meetsRequirements ? 'text-[var(--color-buy)]' : 'text-[var(--color-warning)]'}>
                  {getStrengthLabel(strength.score)}
                </span>
                {isCheckingHIBP && <span className="text-[var(--color-text-muted)]">Checking breaches...</span>}
                {hibpChecked && !strength.isCompromised && (
                  <span className="text-[var(--color-buy)]">Not found in breaches</span>
                )}
              </div>
            </div>
          )}

          {/* Feedback messages */}
          {strength?.feedback && strength.feedback.length > 0 && (
            <ul className="mt-2 text-xs space-y-1">
              {strength.feedback.map((msg, i) => (
                <li
                  key={i}
                  className={
                    msg.includes('breach') || msg.includes('Breach')
                      ? 'text-[var(--color-sell)]'
                      : 'text-[var(--color-warning)]'
                  }
                >
                  {msg}
                </li>
              ))}
            </ul>
          )}
        </div>

        {/* Confirm password */}
        <div>
          <label className="block text-sm font-medium mb-1">Confirm Password</label>
          <input
            type={showPassword ? 'text' : 'password'}
            value={confirmPassword}
            onChange={(e) => setConfirmPassword(e.target.value)}
            className={`w-full bg-[var(--color-bg-card)] rounded px-3 py-2 focus:outline-none focus:ring-2 ${
              confirmPassword && !passwordsMatch
                ? 'ring-2 ring-[var(--color-sell)]'
                : 'focus:ring-[var(--color-info)]'
            }`}
            placeholder="Confirm master password"
            autoComplete="new-password"
          />
          {confirmPassword && !passwordsMatch && (
            <p className="mt-1 text-xs text-[var(--color-sell)]">Passwords do not match</p>
          )}
        </div>

        {/* Security warning */}
        <div className="bg-[var(--color-warning)]/10 border border-[var(--color-warning)]/30 rounded p-4 text-sm">
          <h4 className="font-semibold text-[var(--color-warning)] mb-2">Security Notice</h4>
          <ul className="text-[var(--color-warning)]/80 space-y-1 text-xs">
            <li>Your password is never stored or transmitted - only used for encryption</li>
            <li>There is no password recovery - if you forget it, you'll need to re-enter credentials</li>
            <li>Use a password manager to securely store this password</li>
          </ul>
        </div>

        {/* Acknowledgment checkbox */}
        <label className="flex items-start gap-3 cursor-pointer">
          <input
            type="checkbox"
            checked={acknowledged}
            onChange={(e) => setAcknowledged(e.target.checked)}
            className="mt-0.5 rounded bg-[var(--color-bg-card)] border-[var(--color-border)] text-[var(--color-info)] focus:ring-[var(--color-info)]"
          />
          <span className="text-sm text-[var(--color-text-secondary)]">
            I understand that this password cannot be recovered and I should store it securely.
          </span>
        </label>

        {/* Actions */}
        <div className="flex gap-3 pt-4">
          {onBack && (
            <button
              type="button"
              onClick={onBack}
              className="flex-1 px-4 py-2 bg-[var(--color-bg-card)] rounded hover:bg-[var(--color-bg-elevated)] transition-colors"
            >
              Back
            </button>
          )}
          <button
            type="submit"
            disabled={!canSubmit}
            className={`flex-1 px-4 py-2 rounded transition-colors ${
              canSubmit
                ? 'bg-[var(--color-info)] hover:bg-[var(--color-info)]/80'
                : 'bg-[var(--color-bg-card)] text-[var(--color-text-muted)] cursor-not-allowed'
            }`}
          >
            Continue
          </button>
        </div>

        {/* Link to check for existing credentials (race condition workaround) */}
        {onCheckForCredentials && (
          <div className="text-center pt-4 border-t border-[var(--color-border)] mt-4">
            {checkError && (
              <p className="text-sm text-[var(--color-sell)] mb-2">{checkError}</p>
            )}
            <button
              type="button"
              onClick={onCheckForCredentials}
              disabled={isCheckingCredentials}
              className={`text-sm transition-colors ${
                isCheckingCredentials
                  ? 'text-[var(--color-text-muted)] cursor-wait'
                  : 'text-[var(--color-info-text)] hover:text-[var(--color-info-text)]/80'
              }`}
            >
              {isCheckingCredentials ? 'Checking...' : 'I already have a master password'}
            </button>
          </div>
        )}
      </form>
    </div>
  );
}
