/**
 * useWalkForwardState - Custom hook for Walk-Forward Analysis state management.
 *
 * Encapsulates all state, effects, and business logic for the walk-forward workflow.
 * This includes:
 * - Configuration state (instrument, timeframe, dates, windows, objective)
 * - Job tracking via useBacktestJob hook
 * - Walk-forward execution (progress, results, errors)
 * - Holdout validation state
 *
 * Note: AI analysis is handled in WalkForwardFlow via TerminalChat.
 */
import { useState, useEffect, useMemo, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  Strategy,
  OptimizationObjective,
  WalkForwardResult,
  WalkForwardProgress,
  WalkForwardPeriod,
  ParameterSweepResult,
  ParameterSweepProgress,
  getParameterizedNumber,
} from '../../types/strategy';
import type { TestZone } from './TestZonesPanel';
import { QuarterSegment, getQuarterKey } from './QuarterGrid';
import type { BacktestResult } from './BacktestResultsPanel';
import { getDefaultWalkForwardDates, calculateExpectedWindows } from './walkForwardUtils';
import { BacktestRun, BacktestJob } from './walkForwardTypes';
import { useBacktestJob, BacktestJobCallbacks } from './hooks';

export interface UseWalkForwardStateOptions {
  strategy: Strategy;
  initialBalance?: number;
  rangeValues: RangeValues;
  useDefaultParams?: Record<string, boolean>;
  /** Test zones configured for backtesting (separate from chart zones) */
  testZones: TestZone[];
}

// Range configuration for each parameter
export interface ParameterRange {
  min: number;
  max: number;
  step: number;
}

export type RangeValues = Record<string, ParameterRange>;

export interface WalkForwardState {
  // Config
  anchored: boolean;
  setAnchored: (value: boolean) => void;
  instrument: string;
  setInstrument: (value: string) => void;
  granularity: string;
  setGranularity: (value: string) => void;
  devDateFrom: string;
  setDevDateFrom: (value: string) => void;
  devDateTo: string;
  setDevDateTo: (value: string) => void;
  trainMonths: number;
  setTrainMonths: (value: number) => void;
  testMonths: number;
  setTestMonths: (value: number) => void;
  objective: OptimizationObjective;
  setObjective: (value: OptimizationObjective) => void;

  // Computed config values
  hasOptimizableParams: boolean;
  totalCombinations: number;
  expectedWindows: number;

  // Walk-forward execution
  wfRunning: boolean;
  wfProgress: WalkForwardProgress | null;
  wfResult: WalkForwardResult | null;
  wfError: string | null;
  clearWfError: () => void;
  selectedWindow: WalkForwardPeriod | null;
  setSelectedWindow: (period: WalkForwardPeriod | null) => void;

  // Baseline comparison
  baselineResult: WalkForwardResult | null;
  baselineRunning: boolean;
  rangedParamIds: string[];

  // Parameter sweep
  sweepResult: ParameterSweepResult | null;
  sweepRunning: boolean;
  sweepProgress: ParameterSweepProgress | null;
  runParameterSweep: (paramId: string) => Promise<void>;
  clearSweepResult: () => void;

  // Best OOS parameters (from best performing window)
  bestOosParams: Record<string, number> | null;

  // Holdout validation
  holdoutInstrument: string;
  setHoldoutInstrument: (value: string) => void;
  holdoutGranularity: string;
  setHoldoutGranularity: (value: string) => void;
  selectedHoldout: Set<string>;
  holdoutRuns: BacktestRun[];
  holdoutRunning: boolean;
  holdoutError: string | null;
  holdoutComposite: number | null;

  // Backtest history
  completedJobs: BacktestJob[];
  selectedJobId: string | null;
  selectHistoricalJob: (job: BacktestJob) => void;

  // Computed display values
  oosReturnPct: string | null;
  oosAnnualizedReturnPct: string | null;

  // Actions
  runWalkForward: () => Promise<void>;
  cancelWalkForward: () => Promise<void>;
  toggleHoldoutQuarter: (quarter: QuarterSegment) => void;
  runHoldoutValidation: () => Promise<void>;
  runCustomHoldoutValidation: (dateFrom: string, dateTo: string) => Promise<void>;
}

export const useWalkForwardState = ({
  strategy,
  initialBalance = 1000,
  rangeValues,
  useDefaultParams = {},
  testZones,
}: UseWalkForwardStateOptions): WalkForwardState => {

  // Config state
  const defaultDates = getDefaultWalkForwardDates();
  const [anchored, setAnchored] = useState(false);
  const [instrument, setInstrument] = useState('EUR_USD');
  const [granularity, setGranularity] = useState('H1');
  const [devDateFrom, setDevDateFrom] = useState(defaultDates.from);
  const [devDateTo, setDevDateTo] = useState(defaultDates.to);
  const [trainMonths, setTrainMonths] = useState(6);
  const [testMonths, setTestMonths] = useState(1);
  const [objective, setObjective] = useState<OptimizationObjective>('sharpe_ratio');

  // Walk-forward execution state (specific to this validation type)
  const [wfProgress, setWfProgress] = useState<WalkForwardProgress | null>(null);
  const [wfResult, setWfResult] = useState<WalkForwardResult | null>(null);
  const [selectedWindow, setSelectedWindow] = useState<WalkForwardPeriod | null>(null);

  // Baseline comparison state
  const [baselineResult, setBaselineResult] = useState<WalkForwardResult | null>(null);
  const [baselineRunning, setBaselineRunning] = useState(false);

  // Parameter sweep state
  const [sweepResult, setSweepResult] = useState<ParameterSweepResult | null>(null);
  const [sweepRunning, setSweepRunning] = useState(false);
  const [sweepProgress, setSweepProgress] = useState<ParameterSweepProgress | null>(null);

  // Holdout-specific instrument/granularity (can differ from WFT config)
  const [holdoutInstrument, setHoldoutInstrument] = useState(instrument);
  const [holdoutGranularity, setHoldoutGranularity] = useState(granularity);

  // Callbacks for job lifecycle events
  const jobCallbacks = useMemo<BacktestJobCallbacks<WalkForwardProgress, WalkForwardResult>>(() => ({
    onProgress: (progress) => {
      setWfProgress(progress);
    },
    onComplete: (result) => {
      setWfResult(result);
      setWfProgress(null);
    },
    onError: () => {
      setWfProgress(null);
    },
    onReconnect: (job: BacktestJob, progressDetail: WalkForwardProgress | null) => {
      // Restore progress from reconnected job
      if (progressDetail) {
        setWfProgress(progressDetail);
      } else {
        setWfProgress({
          phase: 'optimization',
          windowNum: 0,
          totalWindows: 0,
          optimizationCurrent: 0,
          optimizationTotal: 0,
          percent: job.progress,
          trainStart: '',
          trainEnd: '',
          testStart: '',
          testEnd: '',
        });
      }
      // Restore config from job params
      if (job.params) {
        try {
          const params = JSON.parse(job.params);
          if (params.instrument) setInstrument(params.instrument);
          if (params.granularity) setGranularity(params.granularity);
          if (params.dateFrom) setDevDateFrom(params.dateFrom);
          if (params.dateTo) setDevDateTo(params.dateTo);
          if (params.trainMonths) setTrainMonths(params.trainMonths);
          if (params.testMonths) setTestMonths(params.testMonths);
          if (params.objective) setObjective(params.objective);
        } catch {
          // Ignore parse errors
        }
      }
    },
    getSuccessNotification: (result) => {
      const pnl = parseFloat(result.oos_total_pnl || '0');
      const pnlStr = pnl >= 0 ? `+$${pnl.toFixed(2)}` : `-$${Math.abs(pnl).toFixed(2)}`;
      return {
        title: 'Walk-Forward Complete',
        body: `${strategy.name}: ${pnlStr} OOS P&L, ${(result.sharpe_efficiency ?? 0).toFixed(0)}% efficiency`,
      };
    },
    getFailureNotification: (error) => ({
      title: 'Walk-Forward Failed',
      body: `${strategy.name}: ${error}`,
    }),
  }), [strategy.name]);

  // Generic job tracking via useBacktestJob hook
  const {
    currentJobId,
    isRunning: wfRunning,
    error: wfError,
    allJobs,
    startJob,
    setError: setWfError,
    finishRunning,
    resetState: resetJobState,
  } = useBacktestJob({
    strategyId: strategy.id,
    callbacks: jobCallbacks,
  });

  // Holdout state
  const [selectedHoldout, setSelectedHoldout] = useState<Set<string>>(new Set());
  const [holdoutRuns, setHoldoutRuns] = useState<BacktestRun[]>([]);
  const [holdoutRunning, setHoldoutRunning] = useState(false);
  const [holdoutError, setHoldoutError] = useState<string | null>(null);

  // Track which historical job is currently selected (null = latest/new run)
  const [selectedJobId, setSelectedJobId] = useState<string | null>(null);

  // Reset UI state when strategy changes
  useEffect(() => {
    // Reset job state via hook
    resetJobState();
    // Reset walk-forward specific state
    setWfProgress(null);
    setWfResult(null);
    setSelectedWindow(null);
    // Reset baseline state
    setBaselineResult(null);
    setBaselineRunning(false);
    // Reset sweep state
    setSweepResult(null);
    setSweepRunning(false);
    setSweepProgress(null);
    // Note: Range values are now managed by parent (BacktestApp), so no reset here
    // Reset holdout state
    setSelectedHoldout(new Set());
    setHoldoutRuns([]);
    setHoldoutRunning(false);
    setHoldoutError(null);
  }, [strategy.id, resetJobState]);

  // Sync holdout instrument/granularity with main config when main config changes
  // (only if no holdout validation has been run yet - let user customize after that)
  useEffect(() => {
    if (holdoutRuns.length === 0) {
      setHoldoutInstrument(instrument);
    }
  }, [instrument, holdoutRuns.length]);

  useEffect(() => {
    if (holdoutRuns.length === 0) {
      setHoldoutGranularity(granularity);
    }
  }, [granularity, holdoutRuns.length]);

  // Check if strategy has any parameters (ranges are now configured in the panel)
  const hasOptimizableParams = (strategy.parameters || []).length > 0;

  // Track which params are in range mode (not using default)
  const rangedParamIds = useMemo(() => {
    return (strategy.parameters || [])
      .filter((p) => !useDefaultParams[p.id])
      .map((p) => p.id);
  }, [strategy.parameters, useDefaultParams]);

  // Calculate total parameter combinations using panel's range values
  // Excludes params marked as "use default" (they contribute 1 combination)
  const totalCombinations = useMemo(() => {
    const params = strategy.parameters || [];
    if (params.length === 0) return 0;
    return params.reduce((acc, p) => {
      // Skip params that are set to "use default" - they only use default value (1 combination)
      if (useDefaultParams[p.id]) return acc;
      const range = rangeValues[p.id];
      if (!range || range.step <= 0) return acc;
      const steps = Math.floor((range.max - range.min) / range.step) + 1;
      return acc * Math.max(1, steps);
    }, 1);
  }, [strategy.parameters, rangeValues, useDefaultParams]);

  // Calculate expected windows
  const expectedWindows = useMemo(() => {
    return calculateExpectedWindows(devDateFrom, devDateTo, trainMonths, testMonths);
  }, [devDateFrom, devDateTo, trainMonths, testMonths]);

  // Compute best OOS parameters from the best performing window
  const bestOosParams = useMemo(() => {
    if (!wfResult?.periods?.length) return null;
    const bestPeriod = [...wfResult.periods].sort(
      (a, b) => b.out_of_sample_sharpe - a.out_of_sample_sharpe
    )[0];
    return bestPeriod.optimized_params;
  }, [wfResult]);

  // Calculate OOS return percentage (total return over the entire period)
  const oosReturnPct = useMemo(() => {
    if (!wfResult) return null;
    const pnl = parseFloat(wfResult.oos_total_pnl || '0');
    if (isNaN(pnl) || initialBalance === 0) return '0.0';
    return ((pnl / initialBalance) * 100).toFixed(1);
  }, [wfResult, initialBalance]);

  // Calculate annualized OOS return using CAGR formula
  const oosAnnualizedReturnPct = useMemo(() => {
    if (!wfResult || !wfResult.periods?.length) return null;

    const pnl = parseFloat(wfResult.oos_total_pnl || '0');
    if (isNaN(pnl) || initialBalance === 0) return '0.0';

    // Calculate total return ratio
    const totalReturnRatio = (initialBalance + pnl) / initialBalance;
    if (totalReturnRatio <= 0) return null; // Can't annualize negative total (lost everything)

    // Get the OOS period duration from first to last window's test dates
    const sortedPeriods = [...wfResult.periods].sort(
      (a, b) => new Date(a.window.test_start).getTime() - new Date(b.window.test_start).getTime()
    );
    const firstTestStart = new Date(sortedPeriods[0].window.test_start);
    const lastTestEnd = new Date(sortedPeriods[sortedPeriods.length - 1].window.test_end);

    // Calculate years (using 365.25 days per year for accuracy)
    const msPerYear = 365.25 * 24 * 60 * 60 * 1000;
    const years = (lastTestEnd.getTime() - firstTestStart.getTime()) / msPerYear;

    if (years <= 0) return '0.0';

    // CAGR = (Ending Value / Beginning Value)^(1/years) - 1
    const cagr = Math.pow(totalReturnRatio, 1 / years) - 1;
    return (cagr * 100).toFixed(1);
  }, [wfResult, initialBalance]);

  // Calculate holdout composite
  const holdoutComposite = useMemo(() => {
    if (holdoutRuns.length === 0) return null;
    const totalReturn = holdoutRuns.reduce(
      (sum, r) => sum + parseFloat(r.result.metrics.totalReturnPct),
      0
    );
    return totalReturn / holdoutRuns.length;
  }, [holdoutRuns]);

  // Get completed jobs for history dropdown (sorted newest first)
  const completedJobs = useMemo(() => {
    if (!allJobs) return [];
    return allJobs
      .filter(j => j.status === 'completed' && j.result)
      .sort((a, b) => (b.completed_at || 0) - (a.completed_at || 0));
  }, [allJobs]);

  // Select a historical job and load its results
  const selectHistoricalJob = useCallback((job: BacktestJob) => {
    if (!job.result) return;
    try {
      const result = JSON.parse(job.result) as WalkForwardResult;
      setWfResult(result);
      setSelectedJobId(job.id);

      // Restore config from job params
      if (job.params) {
        const params = JSON.parse(job.params);
        if (params.instrument) setInstrument(params.instrument);
        if (params.granularity) setGranularity(params.granularity);
        if (params.dateFrom) setDevDateFrom(params.dateFrom);
        if (params.dateTo) setDevDateTo(params.dateTo);
        if (params.trainMonths) setTrainMonths(params.trainMonths);
        if (params.testMonths) setTestMonths(params.testMonths);
        if (params.objective) setObjective(params.objective);
      }

      // Clear holdout state when switching jobs
      setSelectedHoldout(new Set());
      setHoldoutRuns([]);
      // Clear sweep state from previous job
      setSweepResult(null);
      setSweepRunning(false);
      setSweepProgress(null);
    } catch {
      console.error('Failed to parse historical job result');
    }
  }, []);

  // Load result from completed job on mount (cached results)
  useEffect(() => {
    if (wfResult || wfRunning) return;

    if (allJobs && allJobs.length > 0) {
      const completedJobs = allJobs
        .filter(j => j.status === 'completed' && j.result)
        .sort((a, b) => (b.completed_at || 0) - (a.completed_at || 0));

      if (completedJobs.length > 0) {
        const latestJob = completedJobs[0];
        try {
          const result = JSON.parse(latestJob.result!) as WalkForwardResult;
          setWfResult(result);

          if (latestJob.params) {
            const params = JSON.parse(latestJob.params);
            if (params.instrument) setInstrument(params.instrument);
            if (params.granularity) setGranularity(params.granularity);
            if (params.dateFrom) setDevDateFrom(params.dateFrom);
            if (params.dateTo) setDevDateTo(params.dateTo);
            if (params.trainMonths) setTrainMonths(params.trainMonths);
            if (params.testMonths) setTestMonths(params.testMonths);
            if (params.objective) setObjective(params.objective);
          }
        } catch {
          console.error('Failed to parse job result');
        }
      }
    }
  }, [allJobs, wfResult, wfRunning]);

  // Run walk-forward analysis
  const runWalkForward = useCallback(async () => {
    if (wfRunning) return; // Prevent duplicate runs

    if (!hasOptimizableParams) {
      setWfError('No optimizable parameters. Add min/max/step to parameters first.');
      return;
    }
    if (!devDateFrom || !devDateTo) {
      setWfError('Please select a development period.');
      return;
    }
    // Reset walk-forward specific state
    setWfProgress(null);
    setWfResult(null);
    setBaselineResult(null);
    setBaselineRunning(false);
    setSelectedHoldout(new Set());
    setHoldoutRuns([]);

    // Create job params for tracking
    const jobParams = {
      instrument,
      granularity,
      dateFrom: devDateFrom,
      dateTo: devDateTo,
      trainMonths,
      testMonths,
      objective,
      anchored,
      initialBalance,
    };

    // Start job via hook (creates DB record and sets running state)
    let jobId: string;
    try {
      jobId = await startJob('walk_forward', jobParams);
    } catch (e) {
      setWfError(e instanceof Error ? e.message : String(e));
      return;
    }

    try {
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

      const pivotConfigJson = strategy.pivot_config
        ? JSON.stringify(strategy.pivot_config)
        : undefined;

      // Inject panel's range values into parameters for the backend
      // For "use default" params, set min=max=default so only 1 value is tested
      const parametersWithRanges = (strategy.parameters || []).map((p) => {
        if (useDefaultParams[p.id]) {
          // Use default only - min=max=default, step=1
          return { ...p, min: p.default, max: p.default, step: 1 };
        }
        const range = rangeValues[p.id];
        if (range) {
          return { ...p, min: range.min, max: range.max, step: range.step };
        }
        return p;
      });

      const result = await invoke<WalkForwardResult>('run_walk_forward', {
        instrument,
        granularity,
        strategyJson: JSON.stringify(strategy),
        parametersJson: JSON.stringify(parametersWithRanges),
        dateFrom: devDateFrom,
        dateTo: devDateTo,
        initialBalance,
        srZonesJson,
        pivotConfigJson,
        trainMonths,
        testMonths,
        stepMonths: testMonths,
        objective,
        minTradesPerWindow: 5,
        anchored,
        jobId: currentJobId || jobId,
        strategyId: strategy.id,
        strategyName: strategy.name,
      });

      // Note: Result will be set by the job-completed event handler in useBacktestJob
      // This is just a fallback in case events don't fire properly
      setWfResult(result);

      // Automatically run baseline comparison if any params were in range mode
      const currentRangedParams = (strategy.parameters || []).filter((p) => !useDefaultParams[p.id]);
      if (currentRangedParams.length > 0) {
        setBaselineRunning(true);
        try {
          // Build baseline parameters: all params at their defaults (min=max=default, step=1)
          const baselineParams = (strategy.parameters || []).map((p) => ({
            ...p,
            min: p.default,
            max: p.default,
            step: 1,
          }));

          const baselineResultData = await invoke<WalkForwardResult>('run_walk_forward', {
            instrument,
            granularity,
            strategyJson: JSON.stringify(strategy),
            parametersJson: JSON.stringify(baselineParams),
            dateFrom: devDateFrom,
            dateTo: devDateTo,
            initialBalance,
            srZonesJson,
            pivotConfigJson,
            trainMonths,
            testMonths,
            stepMonths: testMonths,
            objective,
            minTradesPerWindow: 5,
            anchored,
            // No jobId for baseline — backend infers baseline from missing jobId
            // and skips the concurrency guard accordingly
            strategyId: strategy.id,
            strategyName: strategy.name,
          });
          setBaselineResult(baselineResultData);
        } catch (baselineErr) {
          // Baseline failure is non-fatal — just log it
          console.warn('[WalkForward] Baseline comparison failed:', baselineErr);
        } finally {
          setBaselineRunning(false);
        }
      }
    } catch (e) {
      const errorMsg = e instanceof Error ? e.message : String(e);
      if (!errorMsg.includes('cancelled')) {
        setWfError(errorMsg);
      }
    } finally {
      // Signal that invoke completed (cleanup running state if events didn't handle it)
      finishRunning();
      setWfProgress(null);
    }
  }, [
    wfRunning,
    hasOptimizableParams,
    devDateFrom,
    devDateTo,
    instrument,
    granularity,
    trainMonths,
    testMonths,
    objective,
    anchored,
    initialBalance,
    startJob,
    strategy,
    testZones,
    currentJobId,
    finishRunning,
    rangeValues,
    useDefaultParams,
  ]);

  // Cancel running walk-forward
  const cancelWalkForward = useCallback(async () => {
    try {
      await invoke('cancel_walk_forward');
    } catch (e) {
      console.error('Failed to cancel:', e);
    }
  }, []);

  // Toggle holdout quarter selection
  const toggleHoldoutQuarter = useCallback((quarter: QuarterSegment) => {
    const key = getQuarterKey(quarter);
    setSelectedHoldout((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }, []);

  // Create strategy with best OOS params applied
  const strategyWithBestParams = useMemo(() => {
    if (!bestOosParams) return strategy;
    const updatedParams = (strategy.parameters || []).map((p) => {
      if (bestOosParams[p.id] !== undefined) {
        return { ...p, default: bestOosParams[p.id] };
      }
      return p;
    });
    return { ...strategy, parameters: updatedParams };
  }, [strategy, bestOosParams]);

  // Run holdout validation on selected quarters
  const runHoldoutValidation = useCallback(async () => {
    if (selectedHoldout.size === 0) {
      setHoldoutError('Please select at least one holdout quarter.');
      return;
    }
    if (!bestOosParams) {
      setHoldoutError('No parameters available from WFT results.');
      return;
    }

    setHoldoutRunning(true);
    setHoldoutError(null);
    setHoldoutRuns([]);

    try {
      const riskSettings = strategyWithBestParams.risk_settings;
      const riskValue = getParameterizedNumber(riskSettings.risk_value, strategyWithBestParams.parameters);
      let riskPercent: number;
      if (riskSettings.risk_method === 'percent') {
        riskPercent = riskValue;
      } else {
        riskPercent = (riskValue / initialBalance) * 100;
      }

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

      const pivotConfigJson = strategyWithBestParams.pivot_config?.enabled
        ? JSON.stringify(strategyWithBestParams.pivot_config)
        : undefined;

      const runs: BacktestRun[] = [];

      for (const key of selectedHoldout) {
        const [yearStr, quarterStr] = key.split('-Q');
        const year = parseInt(yearStr);
        const quarter = parseInt(quarterStr);

        const quarterBounds = [
          { start: '01-01', end: '03-31' },
          { start: '04-01', end: '06-30' },
          { start: '07-01', end: '09-30' },
          { start: '10-01', end: '12-31' },
        ];

        const bounds = quarterBounds[quarter - 1];
        const dateFrom = `${year}-${bounds.start}`;
        const dateTo = `${year}-${bounds.end}`;

        const result = await invoke<BacktestResult>('run_custom_backtest', {
          instrument: holdoutInstrument,
          granularity: holdoutGranularity,
          strategyJson: JSON.stringify(strategyWithBestParams),
          count: undefined,
          dateFrom,
          dateTo,
          initialBalance,
          riskPercent,
          srZonesJson,
          pivotConfigJson,
        });

        runs.push({
          config: { instrument: holdoutInstrument, granularity: holdoutGranularity, dateFrom, dateTo },
          result,
        });
      }

      setHoldoutRuns(runs);
    } catch (e) {
      setHoldoutError(e instanceof Error ? e.message : String(e));
    } finally {
      setHoldoutRunning(false);
    }
  }, [selectedHoldout, strategyWithBestParams, bestOosParams, initialBalance, testZones, holdoutInstrument, holdoutGranularity]);

  // Run holdout validation on a custom date range
  const runCustomHoldoutValidation = useCallback(async (dateFrom: string, dateTo: string) => {
    if (!dateFrom || !dateTo) {
      setHoldoutError('Please provide valid date range.');
      return;
    }
    if (!bestOosParams) {
      setHoldoutError('No parameters available from WFT results.');
      return;
    }

    setHoldoutRunning(true);
    setHoldoutError(null);
    setHoldoutRuns([]);

    try {
      const riskSettings = strategyWithBestParams.risk_settings;
      const riskValue = getParameterizedNumber(riskSettings.risk_value, strategyWithBestParams.parameters);
      let riskPercent: number;
      if (riskSettings.risk_method === 'percent') {
        riskPercent = riskValue;
      } else {
        riskPercent = (riskValue / initialBalance) * 100;
      }

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

      const pivotConfigJson = strategyWithBestParams.pivot_config?.enabled
        ? JSON.stringify(strategyWithBestParams.pivot_config)
        : undefined;

      const result = await invoke<BacktestResult>('run_custom_backtest', {
        instrument: holdoutInstrument,
        granularity: holdoutGranularity,
        strategyJson: JSON.stringify(strategyWithBestParams),
        count: undefined,
        dateFrom,
        dateTo,
        initialBalance,
        riskPercent,
        srZonesJson,
        pivotConfigJson,
      });

      setHoldoutRuns([{
        config: { instrument: holdoutInstrument, granularity: holdoutGranularity, dateFrom, dateTo },
        result,
      }]);
    } catch (e) {
      setHoldoutError(e instanceof Error ? e.message : String(e));
    } finally {
      setHoldoutRunning(false);
    }
  }, [strategyWithBestParams, bestOosParams, initialBalance, testZones, holdoutInstrument, holdoutGranularity]);

  // Listen for parameter sweep progress events
  //
  // BUG-044: Uses cancelled flag to prevent orphaned listeners when the
  // effect re-fires before the async listen() resolves.
  useEffect(() => {
    let cancelled = false;
    let unlistenFn: (() => void) | null = null;

    listen<ParameterSweepProgress>('parameter-sweep-progress', (event) => {
      setSweepProgress(event.payload);
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlistenFn = fn;
      }
    });

    return () => {
      cancelled = true;
      if (unlistenFn) unlistenFn();
    };
  }, []);

  // Run parameter sweep for a specific param
  const runParameterSweep = useCallback(async (paramId: string) => {
    const param = (strategy.parameters || []).find((p) => p.id === paramId);
    if (!param) return;

    const range = rangeValues[paramId];
    if (!range || range.step <= 0) return;

    // Generate list of values from range
    const values: number[] = [];
    for (let v = range.min; v <= range.max + range.step * 0.001; v += range.step) {
      values.push(Math.round(v * 1e8) / 1e8); // avoid float drift
    }
    if (values.length === 0) return;

    setSweepRunning(true);
    setSweepResult(null);
    setSweepProgress(null);

    try {
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

      const pivotConfigJson = strategy.pivot_config
        ? JSON.stringify(strategy.pivot_config)
        : undefined;

      // All params including their ranges (sweep command will override sweep param)
      const parametersWithRanges = (strategy.parameters || []).map((p) => {
        const r = rangeValues[p.id];
        if (r) {
          return { ...p, min: r.min, max: r.max, step: r.step };
        }
        return p;
      });

      const result = await invoke<ParameterSweepResult>('run_parameter_sweep', {
        instrument,
        granularity,
        strategyJson: JSON.stringify(strategy),
        parametersJson: JSON.stringify(parametersWithRanges),
        dateFrom: devDateFrom,
        dateTo: devDateTo,
        initialBalance,
        srZonesJson,
        pivotConfigJson,
        trainMonths,
        testMonths,
        stepMonths: testMonths,
        objective,
        minTradesPerWindow: 5,
        anchored,
        sweepParamId: paramId,
        sweepValues: values,
      });

      setSweepResult(result);
    } catch (e) {
      const errorMsg = e instanceof Error ? e.message : String(e);
      if (!errorMsg.includes('cancelled')) {
        console.error('[ParameterSweep] Failed:', errorMsg);
      }
    } finally {
      setSweepRunning(false);
      setSweepProgress(null);
    }
  }, [
    strategy,
    rangeValues,
    testZones,
    instrument,
    granularity,
    devDateFrom,
    devDateTo,
    initialBalance,
    trainMonths,
    testMonths,
    objective,
    anchored,
  ]);

  return {
    // Config
    anchored,
    setAnchored,
    instrument,
    setInstrument,
    granularity,
    setGranularity,
    devDateFrom,
    setDevDateFrom,
    devDateTo,
    setDevDateTo,
    trainMonths,
    setTrainMonths,
    testMonths,
    setTestMonths,
    objective,
    setObjective,

    // Computed config values
    hasOptimizableParams,
    totalCombinations,
    expectedWindows,

    // Walk-forward execution
    wfRunning,
    wfProgress,
    wfResult,
    wfError,
    clearWfError: () => setWfError(null),
    selectedWindow,
    setSelectedWindow,

    // Best OOS parameters
    bestOosParams,

    // Baseline comparison
    baselineResult,
    baselineRunning,
    rangedParamIds,

    // Parameter sweep
    sweepResult,
    sweepRunning,
    sweepProgress,
    runParameterSweep,
    clearSweepResult: () => setSweepResult(null),

    // Holdout validation
    holdoutInstrument,
    setHoldoutInstrument,
    holdoutGranularity,
    setHoldoutGranularity,
    selectedHoldout,
    holdoutRuns,
    holdoutRunning,
    holdoutError,
    holdoutComposite,

    // Backtest history
    completedJobs,
    selectedJobId,
    selectHistoricalJob,

    // Computed display values
    oosReturnPct,
    oosAnnualizedReturnPct,

    // Actions
    runWalkForward,
    cancelWalkForward,
    toggleHoldoutQuarter,
    runHoldoutValidation,
    runCustomHoldoutValidation,
  };
};
