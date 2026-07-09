import { useState, useRef, useEffect } from 'react';
import { GRANULARITIES } from '../../constants';

type SelectorVariant = 'default' | 'inline';

interface GranularitySelectorProps {
  value: string;
  onChange: (granularity: string) => void;
  label?: string;
  className?: string;
  disabled?: boolean;
  variant?: SelectorVariant;
}

/**
 * Reusable granularity/timeframe selector dropdown.
 * Uses the shared GRANULARITIES constant.
 */
export const GranularitySelector = ({
  value,
  onChange,
  label,
  className = '',
  disabled = false,
  variant = 'default',
}: GranularitySelectorProps) => {
  const [isOpen, setIsOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // Close dropdown when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const currentLabel = GRANULARITIES.find(g => g.value === value)?.label ?? value;

  // Inline variant: title-like text that opens dropdown
  if (variant === 'inline') {
    return (
      <div className={`relative ${className}`} ref={containerRef}>
        <button
          type="button"
          onClick={() => !disabled && setIsOpen(!isOpen)}
          disabled={disabled}
          className={`group flex items-center gap-1 text-sm text-[var(--color-text-muted)] hover:text-[var(--color-info-text)] transition-colors disabled:opacity-50 disabled:cursor-not-allowed`}
        >
          <span>{currentLabel}</span>
          <svg
            className={`w-3 h-3 text-[var(--color-text-muted)] group-hover:text-[var(--color-info-text)] transition-all duration-200 ${isOpen ? 'rotate-180' : ''}`}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}
          >
            <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
          </svg>
        </button>

        {isOpen && (
          <div className="absolute top-full left-0 mt-1 min-w-[100px] bg-[var(--color-bg-elevated)] rounded-lg shadow-lg z-50 py-1 max-h-60 overflow-auto">
            {GRANULARITIES.map((g) => (
              <button
                key={g.value}
                type="button"
                onClick={() => {
                  onChange(g.value);
                  setIsOpen(false);
                }}
                className={`w-full text-left px-3 py-2 text-sm transition-colors ${
                  g.value === value
                    ? 'text-[var(--color-info-text)]'
                    : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-card)]'
                }`}
              >
                {g.label}
              </button>
            ))}
          </div>
        )}
      </div>
    );
  }

  // Default variant: custom dropdown with design tokens
  return (
    <div className={`relative ${className}`} ref={containerRef}>
      {label && (
        <label className="block text-xs text-[var(--color-text-muted)] mb-1">{label}</label>
      )}
      <button
        type="button"
        onClick={() => !disabled && setIsOpen(!isOpen)}
        disabled={disabled}
        className={`w-full flex items-center justify-between bg-transparent border rounded px-3 py-2 text-xs transition-colors focus:outline-none ${
          isOpen
            ? 'border-[var(--color-border-focus)]'
            : 'border-[var(--color-border)] hover:border-[var(--color-border-focus)]'
        } ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}
      >
        <span className="text-[var(--color-text-primary)]">{currentLabel}</span>
        <svg
          className={`w-3 h-3 text-[var(--color-text-muted)] transition-transform duration-200 ${isOpen ? 'rotate-180' : ''}`}
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={2}
        >
          <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
        </svg>
      </button>

      {isOpen && (
        <div className="absolute top-full left-1/2 -translate-x-1/2 mt-1 min-w-[100px] bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded shadow-xl z-50 py-1 max-h-48 overflow-auto">
          {GRANULARITIES.map((g) => (
            <button
              key={g.value}
              type="button"
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => {
                onChange(g.value);
                setIsOpen(false);
              }}
              className={`w-full text-center px-3 py-1.5 text-xs transition-colors ${
                g.value === value
                  ? 'text-[var(--color-text-primary)] bg-[var(--color-bg-active)]'
                  : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-hover)]'
              }`}
            >
              {g.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
