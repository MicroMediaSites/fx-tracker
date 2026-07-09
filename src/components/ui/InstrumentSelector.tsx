import { useState, useRef, useEffect } from 'react';
import { useSettingsStore } from '../../stores/settingsStore';

type SelectorVariant = 'default' | 'outline' | 'inline';

interface InstrumentSelectorProps {
  value: string;
  onChange: (instrument: string) => void;
  onAddSymbol?: () => void;
  label?: string;
  className?: string;
  disabled?: boolean;
  variant?: SelectorVariant;
}

/**
 * Reusable instrument/symbol selector dropdown.
 * Uses the shared mySymbols list from settings store.
 * Includes "Add Symbol..." option that triggers onAddSymbol callback.
 *
 * @deprecated Use `SymbolPicker` from './SymbolPicker' instead for new code.
 * SymbolPicker provides a searchable combobox with design token support.
 * This component will be removed in a future release.
 */
export const InstrumentSelector = ({
  value,
  onChange,
  onAddSymbol,
  label,
  className = '',
  disabled = false,
  variant = 'default',
}: InstrumentSelectorProps) => {
  const { mySymbols } = useSettingsStore();
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

  // For default variant, use native select
  if (variant === 'default') {
    const handleChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
      const newValue = e.target.value;
      if (newValue === '__add_symbol__') {
        onAddSymbol?.();
      } else {
        onChange(newValue);
      }
    };

    return (
      <div className={className}>
        {label && (
          <label className="block text-xs text-gray-500 mb-1">{label}</label>
        )}
        <select
          value={value}
          onChange={handleChange}
          disabled={disabled}
          className="w-full bg-gray-700 border border-gray-600 rounded px-3 text-sm disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {mySymbols.map((inst) => (
            <option key={inst} value={inst}>
              {inst.replace('_', '/')}
            </option>
          ))}
          {onAddSymbol && (
            <option value="__add_symbol__" className="text-blue-400">
              + Add Symbol...
            </option>
          )}
        </select>
      </div>
    );
  }

  // Custom dropdown variants (outline and inline)
  const handleSelect = (inst: string) => {
    onChange(inst);
    setIsOpen(false);
  };

  // Inline variant: title-like text that opens dropdown
  if (variant === 'inline') {
    return (
      <div className={`relative ${className}`} ref={containerRef}>
        <button
          type="button"
          onClick={() => !disabled && setIsOpen(!isOpen)}
          disabled={disabled}
          className={`group flex items-center gap-1 text-base font-semibold text-[var(--color-text-primary)] hover:text-[var(--color-info-text)] transition-colors disabled:opacity-50 disabled:cursor-not-allowed`}
        >
          <span>{value.replace('_', '/')}</span>
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
          <div className="absolute top-full left-0 mt-1 min-w-[140px] bg-[var(--color-bg-elevated)] rounded-lg shadow-lg z-50 py-1 max-h-60 overflow-auto">
            {mySymbols.map((inst) => (
              <button
                key={inst}
                type="button"
                onClick={() => handleSelect(inst)}
                className={`w-full text-left px-3 py-2 text-sm transition-colors ${
                  inst === value
                    ? 'text-[var(--color-info-text)]'
                    : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-card)]'
                }`}
              >
                {inst.replace('_', '/')}
              </button>
            ))}
            {onAddSymbol && (
              <button
                type="button"
                onClick={() => {
                  setIsOpen(false);
                  onAddSymbol();
                }}
                className="w-full text-left px-3 py-2 text-sm text-[var(--color-info-text)] hover:bg-[var(--color-bg-card)] border-t border-[var(--color-border)] mt-1"
              >
                + Add Symbol...
              </button>
            )}
          </div>
        )}
      </div>
    );
  }

  // Outline variant: custom dropdown with chevron
  return (
    <div className={`relative ${className}`} ref={containerRef}>
      {label && (
        <label className="block text-xs text-gray-500 mb-1">{label}</label>
      )}
      <button
        type="button"
        onClick={() => !disabled && setIsOpen(!isOpen)}
        disabled={disabled}
        className={`flex items-center gap-2 bg-transparent border border-gray-700 rounded-lg px-3 py-2 text-sm focus:outline-none focus:border-gray-500 disabled:opacity-50 disabled:cursor-not-allowed transition-colors ${
          isOpen ? 'border-gray-500' : ''
        }`}
      >
        <span>{value.replace('_', '/')}</span>
        <svg
          className={`w-4 h-4 text-gray-500 transition-transform duration-200 ${isOpen ? 'rotate-180' : ''}`}
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={2}
        >
          <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
        </svg>
      </button>

      {isOpen && (
        <div className="absolute top-full left-0 mt-1 min-w-full bg-[#1a1f26] border border-gray-700 rounded-lg shadow-xl z-50 py-1 max-h-60 overflow-auto">
          {mySymbols.map((inst) => (
            <button
              key={inst}
              type="button"
              onClick={() => handleSelect(inst)}
              className={`w-full text-left px-3 py-2 text-sm transition-colors ${
                inst === value
                  ? 'text-white bg-gray-700'
                  : 'text-gray-300 hover:bg-gray-800'
              }`}
            >
              {inst.replace('_', '/')}
            </button>
          ))}
          {onAddSymbol && (
            <button
              type="button"
              onClick={() => {
                setIsOpen(false);
                onAddSymbol();
              }}
              className="w-full text-left px-3 py-2 text-sm text-blue-400 hover:bg-gray-800 border-t border-gray-700 mt-1"
            >
              + Add Symbol...
            </button>
          )}
        </div>
      )}
    </div>
  );
}
