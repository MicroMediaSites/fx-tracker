/**
 * ParameterComparisonCards - Compares walk-forward results with ranged params vs baseline (all defaults).
 *
 * When multiple params are in range mode, shows a single overall comparison card
 * with a note that results reflect combined effects.
 * When one param is in range mode, the comparison is naturally per-parameter.
 */
import { WalkForwardResult, ParameterDefinition } from '../../types/strategy';

interface ParameterComparisonCardsProps {
  /** Main walk-forward result (with ranged params) */
  result: WalkForwardResult;
  /** Baseline walk-forward result (all params at defaults) */
  baselineResult: WalkForwardResult;
  /** IDs of parameters that were in range mode */
  rangedParamIds: string[];
  /** All parameter definitions for display names and defaults */
  parameters: ParameterDefinition[];
  /** Initial balance for return calculation */
  initialBalance: number;
  /** Trigger a parameter sweep for a specific param */
  onRunSweep?: (paramId: string) => void;
  /** Whether a sweep is currently running */
  sweepRunning?: boolean;
}

export const ParameterComparisonCards = ({
  result,
  baselineResult,
  rangedParamIds,
  parameters,
  initialBalance,
  onRunSweep,
  sweepRunning,
}: ParameterComparisonCardsProps) => {
  const mainReturn = initialBalance > 0 ? (parseFloat(result.oos_total_pnl || '0') / initialBalance) * 100 : 0;
  const baselineReturn = initialBalance > 0 ? (parseFloat(baselineResult.oos_total_pnl || '0') / initialBalance) * 100 : 0;
  const mainSharpe = result.oos_avg_sharpe ?? 0;
  const baselineSharpe = baselineResult.oos_avg_sharpe ?? 0;

  const returnDelta = mainReturn - baselineReturn;
  const sharpeDelta = mainSharpe - baselineSharpe;
  const retuningWins = returnDelta > 0 || sharpeDelta > 0;

  // Get display info for ranged params
  const rangedParams = parameters.filter((p) => rangedParamIds.includes(p.id));
  const isMultiParam = rangedParams.length > 1;

  const formatPct = (v: number) => `${v >= 0 ? '+' : ''}${v.toFixed(1)}%`;
  const formatSharpe = (v: number) => v.toFixed(2);

  return (
    <div className="space-y-3">
      {/* Overall comparison card */}
      <div className="bg-[var(--color-bg-elevated)]/50 border border-[var(--color-border)] rounded-lg p-4">
        <h4 className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide mb-3">
          {isMultiParam
            ? 'Your configuration vs. all defaults'
            : `${rangedParams[0]?.name || 'Parameter'} — periodic re-tuning vs. fixed default`}
        </h4>

        <div className="grid grid-cols-[1fr_auto_auto] gap-x-6 gap-y-2 text-sm">
          {/* Header row */}
          <div className="text-[var(--color-text-muted)]" />
          <div className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide text-right">Return</div>
          <div className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide text-right">Sharpe</div>

          {/* Re-tuning row */}
          <div className="text-[var(--color-text-secondary)]">Periodic re-tuning</div>
          <div className={`font-mono text-right ${mainReturn >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
            {formatPct(mainReturn)}
          </div>
          <div className={`font-mono text-right ${mainSharpe >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
            {formatSharpe(mainSharpe)}
          </div>

          {/* Baseline row */}
          <div className="text-[var(--color-text-secondary)]">Fixed at defaults</div>
          <div className={`font-mono text-right ${baselineReturn >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
            {formatPct(baselineReturn)}
          </div>
          <div className={`font-mono text-right ${baselineSharpe >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
            {formatSharpe(baselineSharpe)}
          </div>
        </div>

        {/* Factual comparison statement */}
        <div className={`mt-3 text-sm ${retuningWins ? 'text-[var(--color-buy)]' : 'text-[var(--color-text-muted)]'}`}>
          {returnDelta > 0 && sharpeDelta > 0
            ? 'Periodic re-tuning outperformed fixed defaults on both return and risk-adjusted return.'
            : returnDelta > 0
            ? 'Periodic re-tuning produced higher returns, but lower risk-adjusted return.'
            : sharpeDelta > 0
            ? 'Periodic re-tuning produced better risk-adjusted return, but lower total return.'
            : returnDelta === 0 && sharpeDelta === 0
            ? 'Both approaches produced identical results.'
            : 'Fixed defaults outperformed periodic re-tuning over this test period.'}
        </div>

        {/* Parameter stability for ranged params */}
        {result.parameter_stability?.length > 0 && (
          <div className="mt-3 pt-3 border-t border-[var(--color-border)]/30">
            {rangedParams.map((p) => {
              const stability = result.parameter_stability.find((s) => s.param_id === p.id);
              if (!stability) return null;
              const interpretation =
                stability.stability_pct >= 70
                  ? 'consistent'
                  : stability.stability_pct >= 40
                  ? 'variable'
                  : 'unstable';
              return (
                <div key={p.id} className="flex items-center justify-between text-xs text-[var(--color-text-muted)]">
                  <span>{p.name} stability: {stability.stability_pct.toFixed(0)}%</span>
                  <span className={
                    stability.stability_pct >= 70
                      ? 'text-[var(--color-buy)]'
                      : stability.stability_pct >= 40
                      ? 'text-[var(--color-warning)]'
                      : 'text-[var(--color-sell)]'
                  }>
                    {interpretation}
                  </span>
                </div>
              );
            })}
          </div>
        )}

        {/* Multi-param caveat */}
        {isMultiParam && (
          <div className="mt-2 text-xs text-[var(--color-text-muted)] italic">
            Multiple parameters were tested simultaneously. Results reflect their combined effect.
          </div>
        )}

        {/* Sweep trigger — only for single ranged param */}
        {!isMultiParam && onRunSweep && rangedParams.length === 1 && (
          <div className="mt-3 pt-3 border-t border-[var(--color-border)]/30">
            <button
              onClick={() => onRunSweep(rangedParams[0].id)}
              disabled={sweepRunning}
              className="text-xs text-[var(--color-info)] hover:text-[var(--color-info)]/80 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {sweepRunning ? 'Running sweep...' : 'Find best fixed value'}
            </button>
            <span className="ml-2 text-xs text-[var(--color-text-muted)]">
              Tests each value independently across all windows
            </span>
          </div>
        )}
      </div>
    </div>
  );
};
