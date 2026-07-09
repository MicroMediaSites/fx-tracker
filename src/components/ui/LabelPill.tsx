/**
 * LabelPill - Displays a single label as a pill/chip
 *
 * Used in:
 * - TradeReviewModal (showing labels on a trade)
 * - Trade list cards
 * - LabelPicker (in the dropdown)
 */

interface LabelPillProps {
  name: string;
  color?: string;
  /** Remove button callback - if provided, shows X button */
  onRemove?: () => void;
  /** Click callback for selecting */
  onClick?: () => void;
  /** Size variant */
  size?: 'sm' | 'xs';
  /** Muted style (for unselected state) */
  muted?: boolean;
}

// Default label colors when none specified
const DEFAULT_COLORS = [
  '#6366f1', // indigo
  '#8b5cf6', // violet
  '#ec4899', // pink
  '#f43f5e', // rose
  '#f97316', // orange
  '#eab308', // yellow
  '#22c55e', // green
  '#14b8a6', // teal
  '#06b6d4', // cyan
  '#3b82f6', // blue
];

/**
 * Get a consistent color for a label name (when no color specified)
 */
export function getDefaultColor(name: string): string {
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = name.charCodeAt(i) + ((hash << 5) - hash);
  }
  return DEFAULT_COLORS[Math.abs(hash) % DEFAULT_COLORS.length];
}

export const LabelPill = ({
  name,
  color,
  onRemove,
  onClick,
  size = 'sm',
  muted = false,
}: LabelPillProps) => {
  const bgColor = color || getDefaultColor(name);
  const sizeClasses = size === 'sm' ? 'px-2.5 py-1 text-xs' : 'px-2 py-0.5 text-[11px]';

  return (
    <span
      onClick={onClick}
      className={`inline-flex items-center gap-1.5 rounded-full font-medium transition-all ${sizeClasses} ${
        onClick ? 'cursor-pointer hover:opacity-80' : ''
      } ${muted ? 'opacity-50' : ''}`}
      style={{
        backgroundColor: `${bgColor}20`,
        color: bgColor,
        border: `1px solid ${bgColor}40`,
      }}
    >
      {name}
      {onRemove && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          className="hover:opacity-70 transition-opacity -mr-0.5"
          aria-label={`Remove ${name} label`}
        >
          <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      )}
    </span>
  );
};
