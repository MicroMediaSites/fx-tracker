/**
 * Signals — the wickd daemon's durable alert feed (`alert-queue.ndjson`),
 * rendered on the HOME window. Deliberately informational: the Live Monitor
 * stays reserved for actionable state (running watchers, pending approvals);
 * the fired-signal history lives here.
 *
 * Self-contained polling of `daemon_queue_list` only — no process-table
 * scans from the home window.
 */
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { QueuedAlert } from '../../hooks/useWatchDaemon';
import { CollapsibleSection } from '../ui/CollapsibleSection';
import { OpenChartButton } from './OpenChartButton';

const POLL_INTERVAL_MS = 5000;

const formatTs = (ts: string): string => {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  return date.toLocaleString();
};

export const QueueAlertRow = ({ alert, granularity }: { alert: QueuedAlert; granularity?: string | null }) => {
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
          <OpenChartButton instrument={payload.instrument} granularity={granularity} />
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
          <OpenChartButton instrument={payload.instrument} granularity={granularity} />
          <span className="text-sm text-[var(--color-text-secondary)]">
            {payload.direction} {payload.level} @ {payload.price}
          </span>
        </>
      )}
    </div>
  );
};

export const SignalsSection = () => {
  const [queue, setQueue] = useState<QueuedAlert[]>([]);

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const entries = await invoke<QueuedAlert[]>('daemon_queue_list', { limit: 100 });
        if (!cancelled) setQueue(entries);
      } catch {
        // Daemon store unreadable — keep the last known feed
      }
    };
    void poll();
    const interval = setInterval(() => void poll(), POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  return (
    <CollapsibleSection
      id="home_signals"
      title="Signals"
      badge={queue.length ? <span className="text-xs text-[var(--color-text-muted)]">({queue.length})</span> : undefined}
    >
      {queue.length === 0 ? (
        <p data-testid="queue-empty" className="text-sm text-[var(--color-text-muted)] px-1">
          No signals yet. Fired strategy signals and price-level alerts from the wickd daemon land here.
        </p>
      ) : (
        <div className="space-y-2">
          {queue.map((alert) => (
            <QueueAlertRow key={alert.id} alert={alert} />
          ))}
        </div>
      )}
    </CollapsibleSection>
  );
};
