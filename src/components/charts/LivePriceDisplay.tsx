import type { PriceUpdate, OHLCData } from './chartTypes';
import { getInstrumentPrecision } from './chartConstants';
import { usePriceFlash, getPriceColorClass } from '../../hooks/usePriceFlash';

interface LivePriceDisplayProps {
  isHistoricalView: boolean;
  hoveredCandle: OHLCData | null;
  streaming: boolean;
  currentPrice: PriceUpdate | null;
  instrument: string;
}

export const LivePriceDisplay = ({
  isHistoricalView,
  hoveredCandle,
  streaming,
  currentPrice,
  instrument,
}: LivePriceDisplayProps) => {
  const precision = getInstrumentPrecision(instrument);

  // Price flash hooks - isolated to this component
  const bidDirection = usePriceFlash(currentPrice?.bid);
  const askDirection = usePriceFlash(currentPrice?.ask);

  // Show OHLC when hovering over a candle (any view)
  if (hoveredCandle) {
    return (
      <div className="flex items-center gap-2" data-testid="ohlc-display">
        <span className={`text-xs ${!isHistoricalView && streaming ? 'text-[var(--color-buy)]' : 'text-[var(--color-info-text)]'}`}>&#x25CF;</span>
        <span className="text-[var(--color-text-muted)] text-xs">O</span>
        <span className="font-mono text-[var(--color-text-primary)]">{parseFloat(String(hoveredCandle.open)).toFixed(precision)}</span>
        <span className="text-[var(--color-text-muted)] text-xs">H</span>
        <span className="font-mono text-[var(--color-buy)]">{parseFloat(String(hoveredCandle.high)).toFixed(precision)}</span>
        <span className="text-[var(--color-text-muted)] text-xs">L</span>
        <span className="font-mono text-[var(--color-sell)]">{parseFloat(String(hoveredCandle.low)).toFixed(precision)}</span>
        <span className="text-[var(--color-text-muted)] text-xs">C</span>
        <span className="font-mono text-[var(--color-text-primary)]">{parseFloat(String(hoveredCandle.close)).toFixed(precision)}</span>
      </div>
    );
  }

  if (isHistoricalView) {
    return <span className="text-[var(--color-info-text)] text-sm">Historical View</span>;
  }

  if (currentPrice) {
    return (
      <>
        <span className={`text-xs mr-2 ${streaming ? 'text-[var(--color-buy)]' : 'text-[var(--color-text-muted)]'}`}>&#x25CF;</span>
        <span className={`font-mono cursor-default w-[85px] text-right transition-colors duration-300 ${getPriceColorClass(bidDirection)}`} title="Bid">
          {parseFloat(currentPrice.bid).toFixed(precision)}
        </span>
        <span className="text-[var(--color-text-muted)] mx-2">/</span>
        <span className={`font-mono cursor-default w-[85px] transition-colors duration-300 ${getPriceColorClass(askDirection)}`} title="Ask">
          {parseFloat(currentPrice.ask).toFixed(precision)}
        </span>
        <span className="text-[var(--color-text-muted)] text-xs cursor-default ml-3" title="Spread">
          ({currentPrice.spread})
        </span>
      </>
    );
  }

  // Loading state
  return (
    <>
      <span className="text-xs mr-2 text-[var(--color-text-muted)]">&#x25CF;</span>
      <span className="h-4 w-[85px] bg-[var(--color-bg-card)] rounded animate-pulse"></span>
      <span className="text-[var(--color-text-muted)] mx-2">/</span>
      <span className="h-4 w-[85px] bg-[var(--color-bg-card)] rounded animate-pulse"></span>
      <span className="h-3 w-[50px] bg-[var(--color-bg-card)] rounded animate-pulse ml-3"></span>
    </>
  );
};
