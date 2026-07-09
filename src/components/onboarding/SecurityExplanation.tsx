import { useState } from 'react';
import { securityMessaging, getTechnicalSpecs } from '../../content/security';

const iconMap: Record<string, React.ReactNode> = {
  lock: (
    <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
    </svg>
  ),
  'eye-off': (
    <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
    </svg>
  ),
  shield: (
    <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
    </svg>
  ),
  key: (
    <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
    </svg>
  ),
  server: (
    <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01" />
    </svg>
  ),
};

interface SecurityExplanationProps {
  onContinue: () => void;
  onBack: () => void;
}

export const SecurityExplanation = ({ onContinue, onBack }: SecurityExplanationProps) => {
  const [showTechDetails, setShowTechDetails] = useState(false);
  const technicalSpecs = getTechnicalSpecs();

  return (
    <div className="max-w-lg mx-auto p-6">
      {/* Header */}
      <div className="text-center mb-6">
        <div className="inline-flex items-center justify-center w-14 h-14 rounded-full bg-[var(--color-buy)]/20 text-[var(--color-buy)] mb-4">
          <svg className="w-7 h-7" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
          </svg>
        </div>
        <h2 className="text-2xl font-bold mb-2">{securityMessaging.headline}</h2>
        <p className="text-[var(--color-text-muted)]">{securityMessaging.tagline}</p>
      </div>

      {/* Trust points */}
      <div className="space-y-3 mb-6">
        {securityMessaging.trustPoints.map((point) => (
          <div
            key={point.id}
            className="flex items-center gap-4 p-4 bg-[var(--color-bg-card)]/50 rounded-lg"
          >
            <div className="flex-shrink-0 w-10 h-10 rounded-lg bg-[var(--color-buy)]/20 text-[var(--color-buy)] flex items-center justify-center">
              {iconMap[point.icon] || iconMap.shield}
            </div>
            <div>
              <h3 className="font-medium text-[var(--color-text-primary)]">{point.title}</h3>
              <p className="text-sm text-[var(--color-text-muted)]">{point.description}</p>
            </div>
          </div>
        ))}
      </div>

      {/* Expandable technical details */}
      <div className="mb-6">
        <button
          onClick={() => setShowTechDetails(!showTechDetails)}
          className="flex items-center gap-2 text-sm text-[var(--color-info-text)] hover:text-[var(--color-info-text)]/80 transition-colors"
        >
          <svg
            className={`w-4 h-4 transition-transform ${showTechDetails ? 'rotate-90' : ''}`}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
          {showTechDetails ? 'Hide' : 'Show'} technical details
        </button>

        {showTechDetails && (
          <div className="mt-3 p-4 bg-[var(--color-bg-elevated)] rounded-lg border border-[var(--color-border)]">
            <table className="w-full text-sm">
              <tbody>
                {technicalSpecs.map((spec, index) => (
                  <tr key={spec.title} className={index > 0 ? 'border-t border-[var(--color-border)]' : ''}>
                    <td className="py-2 pr-4 text-[var(--color-text-muted)] font-medium">{spec.title}</td>
                    <td className="py-2 text-[var(--color-text-secondary)] font-mono text-xs">{spec.content}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Actions */}
      <div className="flex gap-3">
        <button
          onClick={onBack}
          className="flex-1 px-4 py-2 bg-[var(--color-bg-card)] rounded hover:bg-[var(--color-bg-elevated)] transition-colors"
        >
          Back
        </button>
        <button
          onClick={onContinue}
          className="flex-1 px-4 py-2 bg-[var(--color-info)] rounded hover:bg-[var(--color-info)]/80 transition-colors"
        >
          I Understand
        </button>
      </div>
    </div>
  );
}
