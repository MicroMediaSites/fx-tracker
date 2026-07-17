/**
 * FeedOverlay — the pull-down market-awareness feed.
 *
 * Renders inside the WindowHeader / ModalTerminalDrawer drawer shell (which
 * owns drag/resize/⌘K). Content is produced out-of-process: the launchd
 * `wickd feed tick` job appends AI analysis items to `~/.wickd/feed.ndjson`
 * every 15 minutes, and this component polls the read-only `feed_list`
 * command — no network, no AI calls from the app.
 */
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

// The producer appends at most every 15 minutes, so a 1-minute poll is
// already generous — this is not a live tick feed like SignalFeed's 5s.
const POLL_INTERVAL_MS = 60_000;

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

/** Poll the feed store, keeping the last known items on read errors. */
export const useFeed = (limit = 100): FeedItem[] => {
  const [items, setItems] = useState<FeedItem[]>([]);

  useEffect(() => {
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
  }, [limit]);

  return items;
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

const SEVERITY_CHIP: Record<FeedSeverity, string> = {
  urgent: 'bg-[var(--color-sell)]/15 text-[var(--color-sell)]',
  watch: 'bg-[var(--color-info)]/15 text-[var(--color-info)]',
  info: 'bg-gray-500/15 text-[var(--color-text-muted)]',
};

const FeedRow = ({ item }: { item: FeedItem }) => (
  <div
    data-testid="feed-item-row"
    className="flex flex-wrap items-start gap-x-3 gap-y-1 px-3 py-2 rounded border border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
  >
    <span
      className={`px-1.5 py-0.5 text-xs font-semibold rounded uppercase ${SEVERITY_CHIP[item.severity] ?? SEVERITY_CHIP.info}`}
    >
      {item.severity}
    </span>
    <div className="flex-1 min-w-0">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm font-semibold text-gray-100">{item.headline}</span>
        {item.pairs.map((pair) => (
          <span
            key={pair}
            className="px-1.5 py-0.5 text-xs rounded border border-[var(--color-border)] text-[var(--color-text-secondary)]"
          >
            {pair}
          </span>
        ))}
      </div>
      {item.body && (
        <p className="mt-0.5 text-xs text-[var(--color-text-secondary)] leading-relaxed">{item.body}</p>
      )}
    </div>
    <span className="text-xs text-[var(--color-text-muted)] whitespace-nowrap" title={item.ts}>
      {formatAge(item.ts)}
    </span>
  </div>
);

interface FeedOverlayProps {
  /** Current drawer height in px; 0 = closed (render nothing). */
  height: number;
  /** Window identity, kept for parity with the drawer hosts. */
  currentWindow: string;
}

export const FeedOverlay = ({ height }: FeedOverlayProps) => {
  const items = useFeed();

  if (height === 0) return null;

  return (
    <div className="h-full flex flex-col font-mono text-sm" data-testid="feed-overlay">
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-2">
        {items.length === 0 ? (
          <p data-testid="feed-empty" className="text-xs text-[var(--color-text-muted)] px-1 py-2">
            No feed items yet — the market-awareness feed refreshes every 15 minutes while
            markets are open.
          </p>
        ) : (
          items.map((item) => <FeedRow key={item.id} item={item} />)
        )}
      </div>
      {/* TODO(feed-followup): an "ask a follow-up" input row lands here once a
          local chat path exists again — the drawer shell already reserves the
          interaction pattern (⌘K, resize). */}
    </div>
  );
};
