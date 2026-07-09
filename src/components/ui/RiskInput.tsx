/**
 * RiskInput - Input field for risk management values (SL/TP/TS)
 *
 * A compact input that combines:
 * - CycleSelector for mode switching (pips/price/%)
 * - Text input for the value
 * - Label
 * - Calculated price display
 *
 * @example
 * ```tsx
 * <RiskInput
 *   label="Stop Loss"
 *   modes={['pips', 'price', '%']}
 *   mode={slMode}
 *   onModeChange={setSlMode}
 *   value={slValue}
 *   onChange={setSlValue}
 *   calculatedPrice={calculateSlPrice()}
 *   variant="loss"
 * />
 * ```
 */
import { CycleSelector } from './CycleSelector';

export type RiskInputVariant = 'loss' | 'profit' | 'neutral';

export interface RiskInputProps<T extends string = string> {
  /** Label text displayed below the input */
  label: string;
  /** Available modes to cycle through */
  modes: readonly T[];
  /** Current mode value */
  mode: T;
  /** Callback when mode changes */
  onModeChange: (mode: T) => void;
  /** Current input value */
  value: string;
  /** Callback when value changes */
  onChange: (value: string) => void;
  /** Calculated price to display (or null if not calculable) */
  calculatedPrice?: string | null;
  /** Visual variant - affects the calculated price color */
  variant?: RiskInputVariant;
  /** Optional className for the container */
  className?: string;
  /** Whether the input is disabled */
  disabled?: boolean;
  /** Placeholder text */
  placeholder?: string;
}

export function RiskInput<T extends string = string>({
  label,
  modes,
  mode,
  onModeChange,
  value,
  onChange,
  calculatedPrice,
  variant = 'neutral',
  className = '',
  disabled = false,
  placeholder = '—',
}: RiskInputProps<T>) {
  const handleFocus = (e: React.FocusEvent<HTMLInputElement>) => {
    // Select all text on focus for easy replacement
    setTimeout(() => e.target.select(), 0);
  };

  // Determine the color for the calculated price based on variant
  const getPriceColor = () => {
    if (!value) return 'text-[var(--color-text-muted)]';
    switch (variant) {
      case 'loss':
        return 'text-[var(--color-sell-text)]';
      case 'profit':
        return 'text-[var(--color-buy-text)]';
      default:
        return 'text-[var(--color-text-muted)]';
    }
  };

  return (
    <div className={`flex flex-col items-center ${className}`}>
      <input
        type="text"
        inputMode="decimal"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onFocus={handleFocus}
        placeholder={placeholder}
        disabled={disabled}
        className="w-full bg-transparent border border-[var(--color-border)] rounded px-3 py-2 text-sm text-center focus:outline-none focus:border-[var(--color-border-focus)] placeholder:text-[var(--color-text-muted)] text-[var(--color-text-primary)] disabled:opacity-50 disabled:cursor-not-allowed"
      />
      <div className="flex items-center gap-1 mt-1 self-start">
        <span className="text-[11px] text-[var(--color-text-muted)]">{label}</span>
        <CycleSelector
          options={modes}
          value={mode}
          onChange={onModeChange}
          disabled={disabled}
        />
      </div>
      <span className={`text-[11px] font-mono self-start ${getPriceColor()}`}>
        @ {value ? (calculatedPrice || '—') : '—'}
      </span>
    </div>
  );
}

export default RiskInput;
