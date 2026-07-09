/**
 * LotSizeSelector - Combobox for selecting position size in units or lots
 *
 * A numeric input with dropdown suggestions for common lot sizes.
 * Supports both direct unit entry and preset lot selections.
 *
 * @example
 * ```tsx
 * <LotSizeSelector
 *   units={units}
 *   onChange={setUnits}
 * />
 *
 * // With custom options
 * <LotSizeSelector
 *   units={units}
 *   onChange={setUnits}
 *   options={[
 *     { units: '1000', label: 'Micro' },
 *     { units: '10000', label: 'Mini' },
 *   ]}
 * />
 * ```
 */
import { useState, useRef, useEffect } from 'react';

export interface LotOption {
  units: string;
  label: string;
}

export const DEFAULT_LOT_OPTIONS: LotOption[] = [
  { units: '1000', label: '.01 lot' },
  { units: '10000', label: '.1 lot' },
  { units: '100000', label: '1 lot' },
  { units: '500000', label: '5 lots' },
];

export interface LotSizeSelectorProps {
  /** Current units value as string */
  units: string;
  /** Callback when units change */
  onChange: (units: string) => void;
  /** Available lot options (defaults to standard FX lots) */
  options?: LotOption[];
  /** Helper text displayed below the input */
  helperText?: string;
  /** Optional className for the container */
  className?: string;
  /** Whether the selector is disabled */
  disabled?: boolean;
}

export function LotSizeSelector({
  units,
  onChange,
  options = DEFAULT_LOT_OPTIONS,
  helperText = 'Position size',
  className = '',
  disabled = false,
}: LotSizeSelectorProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [inputValue, setInputValue] = useState('');
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const unitsNum = parseInt(units, 10) || 0;

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

  // Sync input value with units when not focused
  useEffect(() => {
    if (document.activeElement !== inputRef.current) {
      setInputValue(unitsNum.toLocaleString());
    }
  }, [unitsNum]);

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const raw = e.target.value.replace(/,/g, '');
    setInputValue(e.target.value);
    if (/^\d*$/.test(raw)) {
      onChange(raw || '0');
    }
  };

  const handleFocus = () => {
    if (disabled) return;
    setIsOpen(true);
    // Select all text on focus for easy replacement
    setTimeout(() => inputRef.current?.select(), 0);
  };

  const handleBlur = () => {
    // Format the value on blur
    setInputValue(unitsNum.toLocaleString());
  };

  const handleSelect = (optUnits: string) => {
    onChange(optUnits);
    setIsOpen(false);
    inputRef.current?.blur();
  };

  return (
    <div className={`relative mt-3 w-full ${className}`} ref={containerRef}>
      {/* Combobox input */}
      <div
        className={`w-full flex items-center bg-transparent border rounded transition-colors focus-within:border-[var(--color-border-focus)] ${
          isOpen
            ? 'border-[var(--color-border-focus)]'
            : 'border-[var(--color-border)] hover:border-[var(--color-border-focus)]'
        } ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}
      >
        <input
          ref={inputRef}
          type="text"
          inputMode="numeric"
          value={inputValue}
          onChange={handleInputChange}
          onFocus={handleFocus}
          onBlur={handleBlur}
          disabled={disabled}
          className="min-w-0 flex-1 bg-transparent px-3 py-2 text-sm font-mono font-normal text-[var(--color-text-primary)] outline-none cursor-text disabled:cursor-not-allowed"
          placeholder="Type units or select"
        />
        <button
          type="button"
          onClick={() => {
            if (disabled) return;
            setIsOpen(!isOpen);
            if (!isOpen) inputRef.current?.focus();
          }}
          disabled={disabled}
          className="flex-shrink-0 px-2 py-2 text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors outline-none focus:text-[var(--color-text-primary)] focus:bg-[var(--color-bg-active)] rounded-r disabled:cursor-not-allowed"
        >
          <svg
            className={`w-3 h-3 transition-transform duration-200 ${isOpen ? 'rotate-180' : ''}`}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}
          >
            <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
          </svg>
        </button>
      </div>

      {/* Helper text */}
      <p className="text-[11px] text-[var(--color-text-faint)] mt-1.5">{helperText}</p>

      {/* Dropdown suggestions */}
      {isOpen && (
        <div className="absolute top-full left-0 right-0 mt-1 bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded-lg shadow-xl z-50 py-1 max-h-[130px] overflow-auto">
          {options.map((opt) => (
            <button
              key={opt.units}
              type="button"
              onMouseDown={(e) => e.preventDefault()} // Prevent blur before click
              onClick={() => handleSelect(opt.units)}
              className={`w-full px-3 py-1.5 text-sm text-center transition-colors ${
                units === opt.units
                  ? 'text-[var(--color-text-primary)] bg-[var(--color-bg-active)]'
                  : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-hover)]'
              }`}
            >
              {opt.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export default LotSizeSelector;
