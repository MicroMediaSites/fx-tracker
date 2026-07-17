/**
 * useWatchDaemon — the app as a client of the wickd watch daemon (AGT-652).
 *
 * The desktop app hosts no watcher engine. `wickd watch` (launchd-supervised)
 * is the one watcher runtime on the machine; this hook polls its
 * client-visible outputs through thin Tauri commands:
 *
 * - `daemon_status`        — running watch processes + hub socket presence
 * - `daemon_queue_list`    — the durable signal feed (~/.wickd/alert-queue.ndjson)
 * - `daemon_pending_list`  — the semi-auto pending/approve queue (~/.wickd/pending.json)
 * - `hub_stream_status`    — where the app's prices are flowing from
 *
 * Read-only by design: approving a pending signal stays a deliberate
 * `wickd approve <id>` CLI invocation (the trust ladder lives in the CLI).
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export interface WatchProcess {
  pid: number;
  command: string;
  strategy: string | null;
  instruments: string[];
}

export interface DaemonStatus {
  watchers: WatchProcess[];
  hub_socket_present: boolean;
  pending_count: number;
  queue_len: number;
}

export interface PendingSignal {
  id: string;
  ts: string;
  instrument: string;
  side: string;
  units: number;
  suggested_units?: number;
  strategy: string;
  reason: string;
  sl?: string;
  tp?: string;
  entry_price?: string;
  status: string;
}

export type QueuedPayload =
  | {
      kind: 'price-level';
      instrument: string;
      level: string;
      direction: 'cross-up' | 'cross-down' | 'either';
      price: string;
    }
  | {
      kind: 'strategy-signal';
      instrument: string;
      signal: 'buy' | 'sell';
      proposal: PendingSignal;
      /** Watcher's --account (issue #8). Absent on rows queued before the field existed. */
      account?: string;
      /** Watcher's candle granularity, e.g. "M5" (issue #8). Absent on legacy rows. */
      granularity?: string;
    };

export interface QueuedAlert {
  id: string;
  ts: string;
  payload: QueuedPayload;
}

export interface HubStreamSnapshot {
  mode: 'idle' | 'client' | 'host';
  observed: string[];
  direct: string[];
  last_line_ms: number | null;
}

const POLL_INTERVAL_MS = 3000;
const QUEUE_LIMIT = 50;

export interface WatchDaemonState {
  status: DaemonStatus | null;
  queue: QueuedAlert[];
  pending: PendingSignal[];
  hubStream: HubStreamSnapshot | null;
  /** Non-null when the last poll failed (daemon stores unreadable). */
  error: string | null;
  /** True until the first poll completes. */
  loading: boolean;
  /** Force an immediate re-poll (e.g. after running an approve in the CLI). */
  refresh: () => void;
}

export const useWatchDaemon = (): WatchDaemonState => {
  const [status, setStatus] = useState<DaemonStatus | null>(null);
  const [queue, setQueue] = useState<QueuedAlert[]>([]);
  const [pending, setPending] = useState<PendingSignal[]>([]);
  const [hubStream, setHubStream] = useState<HubStreamSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const inFlightRef = useRef(false);

  const poll = useCallback(async () => {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    try {
      const [nextStatus, nextQueue, nextPending, nextHub] = await Promise.all([
        invoke<DaemonStatus>('daemon_status'),
        invoke<QueuedAlert[]>('daemon_queue_list', { limit: QUEUE_LIMIT }),
        invoke<PendingSignal[]>('daemon_pending_list'),
        invoke<HubStreamSnapshot>('hub_stream_status'),
      ]);
      setStatus(nextStatus);
      setQueue(nextQueue);
      setPending(nextPending);
      setHubStream(nextHub);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      inFlightRef.current = false;
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void poll();
    const interval = setInterval(() => void poll(), POLL_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [poll]);

  return { status, queue, pending, hubStream, error, loading, refresh: poll };
};
