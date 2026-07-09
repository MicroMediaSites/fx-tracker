interface Step {
  id: string;
  label: string;
}

interface OnboardingProgressProps {
  steps: Step[];
  currentStep: string;
  completedSteps: string[];
}

export const OnboardingProgress = ({ steps, currentStep, completedSteps }: OnboardingProgressProps) => {
  return (
    <div className="flex justify-center mb-8">
      <div className="flex items-center gap-2">
        {steps.map((step, index) => {
          const isCompleted = completedSteps.includes(step.id);
          const isCurrent = step.id === currentStep;
          const isPending = !isCompleted && !isCurrent;

          return (
            <div key={step.id} className="flex items-center">
              {/* Step circle */}
              <div className="flex flex-col items-center">
                <div
                  className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-medium transition-colors ${
                    isCompleted
                      ? 'bg-[var(--color-buy)] text-white'
                      : isCurrent
                      ? 'bg-[var(--color-info)] text-white'
                      : 'bg-[var(--color-bg-card)] text-[var(--color-text-muted)]'
                  }`}
                >
                  {isCompleted ? (
                    <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                    </svg>
                  ) : (
                    index + 1
                  )}
                </div>
                {/* Step label - only show for current step on mobile, all on desktop */}
                <span
                  className={`mt-1 text-xs hidden sm:block ${
                    isCurrent ? 'text-[var(--color-info-text)]' : isPending ? 'text-[var(--color-text-muted)]' : 'text-[var(--color-text-secondary)]'
                  }`}
                >
                  {step.label}
                </span>
              </div>

              {/* Connector line */}
              {index < steps.length - 1 && (
                <div
                  className={`w-8 sm:w-12 h-0.5 mx-1 ${
                    completedSteps.includes(steps[index + 1]?.id) || steps[index + 1]?.id === currentStep
                      ? 'bg-[var(--color-info)]'
                      : 'bg-[var(--color-bg-card)]'
                  }`}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
