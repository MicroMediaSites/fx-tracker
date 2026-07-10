/**
 * Inline-editable strategy row under a price tile in the Live Monitor.
 *
 * A row always keeps its clean final look — `[state icon] name … timeframe` —
 * and every part edits in place:
 *
 * - The ICON is a state toggle. Clicking cycles disarmed (grey eye,
 *   struck through; no process) → monitor (blue eye) → semi-auto (orange
 *   bell; proposals require CLI approval) → disarmed. Arming starts a
 *   detached `wickd watch`; changing mode restarts it; disarming stops it.
 *   Autonomous execution (--auto) is deliberately absent — the trust ladder
 *   caps UI rows at semi-auto (red icon reserved for a future CLI-armed
 *   display state).
 * - The STRATEGY NAME swaps for an in-place menu on click.
 * - The TIMEFRAME swaps for an in-place menu on click.
 *
 * Rows for processes the UI doesn't own (launchd/pinned binaries like
 * wickd-h021) render read-only: state reflects the observed flags and the
 * backend refuses to stop them (see stop_watcher).
 */
import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { invoke } from '@tauri-apps/api/core';

export type RowMode = 'disarmed' | 'monitor' | 'semi-auto';

export interface StrategyRowConfig {
  id: string;
  instrument: string;
  strategy: string;
  granularity: string;
  mode: RowMode;
  /** pid of the process this config last started (null when disarmed).
   * Lets transitions stop the right process even before the daemon poll
   * binds it, and lets a freshly armed row render its state immediately. */
  pid?: number | null;
  /** When the config last armed (ms epoch) — grace window for the poll. */
  armedAt?: number | null;
}

export const WATCH_GRANULARITIES = ['M1', 'M5', 'M15', 'M30', 'H1', 'H2', 'H4', 'H6', 'H8', 'H12', 'D', 'W'];

const NEXT_MODE: Record<RowMode, RowMode> = {
  disarmed: 'monitor',
  monitor: 'semi-auto',
  'semi-auto': 'disarmed',
};

const MODE_TITLES: Record<RowMode, string> = {
  disarmed: 'Disarmed — no watcher running. Click to start monitoring.',
  monitor: 'Monitoring — signals stream to the feed. Click for semi-auto.',
  'semi-auto': 'Semi-auto — entry signals become proposals you approve via the CLI. Click to disarm.',
};

/** Small in-place dropdown menu used by the name/timeframe editors.
 * Rendered through a portal with fixed positioning so it can't be clipped by
 * overflow-hidden ancestors (collapsible sections, tile bounds); flips
 * upward when there's no room below the anchor. */
const InlineMenu = ({
  anchor,
  options,
  onPick,
  onClose,
  testId,
}: {
  anchor: HTMLElement | null;
  options: string[];
  onPick: (value: string) => void;
  onClose: () => void;
  testId: string;
}) => {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onOutside = (e: MouseEvent) => {
      const target = e.target as Node;
      if (ref.current && !ref.current.contains(target) && !anchor?.contains(target)) onClose();
    };
    const onEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('mousedown', onOutside);
    document.addEventListener('keydown', onEscape);
    return () => {
      document.removeEventListener('mousedown', onOutside);
      document.removeEventListener('keydown', onEscape);
    };
  }, [onClose, anchor]);

  if (!anchor) return null;
  const rect = anchor.getBoundingClientRect();
  const MENU_MAX_HEIGHT = 192;
  const openUp = window.innerHeight - rect.bottom < MENU_MAX_HEIGHT + 8;
  const style: React.CSSProperties = {
    position: 'fixed',
    left: rect.left,
    minWidth: rect.width,
    zIndex: 300,
    ...(openUp
      ? { bottom: window.innerHeight - rect.top + 2 }
      : { top: rect.bottom + 2 }),
  };

  return createPortal(
    <div
      ref={ref}
      data-testid={testId}
      style={style}
      className="max-h-48 overflow-y-auto rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)] shadow-lg py-0.5 whitespace-nowrap"
    >
      {options.map((option) => (
        <button
          key={option}
          onClick={() => onPick(option)}
          className="block w-full px-2.5 py-1 text-left text-xs text-[var(--color-text-secondary)] hover:bg-[var(--color-bg-card)] hover:text-[var(--color-text-primary)] transition-colors"
        >
          {option}
        </button>
      ))}
    </div>,
    document.body
  );
};

// Trust-ladder state icons (Feather paths — unambiguous at 14px):
// disarmed = grey eye-off, monitor = blue eye, semi-auto = orange alarm bell,
// full-auto (display-only, CLI-armed) = red bolt.
const StateIcon = ({ mode }: { mode: RowMode | 'auto' }) => {
  if (mode === 'disarmed') {
    return (
      <svg className="w-3.5 h-3.5 text-[var(--color-text-muted)] opacity-60" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24" />
        <line x1="1" y1="1" x2="23" y2="23" />
      </svg>
    );
  }
  if (mode === 'monitor') {
    return (
      <svg className="w-3.5 h-3.5 text-[var(--color-info)]" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
        <circle cx="12" cy="12" r="3" />
      </svg>
    );
  }
  if (mode === 'semi-auto') {
    return (
      <svg className="w-3.5 h-3.5 text-orange-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
        <path d="M13.73 21a2 2 0 0 1-3.46 0" />
      </svg>
    );
  }
  // full-auto: red bolt
  return (
    <svg className="w-3.5 h-3.5 text-[var(--color-sell)]" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
      <path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z" />
    </svg>
  );
};

interface StrategyRowProps {
  config: StrategyRowConfig;
  /** pid of the bound running process, when armed and reconciled. */
  pid: number | null;
  /** Startable strategy names for the inline name menu. */
  strategies: string[];
  environment: string;
  /** Persist a config change (already applied to the process). */
  onConfigChange: (next: StrategyRowConfig) => void;
  onDelete: () => void;
  /** Poke the daemon status poll after a start/stop. */
  onProcessChange: () => void;
  /** Report a pid this row stopped, so its process row hides immediately. */
  onPidStopped: (pid: number) => void;
  /** Open the instrument's chart at this row's timeframe. */
  onOpenChart: () => void;
  /** Open a fresh row with its name menu already showing. */
  startEditingName?: boolean;
}

export const StrategyRow = ({
  config,
  pid,
  strategies,
  environment,
  onConfigChange,
  onDelete,
  onProcessChange,
  onPidStopped,
  onOpenChart,
  startEditingName = false,
}: StrategyRowProps) => {
  const [editing, setEditing] = useState<'name' | 'timeframe' | null>(
    startEditingName ? 'name' : null
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Menu anchors as state (not refs) so the portal menu renders once the
  // trigger element mounts — needed for rows that open pre-editing.
  const [nameEl, setNameEl] = useState<HTMLButtonElement | null>(null);
  const [timeframeEl, setTimeframeEl] = useState<HTMLButtonElement | null>(null);

  const stopCurrent = async () => {
    // Prefer the poll-bound pid; fall back to the pid this config started
    // (covers the window before the daemon poll observes the new process).
    const target = pid ?? config.pid ?? null;
    if (target === null) return;
    try {
      await invoke('stop_watcher', { pid: target });
    } catch (err) {
      // Process already gone — that's the outcome we wanted
      if (!String(err).includes('No running wickd watch process')) throw err;
    }
    // Either way it's gone (or going): hide it from external rows immediately
    // instead of letting it ghost around until the next poll.
    onPidStopped(target);
  };

  const startWith = async (next: StrategyRowConfig): Promise<number> => {
    return await invoke<number>('start_watcher', {
      strategy: next.strategy,
      instruments: [next.instrument],
      granularity: next.granularity,
      semiAuto: next.mode === 'semi-auto',
      units: null,
      env: environment,
    });
  };

  /** Apply a config/mode transition: stop the old process if armed, start a
   * new one if the next state is armed.
   *
   * OPTIMISTIC: the new state paints immediately and the process work runs
   * behind it — spawn/kill can stall for seconds on Gatekeeper-burdened
   * machines and the click must not wait for that. On failure the previous
   * state is restored and the error shown. */
  const applyTransition = async (next: StrategyRowConfig) => {
    setBusy(true);
    setError(null);
    const previous = config;
    onConfigChange(
      next.mode !== 'disarmed'
        ? { ...next, armedAt: Date.now() }
        : { ...next, pid: null, armedAt: null }
    );
    try {
      if (previous.mode !== 'disarmed') await stopCurrent();
      if (next.mode !== 'disarmed') {
        const newPid = await startWith(next);
        onConfigChange({ ...next, pid: newPid, armedAt: Date.now() });
      }
      onProcessChange();
    } catch (err) {
      setError(String(err));
      onConfigChange(previous); // revert the optimistic paint
    } finally {
      setBusy(false);
    }
  };

  const cycleMode = () => {
    if (busy) return;
    void applyTransition({ ...config, mode: NEXT_MODE[config.mode] });
  };

  const pickStrategy = (strategy: string) => {
    setEditing(null);
    if (strategy !== config.strategy) void applyTransition({ ...config, strategy });
  };

  const pickTimeframe = (granularity: string) => {
    setEditing(null);
    if (granularity !== config.granularity) void applyTransition({ ...config, granularity });
  };

  /** Delete works in any state: stop the process first if armed. */
  const handleDelete = async () => {
    setBusy(true);
    setError(null);
    try {
      if (config.mode !== 'disarmed') await stopCurrent();
      onDelete();
      onProcessChange();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  };

  return (
    <div
      data-testid="strategy-row"
      className="w-full px-2 py-1 rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)] text-xs"
    >
      <div className="flex items-center gap-1.5">
        <button
          data-testid="strategy-row-state"
          onClick={cycleMode}
          disabled={busy}
          className={`shrink-0 hover:opacity-80 transition-opacity ${busy ? 'animate-pulse' : ''}`}
          title={busy ? 'Working…' : MODE_TITLES[config.mode]}
          aria-label={`State: ${config.mode}. Click to change.`}
        >
          <StateIcon mode={config.mode} />
        </button>

        <span className="min-w-0 flex-1">
          <button
            ref={setNameEl}
            data-testid="strategy-row-name"
            onClick={() => setEditing(editing === 'name' ? null : 'name')}
            className="max-w-full truncate text-left text-[var(--color-text-secondary)] hover:text-[var(--color-text-primary)] transition-colors"
            title="Change strategy"
          >
            {config.strategy}
          </button>
          {editing === 'name' && (
            <InlineMenu
              anchor={nameEl}
              testId="strategy-row-name-menu"
              options={strategies}
              onPick={pickStrategy}
              onClose={() => setEditing(null)}
            />
          )}
        </span>

        <span className="shrink-0">
          <button
            ref={setTimeframeEl}
            data-testid="strategy-row-timeframe"
            onClick={() => setEditing(editing === 'timeframe' ? null : 'timeframe')}
            className="font-mono text-[var(--color-text-muted)] hover:text-[var(--color-text-secondary)] transition-colors"
            title="Change timeframe"
          >
            {config.granularity}
          </button>
          {editing === 'timeframe' && (
            <InlineMenu
              anchor={timeframeEl}
              testId="strategy-row-timeframe-menu"
              options={WATCH_GRANULARITIES}
              onPick={pickTimeframe}
              onClose={() => setEditing(null)}
            />
          )}
        </span>

        <button
          data-testid="strategy-row-chart"
          onClick={onOpenChart}
          className="shrink-0 p-0.5 text-[var(--color-text-muted)] hover:text-[var(--color-info)] transition-colors"
          title={`Open ${config.instrument.replace('_', '/')} chart (${config.granularity})`}
          aria-label="Open chart at this timeframe"
        >
          <svg className="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" d="M4 20h16" />
            <path strokeLinecap="round" d="M7 16v-5M12 16V6M17 16v-8" />
          </svg>
        </button>

        <button
          data-testid="strategy-row-delete"
          onClick={() => void handleDelete()}
          disabled={busy}
          className="shrink-0 p-0.5 text-[var(--color-text-muted)] hover:text-[var(--color-sell)] transition-colors"
          title={config.mode === 'disarmed' ? 'Remove this strategy row' : 'Stop the watcher and remove this row'}
          aria-label="Remove strategy row"
        >
          <svg className="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
      </div>
      {error && (
        <p data-testid="strategy-row-error" className="mt-0.5 text-[var(--color-sell)] truncate" title={error}>
          {error}
        </p>
      )}
    </div>
  );
};

/** Row for a running watcher not bound to a UI config. Plain `wickd` CLI
 * processes get a stop (×) affordance — the backend permits stopping them;
 * pinned/supervised binaries (launchd evals like wickd-h021) render locked
 * and the backend refuses to touch them. */
export const ExternalWatcherRow = ({
  strategy,
  granularity,
  mode,
  pid,
  onStop,
  onOpenChart,
}: {
  strategy: string;
  granularity: string | null;
  mode: RowMode | 'auto';
  pid: number;
  /** Present when the process is stoppable from the UI (plain wickd binary). */
  onStop?: () => void;
  /** Open the instrument's chart at this watcher's timeframe. */
  onOpenChart: () => void;
}) => {
  const [stopping, setStopping] = useState(false);
  return (
    <div
      data-testid="external-watcher-row"
      className={`w-full px-2 py-1 rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)] text-xs flex items-center gap-1.5 ${stopping ? 'opacity-60' : ''}`}
      title={
        onStop
          ? `Started outside this window (pid ${pid}).`
          : `Managed outside the app (pid ${pid}) — launchd or a pinned binary. Control it from the CLI.`
      }
    >
      <span className="shrink-0">
        <StateIcon mode={mode} />
      </span>
      <span className="min-w-0 flex-1 truncate text-[var(--color-text-secondary)]">{strategy}</span>
      <span className="shrink-0 font-mono text-[var(--color-text-muted)]">{granularity ?? '—'}</span>
      <button
        data-testid="external-watcher-chart"
        onClick={onOpenChart}
        className="shrink-0 p-0.5 text-[var(--color-text-muted)] hover:text-[var(--color-info)] transition-colors"
        title={`Open chart (${granularity ?? 'default'})`}
        aria-label="Open chart at this timeframe"
      >
        <svg className="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
          <path strokeLinecap="round" d="M4 20h16" />
          <path strokeLinecap="round" d="M7 16v-5M12 16V6M17 16v-8" />
        </svg>
      </button>
      {onStop ? (
        <button
          data-testid="external-watcher-stop"
          onClick={() => {
            setStopping(true);
            onStop();
          }}
          disabled={stopping}
          className="shrink-0 p-0.5 text-[var(--color-text-muted)] hover:text-[var(--color-sell)] transition-colors"
          title={`Stop this watcher (pid ${pid})`}
          aria-label="Stop watcher"
        >
          <svg className="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
      ) : (
        <svg className="w-3 h-3 shrink-0 text-[var(--color-text-muted)]" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
          <rect x="5" y="11" width="14" height="9" rx="1.5" />
          <path d="M8 11V8a4 4 0 118 0v3" />
        </svg>
      )}
    </div>
  );
};
