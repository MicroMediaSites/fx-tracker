/**
 * Shared types for WalkForward components and backtest job tracking
 */
import { WalkForwardProgress, WalkForwardResult, WalkForwardPeriod } from '../../types/strategy';
import { BacktestResult } from './BacktestResultsPanel';

/**
 * BacktestJob - Database record for tracking long-running backtest jobs.
 * Mirrors the `backtest_job` row in the wickd local store (AGT-645; the shape
 * is the old Zero row minus user_id — the local store is single-user).
 * Note: Uses null for optional fields to match the row type.
 */
export interface BacktestJob {
  id: string;
  strategy_id: string;
  job_type: string; // 'walk_forward' | 'train_test' | 'simple_backtest' | 'optimization'
  status: string;   // 'pending' | 'running' | 'completed' | 'failed' | 'cancelled'
  params: string;   // JSON: job parameters
  progress: number; // 0-100 completion percentage
  progress_detail: string | null; // JSON: detailed progress
  result: string | null;  // JSON: full result when completed
  error_message: string | null;
  created_at: number;
  updated_at: number;
  completed_at: number | null;
}

// Job heartbeat event payload
export interface JobHeartbeat {
  jobId: string;
  strategyId: string;
  status: string;
  progress: number;
  progressDetail: WalkForwardProgress;
}

// Job completed event payload
export interface JobCompleted {
  jobId: string;
  strategyId: string;
  status: 'completed' | 'failed' | 'cancelled';
  hasResult: boolean;
  result?: import('../../types/strategy').WalkForwardResult;
  error?: string;
}

export interface BacktestRun {
  config: {
    instrument: string;
    granularity: string;
    dateFrom?: string;
    dateTo?: string;
  };
  result: BacktestResult;
}

/** Info about the currently selected backtest job for AI context */
export interface BacktestJobInfo {
  jobId: string;
  hasResults: boolean;
  /** Brief metrics summary for AI context */
  metricsSummary?: string;
}

export interface WalkForwardFlowProps {
  strategy: import('../../types/strategy').Strategy;
  initialBalance?: number;
  rangeValues: import('./TestableParametersPanel').RangeTestingParams;
  useDefaultParams?: import('./TestableParametersPanel').UseDefaultParams;
  /** Test zones configured for backtesting (separate from chart zones) */
  testZones: import('./TestZonesPanel').TestZone[];
  /** Called when AI recovery suggests a strategy fix - receives corrected strategy JSON */
  onStrategyFix?: (correctedStrategyJson: string) => void;
  /** Called when AI recovery suggests a fix to apply as a new copy (safer option) */
  onStrategyFixAsCopy?: (correctedStrategyJson: string) => void;
  /** Called when holdout validation results change - provides summary for AI context */
  onHoldoutResultsChange?: (summary: string | null) => void;
  /** Called when the selected backtest job changes - provides job info for AI context */
  onJobInfoChange?: (jobInfo: BacktestJobInfo | null) => void;
  /** Called when WF result or selected window changes - provides context for AI */
  onWfContextChange?: (context: { wfResult: WalkForwardResult | null; selectedWindow: WalkForwardPeriod | null }) => void;
}
