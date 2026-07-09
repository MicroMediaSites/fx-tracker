import { useState } from 'react';

interface GuideStep {
  number: number;
  title: string;
  description: string;
  link?: string;
  tip?: string;
}

const steps: GuideStep[] = [
  {
    number: 1,
    title: 'Log in to OANDA',
    description: 'Go to fxtrade.oanda.com and sign in to your account',
    link: 'https://fxtrade.oanda.com',
  },
  {
    number: 2,
    title: 'Open API Settings',
    description: 'Click your account name in the top right, then select "Manage API Access"',
  },
  {
    number: 3,
    title: 'Generate Token',
    description: 'Click "Generate" to create a new API token. You can only have one active token at a time.',
  },
  {
    number: 4,
    title: 'Copy Your Token',
    description: 'Copy the token immediately - you won\'t be able to see it again',
  },
  {
    number: 5,
    title: 'Find Account ID',
    description: 'Your Account ID is shown at the top of the API management page',
    tip: 'It looks like "101-001-12345678-001" for practice accounts',
  },
];

interface CredentialGuideProps {
  defaultExpanded?: boolean;
}

export const CredentialGuide = ({ defaultExpanded = false }: CredentialGuideProps) => {
  const [isExpanded, setIsExpanded] = useState(defaultExpanded);

  return (
    <div className="bg-[var(--color-info)]/10 border border-[var(--color-info)]/30 rounded-lg overflow-hidden">
      {/* Header - clickable to expand/collapse */}
      <button
        onClick={() => setIsExpanded(!isExpanded)}
        className="w-full flex items-center justify-between p-4 text-left hover:bg-[var(--color-info)]/20 transition-colors"
      >
        <div className="flex items-center gap-3">
          <div className="w-8 h-8 rounded-lg bg-[var(--color-info)]/20 text-[var(--color-info-text)] flex items-center justify-center">
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          </div>
          <span className="font-medium text-[var(--color-info-text)]">How to get your OANDA credentials</span>
        </div>
        <svg
          className={`w-5 h-5 text-[var(--color-info-text)] transition-transform ${isExpanded ? 'rotate-180' : ''}`}
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>

      {/* Expandable content */}
      {isExpanded && (
        <div className="px-4 pb-4 space-y-3">
          {steps.map((step) => (
            <div key={step.number} className="flex gap-3">
              {/* Step number */}
              <div className="flex-shrink-0 w-6 h-6 rounded-full bg-[var(--color-info)] text-white text-xs font-medium flex items-center justify-center">
                {step.number}
              </div>

              {/* Step content */}
              <div className="flex-1 min-w-0">
                <h4 className="font-medium text-[var(--color-text-primary)] text-sm">{step.title}</h4>
                <p className="text-xs text-[var(--color-text-muted)] mt-0.5">{step.description}</p>

                {step.link && (
                  <a
                    href={step.link}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="inline-flex items-center gap-1 mt-1 text-xs text-[var(--color-info-text)] hover:text-[var(--color-info-text)]/80"
                  >
                    Open OANDA
                    <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                    </svg>
                  </a>
                )}

                {step.tip && (
                  <p className="mt-1 text-xs text-[var(--color-warning)]/80 italic">
                    Tip: {step.tip}
                  </p>
                )}
              </div>
            </div>
          ))}

          {/* Video link placeholder */}
          <div className="pt-2 border-t border-[var(--color-info)]/20">
            <p className="text-xs text-[var(--color-text-muted)]">
              Need more help? Check out our{' '}
              <a href="#" className="text-[var(--color-info-text)] hover:text-[var(--color-info-text)]/80">
                setup guide
              </a>
              .
            </p>
          </div>
        </div>
      )}
    </div>
  );
}
