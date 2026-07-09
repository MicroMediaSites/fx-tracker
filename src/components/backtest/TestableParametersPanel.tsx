/**
 * TestableParametersPanel - Shows saved vs testing parameter values for backtesting.
 *
 * Used in:
 * - SimpleHistoricalFlow (mode='single') - Shows single testing values
 * - WalkForwardFlow (mode='range') - Shows min/max/step range inputs
 *
 * Uses accordion-style layout for compact display with expandable editing.
 *
 * Features:
 * - "Use Default" checkbox: Skip this param in optimization (use saved default)
 * - "Resolve" button: Permanently replace $param references with a value
 */
import { useState, useMemo, useCallback } from 'react';
import { ParameterDefinition } from '../../types/strategy';
import { formatParamValue } from '../../utils/formatters';

/**
 * A controlled text input that allows decimal typing without losing the "." character.
 * Stores raw string locally while editing, commits parsed number on blur.
 */
const DecimalInput = ({
  value,
  onChange,
  fallback = 0,
  ...props
}: {
  value: number;
  onChange: (v: number) => void;
  fallback?: number;
} & Omit<React.InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange' | 'type'>) => {
  const [localValue, setLocalValue] = useState<string | null>(null);

  const handleChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    setLocalValue(e.target.value);
    const parsed = parseFloat(e.target.value);
    if (!isNaN(parsed)) {
      onChange(parsed);
    }
  }, [onChange]);

  const handleBlur = useCallback(() => {
    // Commit: if the local string doesn't parse, fall back
    if (localValue !== null) {
      const parsed = parseFloat(localValue);
      if (isNaN(parsed)) {
        onChange(fallback);
      }
      setLocalValue(null);
    }
  }, [localValue, onChange, fallback]);

  const handleFocus = useCallback((e: React.FocusEvent<HTMLInputElement>) => {
    setLocalValue(String(value));
    setTimeout(() => e.target.select(), 0);
  }, [value]);

  return (
    <input
      type="text"
      inputMode="decimal"
      value={localValue !== null ? localValue : value}
      onChange={handleChange}
      onBlur={handleBlur}
      onFocus={handleFocus}
      {...props}
    />
  );
};

// Single value testing (Simple Historical)
export interface SingleTestingParams {
  [paramId: string]: number;
}

// Range testing (Walk-Forward)
export interface RangeTestingParams {
  [paramId: string]: {
    min: number;
    max: number;
    step: number;
  };
}

// Track which params use default (skip optimization)
export interface UseDefaultParams {
  [paramId: string]: boolean;
}

interface TestableParametersPanelProps {
  /** Strategy parameters with defaults */
  parameters: ParameterDefinition[];
  /** Testing mode: 'single' for Simple Historical, 'range' for Walk-Forward */
  mode: 'single' | 'range';
  /** Current testing values (single mode) */
  singleValues?: SingleTestingParams;
  /** Current testing ranges (range mode) */
  rangeValues?: RangeTestingParams;
  /** Which params are set to "use default" (skip optimization) */
  useDefaultParams?: UseDefaultParams;
  /** Callback when single value changes */
  onSingleChange?: (paramId: string, value: number) => void;
  /** Callback when range changes */
  onRangeChange?: (paramId: string, field: 'min' | 'max' | 'step', value: number) => void;
  /** Callback when "use default" toggle changes */
  onUseDefaultChange?: (paramId: string, useDefault: boolean) => void;
  /** Callback to resolve a parameter (replace $param refs with value, remove param) */
  onResolveParameter?: (paramId: string, value: number) => void;
  /** Callback to reset all values to saved defaults */
  onReset?: () => void;
  /** Callback to update testing config (saves ranges/defaults to strategy) */
  onSaveToStrategy?: () => void;
  /** Whether changes have been made */
  hasChanges?: boolean;
}

export const TestableParametersPanel = ({
  parameters,
  mode,
  singleValues = {},
  rangeValues = {},
  useDefaultParams = {},
  onSingleChange,
  onRangeChange,
  onUseDefaultChange,
  onResolveParameter,
  onReset,
  onSaveToStrategy,
  hasChanges = false,
}: TestableParametersPanelProps) => {
  // Track which parameters are expanded
  const [expandedParams, setExpandedParams] = useState<Set<string>>(new Set());
  // Track resolve value inputs per param
  const [resolveValues, setResolveValues] = useState<{ [paramId: string]: number }>({});
  // Track which params are showing resolve confirmation
  const [confirmingResolve, setConfirmingResolve] = useState<string | null>(null);

  const toggleParam = (paramId: string) => {
    setExpandedParams(prev => {
      const next = new Set(prev);
      if (next.has(paramId)) {
        next.delete(paramId);
      } else {
        next.add(paramId);
      }
      return next;
    });
  };

  const expandAll = () => {
    setExpandedParams(new Set(parameters.map(p => p.id)));
  };

  const collapseAll = () => {
    setExpandedParams(new Set());
  };

  if (parameters.length === 0) {
    return null;
  }

  // Calculate grid size for range mode (excluding "use default" params)
  const gridSize = useMemo(() => {
    if (mode !== 'range') return null;

    let total = 1;
    for (const param of parameters) {
      // Skip params that are set to "use default"
      if (useDefaultParams[param.id]) continue;

      const range = rangeValues[param.id];
      if (param.type === 'boolean') {
        // Booleans: 2 if testing both, 1 otherwise
        const min = range?.min ?? param.default;
        const max = range?.max ?? param.default;
        total *= (min === 0 && max === 1) ? 2 : 1;
      } else if (range && range.step > 0) {
        const count = Math.floor((range.max - range.min) / range.step) + 1;
        total *= count;
      }
    }
    return total;
  }, [mode, parameters, rangeValues, useDefaultParams]);

  // Count how many params are being optimized vs using default
  const optimizedCount = parameters.filter(p => !useDefaultParams[p.id]).length;
  const usingDefaultCount = parameters.length - optimizedCount;

  // Helper to format boolean value for display
  const formatBoolValue = (value: number) => value ? 'on' : 'off';

  // Helper to get range summary text
  const getRangeSummary = (param: ParameterDefinition) => {
    if (useDefaultParams[param.id]) {
      return 'default';
    }
    const range = rangeValues[param.id];
    if (param.type === 'boolean') {
      const min = range?.min ?? param.default;
      const max = range?.max ?? param.default;
      if (min === 0 && max === 1) {
        return 'both';
      }
      return formatBoolValue(param.default);
    }
    const min = range?.min ?? param.min ?? Math.floor(param.default * 0.5);
    const max = range?.max ?? param.max ?? Math.ceil(param.default * 1.5);
    return `${formatParamValue(min)}–${formatParamValue(max)}`;
  };

  // Helper to check if param is being optimized (not using default)
  const isParamModified = (param: ParameterDefinition) => {
    if (mode === 'single') {
      return singleValues[param.id] !== undefined && singleValues[param.id] !== param.default;
    } else {
      // In range mode, show dot when param is set to optimize (not using default)
      return !useDefaultParams[param.id];
    }
  };

  // Get the resolve value for a param (defaults to current testing value or saved default)
  const getResolveValue = (param: ParameterDefinition) => {
    if (resolveValues[param.id] !== undefined) {
      return resolveValues[param.id];
    }
    if (mode === 'single') {
      return singleValues[param.id] ?? param.default;
    }
    return param.default;
  };

  const handleResolve = (param: ParameterDefinition) => {
    if (confirmingResolve === param.id) {
      // Actually resolve
      onResolveParameter?.(param.id, getResolveValue(param));
      setConfirmingResolve(null);
    } else {
      // Show confirmation
      setConfirmingResolve(param.id);
    }
  };

  return (
    <div className="pt-4 border-t border-[var(--color-border)]">
      {/* Header */}
      <div className="flex items-center justify-between mb-3">
        <h4 className="text-sm font-medium text-[var(--color-text-secondary)]">Parameters</h4>
        <div className="flex items-center gap-3">
          {parameters.length > 1 && (
            <button
              onClick={expandedParams.size === parameters.length ? collapseAll : expandAll}
              className="text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
            >
              {expandedParams.size === parameters.length ? 'Collapse' : 'Expand'}
            </button>
          )}
          {hasChanges && onReset && (
            <button
              onClick={onReset}
              className="text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
            >
              Reset
            </button>
          )}
        </div>
      </div>

      {/* Parameter List */}
      <div className="space-y-0.5">
        {parameters.map((param) => {
          const isExpanded = expandedParams.has(param.id);
          const isModified = isParamModified(param);
          const isUsingDefault = useDefaultParams[param.id] || false;
          const isConfirmingResolve = confirmingResolve === param.id;

          return (
            <div
              key={param.id}
              className={`rounded transition-colors ${isExpanded ? 'bg-[var(--color-bg-hover)]/40' : ''}`}
            >
              {/* Collapsed Header - always visible */}
              <button
                onClick={() => toggleParam(param.id)}
                className="w-full px-2 py-2 flex items-center justify-between text-left hover:bg-[var(--color-bg-hover)]/30 rounded transition-colors"
              >
                <div className="flex items-center gap-2 min-w-0 flex-1">
                  <svg
                    className={`w-3 h-3 text-[var(--color-text-muted)] transition-transform flex-shrink-0 ${
                      isExpanded ? 'rotate-90' : ''
                    }`}
                    fill="none"
                    stroke="currentColor"
                    viewBox="0 0 24 24"
                  >
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                  </svg>
                  <span className={`text-sm truncate ${isUsingDefault ? 'text-[var(--color-text-muted)]' : 'text-[var(--color-text-primary)]'}`} title={param.name}>
                    {param.name}
                  </span>
                  {param.type === 'boolean' && (
                    <span className="text-[10px] px-1.5 py-0.5 rounded bg-[var(--color-bg-hover)] text-[var(--color-text-muted)] flex-shrink-0">bool</span>
                  )}
                  {isModified && (
                    <span className="w-1.5 h-1.5 rounded-full bg-[var(--color-info)] flex-shrink-0" title="Modified" />
                  )}
                  {isUsingDefault && (
                    <span className="text-xs text-[var(--color-text-muted)] flex-shrink-0">(skip)</span>
                  )}
                </div>
                <div className="flex items-center gap-2 text-xs font-mono flex-shrink-0 ml-2">
                  <span className="text-[var(--color-text-muted)]">
                    {param.type === 'boolean' ? formatBoolValue(param.default) : formatParamValue(param.default)}
                  </span>
                  <span className="text-[var(--color-text-muted)]">→</span>
                  {mode === 'range' ? (
                    <span className={isUsingDefault ? 'text-[var(--color-text-muted)] italic' : 'text-[var(--color-text-primary)]'}>
                      {getRangeSummary(param)}
                    </span>
                  ) : (
                    <span className={isModified ? 'text-[var(--color-info)]' : 'text-[var(--color-text-primary)]'}>
                      {param.type === 'boolean'
                        ? formatBoolValue(singleValues[param.id] ?? param.default)
                        : formatParamValue(singleValues[param.id] ?? param.default)}
                    </span>
                  )}
                </div>
              </button>

              {/* Expanded Content */}
              {isExpanded && (
                <div className="px-2 pb-3 pt-1 ml-5 space-y-3">
                  {/* Boolean parameters - completely different UI */}
                  {param.type === 'boolean' ? (
                    mode === 'single' ? (
                      /* Single mode: toggle switch */
                      <label className="flex items-center gap-3 cursor-pointer">
                        <div
                          onClick={() => onSingleChange?.(param.id, (singleValues[param.id] ?? param.default) ? 0 : 1)}
                          className={`relative w-10 h-5 rounded-full transition-colors cursor-pointer ${
                            (singleValues[param.id] ?? param.default) ? 'bg-[var(--color-info)]' : 'bg-[var(--color-bg-hover)] border border-[var(--color-border)]'
                          }`}
                        >
                          <div
                            className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${
                              (singleValues[param.id] ?? param.default) ? 'translate-x-5' : 'translate-x-0.5'
                            }`}
                          />
                        </div>
                        <span className="text-sm text-[var(--color-text-primary)]">
                          {(singleValues[param.id] ?? param.default) ? 'Enabled' : 'Disabled'}
                        </span>
                      </label>
                    ) : (
                      /* Range mode: segmented control for boolean optimization */
                      <div className="space-y-2">
                        <div className="text-xs text-[var(--color-text-muted)]">Optimization mode</div>
                        <div className="inline-flex rounded border border-[var(--color-border)] overflow-hidden">
                          <button
                            type="button"
                            onClick={() => {
                              onUseDefaultChange?.(param.id, true);
                            }}
                            className={`px-3 py-1.5 text-xs transition-colors ${
                              isUsingDefault
                                ? 'bg-[var(--color-info)] text-white'
                                : 'text-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)]'
                            }`}
                          >
                            Use default ({formatBoolValue(param.default)})
                          </button>
                          <button
                            type="button"
                            onClick={() => {
                              onUseDefaultChange?.(param.id, false);
                              onRangeChange?.(param.id, 'min', 0);
                              onRangeChange?.(param.id, 'max', 1);
                              onRangeChange?.(param.id, 'step', 1);
                            }}
                            className={`px-3 py-1.5 text-xs transition-colors border-l border-[var(--color-border)] ${
                              !isUsingDefault
                                ? 'bg-[var(--color-info)] text-white'
                                : 'text-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)]'
                            }`}
                          >
                            Test both
                          </button>
                        </div>
                        {!isUsingDefault && (
                          <div className="text-xs text-[var(--color-text-muted)]">
                            Will test with enabled and disabled (2 variants)
                          </div>
                        )}
                      </div>
                    )
                  ) : (
                    /* Numeric parameters */
                    <>
                      {/* Mode toggle - only for numeric params in range mode */}
                      {mode === 'range' && onUseDefaultChange && (
                        <div className="space-y-2">
                          <div className="inline-flex rounded border border-[var(--color-border)] overflow-hidden">
                            <button
                              type="button"
                              onClick={() => onUseDefaultChange(param.id, true)}
                              className={`px-3 py-1.5 text-xs transition-colors ${
                                isUsingDefault
                                  ? 'bg-[var(--color-info)] text-white'
                                  : 'text-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)]'
                              }`}
                            >
                              Use default ({formatParamValue(param.default)})
                            </button>
                            <button
                              type="button"
                              onClick={() => onUseDefaultChange(param.id, false)}
                              className={`px-3 py-1.5 text-xs transition-colors border-l border-[var(--color-border)] ${
                                !isUsingDefault
                                  ? 'bg-[var(--color-info)] text-white'
                                  : 'text-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)]'
                              }`}
                            >
                              Test range
                            </button>
                          </div>
                        </div>
                      )}

                      {mode === 'single' ? (
                        /* Single value input */
                        <div className="flex flex-col items-center max-w-[120px]">
                          <DecimalInput
                            value={singleValues[param.id] ?? param.default}
                            onChange={(v) => onSingleChange?.(param.id, v)}
                            className="w-full bg-transparent border border-[var(--color-border)] rounded px-3 py-2 text-sm text-center text-[var(--color-text-primary)] font-mono focus:outline-none focus:border-[var(--color-border-focus)]"
                          />
                          <span className="text-[11px] text-[var(--color-text-muted)] mt-1">test value</span>
                        </div>
                      ) : (
                        /* Range inputs - FX Ticket style */
                        <div className={`grid grid-cols-3 gap-3 ${isUsingDefault ? 'opacity-40 pointer-events-none' : ''}`}>
                          <div className="flex flex-col items-center">
                            <DecimalInput
                              value={rangeValues[param.id]?.min ?? param.min ?? Math.floor(param.default * 0.5)}
                              onChange={(v) => onRangeChange?.(param.id, 'min', v)}
                              disabled={isUsingDefault}
                              className="w-full bg-transparent border border-[var(--color-border)] rounded px-3 py-2 text-sm text-center text-[var(--color-text-primary)] font-mono focus:outline-none focus:border-[var(--color-border-focus)] disabled:opacity-50 disabled:cursor-not-allowed"
                            />
                            <span className="text-[11px] text-[var(--color-text-muted)] mt-1">min</span>
                          </div>
                          <div className="flex flex-col items-center">
                            <DecimalInput
                              value={rangeValues[param.id]?.max ?? param.max ?? Math.ceil(param.default * 1.5)}
                              onChange={(v) => onRangeChange?.(param.id, 'max', v)}
                              disabled={isUsingDefault}
                              className="w-full bg-transparent border border-[var(--color-border)] rounded px-3 py-2 text-sm text-center text-[var(--color-text-primary)] font-mono focus:outline-none focus:border-[var(--color-border-focus)] disabled:opacity-50 disabled:cursor-not-allowed"
                            />
                            <span className="text-[11px] text-[var(--color-text-muted)] mt-1">max</span>
                          </div>
                          <div className="flex flex-col items-center">
                            <DecimalInput
                              value={rangeValues[param.id]?.step ?? param.step ?? Math.max(1, Math.floor(param.default * 0.1))}
                              onChange={(v) => onRangeChange?.(param.id, 'step', v)}
                              fallback={1}
                              disabled={isUsingDefault}
                              className="w-full bg-transparent border border-[var(--color-border)] rounded px-3 py-2 text-sm text-center text-[var(--color-text-primary)] font-mono focus:outline-none focus:border-[var(--color-border-focus)] disabled:opacity-50 disabled:cursor-not-allowed"
                            />
                            <span className="text-[11px] text-[var(--color-text-muted)] mt-1">step</span>
                          </div>
                        </div>
                      )}
                    </>
                  )}

                  {/* Resolve Parameter section - for all param types */}
                  {onResolveParameter && (
                    <div className="pt-2 mt-2 border-t border-[var(--color-border)]/30">
                      {isConfirmingResolve ? (
                        <div className="space-y-2">
                          <p className="text-xs text-[var(--color-warning)]">
                            Replace all ${param.id} refs with this value (removes parameter):
                          </p>
                          <div className="flex items-center gap-2">
                            {param.type === 'boolean' ? (
                              /* Boolean toggle for resolve value */
                              <div className="flex items-center gap-2">
                                <div
                                  onClick={() => setResolveValues(prev => ({
                                    ...prev,
                                    [param.id]: getResolveValue(param) ? 0 : 1
                                  }))}
                                  className={`relative w-10 h-5 rounded-full transition-colors cursor-pointer ${
                                    getResolveValue(param) ? 'bg-[var(--color-info)]' : 'bg-[var(--color-bg-hover)] border border-[var(--color-border)]'
                                  }`}
                                >
                                  <div
                                    className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${
                                      getResolveValue(param) ? 'translate-x-5' : 'translate-x-0.5'
                                    }`}
                                  />
                                </div>
                                <span className="text-sm text-[var(--color-text-primary)] font-mono">
                                  {getResolveValue(param) ? 'true' : 'false'}
                                </span>
                              </div>
                            ) : (
                              /* Number input for numeric params */
                              <input
                                type="number"
                                value={getResolveValue(param)}
                                onChange={(e) => setResolveValues(prev => ({
                                  ...prev,
                                  [param.id]: parseFloat(e.target.value) || 0
                                }))}
                                step={param.step || (param.type === 'integer' ? 1 : 'any')}
                                className="flex-1 bg-[var(--color-bg-hover)] border border-[var(--color-border)] rounded px-2 py-1 text-sm text-[var(--color-text-primary)] font-mono focus:outline-none focus:border-[var(--color-text-muted)]"
                              />
                            )}
                            <button
                              onClick={() => handleResolve(param)}
                              className="px-2 py-1 text-xs text-[var(--color-sell)] hover:text-[var(--color-sell)]/80 transition-colors"
                            >
                              Resolve
                            </button>
                            <button
                              onClick={() => setConfirmingResolve(null)}
                              className="px-2 py-1 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
                            >
                              Cancel
                            </button>
                          </div>
                        </div>
                      ) : (
                        <button
                          onClick={() => handleResolve(param)}
                          className="text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
                        >
                          Set live value...
                        </button>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>

      {/* Footer info */}
      {(mode === 'range' && gridSize !== null) || (hasChanges && onSaveToStrategy) ? (
        <div className="mt-3 pt-3 border-t border-[var(--color-border)] flex items-center justify-between">
          {mode === 'range' && gridSize !== null && (
            <div className="text-xs text-[var(--color-text-muted)]">
              <span className={gridSize > 10000 ? 'text-[var(--color-warning)]' : 'text-[var(--color-text-primary)]'}>
                {gridSize.toLocaleString()}
              </span>
              {' '}combinations
              {usingDefaultCount > 0 && (
                <span className="ml-1">
                  ({usingDefaultCount} skipped)
                </span>
              )}
            </div>
          )}
          {hasChanges && onSaveToStrategy && (
            <button
              onClick={onSaveToStrategy}
              className="text-xs text-[var(--color-info)] hover:text-[var(--color-info)]/80 transition-colors"
            >
              Save to strategy
            </button>
          )}
        </div>
      ) : null}
    </div>
  );
};
