/**
 * DateInput - Abstracted date input component
 *
 * A wrapper around the native date input with consistent styling.
 * This component exists so we can eventually replace it with a custom
 * date picker without changing every usage site.
 *
 * @example
 * ```tsx
 * <DateInput
 *   value={dateFrom}
 *   onChange={setDateFrom}
 *   placeholder="Start date"
 * />
 * ```
 */

interface DateInputProps {
  /** Date value in YYYY-MM-DD format */
  value: string;
  /** Callback when date changes */
  onChange: (value: string) => void;
  /** Optional placeholder text */
  placeholder?: string;
  /** Optional label above the input */
  label?: string;
  /** Whether the input is disabled */
  disabled?: boolean;
  /** Additional className for the container */
  className?: string;
  /** Minimum allowed date (YYYY-MM-DD) */
  min?: string;
  /** Maximum allowed date (YYYY-MM-DD) */
  max?: string;
}

export const DateInput = ({
  value,
  onChange,
  placeholder,
  label,
  disabled = false,
  className = '',
  min,
  max,
}: DateInputProps) => {
  return (
    <div className={className}>
      {label && (
        <label className="block text-xs text-[var(--color-text-muted)] mb-1">
          {label}
        </label>
      )}
      <input
        type="date"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        disabled={disabled}
        min={min}
        max={max}
        className={`w-full bg-transparent border rounded px-3 py-2 text-xs transition-colors focus:outline-none ${
          disabled
            ? 'border-[var(--color-border)] opacity-50 cursor-not-allowed'
            : 'border-[var(--color-border)] hover:border-[var(--color-border-focus)] focus:border-[var(--color-border-focus)]'
        } text-[var(--color-text-primary)] [&::-webkit-calendar-picker-indicator]:filter [&::-webkit-calendar-picker-indicator]:invert [&::-webkit-calendar-picker-indicator]:opacity-50 [&::-webkit-calendar-picker-indicator]:hover:opacity-75`}
      />
    </div>
  );
};

export default DateInput;
