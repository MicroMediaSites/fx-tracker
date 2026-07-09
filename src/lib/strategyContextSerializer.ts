/**
 * Serializes strategy rules and walk-forward results into compact,
 * human-readable text for AI chat context.
 */

import {
  EntryRuleV2,
  ExitRuleV2,
  ParameterDefinition,
  Condition,
  TriggerV2,
  DataSourceV2,
  ParameterizedValue,
  IndicatorDefinition,
  StrategyVariable,
  RiskSettings,
  WalkForwardResult,
  WalkForwardPeriod,
  isIndicatorSourceV2,
  isPriceSourceV2,
  isFixedSourceV2,
  isParameterSourceV2,
  isVariableSource,
  isCrossTriggerV2,
  isCompareTriggerV2,
  isThresholdTriggerV2,
  isParameterReference,
  INDICATOR_FULL_NAMES,
  IndicatorType,
} from '../types/strategy';

// ============================================================================
// Parameter value formatting
// ============================================================================

function formatParamValue(
  value: ParameterizedValue,
  params: ParameterDefinition[],
): string {
  if (isParameterReference(value)) {
    const param = params.find(p => p.id === value.$param);
    if (param) {
      const isFixed = param.min !== undefined && param.max !== undefined && param.min === param.max;
      if (isFixed) return `${param.default} (fixed)`;
      return `${param.default} [${param.min ?? '?'}–${param.max ?? '?'}, step ${param.step ?? 1}]`;
    }
    return `$${value.$param}`;
  }
  return String(value);
}

// ============================================================================
// Data source formatting
// ============================================================================

function formatDataSource(
  source: DataSourceV2,
  indicators: IndicatorDefinition[],
  params: ParameterDefinition[],
  variables?: StrategyVariable[],
): string {
  if (typeof source === 'number') return String(source);

  if (isIndicatorSourceV2(source)) {
    const ind = indicators.find(i => i.id === source.indicator);
    const indName = ind
      ? INDICATOR_FULL_NAMES[ind.type as IndicatorType] || ind.type
      : source.indicator;
    const output = source.output;
    const outputLabel = output;
    const offsetStr = source.offset !== undefined
      ? ` [${formatParamValue(source.offset, params)} bars back]`
      : '';
    const tfStr = source.timeframe ? ` (${source.timeframe})` : '';
    return `${indName} ${outputLabel}${tfStr}${offsetStr}`;
  }

  if (isPriceSourceV2(source)) {
    const offsetStr = source.offset !== undefined
      ? ` [${formatParamValue(source.offset, params)} bars back]`
      : '';
    const tfStr = source.timeframe ? ` (${source.timeframe})` : '';
    return `Price ${source.value}${tfStr}${offsetStr}`;
  }

  if (isFixedSourceV2(source)) return String(source.fixed);
  if (isParameterSourceV2(source)) return formatParamValue(source, params);

  if (isVariableSource(source)) {
    const variable = variables?.find(v => v.id === source.variable);
    return variable?.name || source.variable;
  }

  if ('source' in source && source.source === 'pattern') {
    return `Pattern(${(source as { pattern?: string }).pattern || 'unknown'})`;
  }

  return JSON.stringify(source);
}

// ============================================================================
// Trigger formatting
// ============================================================================

function formatTrigger(
  trigger: TriggerV2,
  indicators: IndicatorDefinition[],
  params: ParameterDefinition[],
  variables?: StrategyVariable[],
): string {
  const fmt = (s: DataSourceV2) => formatDataSource(s, indicators, params, variables);
  const fmtPV = (v: ParameterizedValue) => formatParamValue(v, params);

  if (isCrossTriggerV2(trigger)) {
    const lookback = trigger.lookback !== undefined ? ` within ${fmtPV(trigger.lookback)} bars` : '';
    return `${fmt(trigger.left)} crossed ${trigger.direction} ${fmt(trigger.right)}${lookback}`;
  }

  if (isCompareTriggerV2(trigger)) {
    const op = trigger.operator === 'is_within' ? 'is within' : trigger.operator;
    const lookback = trigger.lookback !== undefined ? ` within ${fmtPV(trigger.lookback)} bars` : '';
    let dist = '';
    if (trigger.distance) {
      dist = ` ${fmtPV(trigger.distance.value)} ${trigger.distance.unit}`;
    }
    return `${fmt(trigger.left)} ${op} ${fmt(trigger.right)}${dist}${lookback}`;
  }

  if (isThresholdTriggerV2(trigger)) {
    const lookback = trigger.lookback !== undefined ? ` within ${fmtPV(trigger.lookback)} bars` : '';
    const opLabel = trigger.operator === 'crosses_above' ? 'crossed above'
      : trigger.operator === 'crosses_below' ? 'crossed below'
      : trigger.operator;
    return `${fmt(trigger.source)} ${opLabel} ${fmtPV(trigger.value)}${lookback}`;
  }

  if (trigger.type === 'givens') {
    return `Market regime: ${trigger.regime}`;
  }

  if (trigger.type === 'time') {
    return `Time condition: ${trigger.condition} ${fmtPV(trigger.value)}`;
  }

  if (trigger.type === 'time_in_range') {
    return `Time between ${trigger.start_hour}:${String(trigger.start_minute).padStart(2, '0')} and ${trigger.end_hour}:${String(trigger.end_minute).padStart(2, '0')} UTC`;
  }

  if (trigger.type === 'day_of_week') {
    const dayNames = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
    const days = trigger.days?.map(d => dayNames[d] || String(d)).join(', ');
    return trigger.exclude ? `Exclude days: ${days}` : `Only on days: ${days}`;
  }

  if (trigger.type === 'risk_reward_reached') {
    return `Risk/reward ratio reached ${fmtPV(trigger.ratio)}`;
  }

  if (trigger.type === 'percent_of_tp_reached') {
    return `${fmtPV(trigger.percent)}% of take profit reached`;
  }

  return JSON.stringify(trigger);
}

// ============================================================================
// Condition formatting
// ============================================================================

function formatCondition(
  condition: Condition,
  indicators: IndicatorDefinition[],
  params: ParameterDefinition[],
  variables?: StrategyVariable[],
): string {
  const fmtT = (t: TriggerV2) => formatTrigger(t, indicators, params, variables);

  const primary = condition.primary.negated
    ? `NOT (${fmtT(condition.primary.trigger)})`
    : fmtT(condition.primary.trigger);

  if (!condition.chain || condition.chain.length === 0) return primary;

  const parts = [primary];
  for (const chained of condition.chain) {
    const triggerStr = chained.trigger.negated
      ? `NOT (${fmtT(chained.trigger.trigger)})`
      : fmtT(chained.trigger.trigger);
    parts.push(`${chained.operator.toUpperCase()} ${triggerStr}`);
  }
  return parts.join(' ');
}

// ============================================================================
// Public API
// ============================================================================

export function serializeStrategyRules(
  entryRules: EntryRuleV2[],
  exitRules: ExitRuleV2[],
  indicators: IndicatorDefinition[],
  params: ParameterDefinition[],
  riskSettings?: RiskSettings,
  variables?: StrategyVariable[],
): string {
  const lines: string[] = [];

  // Entry rules
  if (entryRules.length > 0) {
    lines.push('ENTRY RULES:');
    for (const rule of entryRules) {
      const name = rule.name || rule.id;
      const dir = rule.direction === 'both' ? '' : ` (${rule.direction})`;
      lines.push(`  ${name}${dir} — all must be true:`);
      for (let i = 0; i < rule.conditions.length; i++) {
        const c = rule.conditions[i];
        const disabled = !!c.disabled;
        const prefix = disabled ? '    [DISABLED] ' : '    ';
        const condName = c.name ? `${c.name}: ` : '';
        lines.push(`${prefix}${i + 1}. ${condName}${formatCondition(c, indicators, params, variables)}`);
      }
    }
  }

  // Exit rules
  if (exitRules.length > 0) {
    lines.push('EXIT RULES:');
    for (const rule of exitRules) {
      const name = rule.name || rule.id;
      const dir = rule.direction === 'both' ? '' : ` (${rule.direction})`;
      const pct = formatParamValue(rule.close_percent, params);
      lines.push(`  ${name}${dir} — close ${pct}%, priority ${rule.priority}:`);
      for (let i = 0; i < rule.conditions.length; i++) {
        const c = rule.conditions[i];
        const condName = c.name ? `${c.name}: ` : '';
        lines.push(`    ${i + 1}. ${condName}${formatCondition(c, indicators, params, variables)}`);
      }
    }
  }

  // Risk settings
  if (riskSettings) {
    lines.push('RISK SETTINGS:');
    lines.push(`  Method: ${riskSettings.risk_method}, Value: ${formatParamValue(riskSettings.risk_value, params)}`);
    lines.push(`  R:R Ratio: ${formatParamValue(riskSettings.rr_ratio, params)}`);
    if (riskSettings.spread_buffer_pips !== undefined) {
      lines.push(`  Spread Buffer: ${formatParamValue(riskSettings.spread_buffer_pips, params)} pips`);
    }
  }

  return lines.join('\n');
}

export function serializeParameterDefinitions(params: ParameterDefinition[]): string {
  if (params.length === 0) return '';
  const lines = ['PARAMETERS:'];
  for (const p of params) {
    const isFixed = p.min !== undefined && p.max !== undefined && p.min === p.max;
    const range = p.min !== undefined && p.max !== undefined
      ? isFixed
        ? ` (fixed at ${p.min})`
        : ` range [${p.min}–${p.max}, step ${p.step ?? 1}]`
      : '';
    lines.push(`  ${p.name || p.id}: default=${p.default}${range}`);
  }
  return lines.join('\n');
}

export function serializeWindowSummary(result: WalkForwardResult): string {
  const lines = [`WALK-FORWARD RESULTS (${result.periods?.length ?? 0} windows):`];
  const sharpe = (result.oos_avg_sharpe ?? 0).toFixed(2);
  lines.push(`  Overall: Return ${result.oos_total_return_pct ?? 'N/A'}%, ${result.oos_total_trades ?? 0} trades, Win Rate ${result.oos_win_rate ?? 'N/A'}%, Sharpe ${sharpe}, Robustness ${result.robustness_score ?? 0}/100`);
  lines.push('');

  for (const period of result.periods ?? []) {
    const w = period.window;
    const trainStart = new Date(w.train_start).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
    const testStart = new Date(w.test_start).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
    const testEnd = new Date(w.test_end).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
    const m = period.out_of_sample_metrics;
    const pnl = parseFloat(m.total_pnl) || 0;
    const ret = parseFloat(m.total_return_pct) || 0;
    const params = Object.entries(period.optimized_params)
      .map(([k, v]) => `${k}=${v}`)
      .join(', ');

    const oosSharpe = (period.out_of_sample_sharpe ?? 0).toFixed(2);
    lines.push(`  W${w.window_num}: Train ${trainStart}→${testStart}, Test ${testStart}→${testEnd} | ${period.oos_trade_count ?? 0} trades, ${ret >= 0 ? '+' : ''}${ret.toFixed(1)}% ($${pnl >= 0 ? '+' : ''}${pnl.toFixed(0)}), Sharpe ${oosSharpe} | params: {${params}}`);
  }

  return lines.join('\n');
}

export function serializeSelectedWindow(
  period: WalkForwardPeriod,
  strategyParams: ParameterDefinition[],
): string {
  const w = period.window;
  const trainStart = new Date(w.train_start).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
  const trainEnd = new Date(w.train_end).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
  const testStart = new Date(w.test_start).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
  const testEnd = new Date(w.test_end).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });

  const lines = [`CURRENTLY VIEWING — Window ${w.window_num}:`];
  lines.push(`  Train: ${trainStart} → ${trainEnd}`);
  lines.push(`  Test: ${testStart} → ${testEnd}`);

  // Optimized params with context from definitions
  const paramLines = Object.entries(period.optimized_params).map(([id, value]) => {
    const def = strategyParams.find(p => p.id === id);
    const name = def?.name || id;
    return `${name}=${value}`;
  });
  lines.push(`  Optimized params: {${paramLines.join(', ')}}`);

  // OOS metrics
  const m = period.out_of_sample_metrics;
  const retPct = parseFloat(m.total_return_pct) || 0;
  const oosSharpe = (period.out_of_sample_sharpe ?? 0).toFixed(2);
  lines.push(`  OOS: ${period.oos_trade_count ?? 0} trades, ${retPct >= 0 ? '+' : ''}${retPct.toFixed(1)}%, Sharpe ${oosSharpe}, Win Rate ${m.win_rate ?? 'N/A'}%`);

  // Trades (capped to prevent context bloat)
  const MAX_TRADES = 20;
  const trades = period.oos_trades ?? [];
  if (trades.length > 0) {
    lines.push('  Trades:');
    const displayTrades = trades.slice(0, MAX_TRADES);
    for (const trade of displayTrades) {
      const dir = trade.isLong ? 'Long' : 'Short';
      const entryDate = new Date(trade.entryTime).toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
      const exitDate = trade.exitTime
        ? new Date(trade.exitTime).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
        : 'open';
      const pnl = parseFloat(trade.pnl) || 0;
      const exitReason = trade.exitReason ? ` (${trade.exitReason})` : '';
      const ruleName = trade.entryRuleName ? ` via "${trade.entryRuleName}"` : '';
      lines.push(`    ${dir} ${trade.entryPrice} (${entryDate}) → ${trade.exitPrice || '?'} (${exitDate})${exitReason}${ruleName} | P&L: $${pnl >= 0 ? '+' : ''}${pnl.toFixed(2)}`);
    }
    if (trades.length > MAX_TRADES) {
      lines.push(`    ... and ${trades.length - MAX_TRADES} more trades`);
    }
  }

  return lines.join('\n');
}
