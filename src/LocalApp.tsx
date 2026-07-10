/**
 * LocalApp — the local-first boot window (AGT-642 walking skeleton).
 *
 * Rendered for `?window=local` (the default cold-boot window). It mounts NO
 * auth provider: everything on screen is served from the local SQLite store
 * at `~/.wickd/app.db` via `src/lib/localStore.ts`, so the window works fully
 * offline with no sign-in. (The legacy cloud login window was deleted by
 * AGT-652 with the rest of the SaaS shell.)
 *
 * As the boot window it also owns the app-level chores the deleted account
 * window used to run: opening the configured startup windows, the silent
 * update check + the menu-triggered update modal, and syncing the desktop
 * notification setting to the backend.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import { check, Update } from '@tauri-apps/plugin-updater';
import {
  LocalStrategy,
  deleteStrategy,
  listStrategies,
  localStorePath,
} from './lib/localStore';
import { LocalBacktestsSection } from './components/local/LocalBacktestsSection';
import { UpdateModal } from './components/ui/UpdateModal';
import { SignalsSection } from './components/watcher/SignalFeed';
import { useSettingsStore } from './stores/settingsStore';
import { openBacktestWindow, openChartWindow, openWatcherWindow } from './utils/windows';

const formatDate = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });

export const LocalApp = () => {
  const [strategies, setStrategies] = useState<LocalStrategy[] | null>(null);
  const [storePath, setStorePath] = useState<string>('');
  const [error, setError] = useState<string | null>(null);
  // Strategy selected for the backtests view (AGT-645).
  const [selectedStrategyId, setSelectedStrategyId] = useState<string | null>(null);
  // Provenance filter (AGT-648): all rows, native wickd only, or imported
  // candlesight only.
  const [sourceFilter, setSourceFilter] = useState<'all' | 'wickd' | 'candlesight'>('all');

  const refresh = useCallback(async () => {
    try {
      setStrategies(await listStrategies());
      setError(null);
    } catch (e) {
      setError(String(e));
      setStrategies(null);
    }
  }, []);

  useEffect(() => {
    refresh();
    localStorePath()
      .then(setStorePath)
      .catch(() => setStorePath('~/.wickd/app.db'));
  }, [refresh]);

  const handleDelete = async (id: string) => {
    try {
      await deleteStrategy(id);
      if (selectedStrategyId === id) setSelectedStrategyId(null);
      await refresh();
    } catch (err) {
      setError(String(err));
    }
  };

  // ---- Boot-window chores (moved from the deleted account window) ----
  const { startupWindows, _hasHydrated, desktopNotifications } = useSettingsStore();
  const [showUpdateModal, setShowUpdateModal] = useState(false);
  const [updateTriggeredManually, setUpdateTriggeredManually] = useState(false);
  const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);

  // Sync the desktop-notification setting to the backend.
  useEffect(() => {
    if (!_hasHydrated) return;
    invoke('set_desktop_notifications_enabled', { enabled: desktopNotifications }).catch(console.error);
  }, [_hasHydrated, desktopNotifications]);

  // Open configured startup windows once (only from the boot window, label 'main').
  const startupWindowsOpenedRef = useRef(false);
  useEffect(() => {
    if (!_hasHydrated) return;
    if (startupWindowsOpenedRef.current) return;
    startupWindowsOpenedRef.current = true;

    const openStartupWindows = async () => {
      if (getCurrentWindow().label !== 'main') return;
      if (startupWindows.includes('charting')) await openChartWindow();
      if (startupWindows.includes('backtesting')) await openBacktestWindow();
      if (startupWindows.includes('watcher')) await openWatcherWindow();
    };
    void openStartupWindows();
  }, [_hasHydrated, startupWindows]);

  // Silent update check on startup (production builds only).
  const updateCheckDoneRef = useRef(false);
  useEffect(() => {
    if (!_hasHydrated) return;
    if (updateCheckDoneRef.current) return;
    if (import.meta.env.DEV) return;
    if (getCurrentWindow().label !== 'main') return;

    const timer = setTimeout(async () => {
      updateCheckDoneRef.current = true;
      try {
        const update = await check();
        if (update) {
          setPendingUpdate(update);
          setUpdateTriggeredManually(false);
          setShowUpdateModal(true);
        }
      } catch (err) {
        console.error('Startup update check failed:', err);
      }
    }, 2000);
    return () => clearTimeout(timer);
  }, [_hasHydrated]);

  // Menu: View > Check for Updates...
  useEffect(() => {
    const unlisten = listen('check-for-updates', () => {
      setUpdateTriggeredManually(true);
      setShowUpdateModal(true);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  return (
    <div
      className="min-h-screen bg-[var(--color-bg-page)] text-[var(--color-text-primary)] flex flex-col"
      data-testid="local-app"
    >
      <header className="px-8 pt-8 pb-4 border-b border-white/10">
        <h1 className="text-2xl font-bold tracking-tight">wickd</h1>
        <p className="text-sm text-[var(--color-text-secondary)] mt-1" data-testid="local-store-path">
          Local-first mode — no sign-in required. Store: {storePath || '…'}
        </p>
      </header>

      <main className="flex-1 px-8 py-6 max-w-3xl w-full mx-auto">
        <section aria-labelledby="strategies-heading">
          <div className="flex items-center justify-between mb-3 gap-3">
            <h2 id="strategies-heading" className="text-lg font-semibold">
              Strategies
            </h2>
            {strategies !== null && strategies.some((s) => s.source === 'candlesight') && (
              <select
                value={sourceFilter}
                onChange={(e) =>
                  setSourceFilter(e.target.value as 'all' | 'wickd' | 'candlesight')
                }
                aria-label="Filter strategies by source"
                data-testid="local-source-filter"
                className="px-2 py-1 rounded bg-white/5 border border-white/10 text-xs text-[var(--color-text-secondary)] focus:outline-none focus:border-[var(--color-info)]"
              >
                <option value="all">All sources</option>
                <option value="wickd">wickd only</option>
                <option value="candlesight">candlesight import</option>
              </select>
            )}
          </div>

          {error && (
            <div
              className="mb-4 p-3 rounded border border-[var(--color-sell)]/40 bg-[var(--color-sell)]/10 text-sm"
              data-testid="local-store-error"
            >
              Local store unavailable: {error}
            </div>
          )}

          {strategies === null && !error && (
            <div className="text-[var(--color-text-muted)] text-sm">Loading…</div>
          )}

          {strategies !== null && strategies.length === 0 && (
            <div
              className="text-[var(--color-text-muted)] text-sm mb-4"
              data-testid="local-strategies-empty"
            >
              No strategies in the local store. Strategies are authored with the wickd
              CLI (<code>wickd strategy add</code>) into the unified .rhai store.
            </div>
          )}

          {strategies !== null && strategies.length > 0 && (
            <ul className="space-y-2 mb-6" data-testid="local-strategies-list">
              {strategies
                .filter((s) =>
                  sourceFilter === 'all'
                    ? true
                    : sourceFilter === 'candlesight'
                      ? s.source === 'candlesight'
                      : s.source !== 'candlesight',
                )
                .map((s) => (
                <li
                  key={s.id}
                  className={`p-3 rounded border flex items-start justify-between gap-3 cursor-pointer transition-colors ${
                    selectedStrategyId === s.id
                      ? 'border-[var(--color-info)] bg-[var(--color-info)]/10'
                      : 'border-white/10 bg-white/5 hover:border-white/25'
                  }`}
                  data-testid="local-strategy-row"
                  aria-selected={selectedStrategyId === s.id}
                  onClick={() =>
                    setSelectedStrategyId((prev) => (prev === s.id ? null : s.id))
                  }
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="font-medium truncate">{s.name}</span>
                      {s.source === 'candlesight' && (
                        <span
                          className="text-xs px-1.5 py-0.5 rounded bg-[var(--color-info)]/20 text-[var(--color-info)]"
                          data-testid="candlesight-badge"
                          title="Imported from the CandleSight archive"
                        >
                          candlesight
                        </span>
                      )}
                      {s.is_promoted && (
                        <span className="text-xs px-1.5 py-0.5 rounded bg-[var(--color-buy)]/20 text-[var(--color-buy)]">
                          live
                        </span>
                      )}
                      {s.is_archived && (
                        <span className="text-xs px-1.5 py-0.5 rounded bg-white/10 text-[var(--color-text-muted)]">
                          archived
                        </span>
                      )}
                      {!s.is_active && (
                        <span className="text-xs px-1.5 py-0.5 rounded bg-white/10 text-[var(--color-text-muted)]">
                          inactive
                        </span>
                      )}
                    </div>
                    {s.description && (
                      <p className="text-sm text-[var(--color-text-secondary)] mt-0.5 truncate">
                        {s.description}
                      </p>
                    )}
                    <p className="text-xs text-[var(--color-text-muted)] mt-1">
                      Updated {formatDate(s.updated_at)}
                    </p>
                  </div>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      handleDelete(s.id);
                    }}
                    className="text-xs text-[var(--color-text-muted)] hover:text-[var(--color-sell)] transition-colors shrink-0"
                    aria-label={`Delete ${s.name}`}
                  >
                    Delete
                  </button>
                </li>
              ))}
            </ul>
          )}

        </section>

        {(() => {
          const selected = strategies?.find((s) => s.id === selectedStrategyId);
          return selected ? (
            <LocalBacktestsSection strategyId={selected.id} strategyName={selected.name} />
          ) : null;
        })()}

        {/* Fired-signal history from the wickd daemon. Informational — the
            Live Monitor stays reserved for actionable state. */}
        <div className="mt-6">
          <SignalsSection />
        </div>
      </main>

      <footer className="px-8 py-4 border-t border-white/10 flex items-center justify-between">
        <span className="text-xs text-[var(--color-text-muted)]">
          Everything above is served from the local store — works fully offline.
        </span>
      </footer>

      <UpdateModal
        isOpen={showUpdateModal}
        onClose={() => {
          setShowUpdateModal(false);
          setPendingUpdate(null);
        }}
        triggeredManually={updateTriggeredManually}
        autoCheck={!pendingUpdate}
        preResolvedUpdate={pendingUpdate}
      />
    </div>
  );
};
