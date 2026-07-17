/**
 * WindowDetailModal - Drill-down view for a single walk-forward window
 *
 * Shows:
 * - Window timing and optimized parameters
 * - OOS performance metrics
 * - Individual trades
 * - Market feed drawer (unified pattern)
 */
import { useEffect } from 'react';
import { WalkForwardPeriod, Strategy, SimulatedTrade } from '../../types/strategy';
import { ModalTerminalDrawer } from '../ui/ModalTerminalDrawer';
import { openChartWindow } from '../../utils/windows';
import { formatParamValue } from '../../utils/formatters';
import { CHART_PARAMS_OVERRIDES_KEY } from '../../hooks/useChartParams';

interface WindowDetailModalProps {
  period: WalkForwardPeriod;
  instrument: string;
  granularity: string;
  strategy: Strategy;
  onClose: () => void;
}

export const WindowDetailModal = ({
  period,
  instrument,
  granularity,
  strategy,
  onClose,
}: WindowDetailModalProps) => {
  // Handle escape key
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  // Format duration from entry to exit (RFC3339 strings)
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

  // Format date for display
  const formatDate = (dateStr: string): string => {
    return new Date(dateStr).toLocaleDateString('en-US', {
      month: 'short',
      day: 'numeric',
      year: 'numeric',
    });
  };

  // Format time for trade display (RFC3339 string)
  const formatTradeTime = (dateStr: string): string => {
    if (!dateStr) return '-';
    const date = new Date(dateStr);
    if (isNaN(date.getTime())) return '-';
    return date.toLocaleDateString('en-US', {
      month: 'short',
      day: 'numeric',
    }) + ' ' + date.toLocaleTimeString('en-US', {
      hour: '2-digit',
      minute: '2-digit',
      hour12: false,
    });
  };

  // Open chart window for a specific trade
  const handleOpenChart = (trade: SimulatedTrade, idx: number) => {
    if (!trade.entryTime || !trade.exitTime) return;

    const entryMs = new Date(trade.entryTime).getTime();
    const exitMs = new Date(trade.exitTime).getTime();

    // Calculate candle duration based on granularity
    const candleMs: Record<string, number> = {
      'M1': 60 * 1000,
      'M5': 5 * 60 * 1000,
      'M15': 15 * 60 * 1000,
      'M30': 30 * 60 * 1000,
      'H1': 60 * 60 * 1000,
      'H4': 4 * 60 * 60 * 1000,
      'D': 24 * 60 * 60 * 1000,
    };
    const candleDuration = candleMs[granularity] || 60 * 60 * 1000;

    // Extra buffer for Ichimoku warmup: 52 (Senkou B) + 26 (displacement) = 78 periods
    const ichimokuWarmup = 80;
    // Show 100 candles before entry + warmup buffer, 50 after exit
    const fromTime = entryMs - ((100 + ichimokuWarmup) * candleDuration);
    const toTime = Math.min(exitMs + (50 * candleDuration), Date.now());

    // Units must be negative for short trades (chart uses this for direction)
    const units = parseFloat(trade.units);
    const signedUnits = trade.isLong ? units : -units;

    // Format trade data for chart overlay (must match expected structure)
    const tradeData = [{
      id: `simulated-${idx}`,
      instrument,
      units: String(signedUnits),
      open_price: trade.entryPrice,
      close_price: trade.exitPrice,
      open_time: entryMs,      // Must be number (milliseconds)
      close_time: exitMs,      // Must be number (milliseconds)
      realized_pl: trade.pnl,
    }];

    // Store optimized parameters for the chart to display
    // These are the parameters that were used during this walk-forward window's test
    if (period.optimized_params && Object.keys(period.optimized_params).length > 0) {
      localStorage.setItem(CHART_PARAMS_OVERRIDES_KEY, JSON.stringify(period.optimized_params));
    }

    openChartWindow({
      instrument,
      granularity,
      count: 500,
      from: new Date(fromTime).toISOString(),
      to: new Date(toTime).toISOString(),
      trades: JSON.stringify(tradeData),
    });
  };

  const totalPnl = parseFloat(period.out_of_sample_metrics.total_pnl);
  const isProfitable = totalPnl >= 0;

  return (
    <div
      className="fixed inset-0 z-[150] flex items-center justify-center"
      onClick={onClose}
    >
      <div className="absolute inset-0 bg-black/60" />
      <div
        className="relative bg-[var(--color-bg-elevated)] rounded-lg shadow-xl max-w-4xl w-full mx-4 max-h-[90vh] flex flex-col"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--color-border)]">
          <div>
            <h3 className="text-lg font-semibold text-[var(--color-text-primary)]">Window {period.window.window_num} Details</h3>
            <div className="flex items-center gap-4 text-sm text-[var(--color-text-muted)]">
              <span>
                <span className="text-[var(--color-text-muted)]/70">Train:</span> {formatDate(period.window.train_start)} – {formatDate(period.window.train_end)}
              </span>
              <span>
                <span className="text-[var(--color-text-muted)]/70">Test:</span> {formatDate(period.window.test_start)} – {formatDate(period.window.test_end)}
              </span>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-1 hover:bg-[var(--color-bg-hover)] rounded transition-colors text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)]"
          >
            <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto px-6 py-4">
          <div className="space-y-6">
            {/* Parameters Used (keep purple for WFT branding) */}
            {Object.keys(period.optimized_params).length > 0 && (
              <div className="bg-purple-900/20 border border-purple-500/30 rounded-lg p-3">
                <div className="text-xs text-purple-300 mb-2">Selected Parameters</div>
                <div className="grid grid-cols-2 md:grid-cols-4 gap-2">
                  {Object.entries(period.optimized_params).map(([key, value]) => (
                    <div key={key} className="bg-[var(--color-bg-card)] rounded px-2 py-1.5 text-sm">
                      <span className="text-[var(--color-text-muted)]">{key}:</span>{' '}
                      <span className="font-mono font-medium text-[var(--color-text-primary)]">{formatParamValue(value, key)}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Metrics Summary */}
            <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
              <div className="bg-[var(--color-bg-card)] rounded-lg p-3 text-center">
                <div className="text-xs text-[var(--color-text-muted)] mb-1">Test P&L</div>
                <div className={`text-lg font-semibold ${isProfitable ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                  {isProfitable ? '+' : ''}${totalPnl.toFixed(2)}
                </div>
              </div>
              <div className="bg-[var(--color-bg-card)] rounded-lg p-3 text-center">
                <div className="text-xs text-[var(--color-text-muted)] mb-1">Test Sharpe</div>
                <div className={`text-lg font-semibold ${period.out_of_sample_sharpe >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                  {period.out_of_sample_sharpe.toFixed(2)}
                </div>
              </div>
              <div className="bg-[var(--color-bg-card)] rounded-lg p-3 text-center">
                <div className="text-xs text-[var(--color-text-muted)] mb-1">Win Rate</div>
                <div className="text-lg font-semibold text-[var(--color-text-primary)]">
                  {period.out_of_sample_metrics.win_rate}%
                </div>
              </div>
              <div className="bg-[var(--color-bg-card)] rounded-lg p-3 text-center">
                <div className="text-xs text-[var(--color-text-muted)] mb-1">Trades</div>
                <div className="text-lg font-semibold text-[var(--color-text-primary)]">
                  {period.oos_trade_count}
                </div>
              </div>
            </div>

            {/* IS vs OOS Comparison */}
            <div className="grid grid-cols-3 gap-3">
              <div className="bg-[var(--color-bg-card)]/60 rounded-lg p-3 text-center">
                <div className="text-xs text-[var(--color-text-muted)] mb-1">Train Sharpe</div>
                <div className="text-lg font-semibold text-[var(--color-info)]">
                  {period.in_sample_sharpe.toFixed(2)}
                </div>
              </div>
              <div className="bg-[var(--color-bg-card)]/60 rounded-lg p-3 text-center">
                <div className="text-xs text-[var(--color-text-muted)] mb-1">Test Sharpe</div>
                <div className={`text-lg font-semibold ${period.out_of_sample_sharpe >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                  {period.out_of_sample_sharpe.toFixed(2)}
                </div>
              </div>
              <div className="bg-[var(--color-bg-card)]/60 rounded-lg p-3 text-center">
                <div className="text-xs text-[var(--color-text-muted)] mb-1">Efficiency</div>
                <div className={`text-lg font-semibold ${
                  period.in_sample_sharpe > 0 && period.out_of_sample_sharpe / period.in_sample_sharpe >= 0.5
                    ? 'text-[var(--color-buy)]'
                    : 'text-[var(--color-warning)]'
                }`}>
                  {period.in_sample_sharpe > 0
                    ? `${((period.out_of_sample_sharpe / period.in_sample_sharpe) * 100).toFixed(0)}%`
                    : 'N/A'}
                </div>
              </div>
            </div>

            {/* Trades Table */}
            {period.oos_trades.length > 0 && (
              <div>
                <h4 className="text-sm font-medium text-[var(--color-text-primary)] mb-2">Trades ({period.oos_trades.length})</h4>
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
                      {period.oos_trades.map((trade, idx) => {
                        const pnl = parseFloat(trade.pnl);
                        const isWin = pnl > 0;
                        return (
                          <tr
                            key={idx}
                            className={`border-b border-[var(--color-border)]/50 ${isWin ? 'bg-[var(--color-buy)]/10' : 'bg-[var(--color-sell)]/10'}`}
                          >
                            <td className="py-2 pr-3 text-[var(--color-text-muted)]">{idx + 1}</td>
                            <td className={`py-2 pr-3 ${trade.isLong ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                              {trade.isLong ? 'Long' : 'Short'}
                            </td>
                            <td className="py-2 pr-3 text-xs font-mono text-[var(--color-text-secondary)]">
                              {formatTradeTime(trade.entryTime)}
                            </td>
                            <td className="py-2 pr-3 text-xs font-mono text-[var(--color-text-secondary)]">
                              {trade.exitTime ? formatTradeTime(trade.exitTime) : '-'}
                            </td>
                            <td className="py-2 pr-3 font-mono text-[var(--color-text-primary)]">
                              {parseFloat(trade.entryPrice).toFixed(instrument.includes('JPY') ? 3 : 5)}
                            </td>
                            <td className="py-2 pr-3 font-mono text-[var(--color-text-primary)]">
                              {trade.exitPrice
                                ? parseFloat(trade.exitPrice).toFixed(instrument.includes('JPY') ? 3 : 5)
                                : '-'}
                            </td>
                            <td className={`py-2 pr-3 ${isWin ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                              {pnl >= 0 ? '+' : ''}${pnl.toFixed(2)}
                            </td>
                            <td className="py-2 pr-3 text-[var(--color-text-muted)]">
                              {formatDuration(trade.entryTime, trade.exitTime)}
                            </td>
                            <td className="py-2">
                              {trade.exitTime && (
                                <button
                                  onClick={() => handleOpenChart(trade, idx)}
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

            {period.oos_trades.length === 0 && (
              <div className="text-center text-[var(--color-text-muted)] py-8">
                No trades were executed in this window.
              </div>
            )}
          </div>
        </div>

        {/* Footer */}
        <div className="px-6 py-4 border-t border-[var(--color-border)]">
          <div className="flex justify-end">
            <button
              onClick={onClose}
              className="px-4 py-2 border border-[var(--color-border)] rounded hover:border-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)] transition-colors text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)]"
            >
              Close
            </button>
          </div>
        </div>

        {/* Market feed drawer */}
        <ModalTerminalDrawer
          modalId={`wf-window:${strategy.id}:${period.window.window_num}`}
          topOffset={73}
        />
      </div>
    </div>
  );
}
