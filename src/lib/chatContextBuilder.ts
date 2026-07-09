/**
 * Context builders for the AI chat terminal.
 * Each function creates a ChatContext object that provides relevant
 * information to the AI based on the current window state.
 */

import type { ChatContext } from '../hooks/useTerminalChat';

// ============================================================================
// Context Types (matching Rust backend)
// ============================================================================

export interface AccountContext extends ChatContext {
  type: 'account';
  balance?: string;
  unrealized_pl?: string;
  open_trade_count?: number;
  environment: string;
}

export interface ChartingContext extends ChatContext {
  type: 'charting';
  instrument: string;
  granularity: string;
  strategy_name?: string;
  strategy_id?: string;
  strategy_risk_settings?: Record<string, unknown>;
  indicators: string[];
  /** Current indicator values (latest reading for each indicator output) */
  indicator_values?: Record<string, string>;
  current_price?: string;
  signal_direction?: string;
  /** Recent candles for chart analysis (only loaded when needed) */
  recent_candles?: Array<{
    time: string;
    open: string;
    high: string;
    low: string;
    close: string;
  }>;
}

export interface BacktestingContext extends ChatContext {
  type: 'backtesting';
  /** Strategy ID for AI tool queries */
  strategy_id?: string;
  strategy_name?: string;
  strategy_description?: string;
  strategy_risk_settings?: Record<string, unknown>;
  strategy_type?: string;
  script_content?: string;
  methodology?: string;
  // Note: nested struct fields use camelCase (ParameterInfo has #[serde(rename_all = "camelCase")])
  parameters: Array<{
    name: string;
    currentValue: string;
    defaultValue?: string;
  }>;
  has_results: boolean;
  /** Backtest job ID for walk-forward tests (AI can use this to fetch results) */
  backtest_job_id?: string;
  /** Metrics summary - pre-loaded for simple/holdout, or fetched by AI for WFT */
  metrics_summary?: string;
  /** Holdout validation results summary (if available) */
  holdout_summary?: string;
  /** Human-readable strategy entry/exit rules with resolved parameter values */
  strategy_rules?: string;
  /** Full parameter definitions with min/max/step constraints */
  parameter_definitions?: string;
  /** Per-window walk-forward results summary */
  window_summary?: string;
  /** Details of the currently selected/viewed walk-forward window */
  selected_window?: string;
}

export interface TicketContext extends ChatContext {
  type: 'ticket';
  instrument: string;
  direction?: string;
  units?: string;
  stop_loss?: string;
  take_profit?: string;
  current_price?: string;
  strategy_name?: string;
  strategy_risk_settings?: Record<string, unknown>;
}

export interface WatcherContext extends ChatContext {
  type: 'watcher';
  // Note: nested struct fields use camelCase (WatcherInfo/SignalInfo have #[serde(rename_all = "camelCase")])
  running_strategies: Array<{
    strategyName: string;
    instruments: string[];
    timeframe: string;
  }>;
  pending_signals: Array<{
    instrument: string;
    direction: string;
    strategyName: string;
    entryPrice?: string;
  }>;
  /** User's configured symbols list from settings */
  available_instruments?: string[];
}

export interface TradeAnalysisContext extends ChatContext {
  type: 'tradeAnalysis';
  trade_count: number;
  date_range?: string;
  win_rate?: string;
  profit_factor?: string;
  filters_active: boolean;
  /** Which breakdown tab is currently active (session, day, hour, instrument, etc.) */
  active_breakdown?: string;
}

/** Context for the Trade Subset modal (filtered group of trades) */
export interface TradeSubsetContext extends ChatContext {
  type: 'tradeSubset';
  /** Description of the subset (e.g., "Asian session trades", "Tuesday trades") */
  subset_description: string;
  trade_count: number;
  wins: number;
  losses: number;
  win_rate: string;
  avg_win: string;
  avg_loss: string;
  expectancy: string;
  profit_factor: string;
  total_pl: string;
  /** List of instruments in this subset */
  instruments: string[];
  /** Direction breakdown */
  long_count: number;
  short_count: number;
}

/** Context for the Trade Review modal (single trade deep-dive) */
export interface TradeReviewContext extends ChatContext {
  type: 'tradeReview';
  instrument: string;
  direction: string;
  is_winner: boolean;
  entry_price: string;
  exit_price: string;
  realized_pl: string;
  duration_minutes: number;
  /** Trade quality metrics */
  mae_pips: string;
  mfe_pips: string;
  capture_efficiency?: string;
  risk_efficiency?: string;
  /** Entry timing */
  immediate_drawdown_pips: string;
  candles_to_profit?: number;
  near_swing_point?: string;
  /** Market context at entry */
  rsi_14?: string;
  rsi_zone?: string;
  trend?: string;
  /** Post-exit analysis */
  post_exit_favorable_pips: string;
  post_exit_adverse_pips: string;
  /** AI score if available - nested struct uses camelCase */
  ai_score?: {
    entry: number;
    exit: number;
    riskManagement: number;
    overall: number;
  };
  /** Key insights from analysis */
  key_insights: string[];
  /** Indicator analysis from Score Trade (if available) */
  indicator_analysis?: Array<{
    indicator: string;
    assessment: string;
    supportedTrade: boolean;
    atEntry: string;
    atExit: string;
  }>;
  /** Indicators that conflicted with trade direction */
  conflicting_indicators?: string[];
}

// ============================================================================
// Context Builders
// ============================================================================

/**
 * Build context for the Account window
 */
export function buildAccountContext(params: {
  balance?: string;
  unrealizedPl?: string;
  openTradeCount?: number;
  environment: string;
}): AccountContext {
  return {
    type: 'account',
    balance: params.balance,
    unrealized_pl: params.unrealizedPl,
    open_trade_count: params.openTradeCount,
    environment: params.environment,
  };
}

/**
 * Build context for the Charting window
 */
export function buildChartingContext(params: {
  instrument: string;
  granularity: string;
  strategyName?: string;
  strategyId?: string;
  strategyRiskSettings?: Record<string, unknown>;
  indicators?: string[];
  indicatorValues?: Record<string, string>;
  currentPrice?: string;
  signalDirection?: string;
  recentCandles?: Array<{ time: string; open: string; high: string; low: string; close: string }>;
}): ChartingContext {
  return {
    type: 'charting',
    instrument: params.instrument,
    granularity: params.granularity,
    strategy_name: params.strategyName,
    strategy_id: params.strategyId,
    strategy_risk_settings: params.strategyRiskSettings,
    indicators: params.indicators || [],
    indicator_values: params.indicatorValues,
    current_price: params.currentPrice,
    signal_direction: params.signalDirection,
    recent_candles: params.recentCandles,
  };
}

/**
 * Build context for the Backtesting/Research window
 */
export function buildBacktestingContext(params: {
  strategyId?: string;
  strategyName?: string;
  strategyDescription?: string;
  strategyRiskSettings?: Record<string, unknown>;
  strategyType?: string;
  scriptContent?: string;
  methodology?: string;
  parameters?: Array<{
    name: string;
    currentValue: string;
    defaultValue?: string;
  }>;
  hasResults?: boolean;
  /** Backtest job ID for walk-forward tests */
  backtestJobId?: string;
  metricsSummary?: string;
  holdoutSummary?: string;
  /** Serialized strategy rules */
  strategyRules?: string;
  /** Serialized parameter definitions */
  parameterDefinitions?: string;
  /** Serialized window summary */
  windowSummary?: string;
  /** Serialized selected window details */
  selectedWindow?: string;
}): BacktestingContext {
  return {
    type: 'backtesting',
    strategy_id: params.strategyId,
    strategy_name: params.strategyName,
    strategy_description: params.strategyDescription,
    strategy_risk_settings: params.strategyRiskSettings,
    strategy_type: params.strategyType,
    script_content: params.scriptContent,
    methodology: params.methodology,
    parameters: params.parameters || [],
    has_results: params.hasResults || false,
    backtest_job_id: params.backtestJobId,
    metrics_summary: params.metricsSummary,
    holdout_summary: params.holdoutSummary,
    strategy_rules: params.strategyRules,
    parameter_definitions: params.parameterDefinitions,
    window_summary: params.windowSummary,
    selected_window: params.selectedWindow,
  };
}

/**
 * Build context for the Trading Ticket window
 */
export function buildTicketContext(params: {
  instrument: string;
  direction?: string;
  units?: string;
  stopLoss?: string;
  takeProfit?: string;
  currentPrice?: string;
  strategyName?: string;
  strategyRiskSettings?: Record<string, unknown>;
}): TicketContext {
  return {
    type: 'ticket',
    instrument: params.instrument,
    direction: params.direction,
    units: params.units,
    stop_loss: params.stopLoss,
    take_profit: params.takeProfit,
    current_price: params.currentPrice,
    strategy_name: params.strategyName,
    strategy_risk_settings: params.strategyRiskSettings,
  };
}

/**
 * Build context for the Strategy Watcher window
 */
export function buildWatcherContext(params: {
  runningStrategies?: Array<{
    strategyName: string;
    instruments: string[];
    timeframe: string;
  }>;
  pendingSignals?: Array<{
    instrument: string;
    direction: string;
    strategyName: string;
    entryPrice?: string;
  }>;
  availableInstruments?: string[];
}): WatcherContext {
  return {
    type: 'watcher',
    running_strategies: params.runningStrategies || [],
    pending_signals: params.pendingSignals || [],
    available_instruments: params.availableInstruments,
  };
}

/**
 * Build context for the Trade Analysis window
 */
export function buildTradeAnalysisContext(params: {
  tradeCount: number;
  dateRange?: string;
  winRate?: string;
  profitFactor?: string;
  filtersActive?: boolean;
  activeBreakdown?: string;
}): TradeAnalysisContext {
  return {
    type: 'tradeAnalysis',
    trade_count: params.tradeCount,
    date_range: params.dateRange,
    win_rate: params.winRate,
    profit_factor: params.profitFactor,
    filters_active: params.filtersActive || false,
    active_breakdown: params.activeBreakdown,
  };
}

/**
 * Build context for the Trade Review modal (single trade deep-dive)
 */
export function buildTradeReviewContext(params: {
  instrument: string;
  direction: string;
  isWinner: boolean;
  entryPrice: string;
  exitPrice: string;
  realizedPl: string;
  durationMinutes: number;
  maePips: string;
  mfePips: string;
  captureEfficiency?: string;
  riskEfficiency?: string;
  immediateDrawdownPips: string;
  candlesToProfit?: number;
  nearSwingPoint?: string;
  rsi14?: string;
  rsiZone?: string;
  trend?: string;
  postExitFavorablePips: string;
  postExitAdversePips: string;
  aiScore?: {
    entry: number;
    exit: number;
    riskManagement: number;
    overall: number;
  };
  keyInsights?: string[];
  indicatorAnalysis?: Array<{
    indicator: string;
    assessment: string;
    supportedTrade: boolean;
    atEntry: string;
    atExit: string;
  }>;
  conflictingIndicators?: string[];
}): TradeReviewContext {
  return {
    type: 'tradeReview',
    instrument: params.instrument,
    direction: params.direction,
    is_winner: params.isWinner,
    entry_price: params.entryPrice,
    exit_price: params.exitPrice,
    realized_pl: params.realizedPl,
    duration_minutes: params.durationMinutes,
    mae_pips: params.maePips,
    mfe_pips: params.mfePips,
    capture_efficiency: params.captureEfficiency,
    risk_efficiency: params.riskEfficiency,
    immediate_drawdown_pips: params.immediateDrawdownPips,
    candles_to_profit: params.candlesToProfit,
    near_swing_point: params.nearSwingPoint,
    rsi_14: params.rsi14,
    rsi_zone: params.rsiZone,
    trend: params.trend,
    post_exit_favorable_pips: params.postExitFavorablePips,
    post_exit_adverse_pips: params.postExitAdversePips,
    ai_score: params.aiScore ? {
      entry: params.aiScore.entry,
      exit: params.aiScore.exit,
      riskManagement: params.aiScore.riskManagement,
      overall: params.aiScore.overall,
    } : undefined,
    key_insights: params.keyInsights || [],
    indicator_analysis: params.indicatorAnalysis,
    conflicting_indicators: params.conflictingIndicators,
  };
}

/**
 * Build context for the Trade Subset modal (filtered group of trades)
 */
export function buildTradeSubsetContext(params: {
  subsetDescription: string;
  tradeCount: number;
  wins: number;
  losses: number;
  winRate: string;
  avgWin: string;
  avgLoss: string;
  expectancy: string;
  profitFactor: string;
  totalPl: string;
  instruments: string[];
  longCount: number;
  shortCount: number;
}): TradeSubsetContext {
  return {
    type: 'tradeSubset',
    subset_description: params.subsetDescription,
    trade_count: params.tradeCount,
    wins: params.wins,
    losses: params.losses,
    win_rate: params.winRate,
    avg_win: params.avgWin,
    avg_loss: params.avgLoss,
    expectancy: params.expectancy,
    profit_factor: params.profitFactor,
    total_pl: params.totalPl,
    instruments: params.instruments,
    long_count: params.longCount,
    short_count: params.shortCount,
  };
}

/**
 * Build a minimal context when no specific context is available.
 * Uses the window type to provide basic information.
 */
export function buildMinimalContext(windowType: string): ChatContext {
  switch (windowType) {
    case 'account':
      return buildAccountContext({ environment: 'unknown' });
    case 'charting':
      return buildChartingContext({
        instrument: 'unknown',
        granularity: 'unknown',
      });
    case 'backtesting':
      return buildBacktestingContext({});
    case 'ticket':
      return buildTicketContext({ instrument: 'unknown' });
    case 'watcher':
      return buildWatcherContext({});
    case 'tradeanalysis':
      return buildTradeAnalysisContext({ tradeCount: 0 });
    default:
      return { type: windowType };
  }
}
