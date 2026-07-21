/**
 * FeedOverlay — the pull-down market-awareness terminal.
 *
 * Renders inside the WindowHeader / ModalTerminalDrawer drawer shell (which
 * owns drag/resize/⌘K), styled as a terminal: monospace text lines, no cards.
 * The `>` input row asks follow-up questions via `feed_ask` (which shells
 * `wickd feed ask` — the AI path stays in the CLI); the Q/A transcript is
 * session-local and never persisted.
 *
 * TWO STREAMS, MERGED AT READ TIME:
 *
 *  - Feed items — the AI market-awareness summary, produced out-of-process by
 *    the launchd `wickd feed tick` job (~/.wickd/feed.ndjson, every 15m).
 *  - Fired signals — the watch daemon's alert queue
 *    (~/.wickd/alert-queue.ndjson), polled far more often because their whole
 *    value is seeing them land as they happen.
 *
 * They are interleaved chronologically here and NOWHERE ELSE: signals get no
 * dedicated panel, because they are a log to watch scroll by (or to ask the
 * agent about), not a high-level state to check.
 *
 * The merge is deliberately read-side. Appending signals into feed.ndjson
 * would have been simpler to render but is unsafe and lossy: `feed::prune_at`
 * rewrites that file via tmp+rename with no lock against a second appender
 * (any signal written mid-prune is silently destroyed), a flattened FeedItem
 * cannot be promoted by `wickd queue promote`, and `feed tick` already
 * summarizes recent fires — so merged files would carry two representations
 * of one event. Keeping the stores separate avoids all three.
 */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { QueuedAlert } from '../../hooks/useWatchDaemon';

const POLL_INTERVAL_MS = 60_000;
/**
 * Signals poll far faster than the feed: the feed only changes every 15
 * minutes, but a fired signal is worth seeing promptly. 5s matches what the
 * retired home-window Signals panel already ran at.
 */
const SIGNAL_POLL_INTERVAL_MS = 5_000;
const MAX_QUESTION_LENGTH = 2000;

export type FeedSeverity = 'info' | 'watch' | 'urgent';

export interface FeedItem {
  id: string;
  ts: string;
  run_id: string;
  severity: FeedSeverity;
  pairs: string[];
  headline: string;
  body: string;
  kind?: string | null;
  sources: string[];
}

/**
 * Poll the feed store, keeping the last known items on read errors.
 *
 * `enabled` is the drawer's open state. The drawer shell mounts this component
 * on EVERY window and merely collapses it to zero height, so an ungated
 * interval would poll forever on all four windows for content nobody can see.
 * Polling resumes (with an immediate read) the moment the drawer opens.
 */
export const useFeed = (enabled: boolean, limit = 100): FeedItem[] => {
  const [items, setItems] = useState<FeedItem[]>([]);

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const entries = await invoke<FeedItem[]>('feed_list', { limit });
        if (!cancelled) setItems(entries);
      } catch {
        // Feed store unreadable — keep the last known items
      }
    };
    void poll();
    const interval = setInterval(() => void poll(), POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [enabled, limit]);

  return items;
};

/**
 * Poll the daemon's alert queue, keeping the last known fires on read errors.
 * Gated on the drawer being open for the same reason as [`useFeed`] — and more
 * urgently, since this one runs at 5s.
 */
export const useSignals = (enabled: boolean, limit = 100): QueuedAlert[] => {
  const [alerts, setAlerts] = useState<QueuedAlert[]>([]);

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    const poll = async () => {
      try {
        const entries = await invoke<QueuedAlert[]>('daemon_queue_list', { limit });
        if (!cancelled) setAlerts(Array.isArray(entries) ? entries : []);
      } catch {
        // Queue unreadable (no daemon yet) — keep the last known fires
      }
    };
    void poll();
    const interval = setInterval(() => void poll(), SIGNAL_POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [enabled, limit]);

  return alerts;
};

/**
 * One row of the merged terminal timeline, tagged by which store it came from
 * so the renderer stays a simple switch and the sort has one key.
 */
export type TimelineEntry =
  | { source: 'feed'; key: string; ts: string; item: FeedItem }
  | { source: 'signal'; key: string; ts: string; alert: QueuedAlert };

/**
 * Interleave the two stores oldest-first (terminal order). Ids are only unique
 * within a store, so keys are namespaced. Entries with an unparseable
 * timestamp sort to the end rather than to 1970 — a malformed line should not
 * silently bury the top of the log.
 */
export const mergeTimeline = (items: FeedItem[], alerts: QueuedAlert[]): TimelineEntry[] => {
  const entries: TimelineEntry[] = [
    ...items.map((item): TimelineEntry => ({
      source: 'feed',
      key: `feed:${item.id}`,
      ts: item.ts,
      item,
    })),
    ...alerts.map((alert): TimelineEntry => ({
      source: 'signal',
      key: `signal:${alert.id}`,
      ts: alert.ts,
      alert,
    })),
  ];
  const at = (ts: string) => {
    const ms = new Date(ts).getTime();
    return Number.isNaN(ms) ? Number.POSITIVE_INFINITY : ms;
  };
  return entries.sort((a, b) => at(a.ts) - at(b.ts));
};

/** "just now" / "12m ago" / "3h ago" / locale date for older items. */
export const formatAge = (ts: string, now: Date = new Date()): string => {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  const seconds = Math.floor((now.getTime() - date.getTime()) / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return date.toLocaleString();
};

const SEVERITY_TEXT: Record<FeedSeverity, string> = {
  urgent: 'text-red-400',
  watch: 'text-cyan-400',
  info: 'text-gray-500',
};

/** One feed item as terminal lines: a tagged headline row, dim body below. */
const FeedLine = ({ item }: { item: FeedItem }) => (
  <div data-testid="feed-item-row" className="px-1 py-0.5">
    <div className="leading-relaxed">
      <span className={`${SEVERITY_TEXT[item.severity] ?? SEVERITY_TEXT.info}`}>
        [{item.severity}]
      </span>
      {item.pairs.length > 0 && (
        <span className="text-gray-400"> {item.pairs.join(',')}</span>
      )}
      <span className="text-gray-100"> {item.headline}</span>
      <span className="text-gray-600" title={item.ts}>
        {'  '}· {formatAge(item.ts)}
      </span>
    </div>
    {item.body && (
      <div className="pl-4 text-gray-500 leading-relaxed whitespace-pre-wrap">{item.body}</div>
    )}
  </div>
);

/**
 * One fired signal as a single terminal line.
 *
 * Deliberately quieter than a FeedLine: the `[signal]` tag is dim and there is
 * no body row, because these scroll past continuously (an M1 watcher fires
 * dozens a day) and must not drown out the 15-minute feed summaries. Only the
 * side — the one thing worth catching mid-scroll — carries color.
 */
const SignalLine = ({ alert }: { alert: QueuedAlert }) => {
  const p = alert.payload;
  return (
    <div data-testid="feed-signal-row" className="px-1 py-0.5 leading-relaxed">
      <span className="text-gray-600">[signal]</span>
      <span className="text-gray-400"> {p.instrument}</span>
      {p.kind === 'strategy-signal' ? (
        <>
          <span className={p.signal === 'buy' ? ' text-emerald-400' : ' text-red-400'}>
            {' '}
            {p.signal}
          </span>
          <span className="text-gray-500"> {p.proposal.strategy}</span>
          {p.granularity && <span className="text-gray-600"> {p.granularity}</span>}
          {p.account && <span className="text-gray-600"> · {p.account}</span>}
        </>
      ) : (
        <span className="text-gray-500">
          {' '}
          {p.direction} {p.level} @ {p.price}
        </span>
      )}
      <span className="text-gray-600" title={alert.ts}>
        {'  '}· {formatAge(alert.ts)}
      </span>
    </div>
  );
};

/** A session-local Q or A line in the transcript. */
interface AskLine {
  role: 'user' | 'assistant' | 'error';
  text: string;
}

interface FeedOverlayProps {
  /** Current drawer height in px; 0 = closed (render nothing). */
  height: number;
  /** Window identity, kept for parity with the drawer hosts. */
  currentWindow: string;
}

export const FeedOverlay = ({ height }: FeedOverlayProps) => {
  const isOpen = height > 0;
  const items = useFeed(isOpen);
  const signals = useSignals(isOpen);
  const [askLines, setAskLines] = useState<AskLine[]>([]);
  const [input, setInput] = useState('');
  const [asking, setAsking] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  // Whether the viewport is pinned to the bottom — updated on scroll so new
  // content only auto-follows when the user hasn't scrolled up to read history.
  const atBottomRef = useRef(true);

  // Terminal order: oldest at top, newest at the bottom — both stores in one
  // chronological stream.
  const timeline = useMemo(() => mergeTimeline(items, signals), [items, signals]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    atBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  };

  // Stick to the bottom as content grows — but only if the user was already
  // there (don't yank them mid-read when a 15-minute tick or an answer lands).
  useLayoutEffect(() => {
    if (atBottomRef.current) {
      scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
    }
  }, [timeline, askLines, asking]);

  // On open, land the user at the bottom (newest item + the input), not the top.
  useEffect(() => {
    if (!isOpen) return;
    atBottomRef.current = true;
    requestAnimationFrame(() => {
      const el = scrollRef.current;
      if (el) el.scrollTo({ top: el.scrollHeight });
    });
  }, [isOpen]);

  const send = async () => {
    const question = input.trim();
    if (!question || asking) return;
    setInput('');
    // Snapshot the transcript BEFORE appending the new question so the CLI
    // sees only prior turns as history.
    const history = askLines
      .filter((l) => l.role === 'user' || l.role === 'assistant')
      .map((l) => ({ role: l.role, text: l.text }));
    setAskLines((l) => [...l, { role: 'user', text: question }]);
    setAsking(true);
    try {
      const answer = await invoke<string>('feed_ask', { question, history });
      setAskLines((l) => [...l, { role: 'assistant', text: answer }]);
    } catch (err) {
      setAskLines((l) => [...l, { role: 'error', text: String(err) }]);
    } finally {
      setAsking(false);
      inputRef.current?.focus();
    }
  };

  if (!isOpen) return null;

  return (
    <div
      className="h-full flex flex-col font-mono text-xs sm:text-sm"
      data-testid="feed-overlay"
    >
      <div ref={scrollRef} onScroll={onScroll} className="flex-1 overflow-y-auto px-2 py-1">
        {timeline.length === 0 ? (
          <p data-testid="feed-empty" className="px-1 py-1 text-gray-600">
            nothing yet — signals appear here as watchers fire them, and the market-awareness
            feed refreshes every 15 minutes while markets are open
          </p>
        ) : (
          timeline.map((entry) =>
            entry.source === 'feed' ? (
              <FeedLine key={entry.key} item={entry.item} />
            ) : (
              <SignalLine key={entry.key} alert={entry.alert} />
            )
          )
        )}

        {askLines.map((line, i) => (
          <div key={i} data-testid="feed-ask-line" className="px-1 py-0.5 leading-relaxed whitespace-pre-wrap">
            {line.role === 'user' ? (
              <span className="text-cyan-400">{'> '}{line.text}</span>
            ) : line.role === 'assistant' ? (
              <span className="text-emerald-400">{'← '}{line.text}</span>
            ) : (
              <span className="text-red-400">{'! '}{line.text}</span>
            )}
          </div>
        ))}
        {asking && (
          <div className="px-1 py-0.5 text-gray-500 animate-pulse" data-testid="feed-ask-pending">
            thinking...
          </div>
        )}
      </div>

      {/* Follow-up input — asks about the feed via `wickd feed ask` */}
      <div className="flex items-center gap-1 px-2 py-1 border-t border-gray-800/80">
        <span className="text-cyan-400 select-none">{'>'}</span>
        <input
          ref={inputRef}
          data-testid="feed-ask-input"
          type="text"
          value={input}
          maxLength={MAX_QUESTION_LENGTH}
          disabled={asking}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void send();
          }}
          placeholder="ask about the feed…"
          className="flex-1 bg-transparent outline-none text-gray-100 placeholder-gray-600 disabled:opacity-50"
        />
      </div>
    </div>
  );
};
