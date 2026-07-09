/**
 * WalkForwardFlow - Complete Walk-Forward Analysis workflow.
 *
 * Linear flow:
 * 1. Configuration (instrument, timeframe, date range, windows)
 * 2. Run Analysis
 * 3. Results display (best OOS params, parameter stability, periods)
 * 4. Holdout Validation (instrument/timeframe/custom range or quarters)
 *
 * Job Persistence:
 * - Long-running backtests are tracked in the database
 * - If the app refreshes, reconnects to running jobs via Zero sync
 * - Completed job results are auto-loaded when returning to the strategy
 */
import { useMemo, useState, useRef, useEffect } from 'react';
import { WindowDetailModal } from './WindowDetailModal';
import { StrategyErrorRecovery } from '../ui/StrategyErrorRecovery';

// Extracted components
import { WalkForwardConfig } from './WalkForwardConfig';
import { WalkForwardProgressBar } from './WalkForwardProgressBar';
import { WalkForwardResults } from './WalkForwardResults';
import { HoldoutValidation } from './HoldoutValidation';
import { ParameterSweepResults } from './ParameterSweepResults';

// Extracted state hook
import { useWalkForwardState } from './useWalkForwardState';
import type { WalkForwardFlowProps } from './walkForwardTypes';

// History selector combobox
const HistorySelector = ({
  jobs,
  selectedJobId,
  onSelectJob,
}: {
  jobs: import('./walkForwardTypes').BacktestJob[];
  selectedJobId: string | null;
  onSelectJob: (job: import('./walkForwardTypes').BacktestJob) => void;
}) => {
  const [isOpen, setIsOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // Close on click outside
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  const selectedJob = jobs.find(j => j.id === selectedJobId) || jobs[0];
  const formatJob = (job: import('./walkForwardTypes').BacktestJob, isLatest: boolean) => {
    const date = new Date(job.completed_at || job.created_at);
    const params = job.params ? JSON.parse(job.params) : {};
    const prefix = isLatest ? '(Latest) ' : '';
    return `${prefix}${date.toLocaleDateString()} ${date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })} · ${params.instrument || '?'} ${params.granularity || ''}`;
  };

  return (
    <div className="flex items-center gap-3 text-xs" ref={containerRef}>
      <span className="text-[var(--color-text-muted)]">History</span>
      <div className="relative">
        <button
          type="button"
          onClick={() => setIsOpen(!isOpen)}
          className={`flex items-center gap-2 bg-transparent border rounded px-2 py-1.5 text-xs transition-colors focus:outline-none ${
            isOpen
              ? 'border-[var(--color-border-focus)]'
              : 'border-[var(--color-border)] hover:border-[var(--color-border-focus)]'
          }`}
        >
          <span className="text-[var(--color-text-primary)]">
            {selectedJob ? formatJob(selectedJob, selectedJob.id === jobs[0]?.id) : 'Select...'}
          </span>
          <svg
            className={`w-3 h-3 text-[var(--color-text-muted)] transition-transform ${isOpen ? 'rotate-180' : ''}`}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}
          >
            <path strokeLinecap="round" strokeLinejoin="round" d="M19 9l-7 7-7-7" />
          </svg>
        </button>

        {isOpen && (
          <div className="absolute top-full left-0 mt-1 min-w-full bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded shadow-xl z-50 py-1 max-h-48 overflow-auto">
            {jobs.map((job, idx) => (
              <button
                key={job.id}
                type="button"
                onClick={() => {
                  onSelectJob(job);
                  setIsOpen(false);
                }}
                className={`w-full text-left px-3 py-1.5 text-xs whitespace-nowrap transition-colors ${
                  job.id === selectedJobId
                    ? 'text-[var(--color-text-primary)] bg-[var(--color-bg-active)]'
                    : 'text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-hover)]'
                }`}
              >
                {formatJob(job, idx === 0)}
              </button>
            ))}
          </div>
        )}
      </div>
      <span className="text-[var(--color-text-muted)]">
        {jobs.length} runs
      </span>
    </div>
  );
};

export const WalkForwardFlow = ({
  strategy,
  initialBalance = 1000,
  rangeValues,
  useDefaultParams = {},
  testZones,
  onStrategyFix,
  onStrategyFixAsCopy,
  onHoldoutResultsChange,
  onJobInfoChange,
  onWfContextChange,
}: WalkForwardFlowProps) => {
  const state = useWalkForwardState({
    strategy,
    initialBalance,
    rangeValues,
    useDefaultParams,
    testZones,
  });

  // Toggle for holdout validation panel
  const [showHoldout, setShowHoldout] = useState(false);

  // Memoize strategy JSON for error recovery (stable reference)
  const strategyJson = useMemo(() => JSON.stringify(strategy), [strategy]);

  // Notify parent when selected job changes (for AI context)
  useEffect(() => {
    if (!onJobInfoChange) return;

    if (!state.selectedJobId || !state.wfResult) {
      onJobInfoChange(null);
      return;
    }

    // Build a brief metrics summary for AI
    const result = state.wfResult;
    const oosReturn = state.oosReturnPct !== null ? parseFloat(String(state.oosReturnPct)) : null;
    const annualized = state.oosAnnualizedReturnPct !== null ? parseFloat(String(state.oosAnnualizedReturnPct)) : null;

    const metricsSummary = [
      `OOS Return: ${oosReturn !== null && !isNaN(oosReturn) ? `${oosReturn >= 0 ? '+' : ''}${oosReturn.toFixed(1)}%` : 'N/A'}`,
      `Annualized: ${annualized !== null && !isNaN(annualized) ? `${annualized >= 0 ? '+' : ''}${annualized.toFixed(1)}%` : 'N/A'}`,
      `Windows: ${result.periods?.length || 0} periods`,
      `Best OOS params: ${state.bestOosParams ? JSON.stringify(state.bestOosParams) : 'N/A'}`,
    ].join(', ');

    onJobInfoChange({
      jobId: state.selectedJobId,
      hasResults: true,
      metricsSummary,
    });
  }, [state.selectedJobId, state.wfResult, state.oosReturnPct, state.oosAnnualizedReturnPct, state.bestOosParams, onJobInfoChange]);

  // Notify parent when WF result or selected window changes (for AI context)
  useEffect(() => {
    if (!onWfContextChange) return;
    onWfContextChange({
      wfResult: state.wfResult,
      selectedWindow: state.selectedWindow,
    });
  }, [state.wfResult, state.selectedWindow, onWfContextChange]);

  // Notify parent when holdout results change (for AI context)
  useEffect(() => {
    if (!onHoldoutResultsChange) return;

    if (state.holdoutRuns.length === 0) {
      onHoldoutResultsChange(null);
      return;
    }

    // Build summary of holdout results
    const periods = state.holdoutRuns.map((run) => {
      const returnPct = parseFloat(run.result.metrics.totalReturnPct);
      return `${run.config.dateFrom} to ${run.config.dateTo}: ${returnPct >= 0 ? '+' : ''}${returnPct.toFixed(1)}% (${run.result.metrics.winRate}% win rate, ${run.result.metrics.totalTrades} trades)`;
    });

    const summary = [
      `Periods tested: ${state.holdoutRuns.length}`,
      `Composite return: ${state.holdoutComposite !== null ? `${state.holdoutComposite >= 0 ? '+' : ''}${state.holdoutComposite.toFixed(1)}%` : 'N/A'}`,
      `Instrument: ${state.holdoutInstrument}, Timeframe: ${state.holdoutGranularity}`,
      '',
      'Period breakdown:',
      ...periods,
    ].join('\n');

    onHoldoutResultsChange(summary);
  }, [state.holdoutRuns, state.holdoutComposite, state.holdoutInstrument, state.holdoutGranularity, onHoldoutResultsChange]);

  return (
    <div className="space-y-6">
      {/* Walk-Forward Configuration */}
      <WalkForwardConfig
        anchored={state.anchored}
        setAnchored={state.setAnchored}
        instrument={state.instrument}
        setInstrument={state.setInstrument}
        granularity={state.granularity}
        setGranularity={state.setGranularity}
        devDateFrom={state.devDateFrom}
        setDevDateFrom={state.setDevDateFrom}
        devDateTo={state.devDateTo}
        setDevDateTo={state.setDevDateTo}
        trainMonths={state.trainMonths}
        setTrainMonths={state.setTrainMonths}
        testMonths={state.testMonths}
        setTestMonths={state.setTestMonths}
        objective={state.objective}
        setObjective={state.setObjective}
        wfRunning={state.wfRunning}
        hasOptimizableParams={state.hasOptimizableParams}
        expectedWindows={state.expectedWindows}
        totalCombinations={state.totalCombinations}
        onRunWalkForward={state.runWalkForward}
      />

      {/* Progress */}
      {state.wfRunning && state.wfProgress && (
        <WalkForwardProgressBar
          progress={state.wfProgress}
          onCancel={state.cancelWalkForward}
        />
      )}

      {/* Error with AI Recovery */}
      {state.wfError && onStrategyFix ? (
        <StrategyErrorRecovery
          error={state.wfError}
          strategyJson={strategyJson}
          onApplyFix={(correctedJson) => {
            console.log('[WalkForwardFlow] onApplyFix called with:', correctedJson?.substring(0, 100));
            onStrategyFix(correctedJson);
            state.clearWfError();
          }}
          onApplyFixAsCopy={onStrategyFixAsCopy ? (correctedJson) => {
            console.log('[WalkForwardFlow] onApplyFixAsCopy called with:', correctedJson?.substring(0, 100));
            onStrategyFixAsCopy(correctedJson);
            state.clearWfError();
          } : undefined}
          onDismiss={state.clearWfError}
        />
      ) : state.wfError ? (
        <div className="p-3 bg-[var(--color-sell)]/20 border border-[var(--color-sell)]/50 rounded text-[var(--color-sell)] text-sm">
          {state.wfError}
        </div>
      ) : null}

      {/* Step 2: Walk-Forward Results */}
      {state.wfResult && (
        <>
          {/* Header row: History selector + Holdout button */}
          <div className="flex items-center justify-between">
            {/* History selector - only show if multiple completed jobs */}
            {state.completedJobs.length > 1 ? (
              <HistorySelector
                jobs={state.completedJobs}
                selectedJobId={state.selectedJobId}
                onSelectJob={state.selectHistoricalJob}
              />
            ) : (
              <div />
            )}

            {/* Holdout validation toggle */}
            <button
              type="button"
              onClick={() => setShowHoldout(!showHoldout)}
              className={`flex items-center gap-2 text-xs font-medium px-3 py-1.5 rounded border transition-colors ${
                showHoldout
                  ? 'bg-purple-500/20 border-purple-500 text-purple-400'
                  : 'border-purple-500 text-purple-500 hover:bg-purple-500/10'
              }`}
            >
              <svg className="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
              </svg>
              {showHoldout ? 'Hide holdout test' : 'Test on holdout data'}
            </button>
          </div>

          {/* Holdout Validation - shown above results when toggled */}
          {showHoldout && (
            <HoldoutValidation
              instrument={state.holdoutInstrument}
              granularity={state.holdoutGranularity}
              bestParams={state.bestOosParams}
              selectedHoldout={state.selectedHoldout}
              holdoutRuns={state.holdoutRuns}
              holdoutRunning={state.holdoutRunning}
              holdoutError={state.holdoutError}
              holdoutComposite={state.holdoutComposite}
              onInstrumentChange={state.setHoldoutInstrument}
              onGranularityChange={state.setHoldoutGranularity}
              onToggleQuarter={state.toggleHoldoutQuarter}
              onRunValidation={state.runHoldoutValidation}
              onRunCustomValidation={state.runCustomHoldoutValidation}
            />
          )}

          {/* Parameter Sweep Results (replaces main results when active) */}
          {state.sweepResult ? (
            <ParameterSweepResults
              result={state.sweepResult}
              progress={state.sweepProgress}
              objective={state.objective}
              onBack={state.clearSweepResult}
            />
          ) : (
            <WalkForwardResults
              result={state.wfResult}
              initialBalance={initialBalance}
              oosReturnPct={state.oosReturnPct}
              oosAnnualizedReturnPct={state.oosAnnualizedReturnPct}
              onSelectWindow={state.setSelectedWindow}
              baselineResult={state.baselineResult}
              baselineRunning={state.baselineRunning}
              rangedParamIds={state.rangedParamIds}
              parameters={strategy.parameters || []}
              onRunSweep={state.runParameterSweep}
              sweepRunning={state.sweepRunning}
            />
          )}
        </>
      )}

      {/* Window Detail Modal */}
      {state.selectedWindow && (
        <WindowDetailModal
          period={state.selectedWindow}
          instrument={state.instrument}
          granularity={state.granularity}
          strategy={strategy}
          onClose={() => state.setSelectedWindow(null)}
        />
      )}
    </div>
  );
};
