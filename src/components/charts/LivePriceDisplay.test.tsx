import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '../../test/utils';
import { LivePriceDisplay } from './LivePriceDisplay';

// Mock the price flash hook
vi.mock('../../hooks/usePriceFlash', () => ({
  usePriceFlash: () => 'none',
  getPriceColorClass: () => 'text-[var(--color-text-primary)]',
}));

describe('LivePriceDisplay', () => {
  const defaultProps = {
    isHistoricalView: false,
    hoveredCandle: null,
    streaming: true,
    currentPrice: null,
    instrument: 'EUR_USD',
  };

  describe('display mode priority', () => {
    it('shows loading skeleton when no price data available', () => {
      render(<LivePriceDisplay {...defaultProps} />);

      // Should show loading skeletons (pulse animations)
      const skeletons = document.querySelectorAll('.animate-pulse');
      expect(skeletons.length).toBeGreaterThan(0);
    });

    it('shows live streaming price when currentPrice is available', () => {
      render(
        <LivePriceDisplay
          {...defaultProps}
          currentPrice={{
            instrument: 'EUR_USD',
            bid: '1.08500',
            ask: '1.08520',
            spread: '2.0',
            time: new Date().toISOString(),
            tradeable: true,
          }}
        />
      );

      expect(screen.getByText('1.08500')).toBeInTheDocument();
      expect(screen.getByText('1.08520')).toBeInTheDocument();
      expect(screen.getByText('(2.0)')).toBeInTheDocument();
    });

    it('shows OHLC when hovering over a candle (overrides streaming)', () => {
      render(
        <LivePriceDisplay
          {...defaultProps}
          currentPrice={{
            instrument: 'EUR_USD',
            bid: '1.08500',
            ask: '1.08520',
            spread: '2.0',
            time: new Date().toISOString(),
            tradeable: true,
          }}
          hoveredCandle={{
            open: '1.08000',
            high: '1.09000',
            low: '1.07500',
            close: '1.08800',
          }}
        />
      );

      // Should show OHLC labels
      expect(screen.getByText('O')).toBeInTheDocument();
      expect(screen.getByText('H')).toBeInTheDocument();
      expect(screen.getByText('L')).toBeInTheDocument();
      expect(screen.getByText('C')).toBeInTheDocument();

      // Should show OHLC values
      expect(screen.getByText('1.08000')).toBeInTheDocument();
      expect(screen.getByText('1.09000')).toBeInTheDocument();
      expect(screen.getByText('1.07500')).toBeInTheDocument();
      expect(screen.getByText('1.08800')).toBeInTheDocument();

      // Should NOT show bid/ask
      expect(screen.queryByText('1.08500')).not.toBeInTheDocument();
      expect(screen.queryByText('1.08520')).not.toBeInTheDocument();
    });

    it('shows "Historical View" text when in historical mode', () => {
      render(<LivePriceDisplay {...defaultProps} isHistoricalView={true} />);

      expect(screen.getByText('Historical View')).toBeInTheDocument();
    });

    it('OHLC display takes priority over historical view mode', () => {
      render(
        <LivePriceDisplay
          {...defaultProps}
          isHistoricalView={true}
          hoveredCandle={{
            open: '1.08000',
            high: '1.09000',
            low: '1.07500',
            close: '1.08800',
          }}
        />
      );

      // Should show OHLC, not "Historical View"
      expect(screen.getByText('O')).toBeInTheDocument();
      expect(screen.queryByText('Historical View')).not.toBeInTheDocument();
    });
  });

  describe('streaming indicator', () => {
    it('shows green dot when streaming is active', () => {
      render(
        <LivePriceDisplay
          {...defaultProps}
          streaming={true}
          currentPrice={{
            instrument: 'EUR_USD',
            bid: '1.08500',
            ask: '1.08520',
            spread: '2.0',
            time: new Date().toISOString(),
            tradeable: true,
          }}
        />
      );

      // The dot should have the buy (green) color class
      const dot = document.querySelector('.text-\\[var\\(--color-buy\\)\\]');
      expect(dot).toBeInTheDocument();
    });

    it('shows muted dot when streaming is inactive', () => {
      render(
        <LivePriceDisplay
          {...defaultProps}
          streaming={false}
          currentPrice={{
            instrument: 'EUR_USD',
            bid: '1.08500',
            ask: '1.08520',
            spread: '2.0',
            time: new Date().toISOString(),
            tradeable: true,
          }}
        />
      );

      // The dot should have the muted color class
      const dot = document.querySelector('.text-\\[var\\(--color-text-muted\\)\\]');
      expect(dot).toBeInTheDocument();
    });
  });

  describe('price precision by instrument', () => {
    it('uses 5 decimal places for EUR_USD (standard FX)', () => {
      render(
        <LivePriceDisplay
          {...defaultProps}
          instrument="EUR_USD"
          currentPrice={{
            instrument: 'EUR_USD',
            bid: '1.085',
            ask: '1.0852',
            spread: '2.0',
            time: new Date().toISOString(),
            tradeable: true,
          }}
        />
      );

      // Should pad to 5 decimals
      expect(screen.getByText('1.08500')).toBeInTheDocument();
      expect(screen.getByText('1.08520')).toBeInTheDocument();
    });

    it('uses 3 decimal places for JPY pairs', () => {
      render(
        <LivePriceDisplay
          {...defaultProps}
          instrument="USD_JPY"
          currentPrice={{
            instrument: 'USD_JPY',
            bid: '150.5',
            ask: '150.52',
            spread: '2.0',
            time: new Date().toISOString(),
            tradeable: true,
          }}
        />
      );

      // Should pad to 3 decimals for JPY
      expect(screen.getByText('150.500')).toBeInTheDocument();
      expect(screen.getByText('150.520')).toBeInTheDocument();
    });
  });

  describe('transition from OHLC back to streaming (BUG-037 scenario)', () => {
    it('returns to streaming display when hoveredCandle becomes null', () => {
      const { rerender } = render(
        <LivePriceDisplay
          {...defaultProps}
          currentPrice={{
            instrument: 'EUR_USD',
            bid: '1.08500',
            ask: '1.08520',
            spread: '2.0',
            time: new Date().toISOString(),
            tradeable: true,
          }}
          hoveredCandle={{
            open: '1.08000',
            high: '1.09000',
            low: '1.07500',
            close: '1.08800',
          }}
        />
      );

      // Currently showing OHLC
      expect(screen.getByText('O')).toBeInTheDocument();
      expect(screen.queryByText('(2.0)')).not.toBeInTheDocument();

      // Mouse leaves candle - hoveredCandle becomes null
      rerender(
        <LivePriceDisplay
          {...defaultProps}
          currentPrice={{
            instrument: 'EUR_USD',
            bid: '1.08500',
            ask: '1.08520',
            spread: '2.0',
            time: new Date().toISOString(),
            tradeable: true,
          }}
          hoveredCandle={null}
        />
      );

      // Should now show streaming prices
      expect(screen.queryByText('O')).not.toBeInTheDocument();
      expect(screen.getByText('1.08500')).toBeInTheDocument();
      expect(screen.getByText('(2.0)')).toBeInTheDocument();
    });

    it('returns to loading state when price resets (instrument switch scenario)', () => {
      const { rerender } = render(
        <LivePriceDisplay
          {...defaultProps}
          currentPrice={{
            instrument: 'EUR_USD',
            bid: '1.08500',
            ask: '1.08520',
            spread: '2.0',
            time: new Date().toISOString(),
            tradeable: true,
          }}
        />
      );

      // Currently showing streaming price
      expect(screen.getByText('1.08500')).toBeInTheDocument();

      // Instrument changes - currentPrice reset to null while new subscription establishes
      rerender(
        <LivePriceDisplay
          {...defaultProps}
          instrument="GBP_USD"
          currentPrice={null}
          hoveredCandle={null}
        />
      );

      // Should show loading skeletons
      const skeletons = document.querySelectorAll('.animate-pulse');
      expect(skeletons.length).toBeGreaterThan(0);
    });
  });
});
