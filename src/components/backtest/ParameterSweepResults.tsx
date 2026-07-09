/**
 * ParameterSweepResults - Displays results of a fixed-value parameter sweep.
 *
 * Shows a table of each tested value with its walk-forward results,
 * highlighting the default and best-performing values.
 */
import { useMemo } from 'react';
import { ParameterSweepResult, ParameterSweepProgress } from '../../types/strategy';
import { formatParamValue } from '../../utils/formatters';

interface ParameterSweepResultsProps {
  result: ParameterSweepResult;
  /** Currently running progress */
  progress?: ParameterSweepProgress | null;
  /** Which objective was used */
  objective: string;
  /** Go back to walk-forward results */
  onBack: () => void;
}

export const ParameterSweepResults = ({
  result,
  progress,
  objective,
  onBack,
}: ParameterSweepResultsProps) => {
  // Determine the "best" row based on objective
  const bestIndex = useMemo(() => {
    if (result.results.length === 0) return -1;
    let bestIdx = 0;
    for (let i = 1; i < result.results.length; i++) {
      const current = result.results[i];
      const best = result.results[bestIdx];
      switch (objective) {
        case 'win_rate':
          if (parseFloat(current.oosWinRate) > parseFloat(best.oosWinRate)) bestIdx = i;
          break;
        case 'total_return':
          if (parseFloat(current.oosTotalReturnPct) > parseFloat(best.oosTotalReturnPct)) bestIdx = i;
          break;
        case 'min_drawdown':
          if (parseFloat(current.oosMaxDrawdownPct) < parseFloat(best.oosMaxDrawdownPct)) bestIdx = i;
          break;
        case 'profit_factor':
        case 'sharpe_ratio':
        default:
          if (current.oosAvgSharpe > best.oosAvgSharpe) bestIdx = i;
          break;
      }
    }
    return bestIdx;
  }, [result.results, objective]);

  const objectiveLabel = (() => {
    switch (objective) {
      case 'win_rate': return 'win rate';
      case 'total_return': return 'total return';
      case 'min_drawdown': return 'lowest drawdown';
      case 'profit_factor': return 'profit factor';
      case 'sharpe_ratio':
      default: return 'risk-adjusted return';
    }
  })();

  const bestResult = bestIndex >= 0 ? result.results[bestIndex] : null;

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-medium text-[var(--color-text-primary)]">
            {result.paramName} — Fixed Value Comparison
          </h3>
          <p className="text-xs text-[var(--color-text-muted)] mt-0.5">
            Each value was held constant across all walk-forward windows
          </p>
        </div>
        <button
          onClick={onBack}
          className="text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] transition-colors"
        >
          Back to results
        </button>
      </div>

      {/* Progress indicator if still running */}
      {progress && (
        <div className="flex items-center gap-2 text-sm text-[var(--color-text-muted)]">
          <svg className="w-4 h-4 animate-spin" viewBox="0 0 24 24" fill="none">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
          </svg>
          Testing value {formatParamValue(progress.currentValue)} ({progress.currentIndex + 1}/{progress.totalValues})...
        </div>
      )}

      {/* Results Table */}
      {result.results.length > 0 && (
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-[var(--color-text-muted)] text-left text-xs uppercase tracking-wide">
                <th className="pb-2 pr-4 font-normal">Value</th>
                <th className="pb-2 pr-4 font-normal">Return</th>
                <th className="pb-2 pr-4 font-normal">Sharpe</th>
                <th className="pb-2 pr-4 font-normal">Trades</th>
                <th className="pb-2 pr-4 font-normal">Max DD</th>
                <th className="pb-2 font-normal">Win Rate</th>
              </tr>
            </thead>
            <tbody>
              {result.results.map((row, idx) => {
                const isDefault = row.value === result.defaultValue;
                const isBest = idx === bestIndex;
                const returnPct = parseFloat(row.oosTotalReturnPct);
                const sharpe = row.oosAvgSharpe;

                return (
                  <tr
                    key={row.value}
                    className={`border-l-2 ${
                      isBest
                        ? 'border-l-[var(--color-buy)] bg-[var(--color-buy)]/5'
                        : isDefault
                        ? 'border-l-[var(--color-info)] bg-[var(--color-info)]/5'
                        : 'border-l-transparent'
                    }`}
                  >
                    <td className="py-2 pr-4 pl-2 font-mono text-[var(--color-text-primary)]">
                      {formatParamValue(row.value)}
                      {isDefault && (
                        <span className="ml-1.5 text-[10px] px-1 py-0.5 rounded bg-[var(--color-info)]/20 text-[var(--color-info)]">
                          default
                        </span>
                      )}
                      {isBest && !isDefault && (
                        <span className="ml-1.5 text-[10px] px-1 py-0.5 rounded bg-[var(--color-buy)]/20 text-[var(--color-buy)]">
                          best
                        </span>
                      )}
                      {isBest && isDefault && (
                        <span className="ml-1.5 text-[10px] px-1 py-0.5 rounded bg-[var(--color-buy)]/20 text-[var(--color-buy)]">
                          best
                        </span>
                      )}
                    </td>
                    <td className={`py-2 pr-4 font-mono ${returnPct >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                      {returnPct >= 0 ? '+' : ''}{returnPct.toFixed(1)}%
                    </td>
                    <td className={`py-2 pr-4 font-mono ${sharpe >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                      {sharpe.toFixed(2)}
                    </td>
                    <td className="py-2 pr-4 font-mono text-[var(--color-text-primary)]">
                      {row.oosTotalTrades}
                    </td>
                    <td className="py-2 pr-4 font-mono text-[var(--color-text-primary)]">
                      {row.oosMaxDrawdownPct}%
                    </td>
                    <td className="py-2 font-mono text-[var(--color-text-primary)]">
                      {row.oosWinRate}%
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {/* Summary statement */}
      {bestResult && (
        <div className="text-sm text-[var(--color-text-muted)]">
          {formatParamValue(bestResult.value)} maximized {objectiveLabel} across the test period.
          {bestResult.value === result.defaultValue && (
            <span className="ml-1 text-[var(--color-info)]">This is also the current default.</span>
          )}
        </div>
      )}
    </div>
  );
};
