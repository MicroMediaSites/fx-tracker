import { useEffect, useMemo, useState } from 'react';
import {
  LocalStrategyTrade,
  LocalTrade,
  listStrategyTrades,
  listTrades,
} from '../lib/localStore';

export interface StrategyTradeWithOutcome {
  id: string;
  strategy_id: string;
  strategy_config_id?: string;
  trade_id: string;
  instrument: string;
  timeframe: string;
  direction: string;
  entry_price: string;
  match_time: number;  // when pattern match was detected
  executed_at: number;
  rules_triggered?: string;
  // Trade outcome data
  trade?: {
    id: string;
    units: string;
    open_price: string;
    close_price?: string;
    open_time: number;
    close_time?: number;
    realized_pl?: string;
    state: string;
  };
}

/**
 * Hook to fetch strategy trades linked to actual OANDA trade outcomes.
 * This enables analyzing which trades were executed via a strategy
 * and their performance (P&L, win/loss, etc.)
 */
export const useStrategyTrades = (strategyId: string | undefined) => {
  // Attribution rows for this strategy, from the local store (AGT-650).
  const [strategyTrades, setStrategyTrades] = useState<LocalStrategyTrade[]>([]);
  useEffect(() => {
    let cancelled = false;
    if (!strategyId) {
      setStrategyTrades([]);
      return;
    }
    listStrategyTrades(strategyId)
      .then((rows) => {
        if (!cancelled) setStrategyTrades(rows);
      })
      .catch((err) => {
        console.error('[useStrategyTrades] Failed to load strategy trades from local store:', err);
        if (!cancelled) setStrategyTrades([]);
      });
    return () => {
      cancelled = true;
    };
  }, [strategyId]);

  // All trades to join with, from the local store (AGT-647).
  const [allTrades, setAllTrades] = useState<LocalTrade[]>([]);
  useEffect(() => {
    let cancelled = false;
    listTrades()
      .then((rows) => {
        if (!cancelled) setAllTrades(rows);
      })
      .catch((err) => {
        console.error('[useStrategyTrades] Failed to load trades from local store:', err);
        if (!cancelled) setAllTrades([]);
      });
    return () => {
      cancelled = true;
    };
  }, [strategyId]);

  // Join strategy_trade with trade data
  const tradesWithOutcomes = useMemo<StrategyTradeWithOutcome[]>(() => {
    if (!strategyTrades.length) return [];

    const tradeMap = new Map(allTrades.map((t) => [t.id, t]));

    return strategyTrades
      .map((st) => {
        const trade = tradeMap.get(st.trade_id);
        return {
          id: st.id,
          strategy_id: st.strategy_id,
          strategy_config_id: st.strategy_config_id ?? undefined,
          trade_id: st.trade_id,
          instrument: st.instrument,
          timeframe: st.timeframe,
          direction: st.direction,
          entry_price: st.entry_price,
          match_time: st.match_time,
          executed_at: st.executed_at,
          rules_triggered: st.rules_triggered ?? undefined,
          trade: trade
            ? {
                id: trade.id,
                units: trade.units,
                open_price: trade.open_price,
                close_price: trade.close_price ?? undefined,
                open_time: trade.open_time,
                close_time: trade.close_time ?? undefined,
                realized_pl: trade.realized_pl ?? undefined,
                state: trade.state,
              }
            : undefined,
        };
      })
      .sort((a, b) => b.executed_at - a.executed_at); // Most recent first
  }, [strategyTrades, allTrades]);

  // Calculate aggregate stats
  const stats = useMemo(() => {
    const closedTrades = tradesWithOutcomes.filter(
      (t) => t.trade?.state === 'CLOSED' && t.trade.realized_pl
    );

    if (!closedTrades.length) {
      return {
        totalTrades: tradesWithOutcomes.length,
        closedTrades: 0,
        openTrades: tradesWithOutcomes.filter((t) => t.trade?.state === 'OPEN').length,
        winCount: 0,
        lossCount: 0,
        winRate: 0,
        totalPL: 0,
        avgWin: 0,
        avgLoss: 0,
        profitFactor: 0,
      };
    }

    let winCount = 0;
    let lossCount = 0;
    let totalPL = 0;
    let totalWins = 0;
    let totalLosses = 0;

    closedTrades.forEach((t) => {
      const pl = parseFloat(t.trade!.realized_pl!);
      totalPL += pl;
      if (pl >= 0) {
        winCount++;
        totalWins += pl;
      } else {
        lossCount++;
        totalLosses += Math.abs(pl);
      }
    });

    return {
      totalTrades: tradesWithOutcomes.length,
      closedTrades: closedTrades.length,
      openTrades: tradesWithOutcomes.filter((t) => t.trade?.state === 'OPEN').length,
      winCount,
      lossCount,
      winRate: closedTrades.length > 0 ? (winCount / closedTrades.length) * 100 : 0,
      totalPL,
      avgWin: winCount > 0 ? totalWins / winCount : 0,
      avgLoss: lossCount > 0 ? totalLosses / lossCount : 0,
      profitFactor: totalLosses > 0 ? totalWins / totalLosses : totalWins > 0 ? Infinity : 0,
    };
  }, [tradesWithOutcomes]);

  return {
    trades: tradesWithOutcomes,
    stats,
    isLoading: false,
    hasData: tradesWithOutcomes.length > 0,
  };
}
