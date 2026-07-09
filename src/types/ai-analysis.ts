/**
 * Shared AI Analysis Types
 *
 * Common interfaces for structured AI analysis responses across:
 * - Trade analysis (TradeReviewModal)
 * - Backtest analysis (BacktestResultsPanel)
 * - Walk-forward window analysis (WindowDetailModal)
 */

// ============================================================================
// Backtest Analysis
// ============================================================================

export interface BacktestScore {
  overall: number;           // 1-10 overall assessment
  strategy_quality: number;  // How well does the strategy perform?
  risk_management: number;   // Drawdown, position sizing
  consistency: number;       // Win rate stability, equity curve smoothness
}

export interface BacktestAIAnalysis {
  summary: string;
  performance_assessment: string;
  strategy_assessment: string;
  trade_patterns: string[];
  key_observations: string[];
  score: BacktestScore;
}

// ============================================================================
// Walk-Forward Window Analysis
// ============================================================================

export interface WindowScore {
  overall: number;           // 1-10 overall assessment
  parameter_fit: number;     // How well did params transfer to OOS?
  execution_quality: number; // Trade timing, consistency
}

export interface WindowAIAnalysis {
  summary: string;
  performance_assessment: string;
  parameter_assessment: string;
  trade_patterns: string[];
  key_observations: string[];
  score: WindowScore;
}

// ============================================================================
// Trade Analysis (from TradeReviewModal)
// ============================================================================

export interface IndicatorFinding {
  indicator: string;
  assessment: string;
  supported_trade: boolean;
  at_entry: string;
  at_exit: string;
}

export interface TradeScore {
  entry_timing: number;
  exit_timing: number;
  risk_management: number;
  overall: number;
}

export interface TradeAIAnalysis {
  summary: string;
  entry_assessment: string;
  exit_assessment: string;
  indicator_analysis: IndicatorFinding[];
  conflicting_indicators: string[];
  learning_points: string[];
  score: TradeScore;
}

// ============================================================================
// Persisted AI Analysis (stored in database)
// ============================================================================

export interface AIAnalysisConfigSnapshot {
  instrument?: string;
  granularity?: string;
  dateFrom?: string;
  dateTo?: string;
  candleCount?: number;
  initialBalance?: number;
  // Trade-specific fields
  openTime?: number;
  closeTime?: number;
  openPrice?: string;
  closePrice?: string;
}

export interface AIAnalysisMetricsSnapshot {
  // Backtest metrics
  totalPnl?: string;
  totalReturnPct?: string;
  winRate?: string;
  profitFactor?: string;
  sharpeRatio?: string;
  maxDrawdownPct?: string;
  totalTrades?: number;
  // Trade metrics
  realizedPl?: string;
  units?: string;
}

/**
 * Persisted AI analysis record from the database.
 * Stores context snapshots so users can review past insights.
 */
export interface PersistedAIAnalysis {
  id: string;
  user_id: string;
  analysis_type: 'backtest' | 'trade';
  entity_id: string;
  config_snapshot?: string;   // JSON string of AIAnalysisConfigSnapshot
  metrics_snapshot?: string;  // JSON string of AIAnalysisMetricsSnapshot
  question_type: string;
  custom_question?: string;
  response: string;           // Markdown for backtest, JSON for trade
  ai_model: string;
  created_at: number;
}

/**
 * Parsed version of PersistedAIAnalysis with typed snapshots
 */
export interface ParsedAIAnalysis extends Omit<PersistedAIAnalysis, 'config_snapshot' | 'metrics_snapshot'> {
  configSnapshot?: AIAnalysisConfigSnapshot;
  metricsSnapshot?: AIAnalysisMetricsSnapshot;
}
