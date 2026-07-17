/**
 * FeedOverlay — the pull-down market-awareness terminal.
 *
 * Renders inside the WindowHeader / ModalTerminalDrawer drawer shell (which
 * owns drag/resize/⌘K), styled as a terminal: monospace text lines, no cards.
 * Feed items are produced out-of-process (the launchd `wickd feed tick` job
 * appends to `~/.wickd/feed.ndjson` every 15 minutes) and read via the
 * offline `feed_list` command. The `>` input row asks follow-up questions via
 * `feed_ask` (which shells `wickd feed ask` — the AI path stays in the CLI);
 * the Q/A transcript is session-local and never persisted.
 */
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

const POLL_INTERVAL_MS = 60_000;
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
  const items = useFeed();
  const [askLines, setAskLines] = useState<AskLine[]>([]);
  const [input, setInput] = useState('');
  const [asking, setAsking] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  // Whether the viewport is pinned to the bottom — updated on scroll so new
  // content only auto-follows when the user hasn't scrolled up to read history.
  const atBottomRef = useRef(true);
  const isOpen = height > 0;

  // Terminal order: oldest at top, newest at the bottom.
  const orderedItems = useMemo(() => [...items].reverse(), [items]);

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
  }, [orderedItems, askLines, asking]);

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
        {orderedItems.length === 0 ? (
          <p data-testid="feed-empty" className="px-1 py-1 text-gray-600">
            no feed items yet — the market-awareness feed refreshes every 15 minutes while
            markets are open
          </p>
        ) : (
          orderedItems.map((item) => <FeedLine key={item.id} item={item} />)
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
