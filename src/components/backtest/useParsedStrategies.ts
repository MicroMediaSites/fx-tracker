/**
 * useParsedStrategies — strategy list for the backtest window, served from the
 * wickd local store (AGT-645; was a Zero synced query before the migration).
 *
 * The local SQLite store is authoritative and in-process, so all the Zero-era
 * sync-anomaly machinery (localStorage known-id cache, sync banners, dev
 * seeding through Zero mutators) is gone. Mutation hooks call the returned
 * `refreshStrategies` after every write; AI-driven writes land through the
 * `strategy-created-by-ai` / `strategy-updated-by-ai` events which also
 * trigger a refresh here.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { Strategy, StrategyType } from '../../types/strategy';
import { LocalStrategy, listStrategies } from '../../lib/localStore';
import {
  StoreStrategy,
  storeListStrategies,
  storeReadStrategy,
  storeStrategyId,
} from '../../lib/strategyStore';

interface UseParsedStrategiesProps {
  showArchived: boolean;
}

/**
 * The local store is single-user; the shared `StrategyDefinition` the backend
 * deserializes still requires a `user_id` field, so parsed rows carry this
 * constant until AGT-650 retires the field from the shared types.
 */
export const LOCAL_USER_ID = 'local';

/** Parse one local store row into the rich in-memory Strategy shape. */
export function parseLocalStrategy(s: LocalStrategy): Strategy {
  return {
    id: s.id,
    user_id: LOCAL_USER_ID,
    name: s.name,
    description: s.description,
    schema_version: s.schema_version ?? undefined,
    parameters: s.parameters ? JSON.parse(s.parameters) : [],
    variables: s.variables ? JSON.parse(s.variables) : [],
    indicators: JSON.parse(s.indicators || '[]'),
    entry_rules: JSON.parse(s.entry_rules || '[]'),
    entry_logic: JSON.parse(s.entry_logic || '{"mode":"all"}'),
    exit_rules: JSON.parse(s.exit_rules || '[]'),
    risk_settings: JSON.parse(
      s.risk_settings ||
        '{"risk_method":"percent","risk_value":1,"rr_ratio":2,"spread_buffer_pips":1}'
    ),
    planning_conversation: s.planning_conversation
      ? JSON.parse(s.planning_conversation)
      : undefined,
    auto_note_indicators: s.auto_note_indicators
      ? JSON.parse(s.auto_note_indicators)
      : undefined,
    pivot_config: s.pivot_config ? JSON.parse(s.pivot_config) : undefined,
    strategy_type: (s.strategy_type as StrategyType | null) || 'rules',
    script_content: s.script_content ?? undefined,
    version: s.version,
    is_active: s.is_active,
    is_promoted: s.is_promoted,
    is_locked: s.is_locked,
    is_archived: s.is_archived,
    created_at: s.created_at,
    updated_at: s.updated_at,
  };
}

/**
 * Parse one unified-store entry (a `.rhai` file in `~/.wickd/strategies/`,
 * AGT-651) into the in-memory Strategy shape. Store entries are read-only
 * scripted strategies: metadata comes from the script's own
 * `@parameters`/`@indicators` comments, and the source is loaded eagerly so
 * the runner (strategyToJson → run_custom_backtest) works unchanged.
 */
export function parseStoreStrategy(entry: StoreStrategy, source: string): Strategy {
  return {
    id: storeStrategyId(entry.name),
    user_id: LOCAL_USER_ID,
    name: entry.name,
    description: `.rhai strategy from the unified store (${entry.path})`,
    schema_version: 2,
    parameters: entry.parameters ?? [],
    variables: [],
    indicators: entry.indicators ?? [],
    entry_rules: [],
    entry_logic: { mode: 'all' },
    exit_rules: [],
    risk_settings: {
      risk_method: 'percent',
      risk_value: 1,
      rr_ratio: 2,
      spread_buffer_pips: 1,
    },
    strategy_type: 'scripted',
    script_content: source,
    version: 1,
    is_active: true,
    is_promoted: false,
    is_locked: false,
    is_archived: false,
    created_at: entry.modified_at,
    updated_at: entry.modified_at,
  };
}

export function useParsedStrategies({ showArchived }: UseParsedStrategiesProps) {
  const [rows, setRows] = useState<LocalStrategy[] | null>(null);
  const [storeStrategies, setStoreStrategies] = useState<Strategy[]>([]);
  const [strategiesLoading, setStrategiesLoading] = useState(true);

  // Track mount to avoid setState after unmount from in-flight refreshes.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refreshStrategies = useCallback(async () => {
    try {
      const all = await listStrategies();
      if (!mountedRef.current) return;
      setRows(all);
    } catch (err) {
      console.error('[useParsedStrategies] Failed to load strategies:', err);
      if (!mountedRef.current) return;
      setRows((prev) => prev ?? []);
    } finally {
      if (mountedRef.current) setStrategiesLoading(false);
    }

    // Unified `.rhai` store (AGT-651): list + eagerly load source for valid
    // entries so they are immediately runnable. A missing/broken store is an
    // empty list, never an error state for the window.
    try {
      const entries = await storeListStrategies();
      const parsed = await Promise.all(
        entries
          .filter((e) => e.valid)
          .map(async (e) => {
            const withSource = await storeReadStrategy(e.name);
            return parseStoreStrategy(e, withSource.source);
          })
      );
      if (!mountedRef.current) return;
      setStoreStrategies(parsed);
    } catch (err) {
      console.error('[useParsedStrategies] Failed to load .rhai store strategies:', err);
      if (!mountedRef.current) return;
      setStoreStrategies([]);
    }
  }, []);

  // Initial load.
  useEffect(() => {
    refreshStrategies();
  }, [refreshStrategies]);

  // AI terminal writes strategies out-of-band — refresh when it tells us.
  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    for (const eventName of ['strategy-created-by-ai', 'strategy-updated-by-ai']) {
      listen(eventName, () => {
        refreshStrategies();
      }).then((fn) => {
        if (cancelled) fn();
        else unlisteners.push(fn);
      });
    }
    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, [refreshStrategies]);

  // Parse local rows into the rich Strategy shape. Matches the retired Zero
  // query semantics: only active strategies, archived filtered by toggle.
  const parsedStrategies = useMemo((): Strategy[] => {
    const local = (rows ?? [])
      .filter((s) => s.is_active)
      .filter((s) => showArchived || !s.is_archived)
      .map(parseLocalStrategy);
    // Store strategies first: they are the canonical strategy world
    // (AGT-651); local rows remain for archived/imported data.
    return [...storeStrategies, ...local];
  }, [rows, storeStrategies, showArchived]);

  // All active strategy names (including archived) for uniqueness checking.
  const allStrategyNames = useMemo(() => {
    const names = (rows ?? [])
      .filter((s) => s.is_active)
      .map((s) => s.name.toLowerCase());
    return new Set([...names, ...storeStrategies.map((s) => s.name.toLowerCase())]);
  }, [rows, storeStrategies]);

  return {
    parsedStrategies,
    allStrategyNames,
    strategiesLoading,
    refreshStrategies,
  };
}
