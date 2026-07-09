/**
 * LocalBacktestsSection — read-only backtests view for the local-first window
 * (AGT-645).
 *
 * Renders a selected strategy's saved backtest runs entirely from the local
 * store (~/.wickd/app.db): the runs list, and for the selected run its
 * metrics, equity curve, and trades. No auth provider, no Zero — this is the
 * offline surface of the strategies + backtests domain.
 */

import { useEffect, useMemo, useState } from 'react';
import { LocalBacktest, listBacktests } from '../../lib/localStore';
import { EquityCurveChart } from '../charts';

/** Subset of the persisted run payload this view renders. */
interface RunPayload {
  result?: {
    metrics?: {
      totalPnl?: string;
      totalReturnPct?: string;
      annualizedReturnPct?: string;
      winRate?: string;
      totalTrades?: number;
      finalBalance?: string;
    };
    trades?: Array<{
      tradeNum: number;
      direction: string;
      entryTime: string;
      exitTime: string;
      entryPrice: string;
      exitPrice: string;
      pnl: string;
      pnlPct: string;
    }>;
    equityCurve?: Array<{ time: string; balance: string }>;
  };
  granularity?: string;
  runNumber?: number;
}

interface ParsedRun {
  row: LocalBacktest;
  payload: RunPayload;
}

const formatDate = (ms: number) => new Date(ms).toISOString().slice(0, 10);

const pnlClass = (value: string | undefined) =>
  parseFloat(value ?? '0') >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]';

interface LocalBacktestsSectionProps {
  strategyId: string;
  strategyName: string;
}

export const LocalBacktestsSection = ({
  strategyId,
  strategyName,
}: LocalBacktestsSectionProps) => {
  const [runs, setRuns] = useState<ParsedRun[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setRuns(null);
    setSelectedRunId(null);
    setError(null);
    listBacktests(strategyId)
      .then((rows) => {
        if (cancelled) return;
        const parsed = rows.map((row): ParsedRun => {
          try {
            return { row, payload: JSON.parse(row.results) as RunPayload };
          } catch {
            return { row, payload: {} };
          }
        });
        setRuns(parsed);
        // Latest run selected by default.
        if (parsed.length > 0) setSelectedRunId(parsed[parsed.length - 1].row.id);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [strategyId]);

  const selected = useMemo(
    () => runs?.find((r) => r.row.id === selectedRunId) ?? null,
    [runs, selectedRunId]
  );
  const metrics = selected?.payload.result?.metrics;
  const trades = selected?.payload.result?.trades ?? [];
  const equityCurve = selected?.payload.result?.equityCurve ?? [];

  return (
    <section aria-labelledby="backtests-heading" className="mt-8" data-testid="local-backtests">
      <h2 id="backtests-heading" className="text-lg font-semibold mb-1">
        Backtests
      </h2>
      <p className="text-xs text-[var(--color-text-muted)] mb-3">
        Saved runs for “{strategyName}” — served from the local store.
      </p>

      {error && (
        <div
          className="mb-4 p-3 rounded border border-[var(--color-sell)]/40 bg-[var(--color-sell)]/10 text-sm"
          data-testid="local-backtests-error"
        >
          Backtests unavailable: {error}
        </div>
      )}

      {runs !== null && runs.length === 0 && !error && (
        <div className="text-[var(--color-text-muted)] text-sm" data-testid="local-backtests-empty">
          No saved backtest runs for this strategy yet.
        </div>
      )}

      {runs !== null && runs.length > 0 && (
        <div className="space-y-4">
          {/* Runs list */}
          <ul className="space-y-1" data-testid="local-backtest-runs">
            {[...runs].reverse().map(({ row, payload }) => {
              const m = payload.result?.metrics;
              const isSelected = row.id === selectedRunId;
              return (
                <li key={row.id}>
                  <button
                    onClick={() => setSelectedRunId(row.id)}
                    data-testid="local-backtest-run-row"
                    aria-pressed={isSelected}
                    className={`w-full px-3 py-2 rounded border text-left text-sm flex items-center justify-between gap-3 transition-colors ${
                      isSelected
                        ? 'border-[var(--color-info)] bg-[var(--color-info)]/10'
                        : 'border-white/10 bg-white/5 hover:border-white/25'
                    }`}
                  >
                    <span className="flex items-center gap-2 min-w-0">
                      <span className="text-[var(--color-text-muted)] shrink-0">
                        #{payload.runNumber ?? '—'}
                      </span>
                      <span className="font-medium truncate">
                        {row.instrument}
                        {payload.granularity ? ` · ${payload.granularity}` : ''}
                      </span>
                      <span className="text-xs text-[var(--color-text-muted)] shrink-0">
                        {formatDate(row.start_date)} → {formatDate(row.end_date)}
                      </span>
                    </span>
                    <span className={`shrink-0 ${pnlClass(m?.totalReturnPct)}`}>
                      {m?.totalReturnPct !== undefined
                        ? `${parseFloat(m.totalReturnPct) >= 0 ? '+' : ''}${parseFloat(
                            m.totalReturnPct
                          ).toFixed(1)}%`
                        : '—'}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>

          {/* Selected run detail */}
          {selected && (
            <div
              className="p-4 rounded border border-white/10 bg-white/5 space-y-4"
              data-testid="local-backtest-detail"
            >
              {/* Metrics */}
              <div className="grid grid-cols-2 sm:grid-cols-5 gap-3" data-testid="local-backtest-metrics">
                <div>
                  <div className="text-xs text-[var(--color-text-muted)]">P&L</div>
                  <div className={`text-sm font-semibold ${pnlClass(metrics?.totalPnl)}`}>
                    {metrics?.totalPnl !== undefined
                      ? `$${parseFloat(metrics.totalPnl).toFixed(2)}`
                      : '—'}
                  </div>
                </div>
                <div>
                  <div className="text-xs text-[var(--color-text-muted)]">Return</div>
                  <div className={`text-sm font-semibold ${pnlClass(metrics?.totalReturnPct)}`}>
                    {metrics?.totalReturnPct !== undefined
                      ? `${parseFloat(metrics.totalReturnPct).toFixed(2)}%`
                      : '—'}
                  </div>
                </div>
                <div>
                  <div className="text-xs text-[var(--color-text-muted)]">Win rate</div>
                  <div className="text-sm font-semibold">
                    {metrics?.winRate !== undefined
                      ? `${parseFloat(metrics.winRate).toFixed(1)}%`
                      : '—'}
                  </div>
                </div>
                <div>
                  <div className="text-xs text-[var(--color-text-muted)]">Trades</div>
                  <div className="text-sm font-semibold">{metrics?.totalTrades ?? '—'}</div>
                </div>
                <div>
                  <div className="text-xs text-[var(--color-text-muted)]">Final balance</div>
                  <div className="text-sm font-semibold">
                    {metrics?.finalBalance !== undefined
                      ? `$${parseFloat(metrics.finalBalance).toFixed(2)}`
                      : '—'}
                  </div>
                </div>
              </div>

              {/* Equity curve */}
              {equityCurve.length > 0 && (
                <div data-testid="local-equity-curve">
                  <div className="text-xs text-[var(--color-text-muted)] mb-1">Equity curve</div>
                  <EquityCurveChart data={equityCurve} height={180} />
                </div>
              )}

              {/* Trades */}
              {trades.length > 0 && (
                <div data-testid="local-trades-table">
                  <div className="text-xs text-[var(--color-text-muted)] mb-1">
                    Trades ({trades.length})
                  </div>
                  <div className="max-h-56 overflow-y-auto overflow-x-auto rounded border border-white/10">
                    <table className="w-full text-xs">
                      <thead className="sticky top-0 bg-[var(--color-bg-page)]">
                        <tr className="text-left text-[var(--color-text-muted)]">
                          <th className="px-2 py-1.5 font-medium">#</th>
                          <th className="px-2 py-1.5 font-medium">Dir</th>
                          <th className="px-2 py-1.5 font-medium">Entry</th>
                          <th className="px-2 py-1.5 font-medium">Exit</th>
                          <th className="px-2 py-1.5 font-medium text-right">Entry px</th>
                          <th className="px-2 py-1.5 font-medium text-right">Exit px</th>
                          <th className="px-2 py-1.5 font-medium text-right">P&L</th>
                        </tr>
                      </thead>
                      <tbody>
                        {trades.map((t) => (
                          <tr
                            key={t.tradeNum}
                            className="border-t border-white/5"
                            data-testid="local-trade-row"
                          >
                            <td className="px-2 py-1.5 text-[var(--color-text-muted)]">
                              {t.tradeNum}
                            </td>
                            <td className="px-2 py-1.5 uppercase">{t.direction}</td>
                            <td className="px-2 py-1.5">{t.entryTime.slice(0, 10)}</td>
                            <td className="px-2 py-1.5">{t.exitTime.slice(0, 10)}</td>
                            <td className="px-2 py-1.5 text-right">{t.entryPrice}</td>
                            <td className="px-2 py-1.5 text-right">{t.exitPrice}</td>
                            <td className={`px-2 py-1.5 text-right ${pnlClass(t.pnl)}`}>
                              {parseFloat(t.pnl) >= 0 ? '+' : ''}
                              {parseFloat(t.pnl).toFixed(2)}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </section>
  );
};
