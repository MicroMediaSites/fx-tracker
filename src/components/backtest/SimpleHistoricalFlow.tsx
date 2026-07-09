/**
 * SimpleHistoricalFlow - Simple historical backtesting with quarter grid.
 *
 * Features:
 * - Instrument and timeframe selection
 * - Quarter grid for running backtests
 * - Custom date range option
 * - Composite annualized calculation
 * - Parameter editing with iteration history
 * - No holdout/contamination tracking
 */
import { useState, useMemo, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  deleteBacktestsForStrategy,
  listBacktests,
  saveBacktest,
} from '../../lib/localStore';
import { useSettingsStore } from '../../stores/settingsStore';
import { Strategy, getParameterizedNumber } from '../../types/strategy';
import { SymbolPicker } from '../ui/SymbolPicker';
import { GranularitySelector } from '../ui/GranularitySelector';
import { DateInput } from '../ui/DateInput';
import { StrategyErrorRecovery } from '../ui/StrategyErrorRecovery';
import { QuarterGrid, QuarterSegment } from './QuarterGrid';
import { BacktestResultsPanel, BacktestResult } from './BacktestResultsPanel';
import type { SingleTestingParams } from './TestableParametersPanel';
import type { TestZone } from './TestZonesPanel';

// Parameter values snapshot for a run
type ParameterValues = Record<string, number>;

interface BacktestRun {
  config: {
    instrument: string;
    granularity: string;
    candleCount?: number;
    dateFrom?: string;
    dateTo?: string;
  };
  result: BacktestResult;
  timestamp: number;
  // Parameter snapshot for iteration tracking
  parameterValues: ParameterValues;
  // Run number for iteration history
  runNumber: number;
}

/**
 * Shape of the JSON persisted in the local `backtest` row's `results` column
 * (AGT-645). Carries everything not covered by the row's own columns so a run
 * — metrics, trades, equity curve, parameter snapshot — rehydrates fully.
 */
interface PersistedRunPayload {
  result: BacktestResult;
  parameterValues: ParameterValues;
  runNumber: number;
  granularity: string;
  timestamp: number;
}

/** Format an epoch-ms date back to the YYYY-MM-DD strings the config uses. */
const msToDateString = (ms: number) => new Date(ms).toISOString().slice(0, 10);

interface SimpleHistoricalFlowProps {
  strategy: Strategy;
  initialBalance?: number;
  testingValues: SingleTestingParams;
  /** Test zones configured for backtesting (separate from chart zones) */
  testZones: TestZone[];
  /** Called when AI recovery suggests a strategy fix - receives corrected strategy JSON */
  onStrategyFix?: (correctedStrategyJson: string) => void;
  /** Called when AI recovery suggests a fix to apply as a new copy (safer option) */
  onStrategyFixAsCopy?: (correctedStrategyJson: string) => void;
}

export const SimpleHistoricalFlow = ({
  strategy,
  initialBalance = 1000,
  testingValues,
  testZones,
  onStrategyFix,
  onStrategyFixAsCopy,
}: SimpleHistoricalFlowProps) => {
  const { mySymbols } = useSettingsStore();

  // Config state
  const [instrument, setInstrument] = useState('EUR_USD');
  const [granularity, setGranularity] = useState('H1');
  const [yearsToShow, setYearsToShow] = useState(3);
  const [dateFrom, setDateFrom] = useState('');
  const [dateTo, setDateTo] = useState('');

  // Execution state
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastStrategyJson, setLastStrategyJson] = useState<string | null>(null);
  const [runs, setRuns] = useState<BacktestRun[]>([]);
  const [runCounter, setRunCounter] = useState(1);

  // Show/hide run history panel
  const [showParams, setShowParams] = useState(true);

  // Get current result (most recent run matching current config)
  const currentResult = useMemo(() => {
    const matchingRuns = runs.filter(
      (r) =>
        r.config.instrument === instrument &&
        r.config.granularity === granularity
    );
    // Return the last (most recent) matching run
    return matchingRuns.length > 0 ? matchingRuns[matchingRuns.length - 1].result : null;
  }, [runs, instrument, granularity]);

  // Calculate composite annualized return
  const compositeAnnualized = useMemo(() => {
    const matchingRuns = runs.filter(
      (r) =>
        r.config.instrument === instrument &&
        r.config.granularity === granularity
    );

    if (matchingRuns.length === 0) return null;

    const totalReturn = matchingRuns.reduce((sum, run) => {
      return sum + parseFloat(run.result.metrics.totalReturnPct);
    }, 0);

    // Simple average for now
    const avgReturn = totalReturn / matchingRuns.length;
    return avgReturn;
  }, [runs, instrument, granularity]);

  // Rehydrate this strategy's saved runs from the local store (AGT-645).
  // Runs used to be in-memory only and vanished on window close; now the
  // backtest UI renders runs/equity/trades from local data end-to-end.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const rows = await listBacktests(strategy.id);
        if (cancelled) return;
        const loaded: BacktestRun[] = rows.map((row, idx) => {
          const payload = JSON.parse(row.results) as PersistedRunPayload;
          return {
            config: {
              instrument: row.instrument,
              granularity: payload.granularity,
              dateFrom: msToDateString(row.start_date),
              dateTo: msToDateString(row.end_date),
            },
            result: payload.result,
            timestamp: payload.timestamp ?? row.created_at,
            parameterValues: payload.parameterValues ?? {},
            runNumber: payload.runNumber ?? idx + 1,
          };
        });
        setRuns(loaded);
        setRunCounter(loaded.reduce((max, r) => Math.max(max, r.runNumber), 0) + 1);
      } catch (err) {
        console.error('[SimpleHistoricalFlow] Failed to load saved runs:', err);
      }
    })();
    return () => {
      cancelled = true;
    };
    // Re-hydrate whenever the strategy changes.
  }, [strategy.id]);

  // Add a run to the list (keeps history for iteration tracking) and persist
  // it to the local store so it survives app restarts.
  const addRun = (result: BacktestResult, config: { dateFrom?: string; dateTo?: string }) => {
    const timestamp = Date.now();
    const newRun: BacktestRun = {
      config: {
        instrument,
        granularity,
        dateFrom: config.dateFrom,
        dateTo: config.dateTo,
      },
      result,
      timestamp,
      parameterValues: { ...testingValues },
      runNumber: runCounter,
    };

    setRunCounter((c) => c + 1);
    setRuns((prev) => [...prev, newRun]);

    const payload: PersistedRunPayload = {
      result,
      parameterValues: newRun.parameterValues,
      runNumber: newRun.runNumber,
      granularity,
      timestamp,
    };
    saveBacktest({
      id: crypto.randomUUID(),
      strategy_id: strategy.id,
      instrument,
      start_date: config.dateFrom ? Date.parse(config.dateFrom) : timestamp,
      end_date: config.dateTo ? Date.parse(config.dateTo) : timestamp,
      results: JSON.stringify(payload),
      created_at: timestamp,
    }).catch((err) => {
      console.error('[SimpleHistoricalFlow] Failed to persist run:', err);
    });
  };

  // Get iteration history for current config (same instrument/granularity/date range)
  // Sorted with most recent first
  const iterationHistory = useMemo(() => {
    if (!dateFrom || !dateTo) return [];
    return runs
      .filter(
        (r) =>
          r.config.instrument === instrument &&
          r.config.granularity === granularity &&
          r.config.dateFrom === dateFrom &&
          r.config.dateTo === dateTo
      )
      .sort((a, b) => b.runNumber - a.runNumber); // Most recent first
  }, [runs, instrument, granularity, dateFrom, dateTo]);

  // Find what changed between two runs
  const getParamChanges = (
    prevParams: ParameterValues,
    currentParams: ParameterValues
  ): string[] => {
    const changes: string[] = [];
    const params = strategy.parameters || [];
    params.forEach((p) => {
      const prev = prevParams[p.id];
      const curr = currentParams[p.id];
      if (prev !== curr) {
        changes.push(`${p.name}: ${prev} → ${curr}`);
      }
    });
    return changes;
  };

  // Find best run in iteration history
  const bestRun = useMemo(() => {
    if (iterationHistory.length === 0) return null;
    return iterationHistory.reduce((best, run) =>
      parseFloat(run.result.metrics.totalReturnPct) >
      parseFloat(best.result.metrics.totalReturnPct)
        ? run
        : best
    );
  }, [iterationHistory]);

  // Run backtest for a quarter
  const runQuarterBacktest = async (quarter: QuarterSegment) => {
    setDateFrom(quarter.startDate);
    setDateTo(quarter.endDate);
    setRunning(true);
    setError(null);

    try {
      // Create parameters with testing values as defaults
      const paramsWithTestingValues = (strategy.parameters || []).map((p) => ({
        ...p,
        default: testingValues[p.id] ?? p.default,
      }));

      const strategyJson = JSON.stringify({
        id: strategy.id,
        user_id: strategy.user_id,
        name: strategy.name,
        description: strategy.description,
        parameters: paramsWithTestingValues,
        indicators: strategy.indicators,
        variables: strategy.variables,
        entry_rules: strategy.entry_rules,
        entry_logic: strategy.entry_logic,
        exit_rules: strategy.exit_rules,
        risk_settings: strategy.risk_settings,
        version: strategy.version,
        is_active: strategy.is_active,
        schema_version: strategy.schema_version,
        strategy_type: strategy.strategy_type || 'rules',
        script_content: strategy.script_content,
      });
      // Store for potential error recovery
      setLastStrategyJson(strategyJson);

      const riskSettings = strategy.risk_settings;
      // Use testing params for risk value if it's parameterized
      const riskValue = getParameterizedNumber(riskSettings.risk_value, paramsWithTestingValues);
      let riskPercent: number;
      if (riskSettings.risk_method === 'percent') {
        riskPercent = riskValue;
      } else {
        riskPercent = (riskValue / initialBalance) * 100;
      }

      // Use test zones (not chart zones) for backtesting to avoid look-ahead bias
      const srZonesJson =
        testZones.length > 0
          ? JSON.stringify(
              testZones.map((z) => ({
                id: z.id,
                upper_price: z.upper_price,
                lower_price: z.lower_price,
              }))
            )
          : undefined;

      const pivotConfigJson = strategy.pivot_config?.enabled
        ? JSON.stringify(strategy.pivot_config)
        : undefined;

      const result = await invoke<BacktestResult>('run_custom_backtest', {
        instrument,
        granularity,
        strategyJson,
        count: undefined,
        dateFrom: quarter.startDate,
        dateTo: quarter.endDate,
        initialBalance,
        riskPercent,
        srZonesJson,
        pivotConfigJson,
      });

      addRun(result, {
        dateFrom: quarter.startDate,
        dateTo: quarter.endDate,
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setRunning(false);
    }
  };

  // Run backtest for a full year
  const runYearBacktest = async (
    year: number,
    startDate: string,
    endDate: string
  ) => {
    const yearSegment: QuarterSegment = {
      label: `'${String(year).slice(2)}`,
      year,
      quarter: 0,
      startDate,
      endDate,
    };
    await runQuarterBacktest(yearSegment);
  };

  // Run backtest for custom date range
  const runCustomBacktest = async () => {
    if (!dateFrom || !dateTo) {
      setError('Please select both start and end dates');
      return;
    }

    const customSegment: QuarterSegment = {
      label: 'Custom',
      year: 0,
      quarter: 0,
      startDate: dateFrom,
      endDate: dateTo,
    };
    await runQuarterBacktest(customSegment);
  };

  // Reset runs (state + the strategy's saved runs in the local store)
  const resetRuns = () => {
    setRuns([]);
    setRunCounter(1);
    deleteBacktestsForStrategy(strategy.id).catch((err) => {
      console.error('[SimpleHistoricalFlow] Failed to delete saved runs:', err);
    });
  };

  // Count runs for current instrument/granularity
  const totalRuns = runs.filter(
    (r) =>
      r.config.instrument === instrument &&
      r.config.granularity === granularity
  ).length;

  return (
    <div className="space-y-6">
      {/* Configuration & Period Selection */}
      <div className="flex gap-6">
        {/* Left: Config controls */}
        <div className="flex flex-col gap-3 w-64 flex-shrink-0">
          {/* Instrument & Timeframe */}
          <div className="flex gap-2">
            <div className="flex-1">
              <label className="block text-xs text-[var(--color-text-muted)] mb-1">Instrument</label>
              <SymbolPicker
                value={instrument}
                onChange={setInstrument}
                symbols={mySymbols}
                showChevron
              />
            </div>
            <div className="flex-1">
              <label className="block text-xs text-[var(--color-text-muted)] mb-1">Timeframe</label>
              <GranularitySelector value={granularity} onChange={setGranularity} />
            </div>
          </div>

          {/* From & To */}
          <div className="flex gap-2">
            <DateInput
              value={dateFrom}
              onChange={setDateFrom}
              label="From"
              className="flex-1"
            />
            <DateInput
              value={dateTo}
              onChange={setDateTo}
              label="To"
              className="flex-1"
            />
          </div>

          {/* Run button */}
          <button
            onClick={runCustomBacktest}
            disabled={running || !dateFrom || !dateTo}
            className="w-full px-3 py-2 text-xs border border-[var(--color-border)] text-[var(--color-text-muted)] hover:border-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-hover)] disabled:opacity-50 disabled:cursor-not-allowed rounded transition-colors"
          >
            Run Custom Range
          </button>
        </div>

        {/* Divider */}
        <div className="w-px bg-[var(--color-border)] self-stretch" />

        {/* Right: Quarter Grid */}
        <div className="flex-1">
          {/* Header with composite stats */}
          {totalRuns > 0 && (
            <div className="flex items-center justify-end gap-2 mb-2">
              <span className="text-xs text-[var(--color-text-muted)]">
                Composite{totalRuns > 1 && ` (${totalRuns})`}:
              </span>
              <span
                className={`text-sm font-semibold ${
                  compositeAnnualized !== null && compositeAnnualized >= 0
                    ? 'text-[var(--color-buy)]'
                    : 'text-[var(--color-sell)]'
                }`}
              >
                {compositeAnnualized !== null
                  ? `${compositeAnnualized >= 0 ? '+' : ''}${compositeAnnualized.toFixed(1)}%`
                  : '-'}
              </span>
              <button
                onClick={resetRuns}
                className="text-xs px-1.5 py-0.5 text-[var(--color-text-muted)] hover:text-[var(--color-text-primary)] hover:bg-[var(--color-bg-hover)] rounded"
                title="Reset all runs"
              >
                Reset
              </button>
            </div>
          )}
          <QuarterGrid
            yearsToShow={yearsToShow}
            runs={runs}
            instrument={instrument}
            granularity={granularity}
            onQuarterClick={runQuarterBacktest}
            onYearClick={runYearBacktest}
            onAddYears={() => setYearsToShow((y) => y + 1)}
            disabled={running}
          />
        </div>
      </div>

      {/* Error with AI Recovery */}
      {error && lastStrategyJson && onStrategyFix ? (
        <StrategyErrorRecovery
          error={error}
          strategyJson={lastStrategyJson}
          onApplyFix={(correctedJson) => {
            onStrategyFix(correctedJson);
            setError(null);
            setLastStrategyJson(null);
          }}
          onApplyFixAsCopy={onStrategyFixAsCopy ? (correctedJson) => {
            onStrategyFixAsCopy(correctedJson);
            setError(null);
            setLastStrategyJson(null);
          } : undefined}
          onDismiss={() => {
            setError(null);
            setLastStrategyJson(null);
          }}
        />
      ) : error ? (
        <div className="p-3 bg-[var(--color-sell)]/20 border border-[var(--color-sell)]/50 rounded text-[var(--color-sell)] text-sm">
          {error}
        </div>
      ) : null}

      {/* Run History (only shown if there are runs with the current date range) */}
      {iterationHistory.length > 0 && (
        <div className="border-t border-[var(--color-border)] pt-4">
          {/* Header */}
          <button
            onClick={() => setShowParams(!showParams)}
            className="w-full flex items-center justify-between py-2 hover:bg-[var(--color-bg-hover)]/30 transition-colors"
          >
            <div className="flex items-center gap-3">
              <span className="text-sm font-medium text-[var(--color-text-primary)]">Run History</span>
              <span className="text-xs text-[var(--color-text-muted)]">
                ({iterationHistory.length} run{iterationHistory.length !== 1 ? 's' : ''})
              </span>
            </div>
            <svg
              className={`w-4 h-4 text-[var(--color-text-muted)] transition-transform ${showParams ? 'rotate-180' : ''}`}
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
            </svg>
          </button>

          {/* Content */}
          {showParams && (
            <div className="pt-3">
              <div className="space-y-1 max-h-64 overflow-y-auto">
                <div className="grid grid-cols-[40px_1fr_70px_70px_70px] gap-2 text-xs text-[var(--color-text-muted)] pb-1 border-b border-[var(--color-border)] sticky top-0 bg-[var(--color-bg-page)]">
                  <span>Run</span>
                  <span>Changed</span>
                  <span className="text-right">P&L</span>
                  <span className="text-right">Return</span>
                  <span className="text-right">Ann.</span>
                </div>
                {iterationHistory.map((run) => {
                  // Find the chronologically previous run (lower run number)
                  const prevRun = iterationHistory.find(
                    (r) => r.runNumber === run.runNumber - 1
                  );
                  const changes = prevRun
                    ? getParamChanges(prevRun.parameterValues, run.parameterValues)
                    : [];
                  const isBaseline = !prevRun; // First run chronologically
                  const isBest = bestRun?.runNumber === run.runNumber;
                  const pnl = parseFloat(run.result.metrics.totalPnl);
                  const returnPct = parseFloat(run.result.metrics.totalReturnPct);
                  const annualizedPct = parseFloat(run.result.metrics.annualizedReturnPct);

                  return (
                    <div
                      key={run.runNumber}
                      className="grid grid-cols-[40px_1fr_70px_70px_70px] gap-2 py-1.5 text-sm items-center pl-2"
                      style={isBest ? { borderLeft: '3px solid var(--color-buy)' } : { borderLeft: '3px solid transparent' }}
                    >
                      <span className="text-[var(--color-text-muted)] flex items-center gap-1">
                        #{run.runNumber}
                        {isBest && (
                          <span className="text-[var(--color-buy)] text-xs" title="Best result">
                            ★
                          </span>
                        )}
                      </span>
                      <span className="text-[var(--color-text-muted)] truncate text-xs" title={changes.join(', ')}>
                        {isBaseline ? '(baseline)' : changes.length > 0 ? changes.join(', ') : '(no change)'}
                      </span>
                      <span className={`text-right ${pnl >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                        {pnl >= 0 ? '+' : ''}${pnl.toFixed(0)}
                      </span>
                      <span className={`text-right ${returnPct >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                        {returnPct >= 0 ? '+' : ''}{returnPct.toFixed(1)}%
                      </span>
                      <span className={`text-right ${annualizedPct >= 0 ? 'text-[var(--color-buy)]' : 'text-[var(--color-sell)]'}`}>
                        {annualizedPct >= 0 ? '+' : ''}{annualizedPct.toFixed(0)}%
                      </span>
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      )}

      {/* Results Panel */}
      <BacktestResultsPanel
        result={currentResult}
        running={running}
        selectedStrategy={strategy}
        instrument={instrument}
        granularity={granularity}
        initialBalance={initialBalance}
      />
    </div>
  );
}
