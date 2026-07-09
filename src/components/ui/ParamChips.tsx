/**
 * ParamChips - Displays parameter key-value pairs as styled chips
 *
 * Used in:
 * - WalkForwardResults.tsx (Best OOS Parameters)
 * - WalkForwardPanel.tsx (Parameter Stability)
 * - HoldoutValidation.tsx (Parameters Being Validated)
 */
import { formatParamValue } from '../../utils/formatters';
import { InfoTooltip } from './InfoTooltip';

interface ParamChipsProps {
  params: Record<string, unknown>;
  /** Size variant: 'sm' for larger displays, 'xs' for compact displays */
  size?: 'sm' | 'xs';
  /** Layout: 'grid' for grid display, 'wrap' for flex-wrap */
  layout?: 'grid' | 'wrap';
  /** Optional title to display above chips */
  title?: string;
  /** Optional subtitle (displayed next to title) */
  subtitle?: string;
  /** Optional tooltip for the title */
  titleTooltip?: string;
  /** Whether to show the purple container styling */
  showContainer?: boolean;
}

export const ParamChips = ({
  params,
  size = 'sm',
  layout = 'grid',
  title,
  subtitle,
  titleTooltip,
  showContainer = false,
}: ParamChipsProps) => {
  const entries = Object.entries(params);
  if (entries.length === 0) return null;

  const sizeClasses = size === 'sm' ? 'px-2 py-1.5 text-sm' : 'px-2 py-1 text-xs';
  const layoutClasses = layout === 'grid' ? 'grid grid-cols-2 md:grid-cols-4 gap-2' : 'flex flex-wrap gap-2';

  const content = (
    <>
      {title && (
        <h4 className={`text-${size} font-medium mb-2 text-purple-300 flex items-center gap-1`}>
          {title}
          {titleTooltip && <InfoTooltip text={titleTooltip} />}
          {subtitle && <span className="text-xs text-gray-400 font-normal ml-2">{subtitle}</span>}
        </h4>
      )}
      <div className={layoutClasses}>
        {entries.map(([key, value]) => (
          <div key={key} className={`bg-gray-700/50 rounded ${sizeClasses}`}>
            <span className="text-gray-400">{key}:</span>{' '}
            <span className="font-mono font-medium text-white">{formatParamValue(value, key)}</span>
          </div>
        ))}
      </div>
    </>
  );

  if (showContainer) {
    return (
      <div className="p-3 bg-purple-900/20 border border-purple-500/30 rounded-lg">{content}</div>
    );
  }

  return content;
};

/**
 * Inline param display for use in text contexts
 * e.g. "Tested with: kijun=30, tenkan=11"
 */
export const ParamInline = ({ params }: { params: Record<string, unknown> }) => {
  return (
    <>
      {Object.entries(params)
        .map(([key, value]) => `${key}=${formatParamValue(value, key)}`)
        .join(', ')}
    </>
  );
};
