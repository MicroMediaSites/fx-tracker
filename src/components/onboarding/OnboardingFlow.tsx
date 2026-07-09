import { useState, useCallback } from 'react';
import { useVault } from '../../hooks/useVault';
import { OnboardingProgress } from './OnboardingProgress';
import { WelcomeStep } from './WelcomeStep';
import { SecurityExplanation } from './SecurityExplanation';
import { CreateMasterPassword } from './CreateMasterPassword';
import { EnterCredentials } from './EnterCredentials';

interface CredentialData {
  apiKey: string;
  practiceAccountId: string;
  liveAccountId?: string;
}

interface OnboardingFlowProps {
  onComplete: (
    apiKeyBlob: string,
    practiceAccountId: string,
    liveAccountId: string | null
  ) => Promise<void>;
  onCheckForCredentials?: () => Promise<void>;
  isCheckingCredentials?: boolean;
  checkError?: string | null;
}

type Step = 'welcome' | 'security' | 'password' | 'credentials' | 'saving';

const STEPS = [
  { id: 'welcome', label: 'Welcome' },
  { id: 'security', label: 'Security' },
  { id: 'password', label: 'Password' },
  { id: 'credentials', label: 'Connect' },
];

export const OnboardingFlow = ({ onComplete, onCheckForCredentials, isCheckingCredentials, checkError }: OnboardingFlowProps) => {
  const { encryptCredentials, unlockVault, error } = useVault();

  const [step, setStep] = useState<Step>('welcome');
  const [completedSteps, setCompletedSteps] = useState<string[]>([]);
  const [masterPassword, setMasterPassword] = useState('');
  const [saveError, setSaveError] = useState<string | null>(null);

  const markStepComplete = (stepId: string) => {
    setCompletedSteps((prev) => (prev.includes(stepId) ? prev : [...prev, stepId]));
  };

  const handleWelcomeContinue = useCallback(() => {
    markStepComplete('welcome');
    setStep('security');
  }, []);

  const handleSecurityContinue = useCallback(() => {
    markStepComplete('security');
    setStep('password');
  }, []);

  const handleSecurityBack = useCallback(() => {
    setStep('welcome');
  }, []);

  const handlePasswordComplete = useCallback((password: string) => {
    setMasterPassword(password);
    markStepComplete('password');
    setStep('credentials');
  }, []);

  const handlePasswordBack = useCallback(() => {
    setStep('security');
  }, []);

  const handleCredentialsComplete = useCallback(
    async (data: CredentialData) => {
      setStep('saving');
      setSaveError(null);

      try {
        // Encrypt the API key with master password
        // Account IDs are stored in Zero (not encrypted)
        const apiKeyBlob = await encryptCredentials(
          masterPassword,
          data.apiKey,
          data.practiceAccountId,
          'practice', // Initial environment
        );

        // Call onComplete FIRST to validate account uniqueness and store to Zero
        // This must happen before unlocking vault so we can show errors properly
        await onComplete(
          apiKeyBlob,
          data.practiceAccountId,
          data.liveAccountId ?? null
        );

        // Only unlock the vault AFTER successful validation and storage
        // During onboarding, we only have practice credentials (live is added later)
        await unlockVault(masterPassword, apiKeyBlob, data.practiceAccountId, null, null, 'practice');

        // Clear sensitive data
        setMasterPassword('');
        markStepComplete('credentials');
      } catch (err) {
        // Handle various error formats (Error objects, structured {error, details} objects)
        let message: string;
        if (err instanceof Error) {
          message = err.message;
        } else if (typeof err === 'object' && err !== null) {
          const errObj = err as Record<string, unknown>;
          // Structured errors may have {error: "app", details: "actual message"} format
          // Zero errors might have {error: string} or {message: string}
          message = String(
            errObj.details || // structured error details
            errObj.message || // Standard message
            (typeof errObj.error === 'string' && errObj.error !== 'app' ? errObj.error : null) ||
            JSON.stringify(err)
          );
        } else {
          message = String(err);
        }
        console.error('[OnboardingFlow] Credential save error:', err);
        // Translate server error codes to user-friendly messages
        const userMessage = message.includes('ACCOUNT_NOT_AVAILABLE')
          ? 'Unable to validate account. Please check your credentials and try again.'
          : message;
        setSaveError(userMessage);
        setStep('credentials');
      }
    },
    [masterPassword, encryptCredentials, unlockVault, onComplete]
  );

  const handleCredentialsBack = useCallback(() => {
    setStep('password');
  }, []);

  return (
    <div className="min-h-screen bg-[var(--color-bg-page)] text-[var(--color-text-primary)] flex items-center justify-center overflow-y-auto py-8">
      <div className="w-full max-w-lg">
        {/* Progress indicator - hide on welcome and saving steps */}
        {step !== 'welcome' && step !== 'saving' && (
          <OnboardingProgress
            steps={STEPS}
            currentStep={step}
            completedSteps={completedSteps}
          />
        )}

        {/* Step content */}
        {step === 'welcome' && (
          <WelcomeStep onContinue={handleWelcomeContinue} />
        )}

        {step === 'security' && (
          <SecurityExplanation
            onContinue={handleSecurityContinue}
            onBack={handleSecurityBack}
          />
        )}

        {step === 'password' && (
          <CreateMasterPassword
            onComplete={handlePasswordComplete}
            onBack={handlePasswordBack}
            onCheckForCredentials={onCheckForCredentials}
            isCheckingCredentials={isCheckingCredentials}
            checkError={checkError}
          />
        )}

        {step === 'credentials' && (
          <>
            {/* Error display - prominent position above form */}
            {(saveError || error) && (
              <div className="mx-6 mb-4 p-4 bg-[var(--color-sell)]/10 border border-[var(--color-sell)]/30 rounded">
                <p className="text-[var(--color-sell)] text-sm">{saveError || error}</p>
              </div>
            )}
            <EnterCredentials
              onComplete={handleCredentialsComplete}
              onBack={handleCredentialsBack}
            />
          </>
        )}

        {step === 'saving' && (
          <div className="text-center p-8">
            <div className="flex justify-center mb-4">
              <svg
                className="animate-spin h-12 w-12 text-[var(--color-info-text)]"
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
            <h3 className="text-xl font-semibold mb-2">Securing your credentials...</h3>
            <p className="text-[var(--color-text-muted)]">
              Encrypting and saving your OANDA credentials.
            </p>
          </div>
        )}

      </div>
    </div>
  );
}
