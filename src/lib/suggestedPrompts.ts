/**
 * Contextual suggested prompts for the AI terminal overlay.
 * Returns 2-3 prompts based on window type and current context state.
 * These appear as clickable chips below the welcome content.
 */

export interface SuggestedPrompt {
  /** Short display text for the chip (keep under ~40 chars) */
  label: string;
  /** Full prompt text submitted to AI on click */
  prompt: string;
}

/**
 * Get contextual suggested prompts based on window type and context.
 * Examines context fields to determine whether the user is in an "empty" or "active" state,
 * then returns appropriate prompts for onboarding or deeper exploration.
 */
export function getSuggestedPrompts(
  windowType: string,
  context: Record<string, unknown>
): SuggestedPrompt[] {
  switch (windowType) {
    case 'account':
      return getAccountPrompts(context);
    case 'charting':
      return getChartingPrompts(context);
    case 'backtesting':
      return getBacktestingPrompts(context);
    case 'watcher':
      return getWatcherPrompts(context);
    case 'tradeanalysis':
      return getTradeAnalysisPrompts(context);
    case 'ticket':
      return getTicketPrompts();
    default:
      return [];
  }
}

function getAccountPrompts(ctx: Record<string, unknown>): SuggestedPrompt[] {
  const openTradeCount = typeof ctx.open_trade_count === 'number' ? ctx.open_trade_count : 0;
  const hasBalance = ctx.balance !== undefined && ctx.balance !== null;

  if (!hasBalance || openTradeCount === 0) {
    return [
      { label: 'What can I track here?', prompt: 'What can I track here?' },
      { label: 'How do I place my first trade?', prompt: 'How do I place my first trade?' },
    ];
  }

  return [
    { label: 'Summarize my open positions', prompt: 'Summarize my open positions' },
    { label: 'How is my account performing?', prompt: 'How is my account performing?' },
  ];
}

function getChartingPrompts(ctx: Record<string, unknown>): SuggestedPrompt[] {
  const indicators = Array.isArray(ctx.indicators) ? ctx.indicators : [];

  if (indicators.length === 0) {
    return [
      { label: 'What indicators should I start with?', prompt: 'What indicators should I start with?' },
      { label: 'How do I add indicators?', prompt: 'How do I add indicators?' },
    ];
  }

  return [
    { label: 'What does this chart pattern suggest?', prompt: 'What does this chart pattern suggest?' },
    { label: 'Explain my indicator readings', prompt: 'Explain my indicator readings' },
  ];
}

function getBacktestingPrompts(ctx: Record<string, unknown>): SuggestedPrompt[] {
  const hasStrategy = Boolean(ctx.strategy_name || ctx.strategy_id);
  const hasResults = ctx.has_results === true;

  if (!hasStrategy) {
    return [
      { label: 'How do I create my first strategy?', prompt: 'How do I create my first strategy?' },
      { label: 'What is backtesting?', prompt: 'What is backtesting?' },
    ];
  }

  if (!hasResults) {
    return [
      { label: 'Walk me through running a backtest', prompt: 'Walk me through running a backtest' },
      { label: 'Explain the parameters', prompt: 'Explain the parameters' },
    ];
  }

  return [
    { label: 'Interpret these backtest results', prompt: 'Interpret these backtest results' },
    { label: 'How can I improve this strategy?', prompt: 'How can I improve this strategy?' },
  ];
}

function getWatcherPrompts(ctx: Record<string, unknown>): SuggestedPrompt[] {
  const runningStrategies = Array.isArray(ctx.running_strategies) ? ctx.running_strategies : [];

  if (runningStrategies.length === 0) {
    return [
      { label: 'What is live monitoring?', prompt: 'What is live monitoring?' },
      { label: 'How do I promote a strategy?', prompt: 'How do I promote a strategy?' },
    ];
  }

  return [
    { label: 'Explain the current signals', prompt: 'Explain the current signals' },
    { label: 'How do I act on a signal?', prompt: 'How do I act on a signal?' },
  ];
}

function getTradeAnalysisPrompts(ctx: Record<string, unknown>): SuggestedPrompt[] {
  const tradeCount = typeof ctx.trade_count === 'number' ? ctx.trade_count : 0;

  if (tradeCount === 0) {
    return [
      { label: 'What can trade analysis tell me?', prompt: 'What can trade analysis tell me?' },
      { label: 'How do I get trades to analyze?', prompt: 'How do I get trades to analyze?' },
    ];
  }

  return [
    { label: 'Score my most recent trade', prompt: 'Score my most recent trade' },
    { label: 'What patterns do you see in my trading?', prompt: 'What patterns do you see in my trading?' },
  ];
}

function getTicketPrompts(): SuggestedPrompt[] {
  return [
    { label: 'Explain this order setup', prompt: 'Explain this order setup' },
    { label: 'What should I check before submitting?', prompt: 'What should I check before submitting?' },
  ];
}
