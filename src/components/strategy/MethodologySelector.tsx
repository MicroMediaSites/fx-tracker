/**
 * MethodologySelector - Combobox for selecting backtest methodology.
 *
 * Shows selected methodology with description, expands to show options.
 */
import { useState, useRef, useEffect } from 'react';
import {
  BacktestMethodology,
  METHODOLOGY_LABELS,
} from '../../types/strategy';

interface MethodologySelectorProps {
  value: BacktestMethodology | null;
  onChange: (methodology: BacktestMethodology) => void;
  className?: string;
  /** Disable future methodologies that aren't implemented yet */
  disableUnimplemented?: boolean;
}

/** Methodologies that are currently implemented */
const IMPLEMENTED_METHODOLOGIES: BacktestMethodology[] = [
  'simple',
  'walk_forward',
];

const METHODOLOGY_DESCRIPTIONS: Record<BacktestMethodology, string> = {
  simple: 'Run strategy across the full date range. Basic but prone to overfitting.',
  train_test: '', // Deprecated - not shown in UI
  walk_forward:
    'Rolling or anchored window optimization. Train on past data, test on forward periods. Gold standard.',
  anchored_walk_forward: '', // Consolidated into walk_forward with toggle
  monte_carlo_sequence:
    'Shuffle trade order 1,000 times to understand how much results depend on luck vs edge.',
  monte_carlo_parameter:
    'Test strategy stability by slightly perturbing parameter values (±10%).',
  regime_based:
    'See how strategy performs in different market conditions (trend, range, volatile, quiet).',
  bootstrap:
    'Resample trades with replacement to generate statistical confidence intervals.',
};

export const MethodologySelector = ({
  value,
  onChange,
  className = '',
  disableUnimplemented = true,
}: MethodologySelectorProps) => {
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const methodologies: BacktestMethodology[] = [
    'simple',
    'walk_forward',
    'monte_carlo_sequence',
    'monte_carlo_parameter',
    'regime_based',
    'bootstrap',
  ];

  // Close dropdown when clicking outside
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const handleSelect = (methodology: BacktestMethodology) => {
    onChange(methodology);
    setIsOpen(false);
  };

  const isMethodologyImplemented = (methodology: BacktestMethodology) => {
    return IMPLEMENTED_METHODOLOGIES.includes(methodology);
  };

  return (
    <div className={`relative ${className}`} ref={dropdownRef}>
      {/* Combobox trigger - shows selected value with description */}
      <button
        type="button"
        onClick={() => setIsOpen(!isOpen)}
        className="w-full text-left pl-3 pr-2 py-2 border-l-2 border-[var(--color-border)] hover:border-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)]/50 transition-colors rounded-r"
      >
        <div className="flex items-start justify-between gap-2">
          <div className="flex-1 min-w-0">
            {value ? (
              <>
                <div className="flex items-center gap-2">
                  <span className="font-medium text-[var(--color-text-primary)]">
                    {METHODOLOGY_LABELS[value]}
                  </span>
                </div>
                <div className="text-xs text-[var(--color-text-muted)] mt-0.5 line-clamp-2">
                  {METHODOLOGY_DESCRIPTIONS[value]}
                </div>
              </>
            ) : (
              <span className="text-[var(--color-text-muted)]">Select methodology...</span>
            )}
          </div>
          <svg
            className={`w-4 h-4 text-[var(--color-text-muted)] transition-transform flex-shrink-0 mt-0.5 ${isOpen ? 'rotate-180' : ''}`}
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
          </svg>
        </div>
      </button>

      {/* Dropdown menu */}
      {isOpen && (
        <div className="absolute z-50 w-full mt-1 bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded shadow-lg overflow-hidden">
          {methodologies.map((methodology) => {
            const isImplemented = isMethodologyImplemented(methodology);
            const isDisabled = disableUnimplemented && !isImplemented;
            const isSelected = value === methodology;

            return (
              <button
                key={methodology}
                type="button"
                onClick={() => !isDisabled && handleSelect(methodology)}
                disabled={isDisabled}
                className={`w-full px-3 py-2 text-left transition-colors
                  ${isSelected ? 'bg-[var(--color-info)]/20' : 'hover:bg-[var(--color-bg-hover)]'}
                  ${isDisabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}
                `}
              >
                <div className="flex items-center gap-2">
                  {/* Selection indicator */}
                  <span className="w-4 flex-shrink-0">
                    {isSelected && (
                      <svg className="w-4 h-4 text-[var(--color-info)]" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                      </svg>
                    )}
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className={`text-sm ${isSelected ? 'text-[var(--color-info)]' : 'text-[var(--color-text-primary)]'}`}>
                      {METHODOLOGY_LABELS[methodology]}
                      {isDisabled && (
                        <span className="text-xs text-[var(--color-text-muted)] ml-2">(coming soon)</span>
                      )}
                    </div>
                  </div>
                </div>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
};

/**
 * MethodologyInfo - Shows description of the selected methodology.
 * Now integrated into the selector, but kept for backwards compatibility.
 */
interface MethodologyInfoProps {
  methodology: BacktestMethodology;
  className?: string;
}

export const MethodologyInfo = ({ methodology: _methodology, className: _className = '' }: MethodologyInfoProps) => {
  // Description is now shown in the selector itself, so this is minimal
  return null;
};
