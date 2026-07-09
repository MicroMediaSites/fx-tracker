/**
 * WalkForwardResults - Results display for Walk-Forward Analysis
 *
 * Layout:
 * 1. Summary Cards (Return, Sharpe, Training Consistency, Consistency)
 * 2. Parameter Stability (how consistent params were across windows)
 * 3. Highest-Performing Single Window (collapsible)
 * 4. Efficiency Badge / Warning
 * 5. Quick Stats
 * 6. Period Breakdown Table
 * 7. AI Analysis Section
 */
import { useMemo, useState } from 'react';
import { WalkForwardResult, WalkForwardPeriod, ParameterDefinition } from '../../types/strategy';
import { getEfficiencyBadge, getEfficiencyColor } from './walkForwardUtils';
import { InfoTooltip } from '../ui/InfoTooltip';
import { formatParamValue } from '../../utils/formatters';
import { ParameterComparisonCards } from './ParameterComparisonCards';

interface WalkForwardResultsProps {
  result: WalkForwardResult;
  initialBalance: number;
  oosReturnPct: string | null;
  oosAnnualizedReturnPct: string | null;
  onSelectWindow: (period: WalkForwardPeriod) => void;
  /** Baseline result for comparison (all params at defaults) */
  baselineResult?: WalkForwardResult | null;
  /** Whether baseline is currently running */
  baselineRunning?: boolean;
  /** IDs of parameters that were in range mode */
  rangedParamIds?: string[];
  /** Parameter definitions for display */
  parameters?: ParameterDefinition[];
  /** Trigger a parameter sweep */
  onRunSweep?: (paramId: string) => void;
  /** Whether sweep is running */
  sweepRunning?: boolean;
}

export const WalkForwardResults = ({
  result,
  initialBalance,
  oosReturnPct,
  oosAnnualizedReturnPct,
  onSelectWindow,
  baselineResult,
  baselineRunning,
  rangedParamIds = [],
  parameters = [],
  onRunSweep,
  sweepRunning,
}: WalkForwardResultsProps) => {
  // Find the best OOS performing window
  const bestPeriod = useMemo(() => {
    if (!result.periods?.length) return null;
    return [...result.periods].sort((a, b) => b.out_of_sample_sharpe - a.out_of_sample_sharpe)[0];
  }, [result.periods]);

  const [showBestWindow, setShowBestWindow] = useState(false);

  return (
    <div className="space-y-6">
      <h3 className="text-sm font-medium text-[var(--color-text-primary)]">Walk-Forward Results</h3>

      {/* Baseline Comparison Cards */}
      {baselineRunning && (
        <div className="flex items-center gap-2 text-sm text-[var(--color-text-muted)]">
          <svg className="w-4 h-4 animate-spin" viewBox="0 0 24 24" fill="none">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
          </svg>
          Running baseline comparison...
        </div>
      )}
      {baselineResult && rangedParamIds.length > 0 && (
        <ParameterComparisonCards
          result={result}
          baselineResult={baselineResult}
          rangedParamIds={rangedParamIds}
          parameters={parameters}
          initialBalance={initialBalance}
          onRunSweep={onRunSweep}
          sweepRunning={sweepRunning}
        />
      )}

      {/* Hero Metrics */}
      <div className="bg-[var(--color-bg-elevated)]/50 rounded-lg p-4">
        <div className="grid grid-cols-4 gap-4">
          <div>
            <span className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-1">
              Return (unseen data)
              <InfoTooltip text="Total return from test periods the optimizer never saw during training." />
            </span>
            <div
              className={`text-3xl font-mono font-semibold ${
                parseFloat(result.oos_total_pnl || '0') >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'
              }`}
            >
              {parseFloat(oosReturnPct || '0') >= 0 ? '+' : ''}{oosReturnPct}%
            </div>
            <div className="text-xs text-[var(--color-text-muted)]">
              ${parseFloat(result.oos_total_pnl || '0').toFixed(2)} on ${initialBalance.toLocaleString()}
              {oosAnnualizedReturnPct && (
                <span className="ml-1">({parseFloat(oosAnnualizedReturnPct) >= 0 ? '+' : ''}{oosAnnualizedReturnPct}% ann.)</span>
              )}
            </div>
          </div>
          <div>
            <span className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-1">
              Sharpe (unseen data)
              <InfoTooltip text="Average Sharpe ratio across all test periods on unseen data. Above 1.0 is generally considered strong risk-adjusted returns." />
            </span>
            <div className={`text-3xl font-mono font-semibold ${(result.oos_avg_sharpe ?? 0) >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
              {(result.oos_avg_sharpe ?? 0).toFixed(2)}
            </div>
          </div>
          <div>
            <span className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-1">
              Training Consistency
              <InfoTooltip text="How well training performance predicted results on unseen data (OOS Sharpe / IS Sharpe). Above 50% suggests the strategy generalizes well." />
            </span>
            <div
              className={`text-3xl font-mono font-semibold ${getEfficiencyColor(result.sharpe_efficiency ?? 0, result.oos_avg_sharpe ?? 0)}`}
            >
              {(result.oos_avg_sharpe ?? 0) < 0 ? 'N/A' : `${(result.sharpe_efficiency ?? 0).toFixed(0)}%`}
            </div>
          </div>
          <div>
            <span className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-1">
              Consistency
              <InfoTooltip text="Composite score (0-100) based on how consistently the strategy performed across all test windows." />
            </span>
            <div className="text-3xl font-mono font-semibold text-[var(--color-text-primary)]">{result.robustness_score ?? 0}/100</div>
          </div>
        </div>
      </div>

      {/* Parameter Stability */}
      {result.parameter_stability?.length > 0 && (
        <div>
          <h4 className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide mb-2 flex items-center gap-1">
            Parameter Stability
            <InfoTooltip text="How often each parameter settled on the same value across windows. High stability = parameter is robust. Low = may be noise-fitting." />
          </h4>
          <div className="space-y-2">
            {result.parameter_stability.map((param) => (
              <div key={param.param_id} className="flex items-center gap-3">
                <span className="text-xs text-[var(--color-text-muted)] w-32 truncate">{param.param_name}</span>
                <div className="flex-1 bg-[var(--color-border)] rounded-full h-1.5">
                  <div
                    className={`h-1.5 rounded-full ${
                      param.stability_pct >= 50 ? 'bg-[var(--color-buy)]' : 'bg-[var(--color-warning)]'
                    }`}
                    style={{ width: `${param.stability_pct}%` }}
                  />
                </div>
                <span className="text-xs font-mono text-[var(--color-text-muted)] w-44 text-right">
                  {param.stability_pct.toFixed(0)}% ({formatParamValue(param.mode_value, param.param_name)})
                  {' — '}
                  <span className={param.stability_pct >= 70 ? 'text-[var(--color-buy)]' : param.stability_pct >= 40 ? 'text-[var(--color-warning)]' : 'text-[var(--color-sell)]'}>
                    {param.stability_pct >= 70 ? 'consistent' : param.stability_pct >= 40 ? 'variable' : 'unstable'}
                  </span>
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Highest-Performing Single Window (collapsible) */}
      {bestPeriod && Object.keys(bestPeriod.optimized_params).length > 0 && (
        <div>
          <button
            onClick={() => setShowBestWindow(!showBestWindow)}
            className="flex items-center gap-2 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
          >
            <svg
              className={`w-3 h-3 transition-transform ${showBestWindow ? 'rotate-90' : ''}`}
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
            </svg>
            <span className="uppercase tracking-wide">Highest-Performing Single Window</span>
            <span className="font-normal normal-case">
              Window {bestPeriod.window.window_num} · Sharpe {bestPeriod.out_of_sample_sharpe.toFixed(2)}
            </span>
          </button>
          {showBestWindow && (
            <div className="mt-2 p-3 bg-[var(--color-bg-elevated)]/30 border border-[var(--color-border)] rounded-lg">
              <div className="flex flex-wrap gap-x-6 gap-y-1">
                {Object.entries(bestPeriod.optimized_params).map(([key, value]) => (
                  <div key={key} className="text-sm whitespace-nowrap">
                    <span className="text-[var(--color-text-muted)]">{key}:</span>{' '}
                    <span className="font-mono font-medium text-[var(--color-text-primary)]">{formatParamValue(value, key)}</span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      )}

      {/* Efficiency Badge / Warning */}
      {(() => {
        const badge = getEfficiencyBadge(result.sharpe_efficiency ?? 0, result.oos_avg_sharpe ?? 0, result.oos_total_pnl);
        return (
          <div className="flex items-center gap-2 text-sm">
            {badge.warning && (
              <svg className="w-4 h-4 text-[var(--color-sell)]" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
              </svg>
            )}
            <span className={`font-medium ${badge.warning ? 'text-[var(--color-sell)]' : 'text-[var(--color-text-primary)]'}`}>
              {badge.text}
            </span>
            <span className="text-[var(--color-text-muted)]">
              {badge.warning
                ? 'Periodic re-tuning did not produce positive returns over this test period'
                : `Training-to-unseen consistency: ${(result.sharpe_efficiency ?? 0).toFixed(0)}%`
              }
            </span>
          </div>
        );
      })()}

      {/* Quick Stats */}
      <div className="flex flex-wrap gap-x-6 gap-y-2 text-sm text-[var(--color-text-muted)]">
        <span className="flex items-center gap-1">
          <span className="font-mono font-medium text-[var(--color-text-primary)]">{result.valid_periods ?? 0}</span>
          <span>/</span>
          <span>{result.total_periods ?? 0}</span>
          <span>valid periods</span>
          <InfoTooltip text="Windows where optimization found parameters meeting minimum criteria. Invalid windows had too few trades or failed constraints." />
        </span>
        <span className="flex items-center gap-1">
          <span className="font-mono font-medium text-[var(--color-text-primary)]">{result.profitable_periods ?? 0}</span>
          <span>profitable</span>
          <InfoTooltip text="OOS periods that ended with positive P&L. More profitable periods = more consistent strategy." />
        </span>
        <span className="flex items-center gap-1">
          <span className="font-mono font-medium text-[var(--color-text-primary)]">{result.oos_total_trades ?? 0}</span>
          <span>total trades</span>
        </span>
        <span className="flex items-center gap-1">
          <span>Win Rate</span>
          <span className="font-mono font-medium text-[var(--color-text-primary)]">{result.oos_win_rate ?? 0}%</span>
          <InfoTooltip text="Percentage of OOS trades that were profitable across all windows." />
        </span>
        <span className="flex items-center gap-1">
          <span>Max DD</span>
          <span className="font-mono font-medium text-[var(--color-text-primary)]">{result.oos_max_drawdown_pct ?? 0}%</span>
          <InfoTooltip text="Maximum peak-to-trough equity decline during OOS periods." />
        </span>
      </div>

      {/* Period Breakdown Table */}
      {result.periods?.length > 0 && (
        <div className="border-t border-[var(--color-border)] pt-4">
          <h4 className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide mb-3">Period Breakdown</h4>
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-[var(--color-text-muted)] text-left text-xs uppercase tracking-wide">
                  <th className="pb-2 pr-3 font-normal">#</th>
                  <th className="pb-2 pr-3 font-normal">Test Period</th>
                  <th className="pb-2 pr-3 font-normal">Train Sharpe</th>
                  <th className="pb-2 pr-3 font-normal">Test Sharpe</th>
                  <th className="pb-2 pr-3 font-normal">Test P&L</th>
                  <th className="pb-2 pr-3 font-normal">Trades</th>
                  <th className="pb-2 font-normal">Status</th>
                </tr>
              </thead>
              <tbody>
                {result.periods.map((period) => (
                  <tr
                    key={period.window.window_num}
                    onClick={() => onSelectWindow(period)}
                    className={`border-l-2 cursor-pointer hover:bg-[var(--color-bg-hover)] transition-colors ${
                      period.oos_profitable ? 'border-l-[var(--color-buy)]' : 'border-l-[var(--color-sell)]'
                    }`}
                  >
                    <td className="py-2 pr-3 pl-2 text-[var(--color-text-muted)]">{period.window.window_num}</td>
                    <td className="py-2 pr-3 text-xs font-mono text-[var(--color-text-secondary)]">
                      {new Date(period.window.test_start).toLocaleDateString()} -{' '}
                      {new Date(period.window.test_end).toLocaleDateString()}
                    </td>
                    <td className="py-2 pr-3 font-mono text-[var(--color-text-primary)]">{period.in_sample_sharpe.toFixed(2)}</td>
                    <td className="py-2 pr-3 font-mono text-[var(--color-text-primary)]">{period.out_of_sample_sharpe.toFixed(2)}</td>
                    <td
                      className={`py-2 pr-3 font-mono ${
                        parseFloat(period.out_of_sample_metrics.total_pnl) >= 0
                          ? 'text-[var(--color-buy)]'
                          : 'text-[var(--color-sell)]'
                      }`}
                    >
                      ${parseFloat(period.out_of_sample_metrics.total_pnl).toFixed(2)}
                    </td>
                    <td className="py-2 pr-3 font-mono text-[var(--color-text-primary)]">{period.oos_trade_count}</td>
                    <td className="py-2">
                      {period.oos_profitable ? (
                        <span className="text-[var(--color-buy)]">✓</span>
                      ) : (
                        <span className="text-[var(--color-sell)]">✗</span>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

    </div>
  );
};
