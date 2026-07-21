/**
 * mergeTimeline — the read-side join of the two drawer streams (AI feed items
 * and the watch daemon's fired signals).
 *
 * The merge is the whole reason signals no longer need a panel, so the
 * ordering guarantees are worth pinning: one chronological stream, stable keys
 * across stores, and a malformed timestamp that cannot bury the log.
 */
import { describe, expect, it } from 'vitest';
import { mergeTimeline, type FeedItem } from './FeedOverlay';
import type { PendingSignal, QueuedAlert } from '../../hooks/useWatchDaemon';

/**
 * mergeTimeline only reads `id`/`ts`, so the embedded proposal is filled in
 * just enough to satisfy the type — these tests are about ordering, not about
 * the proposal's contents.
 */
const stubProposal = { strategy: 'rahagod' } as unknown as PendingSignal;

const feedItem = (id: string, ts: string): FeedItem => ({
  id,
  ts,
  run_id: 'run-1',
  severity: 'info',
  pairs: ['EUR_USD'],
  headline: `headline ${id}`,
  body: '',
  sources: [],
});

const signal = (id: string, ts: string): QueuedAlert => ({
  id,
  ts,
  payload: {
    kind: 'strategy-signal',
    instrument: 'EUR_USD',
    signal: 'buy',
    granularity: 'M1',
    account: 'tf-m1',
    proposal: stubProposal,
  },
});

describe('mergeTimeline', () => {
  it('interleaves both stores oldest-first', () => {
    const merged = mergeTimeline(
      [feedItem('f1', '2026-07-20T10:00:00Z'), feedItem('f2', '2026-07-20T12:00:00Z')],
      [signal('s1', '2026-07-20T11:00:00Z')]
    );

    expect(merged.map((e) => e.key)).toEqual(['feed:f1', 'signal:s1', 'feed:f2']);
  });

  it('namespaces keys so colliding ids across stores stay distinct', () => {
    // Ids are only unique within a store; a shared id must not collapse into
    // one React key.
    const merged = mergeTimeline(
      [feedItem('same', '2026-07-20T10:00:00Z')],
      [signal('same', '2026-07-20T11:00:00Z')]
    );

    expect(new Set(merged.map((e) => e.key)).size).toBe(2);
  });

  it('sorts an unparseable timestamp to the end, not to 1970', () => {
    // Date.parse('') is NaN; treating that as 0 would pin a malformed row to
    // the top of the log forever, above every real entry.
    const merged = mergeTimeline(
      [feedItem('good', '2026-07-20T10:00:00Z'), feedItem('bad', 'not-a-date')],
      []
    );

    expect(merged.map((e) => e.key)).toEqual(['feed:good', 'feed:bad']);
  });

  it('handles either store being empty', () => {
    expect(mergeTimeline([], [])).toEqual([]);
    expect(mergeTimeline([feedItem('f1', '2026-07-20T10:00:00Z')], [])).toHaveLength(1);
    expect(mergeTimeline([], [signal('s1', '2026-07-20T10:00:00Z')])).toHaveLength(1);
  });

  it('tags each entry with its source so the renderer can switch', () => {
    const merged = mergeTimeline(
      [feedItem('f1', '2026-07-20T10:00:00Z')],
      [signal('s1', '2026-07-20T11:00:00Z')]
    );

    expect(merged.map((e) => e.source)).toEqual(['feed', 'signal']);
  });
});
