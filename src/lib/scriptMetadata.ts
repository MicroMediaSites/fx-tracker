/**
 * Extract @indicators JSON from a Rhai script's metadata comments.
 *
 * Format (mirrors Rust's extract_metadata_json in scripted_strategy.rs):
 * ```
 * // @indicators: [{ "id": "ema_fast", "type": "ema", "params": { "period": 20 } }]
 * ```
 *
 * JSON may span multiple // comment lines.
 * Returns the raw JSON string, or null if no @indicators found.
 */
export function extractScriptIndicatorsJson(script: string): string | null {
  const tag = '@indicators:';
  const lines = script.split('\n');
  let json = '';
  let collecting = false;

  for (const line of lines) {
    const trimmed = line.trim();

    if (!collecting) {
      if (trimmed.startsWith('//')) {
        const content = trimmed.slice(2).trim();
        if (content.startsWith(tag)) {
          json = content.slice(tag.length).trim();
          collecting = true;
          if (isBalanced(json)) return json;
        }
      }
    } else {
      if (trimmed.startsWith('//')) {
        json += trimmed.slice(2).trim();
        if (isBalanced(json)) return json;
      } else {
        break;
      }
    }
  }

  return collecting && json.length > 0 ? json : null;
}

/** Rough bracket-balance check for JSON arrays/objects */
function isBalanced(s: string): boolean {
  const t = s.trim();
  if (!t.startsWith('[') && !t.startsWith('{')) return false;
  let depth = 0;
  let inString = false;
  let escape = false;
  for (const ch of t) {
    if (escape) { escape = false; continue; }
    if (ch === '\\') { escape = true; continue; }
    if (ch === '"') { inString = !inString; continue; }
    if (inString) continue;
    if (ch === '[' || ch === '{') depth++;
    if (ch === ']' || ch === '}') depth--;
  }
  return depth === 0;
}

/**
 * Build the indicator seed string for passing to open_chart_window.
 * Bundles indicators JSON and parsed strategy parameters into an envelope.
 * Returns the raw indicators string as fallback if envelope creation fails.
 */
export function buildIndicatorSeed(
  indicatorsJson: string,
  rawParams: string | unknown[] | null | undefined,
): string {
  // Parse params safely (handles JSON string or already-parsed array)
  let params: unknown[] | undefined;
  if (Array.isArray(rawParams)) {
    params = rawParams;
  } else if (typeof rawParams === 'string' && rawParams) {
    try { params = JSON.parse(rawParams); } catch { /* ignore */ }
    if (!Array.isArray(params)) params = undefined;
  }

  try {
    const indicators = JSON.parse(indicatorsJson);
    return JSON.stringify({ indicators, parameters: params });
  } catch {
    return indicatorsJson;
  }
}
