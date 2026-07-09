import { useState, useEffect, useCallback, useRef } from 'react';
import { createPortal } from 'react-dom';
import { useSettingsStore } from '../../stores/settingsStore';

export interface TourStep {
  target: string;
  title: string;
  description: string;
  position: 'top' | 'bottom' | 'left' | 'right';
}

interface FirstRunTourProps {
  windowType: string;
  steps: TourStep[];
}

interface TooltipProps {
  step: TourStep;
  currentIndex: number;
  totalSteps: number;
  onNext: () => void;
  onPrev: () => void;
  onSkip: () => void;
  targetRect: DOMRect | null;
}

const Tooltip = ({ step, currentIndex, totalSteps, onNext, onPrev, onSkip, targetRect }: TooltipProps) => {
  if (!targetRect) return null;

  // Calculate tooltip position
  const tooltipWidth = 280;
  const tooltipHeight = 150;
  const padding = 12;
  const arrowSize = 8;

  let top = 0;
  let left = 0;
  let arrowStyle: React.CSSProperties = {};

  switch (step.position) {
    case 'bottom':
      top = targetRect.bottom + padding + arrowSize;
      left = targetRect.left + targetRect.width / 2 - tooltipWidth / 2;
      arrowStyle = {
        top: -arrowSize,
        left: '50%',
        transform: 'translateX(-50%)',
        borderLeft: `${arrowSize}px solid transparent`,
        borderRight: `${arrowSize}px solid transparent`,
        borderBottom: `${arrowSize}px solid var(--color-bg-elevated)`,
      };
      break;
    case 'top':
      top = targetRect.top - tooltipHeight - padding - arrowSize;
      left = targetRect.left + targetRect.width / 2 - tooltipWidth / 2;
      arrowStyle = {
        bottom: -arrowSize,
        left: '50%',
        transform: 'translateX(-50%)',
        borderLeft: `${arrowSize}px solid transparent`,
        borderRight: `${arrowSize}px solid transparent`,
        borderTop: `${arrowSize}px solid var(--color-bg-elevated)`,
      };
      break;
    case 'left':
      top = targetRect.top + targetRect.height / 2 - tooltipHeight / 2;
      left = targetRect.left - tooltipWidth - padding - arrowSize;
      arrowStyle = {
        right: -arrowSize,
        top: '50%',
        transform: 'translateY(-50%)',
        borderTop: `${arrowSize}px solid transparent`,
        borderBottom: `${arrowSize}px solid transparent`,
        borderLeft: `${arrowSize}px solid var(--color-bg-elevated)`,
      };
      break;
    case 'right':
      top = targetRect.top + targetRect.height / 2 - tooltipHeight / 2;
      left = targetRect.right + padding + arrowSize;
      arrowStyle = {
        left: -arrowSize,
        top: '50%',
        transform: 'translateY(-50%)',
        borderTop: `${arrowSize}px solid transparent`,
        borderBottom: `${arrowSize}px solid transparent`,
        borderRight: `${arrowSize}px solid var(--color-bg-elevated)`,
      };
      break;
  }

  // Keep tooltip within viewport
  left = Math.max(padding, Math.min(left, window.innerWidth - tooltipWidth - padding));
  top = Math.max(padding, Math.min(top, window.innerHeight - tooltipHeight - padding));

  return (
    <div
      className="fixed z-[10001] bg-[var(--color-bg-elevated)] rounded-lg shadow-xl border border-[var(--color-border)] p-4"
      style={{ top, left, width: tooltipWidth }}
    >
      {/* Arrow */}
      <div className="absolute w-0 h-0" style={arrowStyle} />

      {/* Content */}
      <h3 className="font-semibold text-[var(--color-text-primary)] mb-1">{step.title}</h3>
      <p className="text-sm text-[var(--color-text-muted)] mb-4">{step.description}</p>

      {/* Progress & Actions */}
      <div className="flex items-center justify-between">
        <span className="text-xs text-[var(--color-text-muted)]">
          {currentIndex + 1} of {totalSteps}
        </span>
        <div className="flex gap-2">
          <button
            onClick={onSkip}
            className="px-2 py-1 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
          >
            Skip
          </button>
          {currentIndex > 0 && (
            <button
              onClick={onPrev}
              className="px-2 py-1 text-xs bg-[var(--color-bg-card)] rounded hover:bg-[var(--color-bg-page)] transition-colors"
            >
              Back
            </button>
          )}
          <button
            onClick={onNext}
            className="px-3 py-1 text-xs bg-[var(--color-info)] rounded hover:bg-[var(--color-info)]/80 transition-colors"
          >
            {currentIndex === totalSteps - 1 ? 'Done' : 'Next'}
          </button>
        </div>
      </div>
    </div>
  );
};

export const FirstRunTour = ({ windowType, steps }: FirstRunTourProps) => {
  const completedTours = useSettingsStore((s) => s.completedTours);
  const setTourCompleted = useSettingsStore((s) => s.setTourCompleted);
  const hasCompleted = completedTours[windowType] === true;

  // Ref to always hold the latest steps, avoiding stale closures in callbacks/effects
  const stepsRef = useRef(steps);
  stepsRef.current = steps;

  const [currentStep, setCurrentStep] = useState(0);
  const [targetRect, setTargetRect] = useState<DOMRect | null>(null);
  const [isVisible, setIsVisible] = useState(false);

  // Show tour after a short delay to let the UI settle
  useEffect(() => {
    if (hasCompleted) return;

    const timer = setTimeout(() => {
      setIsVisible(true);
    }, 500);

    return () => clearTimeout(timer);
  }, [hasCompleted]);

  // Update target element position
  useEffect(() => {
    if (!isVisible || hasCompleted) return;

    const s = stepsRef.current;
    const step = s[currentStep];
    const element = document.querySelector(step.target);

    if (element) {
      setTargetRect(element.getBoundingClientRect());

      // Update on scroll/resize
      const updateRect = () => {
        setTargetRect(element.getBoundingClientRect());
      };

      window.addEventListener('resize', updateRect);
      window.addEventListener('scroll', updateRect, true);

      return () => {
        window.removeEventListener('resize', updateRect);
        window.removeEventListener('scroll', updateRect, true);
      };
    } else {
      // Skip to next step if target not found
      if (currentStep < s.length - 1) {
        setCurrentStep((prev) => prev + 1);
      } else {
        setTourCompleted(windowType);
      }
    }
  }, [currentStep, isVisible, hasCompleted, setTourCompleted, windowType]);

  const handleNext = useCallback(() => {
    const s = stepsRef.current;
    if (currentStep < s.length - 1) {
      setCurrentStep((prev) => prev + 1);
    } else {
      setTourCompleted(windowType);
    }
  }, [currentStep, setTourCompleted, windowType]);

  const handlePrev = useCallback(() => {
    if (currentStep > 0) {
      setCurrentStep((prev) => prev - 1);
    }
  }, [currentStep]);

  const handleSkip = useCallback(() => {
    setTourCompleted(windowType);
  }, [setTourCompleted, windowType]);

  if (hasCompleted || !isVisible) return null;

  const step = steps[currentStep];

  return createPortal(
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 z-[10000] bg-black/50"
        onClick={handleSkip}
      />

      {/* Highlight ring around target */}
      {targetRect && (
        <div
          className="fixed z-[10000] pointer-events-none rounded-lg ring-2 ring-[var(--color-info)] ring-offset-2 ring-offset-[var(--color-bg-page)]"
          style={{
            top: targetRect.top - 4,
            left: targetRect.left - 4,
            width: targetRect.width + 8,
            height: targetRect.height + 8,
          }}
        />
      )}

      {/* Tooltip */}
      <Tooltip
        step={step}
        currentIndex={currentStep}
        totalSteps={steps.length}
        onNext={handleNext}
        onPrev={handlePrev}
        onSkip={handleSkip}
        targetRect={targetRect}
      />
    </>,
    document.body
  );
}
