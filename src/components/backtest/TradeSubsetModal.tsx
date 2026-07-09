/**
 * TradeSubsetModal - Drill-down view for trade subsets (worst, best, direction)
 *
 * Shows:
 * - Subset metrics (P&L, win rate, avg win/loss)
 * - Indicator conflict patterns (computed via batch analysis)
 * - Individual trades with Open in Chart
 * - AI terminal overlay (unified pattern)
 */
import { useEffect, useMemo, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ModalTerminalDrawer } from '../ui/ModalTerminalDrawer';
import { TradeData, DataRange } from './BacktestResultsPanel';
import type { ChatContext } from '../../hooks/useTerminalChat';

type TradeSubset = 'worst' | 'best' | 'direction';

// Types for batch conflict detection
interface ConflictPattern {
  indicatorName: string;
  indicatorType: string;
  count: number;
  totalTrades: number;
  reason: string;
  severity: string;
}

interface BatchConflictResult {
  tradeConflicts: Array<{
    tradeNum: number;
    conflicts: Array<{
      indicator_name: string;
      indicator_type: string;
      value_at_entry: string;
      conflict_reason: string;
      severity: string;
    }>;
  }>;
  patterns: ConflictPattern[];
}

interface TradeSubsetData {
  trades: TradeData[];
  pnl: number;
  count: number;
}

interface TradeSubsets {
  worst: TradeSubsetData;
  best: TradeSubsetData;
  longs: TradeSubsetData;
  shorts: TradeSubsetData;
}

interface TradeSubsetModalProps {
  subset: TradeSubset;
  tradeSubsets: TradeSubsets;
  instrument: string;
  granularity: string;
  dataRange: DataRange;
  onClose: () => void;
  strategyId?: string;
  strategyName?: string;
}

export const TradeSubsetModal = ({
  subset,
  tradeSubsets,
  instrument,
  granularity,
  dataRange,
  onClose,
  strategyId,
  strategyName,
}: TradeSubsetModalProps) => {
  const [conflictPatterns, setConflictPatterns] = useState<ConflictPattern[]>([]);
  const [conflictsLoading, setConflictsLoading] = useState(false);
  const [conflictsError, setConflictsError] = useState<string | null>(null);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  const trades = useMemo(() => {
    if (subset === 'worst') return tradeSubsets.worst.trades;
    if (subset === 'best') return tradeSubsets.best.trades;
    return [...tradeSubsets.longs.trades, ...tradeSubsets.shorts.trades];
  }, [subset, tradeSubsets]);

  // Load indicator conflicts for the trade subset
  useEffect(() => {
    // Only load conflicts for worst/best trades (direction view has too many)
    if (subset === 'direction' || trades.length === 0) {
      setConflictPatterns([]);
      return;
    }

    const loadConflicts = async () => {
      setConflictsLoading(true);
      setConflictsError(null);

      try {
        const tradeInputs = trades.map((t) => ({
          tradeNum: t.tradeNum,
          direction: t.direction,
          entryTime: t.entryTime,
        }));

        const result = await invoke<BatchConflictResult>('get_batch_trade_conflicts', {
          instrument,
          granularity,
          trades: tradeInputs,
        });

        setConflictPatterns(result.patterns);
      } catch (err) {
        console.error('Failed to load conflict patterns:', err);
        setConflictsError(String(err));
      } finally {
        setConflictsLoading(false);
      }
    };

    loadConflicts();
  }, [subset, trades, instrument, granularity]);

  const metrics = useMemo(() => {
    if (trades.length === 0) return null;

    const pnls = trades.map((t) => parseFloat(t.pnl));
    const totalPnl = pnls.reduce((sum, p) => sum + p, 0);
    const winners = pnls.filter((p) => p > 0);
    const losers = pnls.filter((p) => p < 0);
    const winRate = trades.length > 0 ? (winners.length / trades.length) * 100 : 0;
    const avgWin = winners.length > 0 ? winners.reduce((s, p) => s + p, 0) / winners.length : 0;
    const avgLoss = losers.length > 0 ? Math.abs(losers.reduce((s, p) => s + p, 0) / losers.length) : 0;

    return { totalPnl, winRate, avgWin, avgLoss, winners: winners.length, losers: losers.length };
  }, [trades]);

  const directionMetrics = useMemo(() => {
    if (subset !== 'direction') return null;

    const longWinners = tradeSubsets.longs.trades.filter((t) => parseFloat(t.pnl) > 0).length;
    const shortWinners = tradeSubsets.shorts.trades.filter((t) => parseFloat(t.pnl) > 0).length;

    return {
      longWinRate: tradeSubsets.longs.count > 0 ? (longWinners / tradeSubsets.longs.count) * 100 : 0,
      shortWinRate: tradeSubsets.shorts.count > 0 ? (shortWinners / tradeSubsets.shorts.count) * 100 : 0,
    };
  }, [subset, tradeSubsets]);

  const formatTradeTime = (dateStr: string): string => {
    if (!dateStr) return '-';
    const date = new Date(dateStr);
    if (isNaN(date.getTime())) return '-';
    return date.toLocaleDateString('en-US', { month: 'short', day: 'numeric' }) +
      ' ' + date.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit', hour12: false });
  };

  const formatDuration = (entryTime: string, exitTime: string | null): string => {
    if (!entryTime || !exitTime) return '-';
    const start = new Date(entryTime).getTime();
    const end = new Date(exitTime).getTime();
    if (isNaN(start) || isNaN(end)) return '-';

    const minutes = Math.floor((end - start) / (60 * 1000));
    if (minutes < 60) return `${minutes}m`;
    const hours = Math.floor(minutes / 60);
    const mins = minutes % 60;
    if (hours < 24) return mins > 0 ? `${hours}h ${mins}m` : `${hours}h`;
    const days = Math.floor(hours / 24);
    const remainingHours = hours % 24;
    return remainingHours > 0 ? `${days}d ${remainingHours}h` : `${days}d`;
  };

  const handleOpenChart = (trade: TradeData) => {
    if (!trade.entryTime || !trade.exitTime) return;

    const entryMs = new Date(trade.entryTime).getTime();
    const exitMs = new Date(trade.exitTime).getTime();

    const candleMs: Record<string, number> = {
      'M1': 60 * 1000, 'M5': 5 * 60 * 1000, 'M15': 15 * 60 * 1000, 'M30': 30 * 60 * 1000,
      'H1': 60 * 60 * 1000, 'H4': 4 * 60 * 60 * 1000, 'D': 24 * 60 * 60 * 1000,
    };
    const candleDuration = candleMs[granularity] || 60 * 60 * 1000;

    const fromTime = entryMs - (100 * candleDuration);
    const toTime = Math.min(exitMs + (50 * candleDuration), Date.now());

    const tradeData = [{
      entryTime: Math.floor(entryMs / 1000),
      exitTime: Math.floor(exitMs / 1000),
      entryPrice: parseFloat(trade.entryPrice),
      exitPrice: parseFloat(trade.exitPrice),
      direction: trade.direction.toLowerCase() as 'long' | 'short',
      pnl: parseFloat(trade.pnl),
    }];

    localStorage.setItem('chart_trades', JSON.stringify(tradeData));

    invoke('open_chart_window', {
      instrument,
      granularity,
      count: dataRange.totalCandles,
      from: new Date(fromTime).toISOString(),
      to: new Date(toTime).toISOString(),
    });
  };

  const title = subset === 'worst' ? 'Worst Trades' : subset === 'best' ? 'Best Trades' : 'Longs vs Shorts';

  // Build context for AI terminal
  const buildTerminalContext = useCallback((): ChatContext => {
    const subsetLabel = subset === 'worst' ? 'worst performing'
      : subset === 'best' ? 'best performing'
      : 'direction comparison';

    // Format trades as summary for context
    const tradeSummaries = trades.slice(0, 10).map((trade, idx) => {
      const pnl = parseFloat(trade.pnl);
      const duration = formatDuration(trade.entryTime, trade.exitTime);
      return {
        num: idx + 1,
        direction: trade.direction,
        pnl: pnl.toFixed(2),
        duration,
        entryTime: formatTradeTime(trade.entryTime),
      };
    });

    // Format conflict patterns
    const conflicts = conflictPatterns.map(p => ({
      indicator: p.indicatorName,
      count: p.count,
      total: p.totalTrades,
      percentage: Math.round((p.count / p.totalTrades) * 100),
      reason: p.reason,
    }));

    return {
      type: 'backtesting',
      strategy_name: strategyName,
      methodology: 'simple_historical',
      has_results: true,
      // Subset-specific context
      subset_type: subset,
      subset_description: `${trades.length} ${subsetLabel} trades`,
      instrument,
      granularity,
      trade_count: trades.length,
      total_pnl: metrics?.totalPnl.toFixed(2),
      win_rate: metrics?.winRate.toFixed(1),
      winners: metrics?.winners,
      losers: metrics?.losers,
      avg_win: metrics?.avgWin.toFixed(2),
      avg_loss: metrics?.avgLoss.toFixed(2),
      long_win_rate: directionMetrics?.longWinRate.toFixed(1),
      short_win_rate: directionMetrics?.shortWinRate.toFixed(1),
      long_count: tradeSubsets.longs.count,
      short_count: tradeSubsets.shorts.count,
      indicator_conflicts: conflicts.length > 0 ? conflicts : undefined,
      sample_trades: tradeSummaries,
      data_range: {
        start: dataRange.startTime,
        end: dataRange.endTime,
        candles: dataRange.totalCandles,
      },
      parameters: [],
    };
  }, [subset, trades, metrics, directionMetrics, conflictPatterns, instrument, granularity, strategyName, tradeSubsets, dataRange]);

  const isProfitable = metrics ? metrics.totalPnl >= 0 : false;

  return (
    <div className="fixed inset-0 z-[150] flex items-center justify-center" onClick={onClose}>
      <div className="absolute inset-0 bg-black/60" />
      <div
        className="relative bg-[var(--color-bg-elevated)] rounded-lg shadow-xl max-w-4xl w-full mx-4 max-h-[90vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--color-border)]">
          <div>
            <h3 className="text-lg font-semibold text-[var(--color-text-primary)]">{title}</h3>
            <div className="flex items-center gap-2 text-sm text-[var(--color-text-muted)]">
              <span>{trades.length} trade{trades.length !== 1 ? 's' : ''}</span>
              {metrics && (
                <>
                  <span>|</span>
                  <span className={isProfitable ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}>
                    {isProfitable ? '+' : ''}${metrics.totalPnl.toFixed(2)}
                  </span>
                </>
              )}
            </div>
          </div>
          <button onClick={onClose} className="p-1 hover:bg-[var(--color-bg-hover)] rounded transition-colors">
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-6 py-4">
          <div className="space-y-6">
            {/* Metrics */}
            {subset !== 'direction' && metrics && (
              <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
                <div className="bg-[var(--color-bg-card)] rounded-lg p-3 text-center">
                  <div className="text-xs text-[var(--color-text-muted)] mb-1">Total P&L</div>
                  <div className={`text-lg font-semibold ${isProfitable ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                    {isProfitable ? '+' : ''}${metrics.totalPnl.toFixed(2)}
                  </div>
                </div>
                <div className="bg-[var(--color-bg-card)] rounded-lg p-3 text-center">
                  <div className="text-xs text-[var(--color-text-muted)] mb-1">Win Rate</div>
                  <div className="text-lg font-semibold text-[var(--color-text-secondary)]">{metrics.winRate.toFixed(0)}%</div>
                </div>
                <div className="bg-[var(--color-bg-card)] rounded-lg p-3 text-center">
                  <div className="text-xs text-[var(--color-text-muted)] mb-1">Avg Win</div>
                  <div className="text-lg font-semibold text-[var(--color-buy)]">+${metrics.avgWin.toFixed(2)}</div>
                </div>
                <div className="bg-[var(--color-bg-card)] rounded-lg p-3 text-center">
                  <div className="text-xs text-[var(--color-text-muted)] mb-1">Avg Loss</div>
                  <div className="text-lg font-semibold text-[var(--color-sell)]">-${metrics.avgLoss.toFixed(2)}</div>
                </div>
              </div>
            )}

            {/* Indicator Conflict Patterns */}
            {subset !== 'direction' && (
              <div className="bg-[var(--color-bg-card)] rounded-lg p-4">
                <div className="mb-3">
                  <h4 className="text-sm font-medium text-[var(--color-text-primary)] flex items-center gap-2">
                    <svg className="w-4 h-4 text-[var(--color-warning)]" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                    </svg>
                    Indicator Conflicts at Entry
                  </h4>
                  <p className="text-xs text-[var(--color-text-muted)] mt-1 ml-6">How many of these trades entered against indicator signals</p>
                </div>
                {conflictsLoading && (
                  <div className="flex items-center gap-2 text-sm text-[var(--color-text-muted)]">
                    <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-[var(--color-text-muted)]" />
                    Analyzing indicator conflicts...
                  </div>
                )}
                {conflictsError && (
                  <div className="text-sm text-[var(--color-warning)]">{conflictsError}</div>
                )}
                {!conflictsLoading && !conflictsError && conflictPatterns.length === 0 && (
                  <div className="text-sm text-[var(--color-text-muted)]">No indicator conflicts detected at entry.</div>
                )}
                {!conflictsLoading && conflictPatterns.length > 0 && (
                  <div className="flex flex-wrap gap-2">
                    {conflictPatterns.map((pattern, idx) => {
                      const percentage = Math.round((pattern.count / pattern.totalTrades) * 100);
                      const severityColor = pattern.severity === 'high'
                        ? 'border-[var(--color-sell)]/50 bg-[var(--color-sell)]/10'
                        : 'border-[var(--color-warning)]/50 bg-[var(--color-warning)]/10';
                      const countColor = percentage >= 80
                        ? 'text-[var(--color-sell)]'
                        : percentage >= 50
                        ? 'text-[var(--color-warning)]'
                        : 'text-[var(--color-text-muted)]';
                      return (
                        <div
                          key={idx}
                          className={`group relative px-3 py-1.5 rounded-lg border ${severityColor} cursor-default`}
                          title={pattern.reason}
                        >
                          <div className="flex items-center gap-2">
                            <span className="text-xs text-[var(--color-text-secondary)]">{pattern.indicatorName}</span>
                            <span className={`text-xs font-medium ${countColor}`}>
                              {pattern.count}/{pattern.totalTrades}
                            </span>
                          </div>
                          {/* Tooltip on hover */}
                          <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-2 py-1 bg-[var(--color-bg-elevated)] rounded text-xs text-[var(--color-text-secondary)] whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-10 shadow-lg border border-[var(--color-border)]">
                            {pattern.reason}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            )}

            {/* Direction comparison metrics */}
            {subset === 'direction' && directionMetrics && (
              <div className="grid grid-cols-2 gap-4">
                <div className="bg-[var(--color-bg-card)] rounded-lg p-4">
                  <div className="flex items-center justify-between mb-3">
                    <span className="text-[var(--color-buy)] font-medium">Longs</span>
                    <span className="text-sm text-[var(--color-text-muted)]">{tradeSubsets.longs.count} trades</span>
                  </div>
                  <div className="grid grid-cols-2 gap-3">
                    <div>
                      <div className="text-xs text-[var(--color-text-muted)]">P&L</div>
                      <div className={`text-lg font-semibold ${tradeSubsets.longs.pnl >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                        {tradeSubsets.longs.pnl >= 0 ? '+' : ''}${tradeSubsets.longs.pnl.toFixed(2)}
                      </div>
                    </div>
                    <div>
                      <div className="text-xs text-[var(--color-text-muted)]">Win Rate</div>
                      <div className="text-lg font-semibold text-[var(--color-text-secondary)]">{directionMetrics.longWinRate.toFixed(0)}%</div>
                    </div>
                  </div>
                </div>
                <div className="bg-[var(--color-bg-card)] rounded-lg p-4">
                  <div className="flex items-center justify-between mb-3">
                    <span className="text-[var(--color-sell)] font-medium">Shorts</span>
                    <span className="text-sm text-[var(--color-text-muted)]">{tradeSubsets.shorts.count} trades</span>
                  </div>
                  <div className="grid grid-cols-2 gap-3">
                    <div>
                      <div className="text-xs text-[var(--color-text-muted)]">P&L</div>
                      <div className={`text-lg font-semibold ${tradeSubsets.shorts.pnl >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                        {tradeSubsets.shorts.pnl >= 0 ? '+' : ''}${tradeSubsets.shorts.pnl.toFixed(2)}
                      </div>
                    </div>
                    <div>
                      <div className="text-xs text-[var(--color-text-muted)]">Win Rate</div>
                      <div className="text-lg font-semibold text-[var(--color-text-secondary)]">{directionMetrics.shortWinRate.toFixed(0)}%</div>
                    </div>
                  </div>
                </div>
              </div>
            )}

            {/* Trades Table */}
            {trades.length > 0 && (
              <div>
                <h4 className="text-sm font-medium text-[var(--color-text-primary)] mb-2">Trades ({trades.length})</h4>
                <div className="overflow-x-auto max-h-64 overflow-y-auto">
                  <table className="w-full text-sm">
                    <thead className="sticky top-0 bg-[var(--color-bg-elevated)]">
                      <tr className="border-b border-[var(--color-border)] text-[var(--color-text-muted)] text-left">
                        <th className="pb-2 pr-3">#</th>
                        <th className="pb-2 pr-3">Dir</th>
                        <th className="pb-2 pr-3">Entry</th>
                        <th className="pb-2 pr-3">Exit</th>
                        <th className="pb-2 pr-3">Entry Price</th>
                        <th className="pb-2 pr-3">Exit Price</th>
                        <th className="pb-2 pr-3">P&L</th>
                        <th className="pb-2 pr-3">Duration</th>
                        <th className="pb-2"></th>
                      </tr>
                    </thead>
                    <tbody>
                      {trades.map((trade) => {
                        const pnl = parseFloat(trade.pnl);
                        const isWin = pnl > 0;
                        return (
                          <tr key={trade.tradeNum} className={`border-b border-[var(--color-border)]/50 ${isWin ? 'bg-[var(--color-buy)]/10' : 'bg-[var(--color-sell)]/10'}`}>
                            <td className="py-2 pr-3 text-[var(--color-text-muted)]">{trade.tradeNum}</td>
                            <td className={`py-2 pr-3 ${trade.direction === 'LONG' ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                              {trade.direction === 'LONG' ? 'Long' : 'Short'}
                            </td>
                            <td className="py-2 pr-3 text-xs font-mono text-[var(--color-text-secondary)]">{formatTradeTime(trade.entryTime)}</td>
                            <td className="py-2 pr-3 text-xs font-mono text-[var(--color-text-secondary)]">{trade.exitTime ? formatTradeTime(trade.exitTime) : '-'}</td>
                            <td className="py-2 pr-3 font-mono text-[var(--color-text-primary)]">{parseFloat(trade.entryPrice).toFixed(instrument.includes('JPY') ? 3 : 5)}</td>
                            <td className="py-2 pr-3 font-mono text-[var(--color-text-primary)]">{trade.exitPrice ? parseFloat(trade.exitPrice).toFixed(instrument.includes('JPY') ? 3 : 5) : '-'}</td>
                            <td className={`py-2 pr-3 ${isWin ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                              {pnl >= 0 ? '+' : ''}${pnl.toFixed(2)}
                            </td>
                            <td className="py-2 pr-3 text-[var(--color-text-muted)]">{formatDuration(trade.entryTime, trade.exitTime)}</td>
                            <td className="py-2">
                              {trade.exitTime && (
                                <button
                                  onClick={() => handleOpenChart(trade)}
                                  className="p-1 text-[var(--color-info)] hover:text-[var(--color-info)]/80 hover:bg-[var(--color-info)]/20 rounded transition-colors"
                                  title="Open in Chart"
                                >
                                  <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 12l3-3 3 3 4-4M8 21l4-4 4 4M3 4h18M4 4h16v12a1 1 0 01-1 1H5a1 1 0 01-1-1V4z" />
                                  </svg>
                                </button>
                              )}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              </div>
            )}

            {trades.length === 0 && (
              <div className="text-center text-[var(--color-text-muted)] py-8">No trades in this subset.</div>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="px-6 py-4 border-t border-[var(--color-border)]">
          <div className="flex justify-end">
            <button onClick={onClose} className="px-4 py-2 border border-[var(--color-border)] rounded hover:border-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)] transition-colors text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)]">
              Close
            </button>
          </div>
        </div>

        {/* AI Terminal Overlay */}
        <ModalTerminalDrawer
          modalId={`backtest-subset:${strategyId || 'unknown'}:${subset}`}
          contextProvider={buildTerminalContext}
          topOffset={73}
          header={`Ask about ${subset === 'direction' ? 'direction bias' : `${subset} trades`}`}
          headerDescription={
            subset === 'worst' ? 'What patterns do the worst trades share?'
              : subset === 'best' ? 'What made these trades successful?'
              : 'How do longs compare to shorts?'
          }
          welcomeContent={
            subset === 'worst' ? [
              'Try: "What do these losing trades have in common?"',
              'Try: "Was there a market condition that hurt these trades?"',
              'Try: "Should I have exited earlier on any of these?"',
            ] : subset === 'best' ? [
              'Try: "What made these trades successful?"',
              'Try: "Can I replicate this success?"',
              'Try: "What entry conditions led to winners?"',
            ] : [
              'Try: "Am I better at longs or shorts?"',
              'Try: "Should I focus on one direction?"',
              'Try: "What market conditions favor each direction?"',
            ]
          }
        />
      </div>
    </div>
  );
};
