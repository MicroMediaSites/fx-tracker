/**
 * Note-count badges from the local store (AGT-646).
 *
 * Replaces the Zero `myNotes` query that trade lists used to know which
 * trades have journal notes. The local store has no change subscription, so
 * consumers call `refresh` after a mutation point (e.g. when NotesModal
 * reports a change via `onNotesChanged`).
 */
import { useState, useEffect, useCallback } from 'react';
import { listNotes } from '../lib/localStore';

export function useTradeNoteCounts(): {
  tradeNoteCounts: Map<string, number>;
  refresh: () => Promise<void>;
} {
  const [tradeNoteCounts, setTradeNoteCounts] = useState<Map<string, number>>(new Map());

  const refresh = useCallback(async () => {
    try {
      const notes = await listNotes();
      const map = new Map<string, number>();
      for (const n of notes) {
        if (n.trade_id) {
          map.set(n.trade_id, (map.get(n.trade_id) || 0) + 1);
        }
      }
      setTradeNoteCounts(map);
    } catch (err) {
      console.error('[useTradeNoteCounts] Failed to load notes:', err);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { tradeNoteCounts, refresh };
}
