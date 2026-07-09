/**
 * Frontend data-access layer for the unified `.rhai` strategy store
 * (AGT-651): the directory of `.rhai` files at `~/.wickd/strategies/` that
 * the wickd CLI and this app share. Read-only on purpose — the app is a
 * strategy viewer/runner; authoring happens through the CLI
 * (`wickd strategy add/update/remove`, see docs/strategy-store.md).
 *
 * Deliberately a separate module from `localStore.ts`: the local store is
 * the SQLite database (`~/.wickd/app.db`), this is the filesystem store.
 */

import { invoke } from '@tauri-apps/api/core';
import type { ParameterDefinition, IndicatorDefinition } from '../types/strategy';

/** One `.rhai` strategy in the store (metadata parsed from script comments). */
export interface StoreStrategy {
  /** Canonical name = the file stem (`revert_adx.rhai` -> `revert_adx`). */
  name: string;
  /** Absolute path of the `.rhai` file. */
  path: string;
  /** Whether the script passes wickd-core's validate_script. */
  valid: boolean;
  /** Validation error when `valid` is false. */
  error?: string;
  /** Parameters declared in the script's `@parameters` metadata. */
  parameters: ParameterDefinition[];
  /** Indicators declared in the script's `@indicators` metadata. */
  indicators: IndicatorDefinition[];
  /** File modification time (epoch ms). */
  modified_at: number;
  /** Stable content fingerprint. */
  content_hash: string;
}

/** `store_read_strategy` result: the entry plus its full source. */
export interface StoreStrategyWithSource extends StoreStrategy {
  source: string;
}

/** Prefix marking in-memory Strategy rows that come from the file store. */
export const STORE_STRATEGY_ID_PREFIX = 'rhai:';

/** Whether a strategy id refers to a (read-only) file-store entry. */
export function isStoreStrategy(id: string): boolean {
  return id.startsWith(STORE_STRATEGY_ID_PREFIX);
}

/** The store id for a `.rhai` strategy name. */
export function storeStrategyId(name: string): string {
  return `${STORE_STRATEGY_ID_PREFIX}${name}`;
}

/** List every `.rhai` strategy in `~/.wickd/strategies/`. */
export function storeListStrategies(): Promise<StoreStrategy[]> {
  return invoke<StoreStrategy[]>('store_list_strategies');
}

/** Read one stored strategy (metadata + full source) by name. */
export function storeReadStrategy(name: string): Promise<StoreStrategyWithSource> {
  return invoke<StoreStrategyWithSource>('store_read_strategy', { name });
}
