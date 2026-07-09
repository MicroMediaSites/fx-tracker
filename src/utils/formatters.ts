/**
 * Shared formatting utilities
 */

/**
 * Format parameter values to avoid floating point display issues
 * like 0.000150000000000001 showing instead of 0.00015
 */
export const formatParamValue = (value: unknown, paramName?: string): string => {
  // Handle actual booleans
  if (typeof value === 'boolean') {
    return value ? 'true' : 'false';
  }

  if (typeof value !== 'number') return String(value);

  // Detect boolean params stored as 0/1 (common pattern: enable_*, is_*, use_*, has_*)
  const isBooleanParam = paramName && /^(enable_|is_|use_|has_|allow_|show_|hide_)/.test(paramName);
  if (isBooleanParam && (value === 0 || value === 1)) {
    return value === 1 ? 'true' : 'false';
  }

  // Integer or very close to integer - show without decimals
  if (Number.isInteger(value) || Math.abs(value - Math.round(value)) < 1e-10) {
    return Math.round(value).toString();
  }

  // Small decimals (< 1) - use toPrecision to avoid floating point noise
  if (Math.abs(value) < 1) {
    const str = value.toPrecision(6);
    // Remove trailing zeros after decimal
    return parseFloat(str).toString();
  }

  // Larger numbers - use toFixed with reasonable precision
  return parseFloat(value.toFixed(6)).toString();
};

/**
 * Format a record of parameters as an inline string
 * e.g. "kijun=30, tenkan=11, kijun_slope_threshold=0.00015"
 */
export const formatParamsInline = (params: Record<string, unknown>): string => {
  return Object.entries(params)
    .map(([key, value]) => `${key}=${formatParamValue(value, key)}`)
    .join(', ');
};
