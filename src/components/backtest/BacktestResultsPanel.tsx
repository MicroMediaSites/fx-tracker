/**
 * BacktestResultsPanel - Shared results display for all methodologies.
 *
 * Displays:
 * - Key metrics (P&L, return, win rate, etc.)
 * - Equity curve chart
 * - Secondary metrics (Sharpe, drawdown, avg win/loss) - Premium
 * - Trade stats - Premium
 * - Trade subset cards (Worst, Best, Longs vs Shorts) - Premium
 * - AI Analysis section - Pro
 * - Open in Chart button
 */
import { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { EquityCurveChart } from '../charts';
import { InfoTooltip } from '../ui/InfoTooltip';
import { TradeSubsetModal } from './TradeSubsetModal';
import { Strategy } from '../../types/strategy';

type TradeSubset = 'worst' | 'best' | 'direction' | null;

interface BacktestMetrics {
  totalPnl: string;
  totalReturnPct: string;
  annualizedReturnPct: string;
  winningTrades: number;
  losingTrades: number;
  winRate: string;
  avgWin: string;
  avgLoss: string;
  profitFactor: string;
  maxDrawdownPct: string;
  sharpeRatio: string;
  totalTrades: number;
  finalBalance: string;
}

export interface TradeData {
  tradeNum: number;
  direction: string;
  entryTime: string;
  exitTime: string;
  entryPrice: string;
  exitPrice: string;
  units: string;
  pnl: string;
  pnlPct: string;
  cumulativePnl: string;
}

interface EquityPoint {
  time: string;
  balance: string;
}

export interface DataRange {
  startTime: string;
  endTime: string;
  totalCandles: number;
}

export interface BacktestResult {
  metrics: BacktestMetrics;
  trades: TradeData[];
  equityCurve: EquityPoint[];
  dataRange: DataRange;
}

interface BacktestResultsPanelProps {
  result: BacktestResult | null;
  running: boolean;
  selectedStrategy: Strategy | null;
  instrument: string;
  granularity: string;
  initialBalance: number;
}

const formatPnl = (pnl: string) => {
  const value = parseFloat(pnl);
  const formatted = value.toFixed(2);
  return value >= 0 ? `+$${formatted}` : `-$${Math.abs(value).toFixed(2)}`;
};

const pnlColor = (pnl: string) => {
  return parseFloat(pnl) >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]';
};

export const BacktestResultsPanel = ({
  result,
  running,
  selectedStrategy,
  instrument,
  granularity,
  initialBalance,
}: BacktestResultsPanelProps) => {
  const [selectedSubset, setSelectedSubset] = useState<TradeSubset>(null);

  // Compute trade subsets
  const tradeSubsets = useMemo(() => {
    if (!result?.trades?.length) return null;

    const trades = result.trades;

    // Worst 5 trades (lowest P&L)
    const worstTrades = [...trades]
      .filter((t) => parseFloat(t.pnl) < 0)
      .sort((a, b) => parseFloat(a.pnl) - parseFloat(b.pnl))
      .slice(0, 5);
    const worstPnl = worstTrades.reduce((sum, t) => sum + parseFloat(t.pnl), 0);

    // Best 5 trades (highest P&L)
    const bestTrades = [...trades]
      .filter((t) => parseFloat(t.pnl) > 0)
      .sort((a, b) => parseFloat(b.pnl) - parseFloat(a.pnl))
      .slice(0, 5);
    const bestPnl = bestTrades.reduce((sum, t) => sum + parseFloat(t.pnl), 0);

    // Longs vs Shorts
    const longTrades = trades.filter((t) => t.direction === 'LONG');
    const shortTrades = trades.filter((t) => t.direction === 'SHORT');
    const longPnl = longTrades.reduce((sum, t) => sum + parseFloat(t.pnl), 0);
    const shortPnl = shortTrades.reduce((sum, t) => sum + parseFloat(t.pnl), 0);

    return {
      worst: { trades: worstTrades, pnl: worstPnl, count: worstTrades.length },
      best: { trades: bestTrades, pnl: bestPnl, count: bestTrades.length },
      longs: { trades: longTrades, pnl: longPnl, count: longTrades.length },
      shorts: { trades: shortTrades, pnl: shortPnl, count: shortTrades.length },
    };
  }, [result?.trades]);

  return (
    <div className="relative">
      <div className="flex justify-between items-center mb-4">
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-medium text-[var(--color-text-primary)]">Backtest Results</h2>
          {result && result.trades.length > 0 && (
            <button
              onClick={() => {
                // Convert backtest trades to chart overlay format and store in localStorage
                const tradeData = result.trades
                  .filter((trade) => trade.exitTime && trade.exitPrice)
                  .map((trade) => ({
                    entryTime: Math.floor(new Date(trade.entryTime).getTime() / 1000),
                    exitTime: Math.floor(new Date(trade.exitTime).getTime() / 1000),
                    entryPrice: parseFloat(trade.entryPrice),
                    exitPrice: parseFloat(trade.exitPrice),
                    direction: trade.direction.toLowerCase() as 'long' | 'short',
                    pnl: parseFloat(trade.pnl),
                  }));
                if (tradeData.length > 0) {
                  localStorage.setItem('chart_trades', JSON.stringify(tradeData));
                } else {
                  localStorage.removeItem('chart_trades');
                }
                invoke('open_chart_window', {
                  instrument,
                  granularity,
                  count: result.dataRange.totalCandles,
                  from: result.dataRange.startTime,
                  to: result.dataRange.endTime,
                });
              }}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded text-sm text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-hover)] transition-colors"
            >
              <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 12l3-3 3 3 4-4M8 21l4-4 4 4M3 4h18M4 4h16v12a1 1 0 01-1 1H5a1 1 0 01-1-1V4z" />
              </svg>
              Open in Chart
            </button>
          )}
        </div>
        {result && (
          <span className="text-sm text-[var(--color-text-muted)]">
            {new Date(result.dataRange.startTime).toLocaleDateString()} —{' '}
            {new Date(result.dataRange.endTime).toLocaleDateString()}
            <span className="text-[var(--color-text-muted)] ml-2">
              ({result.dataRange.totalCandles.toLocaleString()} candles)
            </span>
          </span>
        )}
      </div>

      {/* Loading overlay */}
      {running && (
        <div className="absolute inset-0 bg-[var(--color-bg-page)]/90 flex items-center justify-center z-10">
          <div className="flex items-center gap-3 text-[var(--color-text-secondary)]">
            <svg className="animate-spin h-5 w-5" viewBox="0 0 24 24">
              <circle
                className="opacity-25"
                cx="12"
                cy="12"
                r="10"
                stroke="currentColor"
                strokeWidth="4"
                fill="none"
              />
              <path
                className="opacity-75"
                fill="currentColor"
                d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
              />
            </svg>
            <span>Running backtest...</span>
          </div>
        </div>
      )}

      {!selectedStrategy && (
        <p className="text-[var(--color-text-muted)]">Select a strategy from the list to run a backtest.</p>
      )}

      {selectedStrategy && !result && (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <svg
            className="h-12 w-12 text-[var(--color-text-muted)] mb-4"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={1.5}
              d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"
            />
          </svg>
          <p className="text-[var(--color-text-muted)] mb-2">
            No backtest results for{' '}
            <span className="text-[var(--color-text-primary)] font-medium">{selectedStrategy.name}</span>{' '}
            {instrument.replace('_', '/')} at {granularity}
          </p>
          <p className="text-[var(--color-text-muted)] text-sm">
            Click a quarter to run a backtest, or configure a custom date range.
          </p>
        </div>
      )}

      {result && (
        <div className="space-y-6">
          {/* Balance Summary - Hero metrics style */}
          <div className="bg-[var(--color-bg-elevated)]/50 rounded-lg p-4">
            <div className="grid grid-cols-4 gap-4">
              <div>
                <span className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide block mb-1">Final Balance</span>
                <div className="text-3xl font-mono font-semibold text-[var(--color-text-primary)]">${result.metrics.finalBalance}</div>
              </div>
              <div>
                <span className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide block mb-1">Starting</span>
                <div className="text-3xl font-mono text-[var(--color-text-secondary)]">${initialBalance.toLocaleString()}</div>
              </div>
              <div>
                <span className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide block mb-1">P&L</span>
                <div className={`text-3xl font-mono font-semibold ${pnlColor(result.metrics.totalPnl)}`}>
                  {formatPnl(result.metrics.totalPnl)}
                </div>
              </div>
              <div>
                <span className="text-xs text-[var(--color-text-muted)] uppercase tracking-wide block mb-1">Trades</span>
                <div className="text-3xl font-mono text-[var(--color-text-primary)]">{result.metrics.totalTrades}</div>
                <span className="text-xs text-[var(--color-text-muted)]">{result.metrics.winningTrades}W / {result.metrics.losingTrades}L</span>
              </div>
            </div>
          </div>

          {/* Key Metrics - floating typography */}
          <div className="grid grid-cols-4 gap-4 px-1">
            <div>
              <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-0.5">
                Return
                <InfoTooltip text="Total percentage gain or loss over the backtest period" />
              </span>
              <div className={`text-lg font-mono ${pnlColor(result.metrics.totalReturnPct)}`}>
                {parseFloat(result.metrics.totalReturnPct) >= 0 ? '+' : ''}
                {result.metrics.totalReturnPct}%
              </div>
            </div>
            <div>
              <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-0.5">
                Annualized
                <InfoTooltip text="Return extrapolated to a full year. Useful for comparing strategies across different time periods." />
              </span>
              <div className={`text-lg font-mono ${pnlColor(result.metrics.annualizedReturnPct)}`}>
                {parseFloat(result.metrics.annualizedReturnPct) >= 0 ? '+' : ''}
                {result.metrics.annualizedReturnPct}%
              </div>
            </div>
            <div>
              <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-0.5">
                Win Rate
                <InfoTooltip text="Percentage of trades that closed in profit. Higher isn't always better—depends on risk/reward." />
              </span>
              <div className="text-lg font-mono text-[var(--color-text-primary)]">
                {result.metrics.winRate}%
              </div>
            </div>
            <div>
              <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-0.5">
                Profit Factor
                <InfoTooltip text="Gross profit ÷ gross loss. Above 1.0 = profitable. Above 1.5 = good. Above 2.0 = excellent." />
              </span>
              <div className="text-lg font-mono text-[var(--color-text-primary)]">
                {result.metrics.profitFactor}
              </div>
            </div>
          </div>

          {/* Equity Curve */}
          {result.equityCurve.length > 0 && (
            <div className="border-t border-[var(--color-border)] pt-4">
              <div className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide mb-2">Equity Curve</div>
              <EquityCurveChart data={result.equityCurve} height={180} />
            </div>
          )}

          {/* Detailed Analytics Section */}
          <div className="space-y-6 border-t border-[var(--color-border)] pt-4">
            {/* Secondary Metrics - floating typography */}
            <div className="grid grid-cols-4 gap-4 px-1">
              <div>
                <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-0.5">
                  Sharpe Ratio
                  <InfoTooltip text="Risk-adjusted return. Above 1.0 = good. Above 2.0 = very good. Negative = losing money." />
                </span>
                <div className="text-lg font-mono text-[var(--color-text-primary)]">{result.metrics.sharpeRatio}</div>
              </div>
              <div>
                <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-0.5">
                  Max Drawdown
                  <InfoTooltip text="Largest peak-to-trough decline. Shows worst-case equity drop you'd have experienced." />
                </span>
                <div className="text-lg font-mono text-[var(--color-sell)]">
                  -{result.metrics.maxDrawdownPct}%
                </div>
              </div>
              <div>
                <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-0.5">
                  Avg Win
                  <InfoTooltip text="Average profit on winning trades. Compare to Avg Loss to assess risk/reward." />
                </span>
                <div className="text-lg font-mono text-[var(--color-buy)]">
                  ${result.metrics.avgWin}
                </div>
              </div>
              <div>
                <span className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide flex items-center gap-1 mb-0.5">
                  Avg Loss
                  <InfoTooltip text="Average loss on losing trades. Smaller relative to Avg Win = better risk/reward." />
                </span>
                <div className="text-lg font-mono text-[var(--color-sell)]">
                  -${result.metrics.avgLoss}
                </div>
              </div>
            </div>

            {/* Trade Subset Cards - clickable analysis drilldowns */}
            {tradeSubsets && (
              <div>
                <div className="text-[10px] text-[var(--color-text-muted)] uppercase tracking-wide mb-3">Trade Analysis</div>
                <div className="grid grid-cols-3 gap-3">
                  {/* Worst Trades Card */}
                  <button
                    onClick={() => setSelectedSubset('worst')}
                    className="p-3 border border-[var(--color-border)] rounded hover:border-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)]/50 transition-colors text-left group"
                  >
                    <div className="flex items-center justify-between mb-1">
                      <span className="text-xs text-[var(--color-text-muted)]">Worst Trades</span>
                      <svg className="w-3 h-3 text-[var(--color-text-muted)] group-hover:text-[var(--color-text-secondary)]" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                      </svg>
                    </div>
                    <div className="text-base font-mono text-[var(--color-sell)]">
                      {tradeSubsets.worst.count > 0 ? `-$${Math.abs(tradeSubsets.worst.pnl).toFixed(2)}` : 'None'}
                    </div>
                    <div className="text-[10px] text-[var(--color-text-muted)]">
                      {tradeSubsets.worst.count} trade{tradeSubsets.worst.count !== 1 ? 's' : ''}
                    </div>
                  </button>

                  {/* Best Trades Card */}
                  <button
                    onClick={() => setSelectedSubset('best')}
                    className="p-3 border border-[var(--color-border)] rounded hover:border-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)]/50 transition-colors text-left group"
                  >
                    <div className="flex items-center justify-between mb-1">
                      <span className="text-xs text-[var(--color-text-muted)]">Best Trades</span>
                      <svg className="w-3 h-3 text-[var(--color-text-muted)] group-hover:text-[var(--color-text-secondary)]" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                      </svg>
                    </div>
                    <div className="text-base font-mono text-[var(--color-buy)]">
                      {tradeSubsets.best.count > 0 ? `+$${tradeSubsets.best.pnl.toFixed(2)}` : 'None'}
                    </div>
                    <div className="text-[10px] text-[var(--color-text-muted)]">
                      {tradeSubsets.best.count} trade{tradeSubsets.best.count !== 1 ? 's' : ''}
                    </div>
                  </button>

                  {/* Longs vs Shorts Card */}
                  <button
                    onClick={() => setSelectedSubset('direction')}
                    className="p-3 border border-[var(--color-border)] rounded hover:border-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)]/50 transition-colors text-left group"
                  >
                    <div className="flex items-center justify-between mb-1">
                      <span className="text-xs text-[var(--color-text-muted)]">Longs vs Shorts</span>
                      <svg className="w-3 h-3 text-[var(--color-text-muted)] group-hover:text-[var(--color-text-secondary)]" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
                      </svg>
                    </div>
                    <div className="flex items-center gap-2">
                      <div>
                        <span className={`text-base font-mono ${tradeSubsets.longs.pnl >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                          {tradeSubsets.longs.pnl >= 0 ? '+' : '-'}${Math.abs(tradeSubsets.longs.pnl).toFixed(0)}
                        </span>
                        <span className="text-[10px] text-[var(--color-text-muted)] ml-0.5">L</span>
                      </div>
                      <span className="text-[var(--color-text-muted)]">/</span>
                      <div>
                        <span className={`text-base font-mono ${tradeSubsets.shorts.pnl >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                          {tradeSubsets.shorts.pnl >= 0 ? '+' : '-'}${Math.abs(tradeSubsets.shorts.pnl).toFixed(0)}
                        </span>
                        <span className="text-[10px] text-[var(--color-text-muted)] ml-0.5">S</span>
                      </div>
                    </div>
                    <div className="text-[10px] text-[var(--color-text-muted)]">
                      {tradeSubsets.longs.count}L / {tradeSubsets.shorts.count}S
                    </div>
                  </button>
                </div>
              </div>
            )}
          </div>

        </div>
      )}

      {/* Trade Subset Modal */}
      {selectedSubset && tradeSubsets && result && (
        <TradeSubsetModal
          subset={selectedSubset}
          tradeSubsets={tradeSubsets}
          instrument={instrument}
          granularity={granularity}
          dataRange={result.dataRange}
          onClose={() => setSelectedSubset(null)}
          strategyId={selectedStrategy?.id}
          strategyName={selectedStrategy?.name}
        />
      )}
    </div>
  );
}
