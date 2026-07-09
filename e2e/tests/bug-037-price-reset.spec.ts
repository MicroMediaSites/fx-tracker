/**
 * BUG-037: Chart price display doesn't reset to live stream when switching
 * instrument or timeframe.
 *
 * Root cause: loadCandles() set hoveredCandle to the last candle's OHLC after
 * fetching data. This caused the LivePriceDisplay to show OHLC values instead
 * of defaulting to the live streaming bid/ask display.
 *
 * Fix: Set hoveredCandle to null after loading candles so the display defaults
 * to streaming mode (or loading skeleton until the first price tick arrives).
 *
 * The same loadCandles() function runs on initial load AND on instrument/timeframe
 * changes, so validating the initial load path covers the switch path too.
 */
import { test, expect } from '../helpers/app-fixture';

test.describe('BUG-037: Price display resets to streaming after instrument switch', () => {
  test('price display shows loading skeleton instead of OHLC after candles load', async ({ appPage }) => {
    // Mock get_candles with realistic data (strings for OHLC, ISO date for time,
    // matching the CandleData struct from src-tauri/src/commands/data.rs)
    await appPage.mockTauriCommand('get_candles', [
      { time: '2025-01-31T15:00:00Z', open: '1.08500', high: '1.09000', low: '1.08000', close: '1.08800', volume: 100, complete: true },
      { time: '2025-01-31T19:00:00Z', open: '1.08800', high: '1.09200', low: '1.08600', close: '1.09100', volume: 120, complete: true },
    ]);
    await appPage.mockTauriCommand('subscribe_to_prices', null);
    await appPage.mockTauriCommand('unsubscribe_from_prices', null);

    await appPage.goto('chart');

    // Wait for the chart header to render
    await expect(appPage.page.getByText('Charting')).toBeVisible();

    // The OHLC display (data-testid="ohlc-display") should NOT be visible after
    // candle load. Before the fix, loadCandles set hoveredCandle to the last
    // candle's OHLC, making the OHLC display appear immediately.
    const ohlcDisplay = appPage.page.getByTestId('ohlc-display');
    await expect(ohlcDisplay).not.toBeVisible();

    // Since no streaming price events arrive in E2E (listen mock is a no-op),
    // the display should show the loading skeleton (animate-pulse elements),
    // confirming it's in "waiting for stream" mode rather than stuck on OHLC.
    const skeleton = appPage.page.locator('.animate-pulse').first();
    await expect(skeleton).toBeVisible();
  });
});
