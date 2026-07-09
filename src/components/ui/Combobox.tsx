/**
 * Combobox - Generic searchable dropdown
 *
 * A compact dropdown with search/filter capability.
 * Supports keyboard navigation and click-outside-to-close.
 */
import { useState, useRef, useEffect } from 'react';

export interface ComboboxOption {
  value: string;
  label: string;
}

export interface ComboboxProps {
  /** Current selected value */
  value: string;
  /** Callback when value changes */
  onChange: (value: string) => void;
  /** List of available options */
  options: ComboboxOption[];
  /** Optional className for the container */
  className?: string;
  /** Whether the picker is disabled */
  disabled?: boolean;
  /** Placeholder when no value is selected */
  placeholder?: string;
  /** Width class for the input */
  width?: string;
  /** Whether to show a chevron button */
  showChevron?: boolean;
}

export function Combobox({
  value,
  onChange,
  options,
  className = '',
  disabled = false,
  placeholder,
  width = 'w-24',
  showChevron = false,
}: ComboboxProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [inputValue, setInputValue] = useState('');
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const selectedOption = options.find((o) => o.value === value);
  const displayValue = selectedOption?.label || value;

  // Filter options based on input
  const filteredOptions = inputValue
    ? options.filter((o) =>
        o.label.toLowerCase().includes(inputValue.toLowerCase())
      )
    : options;

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

  const handleSelect = (option: ComboboxOption) => {
    onChange(option.value);
    setIsOpen(false);
    setInputValue('');
    inputRef.current?.blur();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && filteredOptions.length > 0) {
      handleSelect(filteredOptions[0]);
    } else if (e.key === 'Escape') {
      setIsOpen(false);
      setInputValue('');
      inputRef.current?.blur();
    }
  };

  return (
    <div className={`relative ${width} ${className}`} ref={containerRef}>
      <div
        className={`bg-transparent border rounded transition-colors focus-within:border-[var(--color-border-focus)] ${
          showChevron ? 'flex items-center' : ''
        } ${
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
          className={`w-full bg-transparent px-3 py-1.5 text-sm outline-none text-[var(--color-text-primary)] placeholder:text-[var(--color-text-muted)] disabled:cursor-not-allowed ${showChevron ? 'min-w-0 flex-1' : ''}`}
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
        <div className="absolute top-full left-0 mt-1 min-w-full bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded shadow-xl z-50 py-1 max-h-48 overflow-auto">
          {filteredOptions.length > 0 ? (
            filteredOptions.map((option) => (
              <button
                key={option.value}
                type="button"
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => handleSelect(option)}
                className={`w-full text-center px-3 py-1.5 text-sm transition-colors ${
                  option.value === value
                    ? 'text-[var(--color-text-primary)] bg-[var(--color-bg-active)]'
                    : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-hover)]'
                }`}
              >
                {option.label}
              </button>
            ))
          ) : (
            <div className="px-3 py-1.5 text-sm text-[var(--color-text-muted)] text-center">
              No matches
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export default Combobox;
