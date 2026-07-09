/**
 * TestZonesPanel - Configure custom S/R zones for backtesting.
 *
 * Chart zones are intentionally NOT used in backtests to avoid look-ahead bias.
 * This panel allows users to define hypothetical zones for "what if" testing.
 */
import { useState } from 'react';
import {
  Strategy,
  isGivensTrigger,
  TriggerV2,
  Condition,
} from '../../types/strategy';

export interface TestZone {
  id: string;
  upper_price: number;
  lower_price: number;
}

interface TestZonesPanelProps {
  zones: TestZone[];
  onChange: (zones: TestZone[]) => void;
  /** Whether to show warning (strategy uses sr_tested but no zones configured) */
  showWarning?: boolean;
}

/**
 * Check if a strategy uses the 'sr_tested' (Custom Zone) regime.
 * This traverses all entry and exit rules to find any givens trigger with sr_tested.
 */
export const strategyUsesCustomZones = (strategy: Strategy): boolean => {
  const checkTrigger = (trigger: TriggerV2): boolean => {
    return isGivensTrigger(trigger) && trigger.regime === 'sr_tested';
  };

  const checkCondition = (condition: Condition): boolean => {
    // Check primary trigger
    if (checkTrigger(condition.primary.trigger)) {
      return true;
    }
    // Check chain triggers
    return condition.chain.some((chained) => checkTrigger(chained.trigger.trigger));
  };

  // Check entry rules
  for (const rule of strategy.entry_rules || []) {
    for (const condition of rule.conditions || []) {
      if (checkCondition(condition)) {
        return true;
      }
    }
  }

  // Check exit rules
  for (const rule of strategy.exit_rules || []) {
    for (const condition of rule.conditions || []) {
      if (checkCondition(condition)) {
        return true;
      }
    }
  }

  return false;
};

/**
 * Generate a unique ID for a new zone
 */
const generateZoneId = (): string => {
  return `test-zone-${Date.now()}-${Math.random().toString(36).substr(2, 9)}`;
};

export const TestZonesPanel = ({
  zones,
  onChange,
  showWarning = false,
}: TestZonesPanelProps) => {
  const [isExpanded, setIsExpanded] = useState(zones.length > 0);

  const addZone = () => {
    const newZone: TestZone = {
      id: generateZoneId(),
      upper_price: 0,
      lower_price: 0,
    };
    onChange([...zones, newZone]);
    setIsExpanded(true);
  };

  const updateZone = (id: string, field: 'upper_price' | 'lower_price', value: number) => {
    onChange(
      zones.map((z) =>
        z.id === id ? { ...z, [field]: value } : z
      )
    );
  };

  const removeZone = (id: string) => {
    onChange(zones.filter((z) => z.id !== id));
  };

  return (
    <div className="border border-[var(--color-border)] rounded bg-[var(--color-bg-secondary)]">
      {/* Header */}
      <button
        onClick={() => setIsExpanded(!isExpanded)}
        className="w-full flex items-center justify-between px-3 py-2 hover:bg-[var(--color-bg-hover)]/30 transition-colors"
      >
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[var(--color-text-primary)]">
            Test Zones
          </span>
          {zones.length > 0 && (
            <span className="text-xs text-[var(--color-text-muted)]">
              ({zones.length})
            </span>
          )}
          {showWarning && zones.length === 0 && (
            <span className="text-[var(--color-warning-text)]" title="Strategy uses custom zones">
              <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 20 20">
                <path
                  fillRule="evenodd"
                  d="M8.257 3.099c.765-1.36 2.722-1.36 3.486 0l5.58 9.92c.75 1.334-.213 2.98-1.742 2.98H4.42c-1.53 0-2.493-1.646-1.743-2.98l5.58-9.92zM11 13a1 1 0 11-2 0 1 1 0 012 0zm-1-8a1 1 0 00-1 1v3a1 1 0 002 0V6a1 1 0 00-1-1z"
                  clipRule="evenodd"
                />
              </svg>
            </span>
          )}
        </div>
        <svg
          className={`w-4 h-4 text-[var(--color-text-muted)] transition-transform ${
            isExpanded ? 'rotate-180' : ''
          }`}
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>

      {/* Content */}
      {isExpanded && (
        <div className="px-3 pb-3 space-y-3">
          {/* Warning message */}
          {showWarning && zones.length === 0 && (
            <div className="p-2 bg-[var(--color-warning-bg)] border border-[var(--color-warning-border)] rounded text-xs text-[var(--color-warning-text)]">
              This strategy uses custom S/R zones. Rules with custom zone detection will be
              ignored unless you configure test zones below.
            </div>
          )}

          {/* Zones list */}
          {zones.length > 0 && (
            <div className="space-y-2">
              {zones.map((zone, index) => (
                <div
                  key={zone.id}
                  className="p-2 bg-[var(--color-bg-page)] rounded border border-[var(--color-border)]"
                >
                  {/* Header row with zone number and delete button */}
                  <div className="flex items-center justify-between mb-2">
                    <span className="text-xs text-[var(--color-text-muted)]">
                      Zone #{index + 1}
                    </span>
                    <button
                      onClick={() => removeZone(zone.id)}
                      className="p-1 text-[var(--color-text-muted)] hover:text-[var(--color-sell)] hover:bg-[var(--color-sell)]/10 rounded transition-colors"
                      title="Remove zone"
                    >
                      <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M6 18L18 6M6 6l12 12"
                        />
                      </svg>
                    </button>
                  </div>
                  {/* Inputs row */}
                  <div className="grid grid-cols-2 gap-2">
                    <div>
                      <label className="block text-xs text-[var(--color-text-muted)] mb-1">High</label>
                      <input
                        type="number"
                        step="0.00001"
                        value={zone.upper_price || ''}
                        onChange={(e) =>
                          updateZone(zone.id, 'upper_price', parseFloat(e.target.value) || 0)
                        }
                        placeholder="1.10500"
                        className="w-full px-2 py-1 text-xs bg-[var(--color-bg-card)] border border-[var(--color-border)] rounded text-[var(--color-text-primary)]"
                      />
                    </div>
                    <div>
                      <label className="block text-xs text-[var(--color-text-muted)] mb-1">Low</label>
                      <input
                        type="number"
                        step="0.00001"
                        value={zone.lower_price || ''}
                        onChange={(e) =>
                          updateZone(zone.id, 'lower_price', parseFloat(e.target.value) || 0)
                        }
                        placeholder="1.10000"
                        className="w-full px-2 py-1 text-xs bg-[var(--color-bg-card)] border border-[var(--color-border)] rounded text-[var(--color-text-primary)]"
                      />
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}

          {/* Add zone button */}
          <button
            onClick={addZone}
            className="w-full flex items-center justify-center gap-1 px-3 py-1.5 text-xs border border-dashed border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-hover)] rounded transition-colors"
          >
            <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
            </svg>
            Add Zone
          </button>

          {/* Help text */}
          {zones.length === 0 && !showWarning && (
            <p className="text-xs text-[var(--color-text-muted)] italic">
              Define test zones to evaluate "what if" scenarios. These zones are only used in this
              backtest and don't affect your chart zones.
            </p>
          )}
        </div>
      )}
    </div>
  );
};
