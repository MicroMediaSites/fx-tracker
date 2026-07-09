/**
 * Context loading based on classification category.
 * See docs/ai-context-routing.md for full specification.
 *
 * Layer 1 (Base): Minimal window state - always included
 * Layer 2 (Classified): Additional context based on question category
 */

import type { ChatContext } from '../hooks/useTerminalChat';

/** Question category driving which context layers get loaded. */
export type ContextCategory = 'trade' | 'strategy' | 'backtest' | 'market' | 'general';

/**
 * Prompt classification result. AGT-650: the cloud classifier endpoint is
 * gone — callers pass a static 'general' heuristic result.
 */
export interface ClassificationResult {
  primary: ContextCategory;
  secondary: ContextCategory | null;
  confidence: 'high' | 'medium' | 'low';
  reasoning: string;
  source: 'heuristic' | 'gpt-4o-mini';
}

// Type helpers for accessing ChatContext properties safely
type AnyContext = ChatContext & Record<string, unknown>;

/**
 * Extract base context from full context.
 * This is the minimal state needed for orientation.
 */
export function extractBaseContext(fullContext: ChatContext): ChatContext {
  const ctx = fullContext as AnyContext;
  const windowType = ctx.type as string;

  switch (windowType) {
    case 'ticket':
      return {
        type: 'ticket',
        instrument: ctx.instrument,
        direction: ctx.direction,
        units: ctx.units,
        stop_loss: ctx.stop_loss,
        take_profit: ctx.take_profit,
        current_price: ctx.current_price,
      };

    case 'charting': {
      return {
        type: 'charting',
        instrument: ctx.instrument,
        granularity: ctx.granularity,
        strategy_name: ctx.strategy_name,
        indicators: ctx.indicators || [], // Always include all indicators (they're just labels)
        indicator_values: ctx.indicator_values,
      };
    }

    case 'tradeAnalysis':
      return {
        type: 'tradeAnalysis',
        trade_count: ctx.trade_count,
        date_range: ctx.date_range,
        filters_active: ctx.filters_active,
        active_breakdown: ctx.active_breakdown,
      };

    case 'backtesting':
      return {
        type: 'backtesting',
        strategy_id: ctx.strategy_id,
        strategy_name: ctx.strategy_name,
        methodology: ctx.methodology,
        has_results: ctx.has_results,
        backtest_job_id: ctx.backtest_job_id,
        parameters: [], // Empty for base
      };

    case 'watcher': {
      const runningStrategies = ctx.running_strategies as Array<{ strategyName: string }> | undefined;
      return {
        type: 'watcher',
        running_strategies: runningStrategies?.map(s => ({ strategyName: s.strategyName, instruments: [], timeframe: '' })) || [],
        pending_signals: [],
        available_instruments: [],
      };
    }

    case 'account':
      return {
        type: 'account',
        environment: ctx.environment,
        balance: ctx.balance,
      };

    case 'tradeReview':
      return {
        type: 'tradeReview',
        instrument: ctx.instrument,
        direction: ctx.direction,
        is_winner: ctx.is_winner,
        realized_pl: ctx.realized_pl,
        // Minimal metrics
        entry_price: ctx.entry_price,
        exit_price: ctx.exit_price,
        duration_minutes: ctx.duration_minutes,
        mae_pips: '',
        mfe_pips: '',
        immediate_drawdown_pips: '',
        post_exit_favorable_pips: '',
        post_exit_adverse_pips: '',
        key_insights: [],
      };

    case 'tradeSubset':
      return {
        type: 'tradeSubset',
        subset_description: ctx.subset_description,
        trade_count: ctx.trade_count,
        win_rate: ctx.win_rate,
        wins: ctx.wins || 0,
        losses: ctx.losses || 0,
        avg_win: '',
        avg_loss: '',
        expectancy: '',
        profit_factor: '',
        total_pl: '',
        instruments: [],
        long_count: 0,
        short_count: 0,
      };

    default:
      return { type: windowType };
  }
}

/**
 * Build context based on classification result.
 * Returns base context for general/market, full context for others.
 */
export function buildContextForCategory(
  fullContext: ChatContext,
  classification: ClassificationResult
): ChatContext {
  const { primary, secondary } = classification;

  // For general category, use minimal base context
  if (primary === 'general') {
    // If there's a relevant secondary category, include more context
    if (secondary && secondary !== 'general' && secondary !== 'market') {
      return buildContextForPrimaryCategory(fullContext, secondary);
    }
    return extractBaseContext(fullContext);
  }

  // For market category, include candles for chart analysis if in charting window
  if (primary === 'market') {
    const baseContext = extractBaseContext(fullContext);
    // Include recent candles for chart analysis questions
    if (fullContext.type === 'charting') {
      const chartCtx = fullContext as { recent_candles?: Array<{ time: string; open: string; high: string; low: string; close: string }> };
      if (chartCtx.recent_candles && chartCtx.recent_candles.length > 0) {
        return {
          ...baseContext,
          recent_candles: chartCtx.recent_candles,
        };
      }
    }
    return baseContext;
  }

  // For other categories, include category-specific context
  let context = buildContextForPrimaryCategory(fullContext, primary);

  // If there's a secondary category, merge its context
  if (secondary && secondary !== primary && secondary !== 'general' && secondary !== 'market') {
    const secondaryContext = buildContextForPrimaryCategory(fullContext, secondary);
    context = mergeContexts(context, secondaryContext);
  }

  return context;
}

/**
 * Build context for a specific primary category.
 */
function buildContextForPrimaryCategory(
  fullContext: ChatContext,
  category: ContextCategory
): ChatContext {
  const ctx = fullContext as AnyContext;

  switch (category) {
    case 'backtest':
      // For backtest analysis, include strategy details and results
      return {
        ...fullContext,
        // Ensure these are included
        strategy_id: ctx.strategy_id,
        strategy_name: ctx.strategy_name,
        strategy_description: ctx.strategy_description,
        methodology: ctx.methodology,
        parameters: ctx.parameters,
        has_results: ctx.has_results,
        backtest_job_id: ctx.backtest_job_id,
        metrics_summary: ctx.metrics_summary,
        holdout_summary: ctx.holdout_summary,
      };

    case 'strategy':
      // For strategy building, include current structure but not heavy results
      return {
        ...extractBaseContext(fullContext),
        strategy_name: ctx.strategy_name,
        strategy_description: ctx.strategy_description,
        parameters: ctx.parameters,
        indicators: ctx.indicators,
      };

    case 'trade':
      // For trade analysis, include all trade metrics
      // TradeReview already has everything needed
      return fullContext;

    case 'market':
    case 'general':
    default:
      return extractBaseContext(fullContext);
  }
}

/**
 * Merge two contexts, preferring values from the second.
 */
function mergeContexts(context1: ChatContext, context2: ChatContext): ChatContext {
  const merged: Record<string, unknown> = { ...context1 };

  for (const [key, value] of Object.entries(context2)) {
    if (value !== undefined && value !== null) {
      merged[key] = value;
    }
  }

  return merged as ChatContext;
}

/**
 * Generate a summary of window state for the classifier.
 * This helps GPT-4o-mini make better classification decisions.
 */
export function summarizeWindowContext(fullContext: ChatContext): string {
  const ctx = fullContext as AnyContext;
  const windowType = ctx.type as string;

  switch (windowType) {
    case 'backtesting': {
      const parts = [`Strategy: ${ctx.strategy_name || 'none'}`];
      if (ctx.has_results) parts.push('has backtest results');
      if (ctx.methodology) parts.push(`methodology: ${ctx.methodology}`);
      if (ctx.metrics_summary) parts.push(`metrics: ${ctx.metrics_summary}`);
      return parts.join(', ');
    }

    case 'tradeAnalysis':
      return `${ctx.trade_count || 0} trades, viewing ${ctx.active_breakdown || 'summary'}`;

    case 'tradeReview':
      return `Reviewing ${ctx.direction} ${ctx.instrument} trade (${ctx.is_winner ? 'winner' : 'loser'})`;

    case 'charting':
      return `${ctx.instrument} ${ctx.granularity} chart${ctx.strategy_name ? ` with ${ctx.strategy_name}` : ''}`;

    case 'ticket':
      return `${ctx.direction || 'new'} ${ctx.instrument} order ticket`;

    case 'watcher': {
      const runningStrategies = ctx.running_strategies as unknown[] | undefined;
      const pendingSignals = ctx.pending_signals as unknown[] | undefined;
      const running = runningStrategies?.length || 0;
      const pending = pendingSignals?.length || 0;
      return `${running} strategies running, ${pending} pending signals`;
    }

    case 'account':
      return `${ctx.environment} account`;

    default:
      return windowType;
  }
}
