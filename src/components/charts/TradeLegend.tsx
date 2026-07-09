import type { TradeData } from './TradeOverlayPlugin';

interface TradeLegendProps {
  trades: TradeData[];
}

export const TradeLegend = ({ trades }: TradeLegendProps) => {
  if (trades.length === 0) return null;

  return (
    <div className="flex-shrink-0 px-4 py-2 text-sm text-[var(--color-text-muted)] flex items-center gap-6">
      <span className="flex items-center gap-1">
        <span className="inline-block w-3 h-3 bg-[var(--color-buy)] rounded-full"></span>
        Long Entry
      </span>
      <span className="flex items-center gap-1">
        <span className="inline-block w-3 h-3 bg-[var(--color-sell)] rounded-full"></span>
        Short Entry
      </span>
      <span className="flex items-center gap-1">
        <span className="inline-block w-3 h-3 bg-[var(--color-buy)]/30 border border-[var(--color-buy)]"></span>
        Profit
      </span>
      <span className="flex items-center gap-1">
        <span className="inline-block w-3 h-3 bg-[var(--color-sell)]/30 border border-[var(--color-sell)]"></span>
        Loss
      </span>
      <span className="ml-auto">
        {trades.length} trades shown
      </span>
    </div>
  );
};
