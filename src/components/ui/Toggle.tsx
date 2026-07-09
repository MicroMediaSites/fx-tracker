/**
 * Toggle - A switch control for boolean settings
 *
 * Features:
 * - Label support (left or right positioned)
 * - Disabled state with reduced opacity
 * - Design token support for theming
 *
 * @example
 * ```tsx
 * <Toggle
 *   label="Enable notifications"
 *   checked={isEnabled}
 *   onChange={setIsEnabled}
 * />
 * ```
 */

export interface ToggleProps {
  /** The current checked state */
  checked: boolean;
  /** Callback when toggle is clicked */
  onChange: (checked: boolean) => void;
  /** Optional label text */
  label?: string;
  /** Label position (default: 'left') */
  labelPosition?: 'left' | 'right';
  /** Whether the toggle is disabled */
  disabled?: boolean;
  /** Title/tooltip for the toggle */
  title?: string;
  /** Optional className for the container */
  className?: string;
}

export function Toggle({
  checked,
  onChange,
  label,
  labelPosition = 'left',
  disabled = false,
  title,
  className = '',
}: ToggleProps) {
  const handleClick = () => {
    if (disabled) return;
    onChange(!checked);
  };

  return (
    <div
      className={`flex items-center gap-2 relative group ${className}`}
      title={title}
    >
      {label && labelPosition === 'left' && (
        <span className="text-xs text-[var(--color-text-muted)]">{label}</span>
      )}

      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={label}
        onClick={handleClick}
        disabled={disabled}
        className={`relative w-8 h-5 rounded-full transition-colors ${
          checked
            ? 'bg-[var(--color-info)]'
            : 'bg-[var(--color-bg-tertiary)]'
        } ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}`}
      >
        <div
          className={`absolute top-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
            checked ? 'translate-x-3.5' : 'translate-x-0.5'
          }`}
        />
      </button>

      {label && labelPosition === 'right' && (
        <span className="text-xs text-[var(--color-text-muted)]">{label}</span>
      )}
    </div>
  );
}

export default Toggle;
