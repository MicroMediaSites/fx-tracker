import { Strategy, ParameterDefinition } from '../../types/strategy';
import { DynamicParameter } from './types';

/**
 * Find all $param references in a strategy that need to be resolved before promotion
 */
export const findDynamicParameters = (strategy: Strategy): DynamicParameter[] => {
  const params: DynamicParameter[] = [];
  const seen = new Set<string>();

  const checkValue = (value: unknown, label: string) => {
    if (typeof value === 'object' && value !== null && '$param' in value) {
      const paramRef = value as { $param: string };
      if (!seen.has(paramRef.$param)) {
        seen.add(paramRef.$param);
        // Look up default from strategy.parameters if available
        const paramDef = strategy.parameters?.find(p => p.id === paramRef.$param);
        params.push({
          id: paramRef.$param,
          name: paramDef?.name || label,
          default: paramDef?.default ?? 0,
          type: paramDef?.type ?? 'number',
        });
      }
    }
  };

  // Check risk settings
  if (strategy.risk_settings) {
    checkValue(strategy.risk_settings.risk_value, 'Risk Value');
    checkValue(strategy.risk_settings.rr_ratio, 'R:R Ratio');
    checkValue(strategy.risk_settings.spread_buffer_pips, 'Spread Buffer');
  }

  // Deep scan for $param in any object structure
  const scanForParams = (obj: unknown, path: string) => {
    if (typeof obj === 'object' && obj !== null) {
      if ('$param' in obj) {
        checkValue(obj, path);
      } else if (Array.isArray(obj)) {
        obj.forEach((item, i) => scanForParams(item, `${path}[${i}]`));
      } else {
        Object.entries(obj).forEach(([key, val]) => scanForParams(val, `${path}.${key}`));
      }
    }
  };

  // Check indicators (params can be parameterized, e.g., RSI period)
  if (strategy.indicators) {
    strategy.indicators.forEach((ind, i) => {
      scanForParams(ind.params, `indicators[${i}].params`);
    });
  }

  // Check entry/exit rules
  scanForParams(strategy.entry_rules, 'entry_rules');
  scanForParams(strategy.exit_rules, 'exit_rules');

  return params;
};

/**
 * Replace all $param references with fixed values in an object
 */
export const resolveParams = <T,>(obj: T, paramValues: Record<string, number>): T => {
  if (typeof obj === 'object' && obj !== null) {
    if ('$param' in obj) {
      const paramRef = obj as { $param: string };
      return paramValues[paramRef.$param] as T;
    }
    if (Array.isArray(obj)) {
      return obj.map(item => resolveParams(item, paramValues)) as T;
    }
    const result: Record<string, unknown> = {};
    for (const [key, val] of Object.entries(obj)) {
      result[key] = resolveParams(val, paramValues);
    }
    return result as T;
  }
  return obj;
};

/**
 * Get IDs of all parameters that are actually referenced via $param in the strategy.
 * This includes references in indicators, entry/exit rules, and risk settings.
 */
export const getReferencedParameterIds = (strategy: Strategy): Set<string> => {
  const dynamicParams = findDynamicParameters(strategy);
  return new Set(dynamicParams.map(p => p.id));
};

/**
 * Find parameters that are defined in strategy.parameters but not referenced anywhere.
 * These are "orphaned" parameters that won't affect backtesting or optimization.
 */
export const findOrphanedParameters = (strategy: Strategy): ParameterDefinition[] => {
  const referencedIds = getReferencedParameterIds(strategy);
  return (strategy.parameters || []).filter(p => !referencedIds.has(p.id));
};

/**
 * Filter parameters to only include those that are actually referenced in the strategy.
 * Used by the optimizer to avoid wasting cycles on orphaned parameters.
 *
 * For scripted strategies, all declared parameters are valid (they're referenced
 * via param() calls in the script, not via $param in JSON fields).
 */
export const filterReferencedParameters = (strategy: Strategy): ParameterDefinition[] => {
  if (strategy.strategy_type === 'scripted') {
    return strategy.parameters || [];
  }
  const referencedIds = getReferencedParameterIds(strategy);
  return (strategy.parameters || []).filter(p => referencedIds.has(p.id));
};
