/**
 * Strategy fixture factory functions for E2E tests.
 *
 * Produces Zero-format strategy rows (JSON-stringified columns)
 * matching shared/schema.ts strategy table definition.
 *
 * useParsedStrategies() calls JSON.parse on these columns at runtime,
 * so the JSON must match src/types/strategy.ts types exactly.
 */

// ============================================================================
// Zero-format strategy row (matches shared/schema.ts columns)
// ============================================================================

export interface StrategyRow {
  id: string;
  user_id: string;
  name: string;
  description: string;
  schema_version: number;
  parameters: string; // JSON: ParameterDefinition[]
  variables: string; // JSON: StrategyVariable[]
  indicators: string; // JSON: IndicatorDefinition[]
  entry_rules: string; // JSON: EntryRuleV2[]
  entry_logic: string; // JSON: EntryLogic
  exit_rules: string; // JSON: ExitRuleV2[]
  risk_settings: string; // JSON: RiskSettings
  planning_conversation: string | null;
  auto_note_indicators: string | null;
  pivot_config: string | null;
  version: number;
  is_active: boolean;
  is_promoted: boolean;
  is_locked: boolean;
  is_archived: boolean;
  created_at: number;
  updated_at: number;
  /** Provenance tag (AGT-648): '' = native wickd, 'candlesight' = imported. */
  source: string;
}

// ============================================================================
// Default strategy data (RSI + EMA, compare trigger entry, fixed exit)
// ============================================================================

const defaultIndicators = [
  {
    id: 'rsi-1',
    type: 'rsi',
    params: { period: { $param: 'rsi_period' } },
  },
  {
    id: 'ema-1',
    type: 'ema',
    params: { period: { $param: 'ema_period' } },
  },
];

const defaultParameters = [
  {
    id: 'rsi_period',
    name: 'RSI Period',
    type: 'number',
    default: 14,
    min: 7,
    max: 28,
    step: 7,
    group: 'indicator',
  },
  {
    id: 'ema_period',
    name: 'EMA Period',
    type: 'number',
    default: 50,
    min: 20,
    max: 100,
    step: 10,
    group: 'indicator',
  },
];

const defaultVariables = [
  {
    id: 'price_to_ema',
    name: 'Price to EMA',
    description: 'Distance from current price to EMA',
    expression: {
      type: 'distance',
      left: { source: 'price', value: 'close' },
      right: { indicator: 'ema-1', output: 'value' },
      absolute: true,
    },
  },
];

const defaultEntryRules = [
  {
    id: 'entry-long-1',
    name: 'RSI Oversold + Price Above EMA',
    direction: 'long',
    conditions: [
      {
        name: 'RSI crosses above 30',
        primary: {
          trigger: {
            type: 'threshold',
            source: { indicator: 'rsi-1', output: 'value' },
            operator: 'crosses_above',
            value: 30,
          },
          negated: false,
        },
        chain: [
          {
            operator: 'and',
            trigger: {
              trigger: {
                type: 'compare',
                left: { source: 'price', value: 'close' },
                operator: '>',
                right: { indicator: 'ema-1', output: 'value' },
              },
              negated: false,
            },
          },
        ],
      },
    ],
  },
];

const defaultExitRules = [
  {
    id: 'exit-sl-1',
    name: 'Stop Loss',
    direction: 'both',
    conditions: [
      {
        primary: {
          trigger: {
            type: 'risk_reward_reached',
            ratio: 1,
          },
          negated: true,
        },
        chain: [],
      },
    ],
    close_percent: 100,
    priority: 10,
  },
  {
    id: 'exit-tp-1',
    name: 'Take Profit',
    direction: 'both',
    conditions: [
      {
        primary: {
          trigger: {
            type: 'risk_reward_reached',
            ratio: 2,
          },
          negated: false,
        },
        chain: [],
      },
    ],
    close_percent: 50,
    priority: 5,
  },
];

const defaultRiskSettings = {
  risk_method: 'percent',
  risk_value: 1.5,
  rr_ratio: 2.0,
  spread_buffer_pips: 1,
  stop_loss_source: {
    type: 'fixed_pips',
    pips: 30,
  },
};

const defaultEntryLogic = { mode: 'all' };

// ============================================================================
// Factory functions
// ============================================================================

let fixtureCounter = 0;

/**
 * Create a strategy Zero row with sensible defaults.
 * All JSON columns are pre-stringified to match Zero schema.
 */
export function makeStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  fixtureCounter++;
  const now = Date.now();

  return {
    id: `strat-e2e-${fixtureCounter}`,
    user_id: 'e2e-user-001',
    name: `Test Strategy ${fixtureCounter}`,
    description: 'E2E test strategy with RSI + EMA',
    schema_version: 2,
    parameters: JSON.stringify(defaultParameters),
    variables: JSON.stringify(defaultVariables),
    indicators: JSON.stringify(defaultIndicators),
    entry_rules: JSON.stringify(defaultEntryRules),
    entry_logic: JSON.stringify(defaultEntryLogic),
    exit_rules: JSON.stringify(defaultExitRules),
    risk_settings: JSON.stringify(defaultRiskSettings),
    planning_conversation: null,
    auto_note_indicators: null,
    pivot_config: null,
    version: 1,
    is_active: true,
    is_promoted: false,
    is_locked: false,
    is_archived: false,
    created_at: now,
    updated_at: now,
    source: '',
    ...overrides,
  };
}

/**
 * Create a promoted (live) strategy. Promoted strategies are always locked.
 */
export function makePromotedStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  return makeStrategy({
    is_promoted: true,
    is_locked: true,
    ...overrides,
  });
}

/**
 * Create an archived strategy. Archived strategies are always locked.
 */
export function makeArchivedStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  return makeStrategy({
    is_archived: true,
    is_locked: true,
    ...overrides,
  });
}

// ============================================================================
// Specialized strategy builders for data integrity tests
// ============================================================================

/**
 * Strategy with parameterized values (uses $param references).
 */
export function makeParameterizedStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  const entryRules = [
    {
      id: 'entry-param-1',
      name: 'Parameterized Entry',
      direction: 'long',
      conditions: [
        {
          primary: {
            trigger: {
              type: 'threshold',
              source: { indicator: 'rsi-1', output: 'value' },
              operator: 'crosses_above',
              value: { $param: 'rsi_oversold' },
            },
            negated: false,
          },
          chain: [],
        },
      ],
    },
  ];

  const parameters = [
    ...defaultParameters,
    {
      id: 'rsi_oversold',
      name: 'RSI Oversold Level',
      type: 'number',
      default: 30,
      min: 20,
      max: 40,
      step: 5,
      group: 'entry',
    },
  ];

  return makeStrategy({
    name: 'Parameterized Strategy',
    entry_rules: JSON.stringify(entryRules),
    parameters: JSON.stringify(parameters),
    ...overrides,
  });
}

/**
 * Strategy with chained AND/OR triggers.
 */
export function makeChainedTriggerStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  const entryRules = [
    {
      id: 'entry-chain-1',
      name: 'Chained Entry',
      direction: 'long',
      conditions: [
        {
          name: 'RSI oversold AND price above EMA OR RSI diverging',
          primary: {
            trigger: {
              type: 'threshold',
              source: { indicator: 'rsi-1', output: 'value' },
              operator: '<',
              value: 30,
            },
            negated: false,
          },
          chain: [
            {
              operator: 'and',
              trigger: {
                trigger: {
                  type: 'compare',
                  left: { source: 'price', value: 'close' },
                  operator: '>',
                  right: { indicator: 'ema-1', output: 'value' },
                },
                negated: false,
              },
            },
            {
              operator: 'or',
              trigger: {
                trigger: {
                  type: 'givens',
                  regime: 'trending_up',
                },
                negated: false,
              },
            },
          ],
        },
      ],
    },
  ];

  return makeStrategy({
    name: 'Chained Trigger Strategy',
    entry_rules: JSON.stringify(entryRules),
    ...overrides,
  });
}

/**
 * Strategy with multiple indicator types.
 */
export function makeMultiIndicatorStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  const indicators = [
    { id: 'rsi-1', type: 'rsi', params: { period: 14 } },
    { id: 'ema-1', type: 'ema', params: { period: 50 } },
    { id: 'macd-1', type: 'macd', params: { fast_period: 12, slow_period: 26, signal_period: 9 } },
    { id: 'bb-1', type: 'bollinger', params: { period: 20, std_dev: 2 } },
    { id: 'atr-1', type: 'atr', params: { period: 14 } },
  ];

  return makeStrategy({
    name: 'Multi-Indicator Strategy',
    indicators: JSON.stringify(indicators),
    ...overrides,
  });
}

/**
 * Strategy with all variable expression types.
 */
export function makeVariableStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  const variables = [
    {
      id: 'distance-var',
      name: 'Price to EMA Distance',
      expression: {
        type: 'distance',
        left: { source: 'price', value: 'close' },
        right: { indicator: 'ema-1', output: 'value' },
        absolute: true,
      },
    },
    {
      id: 'ratio-var',
      name: 'RSI/50 Ratio',
      expression: {
        type: 'ratio',
        numerator: { indicator: 'rsi-1', output: 'value' },
        denominator: { fixed: 50 },
      },
    },
    {
      id: 'change-var',
      name: 'RSI 3-Bar Change',
      expression: {
        type: 'change',
        source: { indicator: 'rsi-1', output: 'value' },
        bars: 3,
      },
    },
    {
      id: 'value-var',
      name: 'ATR x 2.5',
      expression: {
        type: 'value',
        source: { indicator: 'rsi-1', output: 'value' },
        operations: [{ operator: '*', operand: { fixed: 2.5 } }],
      },
    },
    {
      id: 'abs-var',
      name: 'Abs RSI Diff',
      expression: {
        type: 'abs',
        source: { indicator: 'rsi-1', output: 'value' },
      },
    },
    {
      id: 'negate-var',
      name: 'Negate Close',
      expression: {
        type: 'negate',
        source: { source: 'price', value: 'close' },
      },
    },
    {
      id: 'min-var',
      name: 'Min RSI EMA',
      expression: {
        type: 'min',
        left: { indicator: 'rsi-1', output: 'value' },
        right: { indicator: 'ema-1', output: 'value' },
      },
    },
    {
      id: 'max-var',
      name: 'Max RSI EMA',
      expression: {
        type: 'max',
        left: { indicator: 'rsi-1', output: 'value' },
        right: { indicator: 'ema-1', output: 'value' },
      },
    },
    {
      id: 'highest-var',
      name: 'Highest Close 20',
      expression: {
        type: 'highest',
        source: { source: 'price', value: 'close' },
        period: 20,
      },
    },
    {
      id: 'lowest-var',
      name: 'Lowest Low 10',
      expression: {
        type: 'lowest',
        source: { source: 'price', value: 'low' },
        period: 10,
      },
    },
    {
      id: 'sum-var',
      name: 'RSI Sum 5',
      expression: {
        type: 'sum',
        source: { indicator: 'rsi-1', output: 'value' },
        period: 5,
      },
    },
    {
      id: 'average-var',
      name: 'RSI Avg 14',
      expression: {
        type: 'average',
        source: { indicator: 'rsi-1', output: 'value' },
        period: 14,
      },
    },
    {
      id: 'cond-var',
      name: 'Conditional RSI',
      expression: {
        type: 'conditional',
        condition_left: { indicator: 'rsi-1', output: 'value' },
        operator: '>',
        condition_right: { fixed: 50 },
        true_value: { source: 'price', value: 'high' },
        false_value: { source: 'price', value: 'low' },
      },
    },
  ];

  return makeStrategy({
    name: 'Variable Strategy',
    variables: JSON.stringify(variables),
    ...overrides,
  });
}

/**
 * Strategy with session filter triggers (TimeInRange + DayOfWeek).
 * Uses London session (08:00-16:00 UTC), excludes Sunday,
 * combined with a simple RSI threshold trigger via AND logic.
 */
export function makeSessionFilterStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  const entryRules = [
    {
      id: 'entry-session-1',
      name: 'RSI Oversold in London Session',
      direction: 'long',
      conditions: [
        {
          name: 'RSI crosses above 30',
          primary: {
            trigger: {
              type: 'threshold',
              source: { indicator: 'rsi-1', output: 'value' },
              operator: 'crosses_above',
              value: 30,
            },
            negated: false,
          },
          chain: [
            {
              operator: 'and',
              trigger: {
                trigger: {
                  type: 'time_in_range',
                  start_hour: 8,
                  start_minute: 0,
                  end_hour: 16,
                  end_minute: 0,
                },
                negated: false,
              },
            },
            {
              operator: 'and',
              trigger: {
                trigger: {
                  type: 'day_of_week',
                  days: [0],
                  exclude: true,
                },
                negated: false,
              },
            },
          ],
        },
      ],
    },
  ];

  return makeStrategy({
    name: 'Session Filter Strategy',
    entry_rules: JSON.stringify(entryRules),
    ...overrides,
  });
}

/**
 * Strategy with pivot config enabled.
 */
export function makePivotStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  return makeStrategy({
    name: 'Pivot Strategy',
    pivot_config: JSON.stringify({ enabled: true, period: 'daily' }),
    ...overrides,
  });
}

/**
 * Strategy with new SP2 indicator types: VWAP, Parabolic SAR, SuperTrend.
 */
export function makeNewIndicatorStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  const indicators = [
    { id: 'vwap-1', type: 'vwap', params: {} },
    { id: 'psar-1', type: 'parabolic_sar', params: { af_start: 0.02, af_increment: 0.02, af_max: 0.2 } },
    { id: 'st-1', type: 'super_trend', params: { period: 10, multiplier: 3 } },
    { id: 'ema-1', type: 'ema', params: { period: 50 } },
  ];

  const entryRules = [
    {
      id: 'entry-new-ind-1',
      name: 'SuperTrend Bullish + Price Above VWAP',
      direction: 'long',
      conditions: [
        {
          name: 'SuperTrend bullish and price above VWAP',
          primary: {
            trigger: {
              type: 'compare',
              left: { indicator: 'st-1', output: 'trend' },
              operator: '==',
              right: { fixed: 1 },
            },
            negated: false,
          },
          chain: [
            {
              operator: 'and',
              trigger: {
                trigger: {
                  type: 'compare',
                  left: { source: 'price', value: 'close' },
                  operator: '>',
                  right: { indicator: 'vwap-1', output: 'vwap' },
                },
                negated: false,
              },
            },
          ],
        },
      ],
    },
  ];

  return makeStrategy({
    name: 'New Indicator Strategy',
    indicators: JSON.stringify(indicators),
    entry_rules: JSON.stringify(entryRules),
    ...overrides,
  });
}

/**
 * Strategy with a candlestick pattern data source in a compare trigger.
 * Entry: if bullish_engulfing == 1 then buy.
 */
export function makeCandlestickPatternStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  const indicators = [
    { id: 'ema-1', type: 'ema', params: { period: 50 } },
  ];

  const entryRules = [
    {
      id: 'entry-pattern-1',
      name: 'Bullish Engulfing Entry',
      direction: 'long',
      conditions: [
        {
          name: 'Bullish engulfing detected',
          primary: {
            trigger: {
              type: 'compare',
              left: { source: 'pattern', pattern: 'bullish_engulfing', offset: 0 },
              operator: '==',
              right: { fixed: 1 },
            },
            negated: false,
          },
          chain: [],
        },
      ],
    },
  ];

  return makeStrategy({
    name: 'Candlestick Pattern Strategy',
    indicators: JSON.stringify(indicators),
    entry_rules: JSON.stringify(entryRules),
    ...overrides,
  });
}

/**
 * Strategy with a pending order (buy stop) on the entry rule.
 * Tests that strategies with pending_order config parse and render correctly.
 */
export function makePendingOrderStrategy(overrides?: Partial<StrategyRow>): StrategyRow {
  const indicators = [
    { id: 'atr-1', type: 'atr', params: { period: 14 } },
    { id: 'donchian-1', type: 'donchian', params: { period: 20 } },
  ];

  const entryRules = [
    {
      id: 'entry-buy-stop-1',
      name: 'Breakout Buy Stop',
      direction: 'long',
      conditions: [
        {
          name: 'Price near upper Donchian',
          primary: {
            trigger: {
              type: 'compare',
              left: { source: 'price', value: 'close' },
              operator: '>',
              right: { indicator: 'donchian-1', output: 'middle' },
            },
            negated: false,
          },
          chain: [],
        },
      ],
      pending_order: {
        order_type: 'buy_stop',
        price: { indicator: 'donchian-1', output: 'upper' },
        expiry_bars: 5,
      },
    },
  ];

  return makeStrategy({
    name: 'Pending Order Strategy',
    description: 'Breakout strategy with buy stop pending orders',
    indicators: JSON.stringify(indicators),
    entry_rules: JSON.stringify(entryRules),
    ...overrides,
  });
}

// ============================================================================
// Watcher / pattern match fixtures
// ============================================================================

export interface WatcherRow {
  id: string;
  user_id: string;
  strategy_id: string;
  strategy_name: string | null;
  instrument: string;
  timeframe: string;
  mode: string;
  signal_filter: string;
  is_active: boolean;
  created_at: number;
  updated_at: number;
}

export function makeWatcher(overrides?: Partial<WatcherRow>): WatcherRow {
  fixtureCounter++;
  const now = Date.now();
  return {
    id: `watcher-e2e-${fixtureCounter}`,
    user_id: 'e2e-user-001',
    strategy_id: 'strat-e2e-1',
    strategy_name: 'Test Strategy',
    instrument: 'EUR_USD',
    timeframe: 'H4',
    mode: 'signal_only',
    signal_filter: 'all',
    is_active: true,
    created_at: now,
    updated_at: now,
    ...overrides,
  };
}

export interface PatternMatchRow {
  id: string;
  user_id: string;
  config_id: string;
  instrument: string;
  match_type: string;
  direction: string | null;
  entry_price: string | null;
  stop_loss: string | null;
  take_profit: string | null;
  position_size: string | null;
  close_percent: string | null;
  reason: string;
  status: string;
  executed_at: number | null;
  created_at: number;
}

export function makePatternMatch(overrides?: Partial<PatternMatchRow>): PatternMatchRow {
  fixtureCounter++;
  return {
    id: `match-e2e-${fixtureCounter}`,
    user_id: 'e2e-user-001',
    config_id: 'config-e2e-1',
    instrument: 'EUR_USD',
    match_type: 'entry',
    direction: 'long',
    entry_price: '1.08500',
    stop_loss: '1.08200',
    take_profit: '1.09100',
    position_size: '10000',
    close_percent: null,
    reason: 'RSI crossed above 30, price above EMA',
    status: 'pending',
    executed_at: null,
    created_at: Date.now(),
    ...overrides,
  };
}

// ============================================================================
// Backtest result fixtures (for Tauri command mocks)
// ============================================================================

export const mockBacktestResult = {
  metrics: {
    total_pnl: '1250.00',
    total_return_pct: '12.50',
    winning_trades: 15,
    losing_trades: 8,
    win_rate: '65.22',
    profit_factor: '1.85',
    max_drawdown_pct: '4.20',
    sharpe_ratio: '1.42',
    total_trades: 23,
    final_balance: '11250.00',
  },
  trades: [
    {
      entryTime: '2025-01-15T10:00:00Z',
      exitTime: '2025-01-15T14:00:00Z',
      entryPrice: '1.08500',
      exitPrice: '1.08750',
      units: '10000',
      pnl: '25.00',
      isLong: true,
      entryRuleId: 'entry-long-1',
      entryRuleName: 'RSI Oversold + Price Above EMA',
      exitReason: 'Take Profit',
      stopLoss: '1.08200',
      takeProfit: '1.08750',
    },
  ],
  equityCurve: [
    { time: '2025-01-01T00:00:00Z', balance: '10000.00' },
    { time: '2025-01-15T14:00:00Z', balance: '10025.00' },
    { time: '2025-03-31T23:59:59Z', balance: '11250.00' },
  ],
  dataRange: {
    start: '2025-01-01T00:00:00Z',
    end: '2025-03-31T23:59:59Z',
  },
};

export const mockWalkForwardResult = {
  config: {
    train_months: 6,
    test_months: 3,
    step_months: 3,
    objective: 'sharpe_ratio',
    min_trades_per_window: 5,
    anchored: false,
  },
  periods: [
    {
      window: {
        window_num: 1,
        train_start: '2024-01-01T00:00:00Z',
        train_end: '2024-06-30T23:59:59Z',
        test_start: '2024-07-01T00:00:00Z',
        test_end: '2024-09-30T23:59:59Z',
      },
      optimized_params: { rsi_period: 14, ema_period: 50 },
      in_sample_metrics: {
        total_pnl: '500.00',
        total_return_pct: '5.00',
        winning_trades: 8,
        losing_trades: 4,
        win_rate: '66.67',
        profit_factor: '1.90',
        max_drawdown_pct: '3.50',
        sharpe_ratio: '1.55',
        total_trades: 12,
        final_balance: '10500.00',
      },
      in_sample_sharpe: 1.55,
      out_of_sample_metrics: {
        total_pnl: '300.00',
        total_return_pct: '3.00',
        winning_trades: 5,
        losing_trades: 3,
        win_rate: '62.50',
        profit_factor: '1.70',
        max_drawdown_pct: '2.80',
        sharpe_ratio: '1.20',
        total_trades: 8,
        final_balance: '10300.00',
      },
      out_of_sample_sharpe: 1.20,
      oos_trade_count: 8,
      oos_profitable: true,
      oos_trades: [],
    },
  ],
  total_periods: 1,
  valid_periods: 1,
  profitable_periods: 1,
  oos_total_pnl: '300.00',
  oos_total_return_pct: '3.00',
  oos_avg_sharpe: 1.2,
  oos_win_rate: '62.50',
  oos_max_drawdown_pct: '2.80',
  oos_total_trades: 8,
  sharpe_efficiency: 0.77,
  return_efficiency: 0.60,
  robustness_score: 72,
  parameter_stability: [
    {
      param_id: 'rsi_period',
      param_name: 'RSI Period',
      mode_value: 14,
      mode_count: 1,
      total_windows: 1,
      stability_pct: 100,
    },
  ],
  oos_equity_curve: ['10000.00', '10150.00', '10300.00'],
};

// ============================================================================
// Local-store backtest run rows (AGT-645)
// ============================================================================

/**
 * A camelCase BacktestResult payload matching what `run_custom_backtest`
 * returns and what BacktestResultsPanel / LocalBacktestsSection render.
 */
export function makeBacktestResultPayload(overrides: Record<string, unknown> = {}) {
  return {
    metrics: {
      totalPnl: '250.00',
      totalReturnPct: '25.00',
      annualizedReturnPct: '100.00',
      winningTrades: 3,
      losingTrades: 1,
      winRate: '75.00',
      avgWin: '100.00',
      avgLoss: '50.00',
      profitFactor: '2.00',
      maxDrawdownPct: '5.00',
      sharpeRatio: '1.50',
      totalTrades: 4,
      finalBalance: '1250.00',
    },
    trades: [
      {
        tradeNum: 1,
        direction: 'long',
        entryTime: '2025-01-06T10:00:00Z',
        exitTime: '2025-01-07T14:00:00Z',
        entryPrice: '1.08500',
        exitPrice: '1.09000',
        units: '10000',
        pnl: '50.00',
        pnlPct: '5.00',
        cumulativePnl: '50.00',
      },
      {
        tradeNum: 2,
        direction: 'short',
        entryTime: '2025-01-10T08:00:00Z',
        exitTime: '2025-01-12T16:00:00Z',
        entryPrice: '1.09500',
        exitPrice: '1.08500',
        units: '-10000',
        pnl: '100.00',
        pnlPct: '10.00',
        cumulativePnl: '150.00',
      },
      {
        tradeNum: 3,
        direction: 'long',
        entryTime: '2025-02-03T09:00:00Z',
        exitTime: '2025-02-04T11:00:00Z',
        entryPrice: '1.07800',
        exitPrice: '1.07300',
        units: '10000',
        pnl: '-50.00',
        pnlPct: '-5.00',
        cumulativePnl: '100.00',
      },
      {
        tradeNum: 4,
        direction: 'long',
        entryTime: '2025-02-20T13:00:00Z',
        exitTime: '2025-02-24T10:00:00Z',
        entryPrice: '1.06900',
        exitPrice: '1.08400',
        units: '10000',
        pnl: '150.00',
        pnlPct: '15.00',
        cumulativePnl: '250.00',
      },
    ],
    equityCurve: [
      { time: '2025-01-01T00:00:00Z', balance: '1000.00' },
      { time: '2025-01-07T00:00:00Z', balance: '1050.00' },
      { time: '2025-01-12T00:00:00Z', balance: '1150.00' },
      { time: '2025-02-04T00:00:00Z', balance: '1100.00' },
      { time: '2025-02-24T00:00:00Z', balance: '1250.00' },
    ],
    dataRange: {
      startTime: '2025-01-01T00:00:00Z',
      endTime: '2025-03-31T00:00:00Z',
      totalCandles: 1560,
    },
    ...overrides,
  };
}

/**
 * A saved run row for the local `backtest` table mock
 * (window.__E2E_LOCAL_BACKTESTS__), matching src/lib/localStore.ts
 * LocalBacktest + the persisted payload shape SimpleHistoricalFlow and
 * LocalBacktestsSection read.
 */
export function makeLocalBacktestRow(
  strategyId: string,
  overrides: {
    id?: string;
    instrument?: string;
    startDate?: string;
    endDate?: string;
    runNumber?: number;
    granularity?: string;
    createdAt?: number;
    result?: Record<string, unknown>;
  } = {},
) {
  const createdAt = overrides.createdAt ?? Date.now();
  return {
    id: overrides.id ?? `bt-${strategyId}-${overrides.runNumber ?? 1}`,
    strategy_id: strategyId,
    instrument: overrides.instrument ?? 'EUR_USD',
    start_date: Date.parse(overrides.startDate ?? '2025-01-01'),
    end_date: Date.parse(overrides.endDate ?? '2025-03-31'),
    results: JSON.stringify({
      result: overrides.result ?? makeBacktestResultPayload(),
      parameterValues: {},
      runNumber: overrides.runNumber ?? 1,
      granularity: overrides.granularity ?? 'H1',
      timestamp: createdAt,
    }),
    created_at: createdAt,
  };
}
