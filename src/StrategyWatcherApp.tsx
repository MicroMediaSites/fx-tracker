/**
 * Live Monitor — a window onto the ONE watcher engine (AGT-652).
 *
 * The app no longer runs strategies in-process. `wickd watch` (typically
 * launchd-supervised) is the single watcher runtime on the machine; this
 * window renders what that daemon publishes:
 *
 * - Daemon status: the running `wickd … watch …` processes + stream hub state.
 * - Live prices for the daemon's watched instruments (ticks come from the
 *   stream hub via the app's hub-first streaming path).
 * - The live signal feed (`~/.wickd/alert-queue.ndjson`).
 * - The semi-auto pending/approve queue (`~/.wickd/pending.json`) — read-only:
 *   approval stays a deliberate `wickd approve <id>` in the CLI.
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useWatchDaemon, type PendingSignal } from './hooks/useWatchDaemon';
import { usePriceStream } from './hooks/usePriceStream';
import { useEnvironmentSync } from './hooks/useEnvironmentSync';
import { WindowHeader } from './components/ui/WindowHeader';
import { CollapsibleSection } from './components/ui/CollapsibleSection';
import { PriceWindow } from './components/ui/PriceDisplay';
import { buildWatcherContext } from './lib/chatContextBuilder';
import { getTerminalWelcome } from './lib/terminalWelcome';

import {
  StrategyRow,
  ExternalWatcherRow,
  type StrategyRowConfig,
  type RowMode,
} from './components/watcher/StrategyRow';
import { OpenChartButton, chartableGranularity } from './components/watcher/OpenChartButton';
import { useSettingsStore } from './stores/settingsStore';

const formatTs = (ts: string): string => {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  return date.toLocaleString();
};

/** Parse `--granularity <X>` out of a watcher's command line. */
const parseGranularity = (command: string): string | null =>
  command.match(/--granularity\s+(\S+)/)?.[1] ?? null;

const CopyApproveCommand = ({ signalId }: { signalId: string }) => {
  const [copied, setCopied] = useState(false);
  const command = `wickd approve ${signalId}`;
  return (
    <button
      data-testid="copy-approve-command"
      onClick={() => {
        void navigator.clipboard?.writeText(command).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        });
      }}
      className="px-2 py-1 text-xs font-mono rounded border border-[var(--color-border)] text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] hover:border-[var(--color-info)] transition-colors"
      title="Copy the CLI approval command — execution stays in wickd"
    >
      {copied ? 'Copied!' : command}
    </button>
  );
};

interface InstrumentWatcher {
  strategy: string;
  granularity: string | null;
  pid: number;
  /** Observed trust-ladder mode, parsed from the command's flags. */
  mode: RowMode | 'auto';
  /** Whether the UI may stop it: plain `wickd` CLI binary — pinned/supervised
   * binaries (e.g. launchd wickd-h021 evals) are protected by the backend. */
  stoppable: boolean;
}

/** localStorage key for the per-instrument strategy row configs. */
const STRATEGY_ROWS_KEY = 'wickd-monitor-strategy-rows';

/** Store entry from store_list_strategies — indicators/parameters are the
 * script's structured @indicators / @parameters metadata. */
interface StoreStrategy {
  name: string;
  valid: boolean;
  indicators?: unknown[];
  parameters?: unknown[];
}

/** How many watcher chips a tile shows before collapsing behind "+N more". */
const MAX_VISIBLE_WATCHER_CHIPS = 3;

/** localStorage key for user-pinned price-only instruments. */
const PINNED_INSTRUMENTS_KEY = 'wickd-monitor-pinned-instruments';

/** Ghost tile at the end of the Watching grid, mirroring a real tile's
 * anatomy so it aligns with the price boxes: a header spot where the pair
 * name would sit ("+ Add Pair") and a dashed body at price-window height.
 * Inline editing philosophy: clicking either swaps nothing around — it just
 * opens the pair dropdown (with "Add all") in place under the header spot;
 * picking a pair pins a price tile there. */
const AddPairTile = ({
  available,
  onAdd,
  onAddAll,
}: {
  available: string[];
  onAdd: (instrument: string) => void;
  onAddAll: () => void;
}) => {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    document.addEventListener('mousedown', onOutside);
    document.addEventListener('keydown', onEscape);
    return () => {
      document.removeEventListener('mousedown', onOutside);
      document.removeEventListener('keydown', onEscape);
    };
  }, [open]);

  if (available.length === 0) return null;

  return (
    <div ref={containerRef} className="w-[200px] relative">
      {/* Header spot — same anatomy as a real tile's pair-name row */}
      <div className="px-1 mb-1 flex items-center gap-1 text-sm font-semibold">
        <button
          data-testid="add-pair-trigger"
          onClick={() => setOpen(!open)}
          className="text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors"
        >
          + Add Pair
        </button>
      </div>
      {/* Body aligned with the price windows */}
      <button
        aria-label="Add pair"
        onClick={() => setOpen(true)}
        className="w-full h-24 rounded border border-dashed border-[var(--color-border)] text-lg text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] hover:border-[var(--color-info)] transition-colors"
      >
        +
      </button>
      {open && (
        <div
          data-testid="add-pair-menu"
          className="absolute left-0 top-7 z-20 w-[184px] max-h-56 overflow-y-auto rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)] shadow-lg py-1"
        >
          <button
            data-testid="add-pair-all"
            onClick={() => {
              onAddAll();
              setOpen(false);
            }}
            className="w-full px-3 py-1.5 text-left text-xs font-medium text-[var(--color-info-text)] hover:bg-[var(--color-bg-card)] transition-colors"
          >
            Add all ({available.length})
          </button>
          <div className="my-1 border-t border-[var(--color-border)]" />
          {available.map((symbol) => (
            <button
              key={symbol}
              onClick={() => {
                onAdd(symbol);
                setOpen(false);
              }}
              className="w-full px-3 py-1.5 text-left text-xs text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-card)] hover:text-[var(--color-text-primary)] transition-colors"
            >
              {symbol.replace('_', '/')}
            </button>
          ))}
        </div>
      )}
    </div>
  );
};

/** Price tile for a watched instrument. Below the price window, one
 * inline-editable row per strategy (see StrategyRow): configs the user
 * created here — reconciled against running processes so a config with a
 * matching process renders armed with its observed mode, and one without
 * renders disarmed — plus read-only rows for externally-managed watchers
 * (launchd/pinned). Rows beyond the cap collapse behind "+N more". */
const WatchedInstrumentTile = ({
  instrument,
  processes,
  rowConfigs,
  strategies,
  environment,
  lastAddedRowId,
  stoppedPids,
  indicatorSeedFor,
  onPidStopped,
  onConfigChange,
  onConfigDelete,
  onAddRow,
  onProcessChange,
  onRemove,
}: {
  instrument: string;
  processes: InstrumentWatcher[];
  rowConfigs: StrategyRowConfig[];
  strategies: string[];
  environment: string;
  lastAddedRowId: string | null;
  stoppedPids: number[];
  indicatorSeedFor: (strategyName: string) => string | undefined;
  onPidStopped: (pid: number) => void;
  onConfigChange: (next: StrategyRowConfig) => void;
  onConfigDelete: (id: string) => void;
  onAddRow: () => void;
  onProcessChange: () => void;
  onRemove?: () => void;
}) => {
  const [expanded, setExpanded] = useState(false);

  // Reconcile configs against running processes. A config that started a
  // process binds by pid ONLY — matching anything else would re-attach it to
  // a stale predecessor after an edit/mode change and lag the UI a full poll.
  // Configs without a pid (legacy/adopting) may claim a matching stoppable
  // process by (strategy, granularity). Auto-mode processes are never claimed.
  // Unclaimed processes render as external rows: stoppable when they're the
  // plain wickd CLI, locked when pinned/supervised.
  const claimedPids = new Set<number>();
  const bindings = rowConfigs.map((config) => {
    const proc =
      config.pid != null
        ? processes.find((p) => !claimedPids.has(p.pid) && p.pid === config.pid)
        : processes.find(
            (p) =>
              !claimedPids.has(p.pid) &&
              p.mode !== 'auto' &&
              p.stoppable &&
              p.strategy === config.strategy &&
              p.granularity === config.granularity
          );
    if (proc) claimedPids.add(proc.pid);
    return { config, proc: proc ?? null };
  });
  const externals = processes
    .filter((p) => !claimedPids.has(p.pid) && !stoppedPids.includes(p.pid))
    .sort((a, b) => a.pid - b.pid);

  // A config renders its own (possibly optimistic) mode while inside the
  // post-transition grace window; after that only a bound process keeps it
  // armed. Lets clicks paint instantly even when process ops are slow
  // (Gatekeeper can stall spawn/kill for seconds on some machines).
  const ARM_GRACE_MS = 10_000;
  const displayedMode = (config: StrategyRowConfig, proc: InstrumentWatcher | null): RowMode => {
    if (proc) return proc.mode as RowMode;
    if (Date.now() - (config.armedAt ?? 0) < ARM_GRACE_MS) return config.mode;
    return 'disarmed';
  };

  // Stable order: running external processes first (sorted by pid), then user
  // configs in creation order — a new row lands at the bottom, next to the
  // "+ strategy" button that created it.
  const rows = [
    ...externals.map((proc) => (
      <ExternalWatcherRow
        key={`ext-${proc.pid}`}
        strategy={proc.strategy}
        granularity={proc.granularity}
        mode={proc.mode}
        pid={proc.pid}
        onOpenChart={() => {
          void invoke('open_chart_window', {
            instrument,
            granularity: chartableGranularity(proc.granularity),
            indicators: indicatorSeedFor(proc.strategy),
          }).catch((err) => console.error('[LiveMonitor] Failed to open chart:', err));
        }}
        onStop={
          proc.stoppable
            ? () => {
                onPidStopped(proc.pid); // hide immediately, don't wait a poll
                void invoke('stop_watcher', { pid: proc.pid })
                  .catch((err) => console.error('[LiveMonitor] Failed to stop watcher:', err))
                  .finally(onProcessChange);
              }
            : undefined
        }
      />
    )),
    ...bindings.map(({ config, proc }) => (
      <StrategyRow
        key={config.id}
        config={{ ...config, mode: displayedMode(config, proc) }}
        pid={proc?.pid ?? null}
        strategies={strategies}
        environment={environment}
        onConfigChange={onConfigChange}
        onDelete={() => onConfigDelete(config.id)}
        onProcessChange={onProcessChange}
        onPidStopped={onPidStopped}
        onOpenChart={() => {
          void invoke('open_chart_window', {
            instrument,
            granularity: chartableGranularity(config.granularity),
            indicators: indicatorSeedFor(config.strategy),
          }).catch((err) => console.error('[LiveMonitor] Failed to open chart:', err));
        }}
        startEditingName={config.id === lastAddedRowId}
      />
    )),
  ];
  const visibleRows = expanded ? rows : rows.slice(0, MAX_VISIBLE_WATCHER_CHIPS);
  const hiddenCount = rows.length - visibleRows.length;

  return (
    <div className="w-[200px]">
      <div className="px-1 mb-1 flex items-center gap-1 text-sm font-semibold">
        {instrument.replace('_', '/')}
        <OpenChartButton instrument={instrument} granularity={processes[0]?.granularity} />
        {onRemove && processes.length === 0 && rowConfigs.length === 0 && (
          <button
            data-testid="remove-instrument"
            onClick={onRemove}
            className="ml-auto p-0.5 text-[var(--color-text-muted)] hover:text-[var(--color-sell)] transition-colors"
            title={`Stop watching ${instrument.replace('_', '/')}`}
            aria-label={`Remove ${instrument.replace('_', '/')}`}
          >
            <svg className="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" d="M6 6l12 12M18 6L6 18" />
            </svg>
          </button>
        )}
      </div>
      <PriceWindow instrument={instrument} />
      <div className="mt-1.5 space-y-1">
        {visibleRows}
        {(hiddenCount > 0 || expanded) && rows.length > MAX_VISIBLE_WATCHER_CHIPS && (
          <button
            data-testid="watcher-chip-expander"
            onClick={() => setExpanded(!expanded)}
            className="w-full px-2 py-0.5 text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors"
          >
            {expanded ? 'show less' : `+${hiddenCount} more`}
          </button>
        )}
        <button
          data-testid="add-strategy-button"
          onClick={onAddRow}
          className="w-full px-2 py-0.5 rounded border border-dashed border-[var(--color-border)] text-xs text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] hover:border-[var(--color-info)] transition-colors"
        >
          + strategy
        </button>
      </div>
    </div>
  );
};

const PendingSignalRow = ({ signal, granularity }: { signal: PendingSignal; granularity?: string | null }) => (
  <div
    data-testid="pending-signal-row"
    className="flex flex-wrap items-center justify-between gap-2 px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
  >
    <div className="flex items-center gap-3 min-w-0">
      <span
        className={`px-1.5 py-0.5 text-xs font-semibold rounded uppercase ${
          signal.side === 'long'
            ? 'bg-[var(--color-buy)]/15 text-[var(--color-buy)]'
            : 'bg-[var(--color-sell)]/15 text-[var(--color-sell)]'
        }`}
      >
        {signal.side}
      </span>
      <span className="font-semibold">{signal.instrument}</span>
      <OpenChartButton instrument={signal.instrument} granularity={granularity} />
      <span className="text-sm text-[var(--color-text-secondary)]">{signal.strategy}</span>
      <span className="text-xs text-[var(--color-text-muted)]">{formatTs(signal.ts)}</span>
    </div>
    <div className="flex items-center gap-3 text-xs text-[var(--color-text-secondary)]">
      <span>{signal.units} units</span>
      {signal.entry_price && <span>@ {signal.entry_price}</span>}
      {signal.sl && <span>SL {signal.sl}</span>}
      {signal.tp && <span>TP {signal.tp}</span>}
      <CopyApproveCommand signalId={signal.id} />
    </div>
    {signal.reason && (
      <div className="w-full text-xs text-[var(--color-text-muted)] truncate" title={signal.reason}>
        {signal.reason}
      </div>
    )}
  </div>
);

export const StrategyWatcherApp = () => {
  // Keep the environment badge in sync across windows (BUG-024).
  useEnvironmentSync();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [environment, setEnvironment] = useState('practice');
  const [startableStrategies, setStartableStrategies] = useState<string[]>([]);
  const { status, pending, hubStream, error, loading, refresh } = useWatchDaemon();

  // Instruments pinned by the user as price-only tiles (persisted per app).
  // Watcher/hub instruments appear automatically; pins let you watch a price
  // first and attach a strategy to it after.
  const [pinnedInstruments, setPinnedInstruments] = useState<string[]>(() => {
    try {
      const raw = JSON.parse(localStorage.getItem(PINNED_INSTRUMENTS_KEY) ?? '[]');
      return Array.isArray(raw) ? raw.filter((v): v is string => typeof v === 'string') : [];
    } catch {
      return [];
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(PINNED_INSTRUMENTS_KEY, JSON.stringify(pinnedInstruments));
    } catch {
      // Persistence is best-effort
    }
  }, [pinnedInstruments]);

  // Current OANDA environment for the add-strategy form (same source as the
  // header badge).
  useEffect(() => {
    invoke<{ environment?: string }>('get_oanda_credentials')
      .then((creds) => setEnvironment(creds?.environment === 'live' ? 'live' : 'practice'))
      .catch(() => setEnvironment('practice'));
  }, []);

  // Startable strategies for the per-tile add-strategy form: valid store
  // scripts plus the CLI's built-ins. The full store entries are kept so
  // chart links can seed the chart with the strategy's declared @indicators.
  const [storeStrategies, setStoreStrategies] = useState<StoreStrategy[]>([]);
  useEffect(() => {
    invoke<StoreStrategy[]>('store_list_strategies')
      .then((list) => {
        const valid = list.filter((s) => s.valid);
        setStoreStrategies(valid);
        const names = valid.map((s) => s.name);
        const builtins = ['ma-crossover', 'rsi'].filter((b) => !names.includes(b));
        setStartableStrategies([...names, ...builtins]);
      })
      .catch(() => setStartableStrategies(['ma-crossover', 'rsi']));
  }, []);

  // JSON envelope for ChartApp's indicator seed (see useChartParams /
  // strategyIndicatorsToChartConfigs): the strategy's declared @indicators
  // with $param refs resolved against its @parameters defaults.
  const indicatorSeedFor = (strategyName: string): string | undefined => {
    const entry = storeStrategies.find((s) => s.name === strategyName);
    if (!entry || !Array.isArray(entry.indicators) || entry.indicators.length === 0) {
      return undefined;
    }
    return JSON.stringify({ indicators: entry.indicators, parameters: entry.parameters });
  };

  // Price monitoring: everything the daemon watches, plus whatever the hub is
  // observed streaming. Subscribing to hub-covered instruments never opens a
  // second upstream connection.
  // Ordered de-dupe, NOT alphabetical: watcher instruments first (daemon
  // order), then hub-observed, then user pins in the order they were added —
  // so a newly added pair appears where you put it.
  const watchedInstruments = useMemo(() => {
    const set = new Set<string>();
    for (const watcher of status?.watchers ?? []) {
      for (const instrument of watcher.instruments) set.add(instrument);
    }
    for (const instrument of hubStream?.observed ?? []) set.add(instrument);
    for (const instrument of pinnedInstruments) set.add(instrument);
    return Array.from(set);
  }, [status, hubStream, pinnedInstruments]);

  usePriceStream(watchedInstruments);

  // Pairs offered by the "+ Add Pair" tile: the user's symbols not already
  // shown in the grid.
  const { mySymbols } = useSettingsStore();
  const availablePairs = useMemo(
    () => mySymbols.filter((s) => !watchedInstruments.includes(s)),
    [mySymbols, watchedInstruments]
  );

  // Instrument -> the watchers trading it, with granularity parsed from each
  // watch command line. Drives the per-tile watcher chips (the strategy and
  // timeframe are what distinguish two processes of the same strategy) and
  // lets chart links open at the timeframe the watcher actually trades.
  const watchersByInstrument = useMemo(() => {
    const map = new Map<string, InstrumentWatcher[]>();
    for (const watcher of status?.watchers ?? []) {
      const granularity = parseGranularity(watcher.command);
      const mode: InstrumentWatcher['mode'] = /\s--auto(\s|$)/.test(watcher.command)
        ? 'auto'
        : /\s--semi-auto(\s|$)/.test(watcher.command)
          ? 'semi-auto'
          : 'monitor';
      const binary = watcher.command.split(/\s+/)[0] ?? '';
      const stoppable = (binary.split('/').pop() ?? '') === 'wickd';
      for (const instrument of watcher.instruments) {
        const list = map.get(instrument) ?? [];
        list.push({
          strategy: watcher.strategy ?? 'unknown',
          granularity,
          pid: watcher.pid,
          mode,
          stoppable,
        });
        map.set(instrument, list);
      }
    }
    return map;
  }, [status]);

  // Strategy row configs: rows the user creates in the UI, persisted locally.
  // Reconciled against running processes each poll — a config with a matching
  // process renders armed (observed mode), without one renders disarmed.
  const [strategyRows, setStrategyRows] = useState<StrategyRowConfig[]>(() => {
    try {
      const raw = JSON.parse(localStorage.getItem(STRATEGY_ROWS_KEY) ?? '[]');
      return Array.isArray(raw) ? raw : [];
    } catch {
      return [];
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(STRATEGY_ROWS_KEY, JSON.stringify(strategyRows));
    } catch {
      // Persistence is best-effort
    }
  }, [strategyRows]);
  const [lastAddedRowId, setLastAddedRowId] = useState<string | null>(null);

  // Pids the UI just stopped: suppressed from external rows immediately so a
  // killed process doesn't linger as a ghost row (with its old timeframe)
  // until the next poll notices it's gone.
  const [stoppedPids, setStoppedPids] = useState<number[]>([]);
  const markPidStopped = (pid: number) =>
    setStoppedPids((prev) => (prev.includes(pid) ? prev : [...prev, pid]));

  // Clear a config's pid once its process is confirmed gone (died/crashed or
  // stopped elsewhere) and the post-arm grace window has passed, so the row
  // settles back to disarmed instead of claiming a dead pid forever. Also
  // prune the stopped-pid suppressions the poll has confirmed dead.
  useEffect(() => {
    if (!status) return;
    const runningPids = new Set(status.watchers.map((w) => w.pid));
    setStoppedPids((prev) => prev.filter((pid) => runningPids.has(pid)));
    setStrategyRows((prev) => {
      let changed = false;
      const next = prev.map((row) => {
        const graceOver = Date.now() - (row.armedAt ?? 0) > 10_000;
        if (row.pid != null && !runningPids.has(row.pid) && graceOver) {
          changed = true;
          return { ...row, pid: null, armedAt: null };
        }
        return row;
      });
      return changed ? next : prev;
    });
  }, [status]);

  const daemonRunning = (status?.watchers.length ?? 0) > 0;
  const streamLabel =
    hubStream?.mode === 'client'
      ? 'attached to wickd stream hub'
      : hubStream?.mode === 'host'
        ? 'hosting stream hub (no CLI stream running)'
        : 'stream idle';

  return (
    <div className="min-h-screen bg-[var(--color-bg-page)] text-[var(--color-text-primary)] flex flex-col">
      <WindowHeader
        title="Live Monitor"
        currentWindow="watcher"
        settingsOpen={settingsOpen}
        onSettingsChange={setSettingsOpen}
        terminalContextProvider={() =>
          buildWatcherContext({
            runningStrategies: (status?.watchers ?? []).map((w) => ({
              strategyName: w.strategy ?? 'unknown',
              instruments: w.instruments,
              timeframe: '',
            })),
            pendingSignals: pending.map((p) => ({
              instrument: p.instrument,
              direction: p.side,
              strategyName: p.strategy,
              entryPrice: p.entry_price,
            })),
            availableInstruments: watchedInstruments,
          })
        }
        terminalHeader={getTerminalWelcome('watcher').header}
        terminalHeaderDescription={getTerminalWelcome('watcher').description}
        terminalWelcomeContent={getTerminalWelcome('watcher').content}
      />

      <main className="flex-1 max-w-7xl w-full mx-auto p-4 space-y-4">
        {/* Daemon status */}
        <section
          data-testid="daemon-status"
          className="flex flex-wrap items-center gap-3 px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
        >
          <span
            className={`inline-flex items-center gap-1.5 text-sm font-medium ${
              daemonRunning ? 'text-[var(--color-buy)]' : 'text-[var(--color-text-muted)]'
            }`}
          >
            <span
              className={`w-2 h-2 rounded-full ${
                daemonRunning ? 'bg-[var(--color-buy)]' : 'bg-[var(--color-text-muted)]'
              }`}
            />
            {daemonRunning
              ? `${status!.watchers.length} wickd watch daemon${status!.watchers.length === 1 ? '' : 's'} running`
              : 'no wickd watch daemon running'}
          </span>
          <span className="text-xs text-[var(--color-text-muted)]">{streamLabel}</span>
          {error && (
            <span data-testid="daemon-error" className="text-xs text-[var(--color-sell)]">
              {error}
            </span>
          )}
          {!daemonRunning && !loading && (
            <span className="w-full text-xs text-[var(--color-text-muted)] font-mono">
              start one: wickd watch &lt;strategy&gt; &lt;instruments&gt; --granularity H1
            </span>
          )}
        </section>

        {/* Watched instruments: live price + the watchers trading each one.
            One tile per instrument replaces the old separate Watchers/Prices
            sections, which listed the same instruments twice. Instrument-first
            flow: pin a price window, then attach strategies under it. */}
        <CollapsibleSection id="watcher_daemon_prices" title="Watching">
          <div data-testid="price-grid" className="flex flex-wrap items-start gap-3">
            {watchedInstruments.map((instrument) => (
              <WatchedInstrumentTile
                key={instrument}
                instrument={instrument}
                processes={watchersByInstrument.get(instrument) ?? []}
                rowConfigs={strategyRows.filter((r) => r.instrument === instrument)}
                strategies={startableStrategies}
                environment={environment}
                lastAddedRowId={lastAddedRowId}
                stoppedPids={stoppedPids}
                indicatorSeedFor={indicatorSeedFor}
                onPidStopped={markPidStopped}
                onConfigChange={(next) =>
                  setStrategyRows((prev) => prev.map((r) => (r.id === next.id ? next : r)))
                }
                onConfigDelete={(id) =>
                  setStrategyRows((prev) => prev.filter((r) => r.id !== id))
                }
                onAddRow={() => {
                  const id = crypto.randomUUID();
                  setStrategyRows((prev) => [
                    ...prev,
                    {
                      id,
                      instrument,
                      strategy: startableStrategies[0] ?? 'ma-crossover',
                      granularity: 'H1',
                      mode: 'disarmed',
                    },
                  ]);
                  setLastAddedRowId(id);
                }}
                onProcessChange={refresh}
                onRemove={
                  pinnedInstruments.includes(instrument)
                    ? () => setPinnedInstruments((prev) => prev.filter((i) => i !== instrument))
                    : undefined
                }
              />
            ))}
            <AddPairTile
              available={availablePairs}
              onAdd={(instrument) =>
                setPinnedInstruments((prev) =>
                  prev.includes(instrument) ? prev : [...prev, instrument]
                )
              }
              onAddAll={() =>
                setPinnedInstruments((prev) =>
                  Array.from(new Set([...prev, ...availablePairs]))
                )
              }
            />
          </div>
        </CollapsibleSection>

        {/* Pending approvals */}
        <CollapsibleSection
          id="watcher_daemon_pending"
          title="Pending approvals"
          badge={pending.length ? <span className="text-xs text-[var(--color-text-muted)]">({pending.length})</span> : undefined}
        >
          {pending.length === 0 ? (
            <p data-testid="pending-empty" className="text-sm text-[var(--color-text-muted)] px-1">
              No signals awaiting approval. Semi-auto proposals from{' '}
              <span className="font-mono">wickd watch --semi-auto</span> appear here; approve them with{' '}
              <span className="font-mono">wickd approve &lt;id&gt;</span>.
            </p>
          ) : (
            <div className="space-y-2">
              {pending.map((signal) => (
                <PendingSignalRow
                  key={signal.id}
                  signal={signal}
                  granularity={watchersByInstrument.get(signal.instrument)?.[0]?.granularity}
                />
              ))}
            </div>
          )}
        </CollapsibleSection>

        {/* Signal history lives on the Home window ("Signals") — this window
            stays reserved for actionable state. */}
      </main>
    </div>
  );
};
