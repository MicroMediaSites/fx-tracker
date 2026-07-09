/**
 * SymbolPicker - Searchable combobox for instrument/symbol selection
 *
 * A compact dropdown with search/filter capability. Displays the current
 * symbol with underscore replaced by slash (e.g., EUR_USD → EUR/USD).
 * Supports keyboard navigation and click-outside-to-close.
 *
 * @example
 * ```tsx
 * <SymbolPicker
 *   value={instrument}
 *   onChange={setInstrument}
 *   symbols={mySymbols}
 * />
 * ```
 */
import { useState, useRef, useEffect } from 'react';

export interface SymbolPickerProps {
  /** Current selected symbol (e.g., "EUR_USD") */
  value: string;
  /** Callback when symbol changes */
  onChange: (symbol: string) => void;
  /** List of available symbols */
  symbols: string[];
  /** Optional className for the container */
  className?: string;
  /** Whether the picker is disabled */
  disabled?: boolean;
  /** Placeholder when no value is selected */
  placeholder?: string;
  /** Format display value - default replaces _ with / */
  formatDisplay?: (symbol: string) => string;
  /** Whether to show a chevron button */
  showChevron?: boolean;
}

const defaultFormatDisplay = (symbol: string) => symbol.replace('_', '/');

export function SymbolPicker({
  value,
  onChange,
  symbols,
  className = '',
  disabled = false,
  placeholder,
  formatDisplay = defaultFormatDisplay,
  showChevron = false,
}: SymbolPickerProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [inputValue, setInputValue] = useState('');
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const displayValue = formatDisplay(value);

  // Filter symbols based on input
  const filteredSymbols = inputValue
    ? symbols.filter((s) =>
        formatDisplay(s).toLowerCase().includes(inputValue.toLowerCase())
      )
    : symbols;

  // Close dropdown when clicking outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
        setInputValue('');
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setInputValue(e.target.value);
    setIsOpen(true);
  };

  const handleFocus = () => {
    if (disabled) return;
    setIsOpen(true);
    setTimeout(() => inputRef.current?.select(), 0);
  };

  const handleSelect = (symbol: string) => {
    onChange(symbol);
    setIsOpen(false);
    setInputValue('');
    inputRef.current?.blur();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && filteredSymbols.length > 0) {
      handleSelect(filteredSymbols[0]);
    } else if (e.key === 'Escape') {
      setIsOpen(false);
      setInputValue('');
      inputRef.current?.blur();
    }
  };

  return (
    <div className={`relative ${className}`} ref={containerRef}>
      <div
        className={`flex items-center bg-transparent border rounded transition-colors focus-within:border-[var(--color-border-focus)] ${
          isOpen
            ? 'border-[var(--color-border-focus)]'
            : 'border-[var(--color-border)] hover:border-[var(--color-border-focus)]'
        } ${disabled ? 'opacity-50 cursor-not-allowed' : ''}`}
      >
        <input
          ref={inputRef}
          type="text"
          value={inputValue || (document.activeElement === inputRef.current ? '' : displayValue)}
          onChange={handleInputChange}
          onFocus={handleFocus}
          onKeyDown={handleKeyDown}
          placeholder={placeholder || displayValue}
          disabled={disabled}
          className={`min-w-0 bg-transparent px-3 py-1.5 text-xs outline-none text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)] disabled:cursor-not-allowed ${showChevron ? 'flex-1 text-left' : 'w-full text-center'}`}
        />
        {showChevron && (
          <button
            type="button"
            onClick={() => {
              if (disabled) return;
              setIsOpen(!isOpen);
              if (!isOpen) inputRef.current?.focus();
            }}
            disabled={disabled}
            className="flex-shrink-0 px-2 py-1.5 text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors outline-none disabled:cursor-not-allowed"
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
        )}
      </div>

      {isOpen && (
        <div className="absolute top-full left-1/2 -translate-x-1/2 mt-1 min-w-[120px] bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded shadow-xl z-50 py-1 max-h-48 overflow-auto">
          {filteredSymbols.length > 0 ? (
            filteredSymbols.map((symbol) => (
              <button
                key={symbol}
                type="button"
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => handleSelect(symbol)}
                className={`w-full text-center px-3 py-1.5 text-xs transition-colors ${
                  symbol === value
                    ? 'text-[var(--color-text-primary)] bg-[var(--color-bg-active)]'
                    : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-hover)]'
                }`}
              >
                {formatDisplay(symbol)}
              </button>
            ))
          ) : (
            <div className="px-3 py-1.5 text-xs text-[var(--color-text-muted)] text-center">
              No matches
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export default SymbolPicker;
