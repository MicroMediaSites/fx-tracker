/**
 * HoldoutValidation - Holdout validation section for walk-forward analysis
 *
 * Validates the best OOS parameters on unseen data. Features:
 * - Instrument/timeframe selectors (can differ from WFT config)
 * - Custom date range option
 * - Quarter grid for quick selection
 * - Explicit display of parameters being used
 */
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SymbolPicker } from '../ui/SymbolPicker';
import { GranularitySelector } from '../ui/GranularitySelector';
import { QuarterGrid, QuarterSegment } from './QuarterGrid';
import { BacktestRun } from './walkForwardTypes';
import { ParamChips } from '../ui/ParamChips';
import { useSettingsStore } from '../../stores/settingsStore';

interface HoldoutValidationProps {
  // From WFT config (default values)
  instrument: string;
  granularity: string;

  // Best OOS parameters to validate
  bestParams: Record<string, number> | null;

  // Quarter selection
  selectedHoldout: Set<string>;
  holdoutRuns: BacktestRun[];
  holdoutRunning: boolean;
  holdoutError: string | null;
  holdoutComposite: number | null;

  // Actions
  onInstrumentChange: (value: string) => void;
  onGranularityChange: (value: string) => void;
  onToggleQuarter: (quarter: QuarterSegment) => void;
  onRunValidation: () => void;
  onRunCustomValidation: (dateFrom: string, dateTo: string) => void;
}

export const HoldoutValidation = ({
  instrument,
  granularity,
  bestParams,
  selectedHoldout,
  holdoutRuns,
  holdoutRunning,
  holdoutError,
  holdoutComposite,
  onInstrumentChange,
  onGranularityChange,
  onToggleQuarter,
  onRunValidation,
  onRunCustomValidation,
}: HoldoutValidationProps) => {
  const { mySymbols } = useSettingsStore();

  // Custom date range state
  const [customDateFrom, setCustomDateFrom] = useState('');
  const [customDateTo, setCustomDateTo] = useState('');
  const [validationMode, setValidationMode] = useState<'quarters' | 'custom'>('quarters');

  const handleCustomValidation = () => {
    if (customDateFrom && customDateTo) {
      onRunCustomValidation(customDateFrom, customDateTo);
    }
  };

  // Combine all holdout trades for chart
  const openHoldoutInChart = () => {
    if (holdoutRuns.length === 0) return;

    // Collect all trades from all holdout runs
    const allTrades = holdoutRuns.flatMap((run) =>
      run.result.trades
        .filter((trade) => trade.exitTime && trade.exitPrice)
        .map((trade) => ({
          entryTime: Math.floor(new Date(trade.entryTime).getTime() / 1000),
          exitTime: Math.floor(new Date(trade.exitTime).getTime() / 1000),
          entryPrice: parseFloat(trade.entryPrice),
          exitPrice: parseFloat(trade.exitPrice),
          direction: trade.direction.toLowerCase() as 'long' | 'short',
          pnl: parseFloat(trade.pnl),
        }))
    );

    if (allTrades.length > 0) {
      localStorage.setItem('chart_trades', JSON.stringify(allTrades));
    } else {
      localStorage.removeItem('chart_trades');
    }

    // Get date range spanning all holdout runs
    const allDates = holdoutRuns
      .flatMap((run) => [run.config.dateFrom, run.config.dateTo])
      .filter((d): d is string => !!d)
      .map((d) => new Date(d).getTime());
    const minDate = new Date(Math.min(...allDates)).toISOString();
    const maxDate = new Date(Math.max(...allDates)).toISOString();

    invoke('open_chart_window', {
      instrument,
      granularity,
      from: minDate,
      to: maxDate,
    });
  };

  return (
    <div className="space-y-4">
      {/* Parameters being used */}
      {bestParams && Object.keys(bestParams).length > 0 && (
        <ParamChips
          params={bestParams}
          size="xs"
          layout="wrap"
          title="Parameters Being Validated"
          titleTooltip="These are the best parameters from walk-forward optimization. To change them, resolve the parameters to static values in your strategy's parameter settings."
          showContainer
        />
      )}

      {/* Main layout: Left controls | Divider | Right quarter grid */}
      <div className="flex gap-6">
        {/* Left: Config controls */}
        <div className="flex flex-col gap-3 w-48 flex-shrink-0">
          {/* Instrument */}
          <div>
            <label className="block text-xs text-[var(--color-text-muted)] mb-1">Instrument</label>
            <SymbolPicker
              value={instrument}
              onChange={onInstrumentChange}
              symbols={mySymbols}
              showChevron
            />
          </div>

          {/* Timeframe */}
          <div>
            <label className="block text-xs text-[var(--color-text-muted)] mb-1">Timeframe</label>
            <GranularitySelector value={granularity} onChange={onGranularityChange} />
          </div>

          {/* Mode Toggle */}
          <div>
            <label className="block text-xs text-[var(--color-text-muted)] mb-1">Range</label>
            <div className="flex border border-[var(--color-border)] rounded overflow-hidden">
              <button
                onClick={() => setValidationMode('quarters')}
                className={`flex-1 px-2 py-2 text-xs transition-colors ${
                  validationMode === 'quarters'
                    ? 'bg-[var(--color-bg-active)] text-[var(--color-text-primary)]'
                    : 'text-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)]'
                }`}
              >
                Quarters
              </button>
              <button
                onClick={() => setValidationMode('custom')}
                className={`flex-1 px-2 py-2 text-xs border-l border-[var(--color-border)] transition-colors ${
                  validationMode === 'custom'
                    ? 'bg-[var(--color-bg-active)] text-[var(--color-text-primary)]'
                    : 'text-[var(--color-text-muted)] hover:bg-[var(--color-bg-hover)]'
                }`}
              >
                Custom
              </button>
            </div>
          </div>

          {/* Custom date inputs (only shown in custom mode) */}
          {validationMode === 'custom' && (
            <>
              <div>
                <label className="block text-xs text-[var(--color-text-muted)] mb-1">From</label>
                <input
                  type="date"
                  value={customDateFrom}
                  onChange={(e) => setCustomDateFrom(e.target.value)}
                  className="w-full bg-transparent border border-[var(--color-border)] rounded px-3 py-2 text-xs text-[var(--color-text-primary)] focus:outline-none focus:border-[var(--color-border-focus)] hover:border-[var(--color-border-focus)] transition-colors"
                />
              </div>
              <div>
                <label className="block text-xs text-[var(--color-text-muted)] mb-1">To</label>
                <input
                  type="date"
                  value={customDateTo}
                  onChange={(e) => setCustomDateTo(e.target.value)}
                  className="w-full bg-transparent border border-[var(--color-border)] rounded px-3 py-2 text-xs text-[var(--color-text-primary)] focus:outline-none focus:border-[var(--color-border-focus)] hover:border-[var(--color-border-focus)] transition-colors"
                />
              </div>
              <button
                onClick={handleCustomValidation}
                disabled={holdoutRunning || !customDateFrom || !customDateTo}
                className="w-full px-3 py-2 text-xs border border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-hover)] disabled:opacity-50 disabled:cursor-not-allowed rounded transition-colors"
              >
                {holdoutRunning ? 'Validating...' : 'Run Custom Range'}
              </button>
            </>
          )}
        </div>

        {/* Divider */}
        <div className="w-px bg-[var(--color-border)] self-stretch" />

        {/* Right: Quarter Grid or Custom mode placeholder */}
        <div className="flex-1">
          {validationMode === 'quarters' ? (
            <>
              {/* Header */}
              <div className="flex items-start justify-between mb-3">
                <div>
                  <h4 className="text-sm font-medium text-[var(--color-text-primary)]">Holdout Periods</h4>
                  <p className="text-xs text-[var(--color-text-muted)] mt-0.5">
                    Select quarters to validate optimized parameters on unseen data
                  </p>
                </div>
                {/* Composite stats */}
                {holdoutRuns.length > 0 && (
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-[var(--color-text-muted)]">
                      Composite{holdoutRuns.length > 1 && ` (${holdoutRuns.length})`}:
                    </span>
                    <span
                      className={`text-sm font-mono font-semibold ${
                        holdoutComposite !== null && holdoutComposite >= 0
                          ? 'text-[var(--color-buy)]'
                          : 'text-[var(--color-sell)]'
                      }`}
                    >
                      {holdoutComposite !== null
                        ? `${holdoutComposite >= 0 ? '+' : ''}${holdoutComposite.toFixed(1)}%`
                        : '-'}
                    </span>
                    <button
                      onClick={openHoldoutInChart}
                      className="text-xs px-1.5 py-0.5 text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-hover)] rounded"
                      title="Open in chart"
                    >
                      Chart
                    </button>
                  </div>
                )}
              </div>

              <QuarterGrid
                yearsToShow={2}
                runs={holdoutRuns}
                instrument={instrument}
                granularity={granularity}
                onQuarterClick={onToggleQuarter}
                disabled={holdoutRunning}
                mode="holdout-selection"
                selectedQuarters={selectedHoldout}
              />

              {/* Run Validation Button - always rendered to prevent layout shift */}
              <div className="flex justify-end mt-3">
                <button
                  onClick={onRunValidation}
                  disabled={holdoutRunning || selectedHoldout.size === 0}
                  className={`px-4 py-2 text-xs font-medium border rounded transition-all ${
                    selectedHoldout.size > 0
                      ? 'border-[var(--color-warning)] text-[var(--color-warning)] hover:bg-[var(--color-warning)]/10 disabled:opacity-50'
                      : 'border-transparent text-transparent cursor-default'
                  } disabled:cursor-not-allowed`}
                >
                  {holdoutRunning
                    ? 'Validating...'
                    : `Validate ${selectedHoldout.size || 1} Quarter${selectedHoldout.size > 1 ? 's' : ''}`}
                </button>
              </div>
            </>
          ) : (
            /* Custom mode - show results summary or instructions */
            <div className="flex items-center justify-center h-full text-sm text-[var(--color-text-muted)]">
              {holdoutRuns.length > 0 ? (
                <div className="text-center">
                  <div className="flex items-center justify-center gap-2 mb-1">
                    <span>Composite:</span>
                    <span
                      className={`font-mono font-semibold ${
                        holdoutComposite !== null && holdoutComposite >= 0
                          ? 'text-[var(--color-buy)]'
                          : 'text-[var(--color-sell)]'
                      }`}
                    >
                      {holdoutComposite !== null
                        ? `${holdoutComposite >= 0 ? '+' : ''}${holdoutComposite.toFixed(1)}%`
                        : '-'}
                    </span>
                  </div>
                  <div className="text-xs">{holdoutRuns.length} periods tested</div>
                </div>
              ) : (
                <span>Select dates and run validation</span>
              )}
            </div>
          )}
        </div>
      </div>

      {/* Error Display */}
      {holdoutError && (
        <div className="p-3 bg-[var(--color-sell)]/20 border border-[var(--color-sell)]/50 rounded text-[var(--color-sell)] text-sm">
          {holdoutError}
        </div>
      )}

      {/* Holdout Results Table */}
      {holdoutRuns.length > 0 && (
        <div className="border-t border-[var(--color-border)] pt-4">
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-[var(--color-text-muted)] text-left text-xs uppercase tracking-wide">
                  <th className="pb-2 pr-3 font-normal">Period</th>
                  <th className="pb-2 pr-3 font-normal">Instrument</th>
                  <th className="pb-2 pr-3 font-normal">P&L</th>
                  <th className="pb-2 pr-3 font-normal">Return</th>
                  <th className="pb-2 pr-3 font-normal">Win Rate</th>
                  <th className="pb-2 font-normal">Trades</th>
                </tr>
              </thead>
              <tbody>
                {holdoutRuns.map((run, idx) => (
                  <tr
                    key={idx}
                    className={`border-l-2 ${
                      parseFloat(run.result.metrics.totalReturnPct) >= 0
                        ? 'border-l-[var(--color-buy)]'
                        : 'border-l-[var(--color-sell)]'
                    }`}
                  >
                    <td className="py-2 pr-3 pl-2 text-xs font-mono text-[var(--color-text-secondary)]">
                      {run.config.dateFrom} - {run.config.dateTo}
                    </td>
                    <td className="py-2 pr-3 text-xs text-[var(--color-text-secondary)]">
                      {run.config.instrument} / {run.config.granularity}
                    </td>
                    <td
                      className={`py-2 pr-3 font-mono ${
                        parseFloat(run.result.metrics.totalPnl) >= 0
                          ? 'text-[var(--color-buy)]'
                          : 'text-[var(--color-sell)]'
                      }`}
                    >
                      ${parseFloat(run.result.metrics.totalPnl).toFixed(2)}
                    </td>
                    <td
                      className={`py-2 pr-3 font-mono ${
                        parseFloat(run.result.metrics.totalReturnPct) >= 0
                          ? 'text-[var(--color-buy)]'
                          : 'text-[var(--color-sell)]'
                      }`}
                    >
                      {parseFloat(run.result.metrics.totalReturnPct) >= 0 ? '+' : ''}
                      {run.result.metrics.totalReturnPct}%
                    </td>
                    <td className="py-2 pr-3 font-mono text-[var(--color-text-primary)]">{run.result.metrics.winRate}%</td>
                    <td className="py-2 font-mono text-[var(--color-text-primary)]">{run.result.metrics.totalTrades}</td>
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
