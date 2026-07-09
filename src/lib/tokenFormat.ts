/**
 * Format token count for display (e.g., 1,234,567 or 1.2M).
 * (Relocated from the retired aiQuotaApi module — AGT-650.)
 */
export function formatTokenCount(tokens: number): string {
  if (tokens >= 1_000_000) {
    return `${(tokens / 1_000_000).toFixed(1)}M`;
  }
  return tokens.toLocaleString();
}
