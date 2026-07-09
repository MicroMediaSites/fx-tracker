/**
 * Frontend data-access layer for the wickd local app store (AGT-642).
 *
 * The desktop app's local-first data lives in a SQLite database at
 * `~/.wickd/app.db` (the same data home the wickd CLI uses), owned by the
 * Rust side (`src-tauri/src/local_store/`). This module is the only place
 * the frontend talks to it — components import these functions instead of
 * calling `invoke` directly, so follow-up domain migrations (AGT-645/646/647)
 * have one obvious place to extend.
 *
 * Row shapes intentionally mirror the Zero schema rows (snake_case fields,
 * JSON-encoded sub-objects as strings, epoch-ms timestamps) so components
 * can switch from Zero queries to the local store without remapping.
 */

import { invoke } from '@tauri-apps/api/core';

/** One strategy row in the local store (mirrors Zero's `strategy` table, minus user_id). */
export interface LocalStrategy {
  id: string;
  name: string;
  description: string;
  schema_version: number | null;
  parameters: string | null;
  variables: string | null;
  /** JSON: IndicatorDefinition[] */
  indicators: string;
  /** JSON: EntryRule[] */
  entry_rules: string;
  entry_logic: string | null;
  /** JSON: ExitRule[] */
  exit_rules: string;
  /** JSON: RiskSettings */
  risk_settings: string;
  planning_conversation: string | null;
  auto_note_indicators: string | null;
  pivot_config: string | null;
  strategy_type: string | null;
  script_content: string | null;
  version: number;
  is_active: boolean;
  is_promoted: boolean;
  is_locked: boolean;
  is_archived: boolean;
  /** Epoch milliseconds */
  created_at: number;
  /** Epoch milliseconds */
  updated_at: number;
  /**
   * Provenance tag (AGT-648): `''` = native wickd data, `'candlesight'` =
   * imported from the CandleSight archive (see docs/candlesight-archive.md).
   */
  source: string;
}

/**
 * One trade row in the local store (mirrors Zero's `trade` table, minus
 * user_id). `id` is the raw OANDA trade id; prices/units/P&L are decimal
 * strings (AGT-647).
 */
export interface LocalTrade {
  id: string;
  account_id: string | null;
  instrument: string;
  units: string;
  open_price: string;
  close_price: string | null;
  /** Epoch milliseconds */
  open_time: number;
  /** Epoch milliseconds */
  close_time: number | null;
  realized_pl: string | null;
  /** 'OPEN' | 'CLOSED' | 'CLOSE_WHEN_TRADEABLE' */
  state: string;
  /** Epoch milliseconds */
  synced_at: number;
  /** Epoch milliseconds */
  created_at: number;
  /** Epoch milliseconds */
  updated_at: number;
}

/**
 * One stored AI trade score (mirrors Zero's `trade_score` table, minus
 * user_id). One score per trade — closed trades don't change (AGT-647).
 */
export interface LocalTradeScore {
  id: string;
  trade_id: string;
  score_entry: number;
  score_exit: number;
  score_risk_management: number;
  score_overall: number;
  summary: string;
  entry_assessment: string;
  exit_assessment: string;
  /** JSON: IndicatorFinding[] */
  indicator_analysis: string;
  /** JSON: string[] */
  conflicting_indicators: string;
  /** JSON: string[] */
  learning_points: string;
  /** Epoch milliseconds */
  created_at: number;
}

/** Absolute path of the local store file (for display/diagnostics). */
export function localStorePath(): Promise<string> {
  return invoke<string>('local_store_path');
}

/** All strategies, most recently updated first (includes archived/inactive). */
export function listStrategies(): Promise<LocalStrategy[]> {
  return invoke<LocalStrategy[]>('local_list_strategies');
}

/** One strategy by id, or null if absent. */
export function getStrategy(id: string): Promise<LocalStrategy | null> {
  return invoke<LocalStrategy | null>('local_get_strategy', { id });
}

/** Insert or update (upsert on id) a strategy. */
export function saveStrategy(strategy: LocalStrategy): Promise<void> {
  return invoke<void>('local_save_strategy', { strategy });
}

/** Delete a strategy by id. Resolves to whether a row was removed. */
export function deleteStrategy(id: string): Promise<boolean> {
  return invoke<boolean>('local_delete_strategy', { id });
}

// =============================================================================
// S/R zones (AGT-646)
// =============================================================================

/** One S/R zone row in the local store (mirrors Zero's `sr_zone` table, minus user_id). */
export interface LocalSRZone {
  id: string;
  instrument: string;
  /** Decimal as string — never a float */
  upper_price: string;
  /** Decimal as string — never a float */
  lower_price: string;
  label: string | null;
  color: string | null;
  /** Epoch milliseconds */
  created_at: number;
  /** Epoch milliseconds */
  updated_at: number;
}

/**
 * S/R zones, oldest first. Pass an instrument to scope to one chart; omit it
 * for all zones (watcher trigger maps).
 */
export function listSRZones(instrument?: string): Promise<LocalSRZone[]> {
  return invoke<LocalSRZone[]>('local_list_sr_zones', { instrument: instrument ?? null });
}

/** Insert or update (upsert on id) an S/R zone. */
export function saveSRZone(zone: LocalSRZone): Promise<void> {
  return invoke<void>('local_save_sr_zone', { zone });
}

/** Delete an S/R zone by id. Resolves to whether a row was removed. */
export function deleteSRZone(id: string): Promise<boolean> {
  return invoke<boolean>('local_delete_sr_zone', { id });
}

/** Delete every S/R zone for an instrument. Resolves to the number removed. */
export function clearSRZones(instrument: string): Promise<number> {
  return invoke<number>('local_clear_sr_zones', { instrument });
}

// =============================================================================
// Notes (AGT-646)
// =============================================================================

/** One note row in the local store (mirrors Zero's `note` table, minus user_id). */
export interface LocalNote {
  id: string;
  trade_id: string | null;
  strategy_id: string | null;
  title: string;
  /** Markdown */
  content: string;
  /** Epoch milliseconds */
  created_at: number;
  /** Epoch milliseconds */
  updated_at: number;
}

/**
 * List notes, most recent first. Pass a filter to scope to one trade or one
 * strategy; no filter returns all notes (note-count badges).
 */
export function listNotes(filter?: { tradeId?: string; strategyId?: string }): Promise<LocalNote[]> {
  return invoke<LocalNote[]>('local_list_notes', {
    tradeId: filter?.tradeId ?? null,
    strategyId: filter?.strategyId ?? null,
  });
}

/** Insert or update (upsert on id) a note. */
export function saveNote(note: LocalNote): Promise<void> {
  return invoke<void>('local_save_note', { note });
}

/** Delete a note by id. Resolves to whether a row was removed. */
export function deleteNote(id: string): Promise<boolean> {
  return invoke<boolean>('local_delete_note', { id });
}

/** A fresh note row attached to a trade or strategy, ready for `saveNote`. */
export function newLocalNote(
  content: string,
  attach: { tradeId?: string; strategyId?: string },
): LocalNote {
  const now = Date.now();
  return {
    id: crypto.randomUUID(),
    trade_id: attach.tradeId ?? null,
    strategy_id: attach.strategyId ?? null,
    title: '',
    content,
    created_at: now,
    updated_at: now,
  };
}

// =============================================================================
// Chart config (AGT-646)
// =============================================================================

/** The persisted chart indicator config JSON for an instrument, or null. */
export function getChartConfig(instrument: string): Promise<string | null> {
  return invoke<string | null>('local_get_chart_config', { instrument });
}

/** Persist the chart indicator config JSON for an instrument. */
export function setChartConfig(instrument: string, indicators: string): Promise<void> {
  return invoke<void>('local_set_chart_config', { instrument, indicators });
}

// =============================================================================
// Trades / AI trade scores (AGT-647)
// =============================================================================

/** All trades, most recently opened first (open and closed). */
export function listTrades(): Promise<LocalTrade[]> {
  return invoke<LocalTrade[]>('local_list_trades');
}

/** Closed trades for one instrument (chart trade-marker overlay). */
export function listClosedTradesByInstrument(instrument: string): Promise<LocalTrade[]> {
  return invoke<LocalTrade[]>('local_list_closed_trades_by_instrument', { instrument });
}

/** All stored AI trade scores. */
export function listTradeScores(): Promise<LocalTradeScore[]> {
  return invoke<LocalTradeScore[]>('local_list_trade_scores');
}

/** The stored AI score for one trade, or null if it was never scored. */
export function getTradeScoreByTrade(tradeId: string): Promise<LocalTradeScore | null> {
  return invoke<LocalTradeScore | null>('local_get_trade_score_by_trade', { tradeId });
}

/** Persist the AI score for a trade (upsert on trade_id — one per trade). */
export function saveTradeScore(score: LocalTradeScore): Promise<void> {
  return invoke<void>('local_save_trade_score', { score });
}

/**
 * Read-modify-write partial update (upsert is full-row). Mirrors the partial
 * `zero.mutate.strategy.update` semantics the domain hooks were written
 * against. Rejects if the strategy does not exist.
 */
export async function updateStrategy(
  id: string,
  patch: Partial<Omit<LocalStrategy, 'id'>>,
): Promise<LocalStrategy> {
  const existing = await getStrategy(id);
  if (!existing) {
    throw new Error(`Strategy ${id} not found in local store`);
  }
  const next: LocalStrategy = { ...existing, ...patch, id };
  await saveStrategy(next);
  return next;
}

// =============================================================================
// Backtests domain (AGT-645)
// =============================================================================

/** One saved backtest run (mirrors Zero's `backtest` table, minus user_id). */
export interface LocalBacktest {
  id: string;
  strategy_id: string;
  instrument: string;
  /** Epoch milliseconds */
  start_date: number;
  /** Epoch milliseconds */
  end_date: number;
  /** JSON: full run payload (metrics + trades + equity curve + config) */
  results: string;
  /** Epoch milliseconds */
  created_at: number;
}

/** One backtest/walk-forward job (mirrors Zero's `backtest_job`, minus user_id). */
export interface LocalBacktestJob {
  id: string;
  strategy_id: string;
  /** 'walk_forward' | 'simple_backtest' | 'optimization' */
  job_type: string;
  /** 'pending' | 'running' | 'completed' | 'failed' | 'cancelled' */
  status: string;
  /** JSON: job parameters (instrument, dates, ...) */
  params: string;
  /** 0-100 completion percentage */
  progress: number;
  /** JSON: detailed progress (phase, window, ...) */
  progress_detail: string | null;
  /** JSON: full result when completed */
  result: string | null;
  error_message: string | null;
  /** Epoch milliseconds */
  created_at: number;
  /** Epoch milliseconds */
  updated_at: number;
  /** Epoch milliseconds; set when the job finished */
  completed_at: number | null;
}

/** One promotion/demotion audit row (mirrors Zero's `promotion_audit`, minus user_id). */
export interface LocalPromotionAudit {
  id: string;
  strategy_id: string;
  strategy_name: string;
  /** 'promote' | 'demote' */
  action: string;
  /** Epoch milliseconds */
  created_at: number;
}

/** Saved backtest runs for a strategy, oldest first (run order). */
export function listBacktests(strategyId: string): Promise<LocalBacktest[]> {
  return invoke<LocalBacktest[]>('local_list_backtests', { strategyId });
}

/** Insert or update (upsert on id) a backtest run. */
export function saveBacktest(backtest: LocalBacktest): Promise<void> {
  return invoke<void>('local_save_backtest', { backtest });
}

/** Delete every saved run for a strategy. Resolves to how many were removed. */
export function deleteBacktestsForStrategy(strategyId: string): Promise<number> {
  return invoke<number>('local_delete_backtests_for_strategy', { strategyId });
}

/** Jobs for a strategy, most recently updated first. */
export function listBacktestJobs(strategyId: string): Promise<LocalBacktestJob[]> {
  return invoke<LocalBacktestJob[]>('local_list_backtest_jobs', { strategyId });
}

/** One job by id, or null if absent. */
export function getBacktestJob(id: string): Promise<LocalBacktestJob | null> {
  return invoke<LocalBacktestJob | null>('local_get_backtest_job', { id });
}

/** Insert or update (upsert on id) a backtest job. */
export function saveBacktestJob(job: LocalBacktestJob): Promise<void> {
  return invoke<void>('local_save_backtest_job', { job });
}

/**
 * Read-modify-write partial job update (mirrors `zero.mutate.backtest_job.update`).
 * Rejects if the job does not exist.
 */
export async function updateBacktestJob(
  id: string,
  patch: Partial<Omit<LocalBacktestJob, 'id'>>,
): Promise<LocalBacktestJob> {
  const existing = await getBacktestJob(id);
  if (!existing) {
    throw new Error(`Backtest job ${id} not found in local store`);
  }
  const next: LocalBacktestJob = { ...existing, ...patch, id };
  await saveBacktestJob(next);
  return next;
}

/** Append one promotion/demotion audit row. */
export function recordPromotion(audit: LocalPromotionAudit): Promise<void> {
  return invoke<void>('local_record_promotion', { audit });
}

// =============================================================================
// Labels domain (AGT-650)
// =============================================================================

/** One label row (mirrors Zero's `label` table, minus user_id). */
export interface LocalLabel {
  id: string;
  name: string;
  /** Optional hex color */
  color: string | null;
  /** Epoch milliseconds */
  created_at: number;
}

/** Trade↔label junction row (mirrors Zero's `trade_label`, minus user_id). */
export interface LocalTradeLabel {
  id: string;
  trade_id: string;
  label_id: string;
  /** Epoch milliseconds */
  created_at: number;
}

/** Strategy↔label junction row (mirrors Zero's `strategy_label`, minus user_id). */
export interface LocalStrategyLabel {
  id: string;
  strategy_id: string;
  label_id: string;
  /** Epoch milliseconds */
  created_at: number;
}

/** All labels, alphabetical. */
export function listLabels(): Promise<LocalLabel[]> {
  return invoke<LocalLabel[]>('local_list_labels');
}

/** Insert or update (upsert on id) a label. */
export function saveLabel(label: LocalLabel): Promise<void> {
  return invoke<void>('local_save_label', { label });
}

/** A fresh label row, ready for `saveLabel`. */
export function newLocalLabel(name: string, color: string | null = null): LocalLabel {
  return { id: crypto.randomUUID(), name, color, created_at: Date.now() };
}

/** Trade↔label junctions (scoped to one trade when given). */
export function listTradeLabels(tradeId?: string): Promise<LocalTradeLabel[]> {
  return invoke<LocalTradeLabel[]>('local_list_trade_labels', { tradeId: tradeId ?? null });
}

/** Attach a label to a trade. */
export function addTradeLabel(tradeId: string, labelId: string): Promise<void> {
  const tradeLabel: LocalTradeLabel = {
    id: crypto.randomUUID(),
    trade_id: tradeId,
    label_id: labelId,
    created_at: Date.now(),
  };
  return invoke<void>('local_add_trade_label', { tradeLabel });
}

/** Detach a label from a trade (by junction id). */
export function deleteTradeLabel(id: string): Promise<boolean> {
  return invoke<boolean>('local_delete_trade_label', { id });
}

/** Strategy↔label junctions (scoped to one strategy when given). */
export function listStrategyLabels(strategyId?: string): Promise<LocalStrategyLabel[]> {
  return invoke<LocalStrategyLabel[]>('local_list_strategy_labels', {
    strategyId: strategyId ?? null,
  });
}

/** Attach a label to a strategy. */
export function addStrategyLabel(strategyId: string, labelId: string): Promise<void> {
  const strategyLabel: LocalStrategyLabel = {
    id: crypto.randomUUID(),
    strategy_id: strategyId,
    label_id: labelId,
    created_at: Date.now(),
  };
  return invoke<void>('local_add_strategy_label', { strategyLabel });
}

/** Detach a label from a strategy (by junction id). */
export function deleteStrategyLabel(id: string): Promise<boolean> {
  return invoke<boolean>('local_delete_strategy_label', { id });
}

// =============================================================================
// Strategy-trade attribution (AGT-650)
// =============================================================================

/** One strategy↔OANDA-trade attribution row (mirrors Zero's `strategy_trade`, minus user_id). */
export interface LocalStrategyTrade {
  id: string;
  strategy_id: string;
  strategy_config_id: string | null;
  /** Raw OANDA trade id */
  trade_id: string;
  instrument: string;
  /** e.g. 'H1', 'H4' */
  timeframe: string;
  /** 'long' | 'short' */
  direction: string;
  /** Decimal as string — never a float */
  entry_price: string;
  /** Epoch milliseconds; when the pattern match was detected */
  match_time: number;
  /** Epoch milliseconds; when the trade was executed */
  executed_at: number;
  /** JSON: which entry rules fired */
  rules_triggered: string | null;
  /** Epoch milliseconds */
  created_at: number;
}

/** Attribution rows (scoped to one strategy when given), newest first. */
export function listStrategyTrades(strategyId?: string): Promise<LocalStrategyTrade[]> {
  return invoke<LocalStrategyTrade[]>('local_list_strategy_trades', {
    strategyId: strategyId ?? null,
  });
}

/** Record one strategy↔trade attribution row (upsert on id). */
export function saveStrategyTrade(strategyTrade: LocalStrategyTrade): Promise<void> {
  return invoke<void>('local_save_strategy_trade', { strategyTrade });
}

// =============================================================================
// Strategy-watcher configs (AGT-650)
// =============================================================================

/** One persisted watcher config (mirrors Zero's `strategy_watcher`, minus user_id). */
export interface LocalStrategyWatcher {
  /** Config id: `strategy_id-instrument-timeframe` */
  id: string;
  strategy_id: string;
  /** Cached strategy name for display */
  strategy_name: string | null;
  instrument: string;
  timeframe: string;
  /** 'signal_only' | 'confirm_execute' | 'auto_execute' */
  mode: string;
  /** 'all' | 'entries' | 'exits' | 'longs' | 'shorts' */
  signal_filter: string;
  /** Whether to auto-start on app load */
  is_active: boolean;
  /** Epoch milliseconds */
  created_at: number;
  /** Epoch milliseconds */
  updated_at: number;
}

/** All persisted watcher configs, most recently updated first. */
export function listStrategyWatchers(): Promise<LocalStrategyWatcher[]> {
  return invoke<LocalStrategyWatcher[]>('local_list_strategy_watchers');
}

/** Insert or update (upsert on id) a watcher config. */
export function saveStrategyWatcher(watcher: LocalStrategyWatcher): Promise<void> {
  return invoke<void>('local_save_strategy_watcher', { watcher });
}

/** Delete a watcher config by id. */
export function deleteStrategyWatcher(id: string): Promise<boolean> {
  return invoke<boolean>('local_delete_strategy_watcher', { id });
}

// =============================================================================
// Device credentials (AGT-650)
// =============================================================================

/**
 * The device's encrypted OANDA credential row (mirrors the cloud
 * `user_credentials` table, minus user_id). Blobs are ciphertext produced by
 * the Rust crypto vault — this row is only where the ciphertext lives.
 */
export interface LocalCredential {
  id: string;
  device_id: string;
  practice_blob: string | null;
  practice_account_id: string | null;
  live_blob: string | null;
  live_account_id: string | null;
  /** Epoch milliseconds */
  created_at: number;
  /** Epoch milliseconds */
  updated_at: number;
}

/** The stored credential row, or null before onboarding. */
export function getCredential(): Promise<LocalCredential | null> {
  return invoke<LocalCredential | null>('local_get_credential');
}

/** Insert or update (upsert on id) the device credential row. */
export function saveCredential(credential: LocalCredential): Promise<void> {
  return invoke<void>('local_save_credential', { credential });
}

/** Delete all stored credential rows (the "reset credentials" flow). */
export function deleteCredentials(): Promise<number> {
  return invoke<number>('local_delete_credentials');
}

/** A fresh strategy row with sane defaults, ready for `saveStrategy`. */
export function newLocalStrategy(name: string, description = ''): LocalStrategy {
  const now = Date.now();
  return {
    id: crypto.randomUUID(),
    name,
    description,
    schema_version: 2,
    parameters: null,
    variables: null,
    indicators: '[]',
    entry_rules: '[]',
    entry_logic: null,
    exit_rules: '[]',
    risk_settings: '{}',
    planning_conversation: null,
    auto_note_indicators: null,
    pivot_config: null,
    strategy_type: 'rules',
    script_content: null,
    version: 1,
    is_active: true,
    is_promoted: false,
    is_locked: false,
    is_archived: false,
    created_at: now,
    updated_at: now,
    source: '',
  };
}
