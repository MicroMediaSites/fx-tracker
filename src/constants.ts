/**
 * Shared constants used across the application.
 */

/**
 * Candle granularity/timeframe options.
 * Used for chart timeframes, backtest configuration, and strategy watchers.
 */
export const GRANULARITIES = [
  { value: 'M1', label: 'M1' },
  { value: 'M5', label: 'M5' },
  { value: 'M15', label: 'M15' },
  { value: 'M30', label: 'M30' },
  { value: 'H1', label: 'H1' },
  { value: 'H4', label: 'H4' },
  { value: 'H8', label: 'H8' },
  { value: 'D', label: 'D1' },
  { value: 'W', label: 'W1' },
] as const;

export type Granularity = (typeof GRANULARITIES)[number]['value'];

/**
 * Default granularity for new charts/strategies.
 */
export const DEFAULT_GRANULARITY: Granularity = 'H1';

/**
 * AI disclaimer text shown at the end of performance metrics.
 */
export const AI_DISCLAIMER_TEXT = 'This analysis is for informational purposes only. Past performance does not guarantee future results.';

/**
 * Patterns that indicate AI compliance language (declining to give trading advice).
 * Any sentence containing one of these patterns is highlighted in amber.
 * The AI is instructed to include these phrases when asked for advice.
 */
export const AI_COMPLIANCE_PATTERNS = [
  'cannot advise',
  'cannot recommend',
  "can't advise",
  "can't recommend",
  'the decision is yours',
  'the choice is yours',
  'not personalized trading advice',
  'cannot tell you what to',
  "can't tell you what to",
] as const;
