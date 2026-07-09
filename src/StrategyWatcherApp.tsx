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
import { useMemo, useState } from 'react';
import { useWatchDaemon, type PendingSignal, type QueuedAlert } from './hooks/useWatchDaemon';
import { usePriceStream } from './hooks/usePriceStream';
import { useEnvironmentSync } from './hooks/useEnvironmentSync';
import { WindowHeader } from './components/ui/WindowHeader';
import { CollapsibleSection } from './components/ui/CollapsibleSection';
import { PriceWindow } from './components/ui/PriceDisplay';
import { buildWatcherContext } from './lib/chatContextBuilder';
import { getTerminalWelcome } from './lib/terminalWelcome';

const formatTs = (ts: string): string => {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  return date.toLocaleString();
};

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

const PendingSignalRow = ({ signal }: { signal: PendingSignal }) => (
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

const QueueAlertRow = ({ alert }: { alert: QueuedAlert }) => {
  const payload = alert.payload;
  return (
    <div
      data-testid="queue-alert-row"
      className="flex flex-wrap items-center gap-3 px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
    >
      <span className="text-xs text-[var(--color-text-muted)] whitespace-nowrap">{formatTs(alert.ts)}</span>
      {payload.kind === 'strategy-signal' ? (
        <>
          <span
            className={`px-1.5 py-0.5 text-xs font-semibold rounded uppercase ${
              payload.signal === 'buy'
                ? 'bg-[var(--color-buy)]/15 text-[var(--color-buy)]'
                : 'bg-[var(--color-sell)]/15 text-[var(--color-sell)]'
            }`}
          >
            {payload.signal}
          </span>
          <span className="font-semibold">{payload.instrument}</span>
          <span className="text-sm text-[var(--color-text-secondary)]">{payload.proposal.strategy}</span>
          <span className="flex-1 min-w-0 text-xs text-[var(--color-text-muted)] truncate" title={payload.proposal.reason}>
            {payload.proposal.reason}
          </span>
        </>
      ) : (
        <>
          <span className="px-1.5 py-0.5 text-xs font-semibold rounded uppercase bg-[var(--color-info)]/15 text-[var(--color-info)]">
            level
          </span>
          <span className="font-semibold">{payload.instrument}</span>
          <span className="text-sm text-[var(--color-text-secondary)]">
            {payload.direction} {payload.level} @ {payload.price}
          </span>
        </>
      )}
    </div>
  );
};

export const StrategyWatcherApp = () => {
  // Keep the environment badge in sync across windows (BUG-024).
  useEnvironmentSync();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const { status, queue, pending, hubStream, error, loading } = useWatchDaemon();

  // Price monitoring: everything the daemon watches, plus whatever the hub is
  // observed streaming. Subscribing to hub-covered instruments never opens a
  // second upstream connection.
  const watchedInstruments = useMemo(() => {
    const set = new Set<string>();
    for (const watcher of status?.watchers ?? []) {
      for (const instrument of watcher.instruments) set.add(instrument);
    }
    for (const instrument of hubStream?.observed ?? []) set.add(instrument);
    return Array.from(set).sort();
  }, [status, hubStream]);

  usePriceStream(watchedInstruments);

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

        {/* Running watchers */}
        {daemonRunning && (
          <CollapsibleSection id="watcher_daemon_processes" title="Watchers">
            <div className="space-y-2">
              {status!.watchers.map((watcher) => (
                <div
                  key={watcher.pid}
                  data-testid="watch-process-row"
                  className="flex flex-wrap items-center gap-3 px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
                >
                  <span className="font-semibold">{watcher.strategy ?? 'unknown strategy'}</span>
                  <span className="text-sm text-[var(--color-text-secondary)]">
                    {watcher.instruments.join(', ') || '—'}
                  </span>
                  <span className="ml-auto text-xs text-[var(--color-text-muted)] font-mono" title={watcher.command}>
                    pid {watcher.pid}
                  </span>
                </div>
              ))}
            </div>
          </CollapsibleSection>
        )}

        {/* Live prices */}
        {watchedInstruments.length > 0 && (
          <CollapsibleSection id="watcher_daemon_prices" title="Prices">
            <div data-testid="price-grid" className="flex flex-wrap gap-3">
              {watchedInstruments.map((instrument) => (
                <div key={instrument} className="w-[200px]">
                  <div className="px-1 mb-1 text-sm font-semibold">{instrument.replace('_', '/')}</div>
                  <PriceWindow instrument={instrument} />
                </div>
              ))}
            </div>
          </CollapsibleSection>
        )}

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
                <PendingSignalRow key={signal.id} signal={signal} />
              ))}
            </div>
          )}
        </CollapsibleSection>

        {/* Signal feed */}
        <CollapsibleSection
          id="watcher_daemon_feed"
          title="Signal feed"
          badge={status?.queue_len ? <span className="text-xs text-[var(--color-text-muted)]">({status.queue_len})</span> : undefined}
        >
          {queue.length === 0 ? (
            <p data-testid="queue-empty" className="text-sm text-[var(--color-text-muted)] px-1">
              No alerts yet. Fired strategy signals and price-level alerts from the wickd daemon land here.
            </p>
          ) : (
            <div className="space-y-2">
              {queue.map((alert) => (
                <QueueAlertRow key={alert.id} alert={alert} />
              ))}
            </div>
          )}
        </CollapsibleSection>
      </main>
    </div>
  );
};
