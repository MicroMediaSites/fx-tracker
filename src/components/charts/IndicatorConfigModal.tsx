import { useState, useEffect, useRef } from 'react';
import {
  INDICATOR_DEFAULTS,
  INDICATOR_METADATA,
  type IndicatorType,
} from '../../types/strategy';
import type { ChartIndicatorConfig } from './chartTypes';
import { formatIndicatorLabel } from './indicatorHelpers';
import { INDICATOR_COLORS } from './chartConstants';

interface IndicatorConfigModalProps {
  isOpen: boolean;
  /** Existing indicator to edit (if editing) */
  indicator?: ChartIndicatorConfig;
  /** Indicator type to add (if adding new) */
  indicatorType?: IndicatorType;
  onSave: (type: IndicatorType, params: Record<string, number>, colors?: Record<string, string>) => void;
  onClose: () => void;
}

/** Colorable outputs per indicator type (excluding histograms which auto-color) */
const COLORABLE_OUTPUTS: Record<string, { key: string; label: string }[]> = {
  sma: [{ key: 'value', label: 'Line' }],
  ema: [{ key: 'value', label: 'Line' }],
  rsi: [{ key: 'value', label: 'Line' }],
  mfi: [{ key: 'value', label: 'Line' }],
  atr: [{ key: 'value', label: 'Line' }],
  adr: [{ key: 'value', label: 'Line' }],
  adx: [
    { key: 'value', label: 'ADX' },
    { key: 'plus_di', label: '+DI' },
    { key: 'minus_di', label: '-DI' },
  ],
  bollinger: [
    { key: 'upper', label: 'Upper' },
    { key: 'middle', label: 'Middle' },
    { key: 'lower', label: 'Lower' },
  ],
  ma_bands: [
    { key: 'upper', label: 'Upper' },
    { key: 'middle', label: 'Middle' },
    { key: 'lower', label: 'Lower' },
  ],
  macd: [
    { key: 'macd', label: 'MACD' },
    { key: 'signal', label: 'Signal' },
  ],
  stochastic: [
    { key: 'k', label: '%K' },
    { key: 'd', label: '%D' },
  ],
  dss: [
    { key: 'dss', label: 'DSS' },
    { key: 'signal', label: 'Signal' },
  ],
  chandelier: [
    { key: 'exit_long', label: 'Exit Long' },
    { key: 'exit_short', label: 'Exit Short' },
  ],
  ichimoku: [
    { key: 'tenkan', label: 'Tenkan' },
    { key: 'kijun', label: 'Kijun' },
    { key: 'senkou_a', label: 'Senkou A' },
    { key: 'senkou_b', label: 'Senkou B' },
    { key: 'chikou', label: 'Chikou' },
  ],
  donchian: [
    { key: 'upper', label: 'Upper' },
    { key: 'middle', label: 'Middle' },
    { key: 'lower', label: 'Lower' },
  ],
};

/** Get default color for an indicator output */
const getDefaultColor = (type: string, outputKey: string): string => {
  const colorKey = outputKey === 'value' ? type : `${type}.${outputKey}`;
  return INDICATOR_COLORS[colorKey] || INDICATOR_COLORS.default;
};

/** User-friendly labels for parameter names */
const PARAM_LABELS: Record<string, string> = {
  period: 'Period',
  fast_period: 'Fast Period',
  slow_period: 'Slow Period',
  signal_period: 'Signal Period',
  k_period: '%K Period',
  d_period: '%D Period',
  slowing: 'Slowing',
  std_dev: 'Std Deviations',
  multiplier: 'Multiplier',
  distance: 'Distance (pips)',
  tenkan_period: 'Tenkan Period',
  kijun_period: 'Kijun Period',
  senkou_b_period: 'Senkou B Period',
  displacement: 'Displacement',
  stoch_period: 'Stochastic Period',
  ema_period: 'EMA Period',
  strength: 'Strength',
};

/** Check if param accepts decimals */
const isDecimalParam = (param: string): boolean => {
  return param === 'std_dev' || param === 'multiplier';
};

export const IndicatorConfigModal = ({
  isOpen,
  indicator,
  indicatorType,
  onSave,
  onClose,
}: IndicatorConfigModalProps) => {
  const inputRef = useRef<HTMLInputElement>(null);
  const isEditing = !!indicator;

  // Determine the indicator type
  const type = indicator?.type as IndicatorType ?? indicatorType;
  const meta = type ? INDICATOR_METADATA[type] : null;
  const defaults = type ? INDICATOR_DEFAULTS[type] : {};

  // Form state - params and colors
  const [params, setParams] = useState<Record<string, number>>({});
  const [colors, setColors] = useState<Record<string, string>>({});

  // Get colorable outputs for this indicator type
  const colorableOutputs = type ? COLORABLE_OUTPUTS[type] ?? [] : [];

  // Get default colors for this indicator type
  const getDefaultColors = (): Record<string, string> => {
    if (!type) return {};
    const defaultColors: Record<string, string> = {};
    for (const output of colorableOutputs) {
      defaultColors[output.key] = getDefaultColor(type, output.key);
    }
    return defaultColors;
  };

  // Reset form when modal opens/changes
  useEffect(() => {
    if (isOpen && type) {
      if (indicator) {
        // Editing - use existing params and colors
        setParams({ ...indicator.params });
        setColors(indicator.colors ?? getDefaultColors());
      } else {
        // Adding - use defaults
        setParams({ ...defaults });
        setColors(getDefaultColors());
      }
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen, indicator, type, defaults]);

  // Focus first input when modal opens
  useEffect(() => {
    if (isOpen) {
      setTimeout(() => inputRef.current?.focus(), 100);
    }
  }, [isOpen]);

  // Handle escape key — only register listener when modal is open (Bug #18)
  useEffect(() => {
    if (!isOpen) return;
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onClose();
      }
    };
    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [isOpen, onClose]);

  const handleParamChange = (param: string, value: number) => {
    setParams((prev) => ({ ...prev, [param]: value }));
  };

  const handleSave = () => {
    if (type) {
      onSave(type, params, colors);
    }
  };

  const handleResetToDefaults = () => {
    setParams({ ...defaults });
    setColors(getDefaultColors());
  };

  const handleColorChange = (outputKey: string, color: string) => {
    setColors((prev) => ({ ...prev, [outputKey]: color }));
  };

  if (!isOpen || !type || !meta) return null;

  const paramEntries = Object.entries(defaults);
  const title = isEditing
    ? `Edit ${formatIndicatorLabel(indicator!)}`
    : `Add ${meta.fullName ?? meta.label}`;

  return (
    <div
      className="fixed inset-0 z-[150] flex items-center justify-center"
      onClick={onClose}
    >
      <div className="absolute inset-0 bg-black/60" />
      <div
        className="relative bg-[var(--color-bg-page)] rounded-lg shadow-xl max-w-sm w-full mx-4"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="p-4 border-b border-[var(--color-border)]">
          <h3 className="text-lg font-semibold text-[var(--color-text-primary)]">
            {title}
          </h3>
          <p className="text-xs text-[var(--color-text-muted)] mt-1">
            {meta.fullName ?? meta.label}
          </p>
        </div>

        {/* Content */}
        <div className="p-4 space-y-4">
          {/* Parameters */}
          {paramEntries.length === 0 ? (
            <p className="text-sm text-[var(--color-text-muted)]">
              This indicator has no configurable parameters.
            </p>
          ) : (
            <div className={`grid gap-3 ${
              paramEntries.length === 1 ? 'grid-cols-1' :
              paramEntries.length === 2 ? 'grid-cols-2' :
              paramEntries.length === 3 ? 'grid-cols-3' :
              paramEntries.length === 4 ? 'grid-cols-2' :
              paramEntries.length % 2 === 0 ? 'grid-cols-2' : 'grid-cols-3'
            }`}>
              {paramEntries.map(([param], index) => {
                const isDecimal = isDecimalParam(param);
                return (
                  <div key={param}>
                    <label className="block text-xs text-[var(--color-text-muted)] mb-1">
                      {PARAM_LABELS[param] ?? param}
                    </label>
                    <input
                      ref={index === 0 ? inputRef : undefined}
                      type="text"
                      inputMode={isDecimal ? 'decimal' : 'numeric'}
                      value={params[param] ?? defaults[param]}
                      onChange={(e) => {
                        const val = isDecimal
                          ? parseFloat(e.target.value)
                          : parseInt(e.target.value);
                        if (!isNaN(val)) {
                          handleParamChange(param, val);
                        }
                      }}
                      onFocus={(e) => setTimeout(() => e.target.select(), 0)}
                      className="w-full bg-transparent border border-[var(--color-border)] rounded px-3 py-2 text-sm text-center focus:outline-none focus:border-[var(--color-border-focus)] text-[var(--color-text-primary)]"
                    />
                  </div>
                );
              })}
            </div>
          )}

          {/* Colors */}
          {colorableOutputs.length > 0 && (
            <div className="border-t border-[var(--color-border)] pt-4">
              <label className="block text-xs text-[var(--color-text-muted)] mb-2">
                Colors
              </label>
              <div className={`grid gap-3 ${
                colorableOutputs.length === 1 ? 'grid-cols-1' :
                colorableOutputs.length === 2 ? 'grid-cols-2' :
                colorableOutputs.length === 3 ? 'grid-cols-3' :
                colorableOutputs.length === 4 ? 'grid-cols-4' :
                'grid-cols-5'
              }`}>
                {colorableOutputs.map((output) => (
                  <div key={output.key} className="flex flex-col items-center gap-1">
                    <input
                      type="color"
                      value={colors[output.key] || getDefaultColor(type!, output.key)}
                      onChange={(e) => handleColorChange(output.key, e.target.value)}
                      className="w-8 h-8 rounded cursor-pointer border border-[var(--color-border)] bg-transparent"
                    />
                    <span className="text-xs text-[var(--color-text-muted)]">
                      {output.label}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="p-4 border-t border-[var(--color-border)] flex justify-between">
          <button
            onClick={handleResetToDefaults}
            className="text-sm text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors"
          >
            Use Defaults
          </button>
          <div className="flex gap-3">
            <button
              onClick={onClose}
              className="px-4 py-2 border border-[var(--color-border)] rounded hover:border-[var(--color-text-muted)] text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors"
            >
              Cancel
            </button>
            <button
              onClick={handleSave}
              className="px-4 py-2 bg-[var(--color-info)]/20 border border-[var(--color-info)] rounded text-[var(--color-info-text)] hover:bg-[var(--color-info)]/30 transition-colors"
            >
              {isEditing ? 'Save' : 'Add'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
