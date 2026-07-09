/**
 * CycleSelector - Dot-indicator toggle that cycles through options
 *
 * A compact control that displays the current selection with dot indicators
 * and cycles through options on click. Commonly used for mode selectors
 * like order type, input mode (pips/price/%), etc.
 *
 * Memoized to prevent unnecessary re-renders and visual flicker (BUG-068).
 *
 * @example
 * ```tsx
 * // Simple usage with strings
 * <CycleSelector
 *   options={['market', 'limit', 'stop']}
 *   value={orderType}
 *   onChange={setOrderType}
 * />
 *
 * // With custom labels and colors
 * <CycleSelector
 *   options={[
 *     { value: 'standard', label: 'standard logic' },
 *     { value: 'inverted', label: 'inverted logic', activeColor: 'var(--color-sell)' },
 *   ]}
 *   value={mode}
 *   onChange={setMode}
 * />
 * ```
 */

import { useMemo, useCallback, memo } from 'react';

export interface CycleSelectorOption<T extends string = string> {
  value: T;
  label: string;
  /** Optional color applied to dot and label when this option is selected */
  activeColor?: string;
}

export interface CycleSelectorProps<T extends string = string> {
  /** Array of options - can be strings or objects with value/label */
  options: readonly T[] | readonly CycleSelectorOption<T>[];
  /** Current selected value */
  value: T;
  /** Callback when value changes */
  onChange: (value: T) => void;
  /** Optional className for the container */
  className?: string;
  /** Whether the selector is disabled */
  disabled?: boolean;
  /** Capitalize the first letter of the label (default: false) */
  capitalize?: boolean;
  /** Custom label formatter - overrides default label display */
  formatLabel?: (value: T) => string;
}

/**
 * Normalizes option input to always return CycleSelectorOption[]
 */
function normalizeOptions<T extends string>(
  options: readonly T[] | readonly CycleSelectorOption<T>[]
): CycleSelectorOption<T>[] {
  return options.map((opt) =>
    typeof opt === 'string' ? { value: opt, label: opt } : opt
  );
}

const MAX_VISIBLE_DOTS = 5;

function CycleSelectorInner<T extends string = string>({
  options,
  value,
  onChange,
  className = '',
  disabled = false,
  capitalize = false,
  formatLabel,
}: CycleSelectorProps<T>) {
  // Memoize normalizedOptions to avoid creating new arrays on every render.
  // Uses JSON key derived from option values for stable identity.
  const normalizedOptions = useMemo(
    () => normalizeOptions(options),
    // options is typically a literal array or `as const`, so we serialize
    // the values to get a stable dependency.  This is cheap for the small
    // option lists CycleSelector is designed for.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [JSON.stringify(options)],
  );

  const currentIndex = useMemo(
    () => normalizedOptions.findIndex((opt) => opt.value === value),
    [normalizedOptions, value],
  );

  const handleClick = useCallback(
    (e?: React.MouseEvent | React.KeyboardEvent) => {
      if (disabled) return;
      const nextIndex = e?.shiftKey
        ? (currentIndex - 1 + normalizedOptions.length) % normalizedOptions.length
        : (currentIndex + 1) % normalizedOptions.length;
      onChange(normalizedOptions[nextIndex].value);
    },
    [disabled, currentIndex, normalizedOptions, onChange],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (disabled) return;
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        const nextIndex = (currentIndex + 1) % normalizedOptions.length;
        onChange(normalizedOptions[nextIndex].value);
      } else if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
        e.preventDefault();
        const nextIndex = (currentIndex + 1) % normalizedOptions.length;
        onChange(normalizedOptions[nextIndex].value);
      } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
        e.preventDefault();
        const prevIndex =
          (currentIndex - 1 + normalizedOptions.length) % normalizedOptions.length;
        onChange(normalizedOptions[prevIndex].value);
      }
    },
    [disabled, currentIndex, normalizedOptions, onChange],
  );

  const currentOption = normalizedOptions[currentIndex];
  let displayLabel = currentOption?.label ?? value;

  const shouldCondense = normalizedOptions.length > MAX_VISIBLE_DOTS;

  // Calculate visible window of dots
  const { visibleOptions, visibleStartIndex, visibleEndIndex } = useMemo(() => {
    let startIdx = 0;
    let endIdx = normalizedOptions.length;

    if (shouldCondense) {
      const halfWindow = Math.floor(MAX_VISIBLE_DOTS / 2);
      startIdx = Math.max(
        0,
        Math.min(currentIndex - halfWindow, normalizedOptions.length - MAX_VISIBLE_DOTS),
      );
      endIdx = startIdx + MAX_VISIBLE_DOTS;
    }

    return {
      visibleOptions: normalizedOptions.slice(startIdx, endIdx),
      visibleStartIndex: startIdx,
      visibleEndIndex: endIdx,
    };
  }, [normalizedOptions, currentIndex, shouldCondense]);

  // Calculate opacity for fading effect on edge dots
  const getDotOpacity = useCallback(
    (visibleIdx: number): number => {
      if (!shouldCondense) return 1;

      const hasMoreBefore = visibleStartIndex > 0;
      const hasMoreAfter = visibleEndIndex < normalizedOptions.length;

      // Fade first 2 dots if there are more before
      if (hasMoreBefore) {
        if (visibleIdx === 0) return 0.25;
        if (visibleIdx === 1) return 0.5;
      }

      // Fade last 2 dots if there are more after
      if (hasMoreAfter) {
        if (visibleIdx === MAX_VISIBLE_DOTS - 1) return 0.25;
        if (visibleIdx === MAX_VISIBLE_DOTS - 2) return 0.5;
      }

      return 1;
    },
    [shouldCondense, visibleStartIndex, visibleEndIndex, normalizedOptions.length],
  );

  // Apply formatLabel if provided, otherwise apply capitalize if enabled
  if (formatLabel) {
    displayLabel = formatLabel(value);
  } else if (capitalize) {
    displayLabel = displayLabel.charAt(0).toUpperCase() + displayLabel.slice(1);
  }

  // Memoize allLabels to avoid recalculating on every render
  const allLabels = useMemo(
    () =>
      normalizedOptions.map((opt) => {
        let label = opt.label;
        if (formatLabel) {
          label = formatLabel(opt.value);
        } else if (capitalize) {
          label = label.charAt(0).toUpperCase() + label.slice(1);
        }
        return label;
      }),
    [normalizedOptions, formatLabel, capitalize],
  );

  return (
    <button
      type="button"
      role="listbox"
      aria-label={`Select option, current: ${displayLabel}`}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      disabled={disabled}
      className={`flex flex-col items-center gap-0.5 group px-2 pt-1 pb-1 transition-opacity ${
        disabled ? 'opacity-50 cursor-not-allowed' : ''
      } ${className}`}
    >
      {/* Dot indicators */}
      <div className="flex gap-1" role="presentation">
        {visibleOptions.map((opt, visibleIdx) => {
          const opacity = getDotOpacity(visibleIdx);
          const isSelected = opt.value === value;
          return (
            <div
              key={opt.value}
              className={`w-1 h-1 rounded-full transition-all ${
                isSelected
                  ? (opt.activeColor ? '' : 'bg-[var(--color-indicator-active)]')
                  : 'bg-[var(--color-indicator-inactive)] group-hover:bg-[var(--color-indicator-hover)]'
              }`}
              style={{
                ...(isSelected && opt.activeColor ? { backgroundColor: opt.activeColor } : {}),
                opacity,
              }}
            />
          );
        })}
      </div>
      {/* Current value label - uses grid to stack all options and size to widest */}
      <span className="grid text-sm">
        {allLabels.map((label, i) => {
          const isSelected = normalizedOptions[i].value === value;
          const activeColor = normalizedOptions[i].activeColor;
          return (
            <span
              key={normalizedOptions[i].value}
              className={`col-start-1 row-start-1 text-center whitespace-nowrap ${
                isSelected
                  ? (activeColor ? '' : 'text-[var(--color-text-muted)] group-hover:text-[var(--color-text-primary)]')
                  : 'invisible'
              }`}
              style={isSelected && activeColor ? { color: activeColor } : undefined}
              aria-hidden={!isSelected}
            >
              {label}
            </span>
          );
        })}
      </span>
    </button>
  );
}

// Wrap with React.memo to prevent re-renders when parent re-renders
// with the same props. The generic type is preserved by exporting both
// the memo'd component and the original types.
export const CycleSelector = memo(CycleSelectorInner) as typeof CycleSelectorInner;

export default CycleSelector;
