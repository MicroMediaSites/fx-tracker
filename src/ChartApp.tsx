import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { WindowHeader } from './components/ui/WindowHeader';
import { useEnvironmentSync } from './hooks/useEnvironmentSync';
import { buildChartingContext } from './lib/chatContextBuilder';
import { getTerminalWelcome } from './lib/terminalWelcome';
import {
  createChart,
  CandlestickSeries,
  type IChartApi,
  type ISeriesApi,
} from 'lightweight-charts';
import { TradeOverlayPlugin, type TradeData } from './components/charts/TradeOverlayPlugin';
import { IchimokuCloudPlugin } from './components/charts/IchimokuCloudPlugin';
import { SRZoneOverlay } from './components/charts/SRZoneOverlay';
import { SRZoneEditor } from './components/charts/SRZoneEditor';
import {
  getStrategy,
  listClosedTradesByInstrument,
  type LocalStrategy,
  type LocalTrade,
} from './lib/localStore';
import { addDebugLog } from './components/ui/DebugOverlay';
import { GRANULARITIES } from './constants';
import {
  ChartHeader,
  TradeLegend,
  renderIndicators,
  clearIndicators,
  getGranularitySeconds,
  getInstrumentPrecision,
  INITIAL_VISIBLE_CANDLES,
  FUTURE_CANDLE_SLOTS,
  formatIndicatorLabel,
  strategyIndicatorsToChartConfigs,
  IndicatorConfigModal,
} from './components/charts';
import type { IndicatorSeries, IndicatorConfig, OHLCData, ChartIndicatorConfig } from './components/charts/chartTypes';
import { useChartIndicatorStore } from './stores/chartIndicatorStore';
import type { IndicatorType } from './types/strategy';
import { FirstRunTour } from './components/onboarding/FirstRunTour';
import { chartTourSteps } from './lib/tourSteps';
import { useChartParams } from './hooks/useChartParams';
import { usePriceStreaming } from './hooks/usePriceStreaming';
import { useSRZones } from './hooks/useSRZones';
import {
  type CandleData,
  type TimeMapState,
  createTimeMapState,
  convertCandles as convertCandlesUtil,
  toBusinessTime as toBusinessTimeUtil,
  syncIndicatorTimestamps,
} from './components/charts/chartTimeUtils';

export const ChartApp = () => {
  // BUG-024: Sync dataSource across windows when user switches accounts
  useEnvironmentSync();

  // Refs
  const chartContainerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const candleSeriesRef = useRef<ISeriesApi<any> | null>(null);
  const tradeOverlayRef = useRef<TradeOverlayPlugin | null>(null);
  const ichimokuCloudRef = useRef<IchimokuCloudPlugin | null>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const indicatorSeriesRef = useRef<Map<string, ISeriesApi<any>>>(new Map());
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const entryPriceLineRef = useRef<any>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const slPriceLineRef = useRef<any>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const tpPriceLineRef = useRef<any>(null);
  const candlesDataRef = useRef<CandleData[]>([]);
  const timeMapStateRef = useRef<TimeMapState>(createTimeMapState());
  const chartIndicatorsRef = useRef<ChartIndicatorConfig[]>([]);
  const indicatorDataRef = useRef<IndicatorSeries[]>([]);

  // Initial params from URL
  const initialParams = useChartParams();

  // Chart state
  const [instrument, setInstrument] = useState(initialParams.instrument);
  const [granularity, setGranularity] = useState(initialParams.granularity);
  const [candleCount] = useState(initialParams.count);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [trades, setTrades] = useState<TradeData[] | null>(initialParams.trades);
  // Track the instrument that trades were loaded for, so we can clear them on instrument change
  const tradesInstrumentRef = useRef<string>(initialParams.instrument);
  // Counter that increments when candles finish loading, triggering trade overlay effect
  const [candlesLoadedCount, setCandlesLoadedCount] = useState(0);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [strategyId] = useState<string | null>(initialParams.strategyId);
  const [signalDirection] = useState<'long' | 'short' | null>(initialParams.signalDirection);
  const [stopLoss] = useState<string | null>(initialParams.stopLoss);
  const [takeProfit] = useState<string | null>(initialParams.takeProfit);
  const [entryPrice] = useState<string | null>(initialParams.entryPrice);
  const [fromDate] = useState<string | null>(initialParams.from);
  const [toDate] = useState<string | null>(initialParams.to);
  const [parameterOverrides] = useState<Record<string, number> | null>(initialParams.parameterOverrides);
  const [hoveredCandle, setHoveredCandle] = useState<OHLCData | null>(null);
  const [selectedIndicator, setSelectedIndicator] = useState<{ id: string; label: string } | null>(null);
  const selectedIndicatorRef = useRef<{ id: string; label: string } | null>(null);
  const hoveredIndicatorRef = useRef<{ id: string; label: string } | null>(null);
  const indicatorLabelRef = useRef<HTMLDivElement>(null);
  const [editingIndicatorFromLabel, setEditingIndicatorFromLabel] = useState<string | null>(null);
  // Keep ref in sync with state for use in event handlers
  selectedIndicatorRef.current = selectedIndicator;

  // Indicator selection state - Zustand store is the single source of truth
  const [indicatorMenuOpen, setIndicatorMenuOpen] = useState(false);
  const chartIndicators = useChartIndicatorStore((state) => state.indicators);
  const addIndicator = useChartIndicatorStore((state) => state.addIndicator);
  const updateIndicator = useChartIndicatorStore((state) => state.updateIndicator);
  const removeIndicator = useChartIndicatorStore((state) => state.removeIndicator);
  const initWithSeed = useChartIndicatorStore((state) => state.initWithSeed);
  const switchInstrument = useChartIndicatorStore((state) => state.switchInstrument);
  // Keep ref in sync for crosshair handler
  chartIndicatorsRef.current = chartIndicators;

  const isHistoricalView = Boolean(fromDate && toDate);
  // Show S/R tools on any live chart (not backtest, not trade analytics from URL)
  const showSRTools = !isHistoricalView && !trades;

  // Clear trade markers when instrument changes away from the instrument
  // trades were originally loaded for (BUG-039 fix).
  // The plugin detach is handled in loadCandles when trades becomes null.
  useEffect(() => {
    if (trades && instrument !== tradesInstrumentRef.current) {
      setTrades(null);
    }
  }, [instrument, trades]);

  // Load the strategy from the local store if we have a strategyId (AGT-646)
  const [strategy, setStrategy] = useState<LocalStrategy | null>(null);
  useEffect(() => {
    if (!strategyId) {
      setStrategy(null);
      return;
    }
    let cancelled = false;
    getStrategy(strategyId)
      .then((s) => {
        if (!cancelled) setStrategy(s);
      })
      .catch((err) => console.error('[ChartApp] Failed to load strategy:', err));
    return () => {
      cancelled = true;
    };
  }, [strategyId]);
  const strategyRef = useRef(strategy);
  strategyRef.current = strategy;

  // Initialize indicator store. ONE path:
  // - If caller passed indicators via URL (watcher does this), use them
  // - Otherwise, load from per-instrument persistence
  // On instrument change: save current, load new (or carry over)
  const indicatorStoreInitRef = useRef(false);
  useEffect(() => {
    if (!indicatorStoreInitRef.current) {
      indicatorStoreInitRef.current = true;

      // Parse indicator seed from URL param (passed by watcher via open_chart_window)
      // Format: either raw IndicatorDefinition[] or envelope { indicators, parameters }
      let seed: import('./components/charts/chartTypes').ChartIndicatorConfig[] | null = null;
      if (initialParams.indicatorSeed) {
        try {
          const parsed = JSON.parse(initialParams.indicatorSeed);
          // Check if it's an envelope with parameters
          if (parsed && parsed.indicators && Array.isArray(parsed.indicators) && parsed.indicators.length > 0) {
            const params = Array.isArray(parsed.parameters) ? parsed.parameters : undefined;
            seed = strategyIndicatorsToChartConfigs(parsed.indicators, params);
          } else if (Array.isArray(parsed) && parsed.length > 0) {
            // Legacy: raw IndicatorDefinition[]
            seed = strategyIndicatorsToChartConfigs(parsed);
          }
        } catch (e) {
          console.warn('[ChartApp] Failed to parse indicator seed:', e);
        }
      }

      initWithSeed(instrument, seed);
    } else {
      switchInstrument(instrument);
    }
  }, [instrument, initWithSeed, switchInstrument]);

  // Closed trades for this instrument from the local store (used when the
  // chart opens from the menu, AGT-647). Skipped when URL trades already
  // exist to avoid an unnecessary read.
  const [localTrades, setLocalTrades] = useState<LocalTrade[]>([]);
  useEffect(() => {
    if (trades) {
      setLocalTrades([]);
      return;
    }
    let cancelled = false;
    listClosedTradesByInstrument(instrument)
      .then((rows) => {
        if (!cancelled) setLocalTrades(rows);
      })
      .catch((err) => {
        console.error('[ChartApp] Failed to load trades from local store:', err);
        if (!cancelled) setLocalTrades([]);
      });
    return () => {
      cancelled = true;
    };
  }, [trades, instrument]);

  // Convert local trade rows to TradeData format for the overlay.
  // Only used when no URL/localStorage trades were provided (i.e. chart opened from menu).
  // NOTE: TradeData uses number for prices/pnl. This is acceptable for chart overlay rendering
  // (visual positioning only), but these values should NOT be used for financial calculations.
  const localTradeData: TradeData[] | null = useMemo(() => {
    if (trades) return null; // URL trades take priority
    if (localTrades.length === 0) return null;
    return localTrades
      .filter((t) => t.close_time != null && t.close_price != null)
      .map((t) => ({
        entryTime: Math.floor(t.open_time / 1000),
        exitTime: Math.floor((t.close_time ?? t.open_time) / 1000),
        entryPrice: parseFloat(t.open_price),
        exitPrice: parseFloat(t.close_price ?? t.open_price),
        direction: (parseFloat(t.units) > 0 ? 'long' : 'short') as 'long' | 'short',
        pnl: parseFloat(t.realized_pl ?? '0'),
      }));
  }, [trades, localTrades]);

  // The effective trades to display: URL-provided trades or local-store trades
  const effectiveTrades = trades ?? localTradeData;

  // Use extracted hooks
  const srZones = useSRZones({ instrument, isMainChart: showSRTools, candleSeriesRef });

  // Refresh indicator overlays when a new candle boundary is crossed during streaming.
  // This callback is stored via ref inside usePriceStreaming, so it always sees fresh state
  // without needing to be a stable reference.
  //
  // BUG-072 fix: After fetching indicator data, we extract all timestamps from the results
  // and ensure they exist in the time map before rendering. This is necessary because the
  // streaming code detects candle boundaries using epoch-aligned math, while OANDA uses
  // dailyAlignment=3. For timeframes like H4, this mismatch means new OANDA candle
  // timestamps (e.g., 07:00, 11:00 UTC) may not match the client-calculated boundaries
  // (04:00, 08:00 UTC). Without mapping these timestamps, toBusinessTime falls back to
  // "closest match" which places new indicator data points at incorrect chart positions,
  // making overlays appear frozen.
  const refreshIndicatorsOnNewCandle = useCallback(async () => {
    if (!chartRef.current || !candleSeriesRef.current) return;

    // Indicator configs come from the Zustand store (single source of truth)
    if (chartIndicatorsRef.current.length === 0) return;

    // Don't render through a time map built for different params (see
    // loadedTimeMapKeyRef) — the pending loadCandles re-triggers indicators.
    if (loadedTimeMapKeyRef.current !== timeMapKey) return;

    const indicatorConfigs: IndicatorConfig[] = chartIndicatorsRef.current.map((ind) => ({
      id: ind.id,
      type: ind.type,
      params: ind.params,
    }));
    let customColors: Map<string, Record<string, string>> | undefined = new Map();
    for (const ind of chartIndicatorsRef.current) {
      if (ind.colors && Object.keys(ind.colors).length > 0) {
        customColors.set(ind.id, ind.colors);
      }
    }
    if (customColors.size === 0) customColors = undefined;

    try {
      const indicatorResults = await invoke<IndicatorSeries[]>('get_indicator_data', {
        instrument,
        granularity,
        count: candleCount,
        from: fromDate,
        to: toDate,
        indicatorsJson: JSON.stringify(indicatorConfigs),
      });

      // BUG-072: Sync any new candle timestamps from indicator data into the time map
      syncIndicatorTimestamps(indicatorResults, timeMapStateRef.current);

      clearIndicators(
        chartRef.current,
        indicatorSeriesRef.current,
        candleSeriesRef.current,
        ichimokuCloudRef.current
      );

      const { indicatorSeries, ichimokuCloud } = renderIndicators({
        chart: chartRef.current,
        candleSeries: candleSeriesRef.current!,
        indicatorResults,
        indicatorConfigs,
        customColors,
        candleSeconds: getGranularitySeconds(granularity),
        toBusinessTime,
        signalDirection,
      });

      indicatorSeriesRef.current = indicatorSeries;
      ichimokuCloudRef.current = ichimokuCloud;
      indicatorDataRef.current = indicatorResults;
    } catch (err) {
      console.error('[Chart] Failed to refresh indicators on new candle:', err);
    }
  // All mutable data (chartIndicatorsRef, strategyRef, chartRef, etc.) accessed via refs
  // to avoid stale closures. Only stable values that determine the fetch params are in deps.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instrument, granularity, candleCount, fromDate, toDate, signalDirection]);

  const priceStreaming = usePriceStreaming({
    instrument,
    granularity,
    isHistoricalView,
    timeMapStateRef,
    candleSeriesRef,
    onNewCandle: refreshIndicatorsOnNewCandle,
  });

  // Merge SR zone error with local error
  useEffect(() => {
    if (srZones.error) {
      setError(srZones.error);
    }
  }, [srZones.error]);

  // Time conversion helpers
  const convertCandles = (candles: CandleData[]) => {
    return convertCandlesUtil(candles, timeMapStateRef.current);
  };

  const toBusinessTime = (actualTime: number): number => {
    return toBusinessTimeUtil(actualTime, timeMapStateRef.current.timeMap);
  };

  // Request ID ref for cancelling stale loadCandles responses (Bug #2)
  const loadCandlesRequestIdRef = useRef(0);

  // Identifies which (instrument, granularity, range) the shared time map was
  // last rebuilt for. Candles and indicators load in independent effects; if
  // the indicator fetch resolves before loadCandles rebuilds the map (a race,
  // so the bug is sporadic — e.g. on H1→H4 switches), the indicator series get
  // converted through the previous granularity's map and every point lands at
  // the wrong chart position. Indicator renders are gated on this key matching
  // their own fetch params; candlesLoadedCount re-triggers them once the map
  // for the new params is in place.
  const loadedTimeMapKeyRef = useRef<string | null>(null);
  const timeMapKey = `${instrument}|${granularity}|${candleCount}|${fromDate}|${toDate}`;

  // Load candle data and indicators
  const loadCandles = useCallback(async () => {
    const requestId = ++loadCandlesRequestIdRef.current;
    setLoading(true);
    setError(null);

    try {
      const candles = await invoke<CandleData[]>('get_candles', {
        instrument,
        granularity,
        count: candleCount,
        from: fromDate,
        to: toDate,
      });

      // Discard stale response from a previous instrument/granularity (Bug #2)
      if (requestId !== loadCandlesRequestIdRef.current) return;

      if (candleSeriesRef.current && candles.length > 0) {
        const firstCandleData = candles[0];
        const lastCandleData = candles[candles.length - 1];
        addDebugLog('CANDLES', `Loaded ${candles.length} for ${instrument}`, {
          count: candles.length,
          instrument,
          granularity,
          firstDate: new Date(firstCandleData.time).toLocaleDateString(),
          lastDate: new Date(lastCandleData.time).toLocaleDateString(),
        });

        candlesDataRef.current = candles;
        // BUG-037: Reset hoveredCandle so display defaults to live streaming prices.
        // Previously this set hoveredCandle to the last candle's OHLC, which caused
        // the price display to stay stuck on OHLC after instrument/timeframe changes
        // instead of showing the live bid/ask stream.
        setHoveredCandle(null);

        const chartData = convertCandles(candles);
        candleSeriesRef.current.setData(chartData);

        // The time map now belongs to this fetch's params. Any indicator
        // series rendered for a previous map are misaligned — clear them;
        // the indicator effect re-renders via the candlesLoadedCount bump.
        loadedTimeMapKeyRef.current = timeMapKey;
        if (chartRef.current) {
          clearIndicators(
            chartRef.current,
            indicatorSeriesRef.current,
            candleSeriesRef.current,
            ichimokuCloudRef.current
          );
          ichimokuCloudRef.current = null;
        }

        // Add entry/SL/TP price lines if provided (from watcher signal)
        if (candleSeriesRef.current) {
          // Clean up previous lines
          if (entryPriceLineRef.current) {
            candleSeriesRef.current.removePriceLine(entryPriceLineRef.current);
            entryPriceLineRef.current = null;
          }
          if (slPriceLineRef.current) {
            candleSeriesRef.current.removePriceLine(slPriceLineRef.current);
            slPriceLineRef.current = null;
          }
          if (tpPriceLineRef.current) {
            candleSeriesRef.current.removePriceLine(tpPriceLineRef.current);
            tpPriceLineRef.current = null;
          }

          if (entryPrice) {
            const price = parseFloat(entryPrice);
            if (!isNaN(price)) {
              entryPriceLineRef.current = candleSeriesRef.current.createPriceLine({
                price,
                color: '#f59e0b',
                lineWidth: 2,
                lineStyle: 0,
                axisLabelVisible: true,
                title: 'Entry',
              });
            }
          }
          if (stopLoss) {
            const price = parseFloat(stopLoss);
            if (!isNaN(price)) {
              slPriceLineRef.current = candleSeriesRef.current.createPriceLine({
                price,
                color: '#ef4444',
                lineWidth: 1,
                lineStyle: 2, // dashed
                axisLabelVisible: true,
                title: 'SL',
              });
            }
          }
          if (takeProfit) {
            const price = parseFloat(takeProfit);
            if (!isNaN(price)) {
              tpPriceLineRef.current = candleSeriesRef.current.createPriceLine({
                price,
                color: '#22c55e',
                lineWidth: 1,
                lineStyle: 2, // dashed
                axisLabelVisible: true,
                title: 'TP',
              });
            }
          }
        }

        // Signal that candles (and time map) are ready — the trade overlay
        // useEffect will pick this up and attach/update the overlay.
        setCandlesLoadedCount(c => c + 1);

        // Set visible range - wrapped in try-catch since chart data may not be fully synced
        if (chartRef.current && chartData.length > 0) {
          try {
            const fromIndex = Math.max(0, chartData.length - INITIAL_VISIBLE_CANDLES);
            chartRef.current.timeScale().setVisibleRange({
              from: chartData[fromIndex].time,
              to: chartData[chartData.length - 1].time,
            });
          } catch {
            // Ignore - chart will auto-fit or we'll retry on next data update
          }
        }
      }
    } catch (err) {
      // Discard errors from stale requests (Bug #2)
      if (requestId !== loadCandlesRequestIdRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (requestId === loadCandlesRequestIdRef.current) {
        setLoading(false);
      }
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instrument, granularity, candleCount, fromDate, toDate, entryPrice, stopLoss, takeProfit]);

  // Initialize chart
  useEffect(() => {
    if (!chartContainerRef.current) return;

    const initialWidth = chartContainerRef.current.clientWidth || 800;
    const initialHeight = chartContainerRef.current.clientHeight || 600;

    // Note: lightweight-charts requires hex colors, these match design tokens:
    // --color-bg-page: #0e1117, --color-bg-elevated: #1a1f26, --color-border: #2d333b
    const chart = createChart(chartContainerRef.current, {
      width: initialWidth,
      height: initialHeight,
      autoSize: false,
      layout: {
        background: { color: '#0e1117' },
        textColor: '#9ca3af',
        attributionLogo: false,
      },
      grid: {
        vertLines: { color: '#1a1f26' },
        horzLines: { color: '#1a1f26' },
      },
      crosshair: { mode: 0 },  // Normal mode - follows mouse directly
      rightPriceScale: { borderColor: '#2d333b' },
      timeScale: {
        borderColor: '#2d333b',
        timeVisible: true,
        secondsVisible: false,
        rightOffset: FUTURE_CANDLE_SLOTS,
      },
      localization: {
        timeFormatter: (time: number) => {
          // Convert business time back to actual time using the reverse map
          const actualTime = timeMapStateRef.current.reverseTimeMap.get(time) ?? time;
          const date = new Date(actualTime * 1000);
          return date.toLocaleString('en-US', {
            month: 'short',
            day: 'numeric',
            year: '2-digit',
            hour: '2-digit',
            minute: '2-digit',
            hour12: false,
          });
        },
      },
    });

    const precision = getInstrumentPrecision(initialParams.instrument);
    const candleSeries = chart.addSeries(CandlestickSeries, {
      upColor: 'transparent',
      downColor: '#ef4444',
      borderVisible: true,
      borderUpColor: '#22c55e',
      borderDownColor: '#ef4444',
      wickUpColor: '#22c55e',
      wickDownColor: '#ef4444',
      priceFormat: {
        type: 'price',
        precision: precision,
        minMove: 1 / Math.pow(10, precision),
      },
    });

    chartRef.current = chart;
    candleSeriesRef.current = candleSeries;

    // Helper to update indicator label DOM directly (avoids re-renders)
    const updateIndicatorLabel = (indicator: { id: string; label: string } | null) => {
      hoveredIndicatorRef.current = indicator;
      if (indicatorLabelRef.current) {
        // Show selected indicator if set, otherwise show hovered
        const displayIndicator = selectedIndicatorRef.current || indicator;
        if (displayIndicator) {
          indicatorLabelRef.current.textContent = displayIndicator.label;
          indicatorLabelRef.current.style.display = 'block';
        } else {
          indicatorLabelRef.current.style.display = 'none';
        }
      }
    };

    // Subscribe to crosshair move for OHLC display and indicator hover
    chart.subscribeCrosshairMove((param) => {
      if (!param.time || !param.seriesData || param.seriesData.size === 0) {
        setHoveredCandle(null);
        updateIndicatorLabel(null);
        return;
      }

      // Get OHLC from seriesData
      const candleData = param.seriesData.get(candleSeries);
      if (candleData && 'open' in candleData) {
        const ohlc = candleData as { open: number; high: number; low: number; close: number };
        setHoveredCandle({
          open: ohlc.open.toString(),
          high: ohlc.high.toString(),
          low: ohlc.low.toString(),
          close: ohlc.close.toString(),
        });
      } else {
        setHoveredCandle(null);
      }

      // Check for indicator hover - only if we have indicators
      if (!param.point || indicatorSeriesRef.current.size === 0) {
        updateIndicatorLabel(null);
        return;
      }

      const mouseY = param.point.y;
      let closestDistance = Infinity;
      let closestIndicator: { id: string; label: string } | null = null;
      const HOVER_THRESHOLD = 10;

      for (const [seriesKey, series] of indicatorSeriesRef.current) {
        const seriesData = param.seriesData.get(series);
        if (seriesData && 'value' in seriesData) {
          const value = (seriesData as { value: number }).value;
          const yCoord = series.priceToCoordinate(value);
          if (yCoord !== null) {
            const distance = Math.abs(yCoord - mouseY);
            if (distance < HOVER_THRESHOLD && distance < closestDistance) {
              closestDistance = distance;
              const [indId] = seriesKey.split('.');
              const indConfig = chartIndicatorsRef.current.find(ind => ind.id === indId);
              closestIndicator = { id: indId, label: indConfig ? formatIndicatorLabel(indConfig) : indId };
            }
          }
        }
      }

      updateIndicatorLabel(closestIndicator);
    });

    // Handle resize
    const resizeObserver = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width, height } = entry.contentRect;
        if (width > 0 && height > 0) {
          chart.applyOptions({ width, height });
        }
      }
    });

    if (chartContainerRef.current) {
      resizeObserver.observe(chartContainerRef.current);
    }

    loadCandles();

    return () => {
      resizeObserver.disconnect();
      chart.remove();
    };
  }, []);

  // Update price precision and window title when instrument changes
  useEffect(() => {
    if (candleSeriesRef.current) {
      const precision = getInstrumentPrecision(instrument);
      candleSeriesRef.current.applyOptions({
        priceFormat: {
          type: 'price',
          precision: precision,
          minMove: 1 / Math.pow(10, precision),
        },
      });
    }
    const displayInstrument = instrument.replace('_', '/');
    const displayGranularity = GRANULARITIES.find(g => g.value === granularity)?.label || granularity;
    getCurrentWindow().setTitle(`Chart - ${displayInstrument} ${displayGranularity}`);
  }, [instrument, granularity]);

  // Reload when instrument/granularity/count changes or strategy loads
  useEffect(() => {
    if (chartRef.current) {
      loadCandles();
    }
  }, [instrument, granularity, candleCount, loadCandles]);

  // Single owner of trade overlay lifecycle. Runs when:
  // 1. Candles finish loading (candlesLoadedCount increments, time map is populated)
  // 2. Trade data arrives/changes (effectiveTrades updates)
  // This avoids duplicating overlay attachment logic in loadCandles.
  useEffect(() => {
    if (!candleSeriesRef.current || !chartRef.current) return;

    // Guard: time map must be populated before we can convert trade timestamps
    if (timeMapStateRef.current.timeMap.size === 0) return;

    if (!effectiveTrades || effectiveTrades.length === 0) {
      // No trades to display - detach existing overlay if any
      if (tradeOverlayRef.current) {
        candleSeriesRef.current.detachPrimitive(tradeOverlayRef.current);
        tradeOverlayRef.current = null;
      }
      return;
    }

    const tradesInBusinessTime = effectiveTrades.map(trade => ({
      ...trade,
      entryTime: toBusinessTimeUtil(trade.entryTime, timeMapStateRef.current.timeMap),
      exitTime: toBusinessTimeUtil(trade.exitTime, timeMapStateRef.current.timeMap),
    }));

    if (tradeOverlayRef.current) {
      // Update existing overlay in-place
      tradeOverlayRef.current.updateTrades(tradesInBusinessTime);
    } else {
      // Create and attach new overlay
      tradeOverlayRef.current = new TradeOverlayPlugin(tradesInBusinessTime);
      candleSeriesRef.current.attachPrimitive(tradeOverlayRef.current);
    }
  // candlesLoadedCount triggers re-run after time map is populated by loadCandles
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [effectiveTrades, candlesLoadedCount]);

  // Load indicators when chartIndicators changes
  useEffect(() => {
    if (!chartRef.current) {
      // Chart not initialized yet, nothing to clear
      return;
    }

    if (chartIndicators.length === 0) {
      clearIndicators(
        chartRef.current,
        indicatorSeriesRef.current,
        candleSeriesRef.current,
        ichimokuCloudRef.current
      );
      ichimokuCloudRef.current = null;
      return;
    }

    // Gate on the time map matching this effect's fetch params: rendering
    // indicator data through a map built for different params misplaces every
    // series (see loadedTimeMapKeyRef). When loadCandles finishes rebuilding
    // the map, candlesLoadedCount bumps and this effect runs again.
    if (loadedTimeMapKeyRef.current !== timeMapKey) return;

    // Convert ChartIndicatorConfig[] to IndicatorConfig[] for backend
    const indicatorConfigs: IndicatorConfig[] = chartIndicators.map((ind) => ({
      id: ind.id,
      type: ind.type,
      params: ind.params,
    }));

    // Build custom colors map from chartIndicators
    const customColors = new Map<string, Record<string, string>>();
    for (const ind of chartIndicators) {
      if (ind.colors && Object.keys(ind.colors).length > 0) {
        customColors.set(ind.id, ind.colors);
      }
    }

    // Cancellation flag to prevent touching the chart after destruction (Bug #19)
    let cancelled = false;

    const loadSelectedIndicators = async () => {
      try {
        const indicatorResults = await invoke<IndicatorSeries[]>('get_indicator_data', {
          instrument,
          granularity,
          count: candleCount,
          from: fromDate,
          to: toDate,
          indicatorsJson: JSON.stringify(indicatorConfigs),
        });

        // Discard if chart was destroyed while awaiting (Bug #19)
        if (cancelled || !chartRef.current) return;

        // BUG-072: Sync any new candle timestamps from indicator data into the time map
        syncIndicatorTimestamps(indicatorResults, timeMapStateRef.current);

        clearIndicators(
          chartRef.current!,
          indicatorSeriesRef.current,
          candleSeriesRef.current,
          ichimokuCloudRef.current
        );

        const { indicatorSeries, ichimokuCloud } = renderIndicators({
          chart: chartRef.current!,
          candleSeries: candleSeriesRef.current!,
          indicatorResults,
          indicatorConfigs,
          customColors: customColors.size > 0 ? customColors : undefined,
          candleSeconds: getGranularitySeconds(granularity),
          toBusinessTime,
          signalDirection,
        });

        indicatorSeriesRef.current = indicatorSeries;
        ichimokuCloudRef.current = ichimokuCloud;
        indicatorDataRef.current = indicatorResults;
      } catch (err) {
        if (!cancelled) {
          console.error('[Chart] Failed to load selected indicators:', err);
        }
      }
    };

    loadSelectedIndicators().catch(() => { /* handled inside */ });

    return () => { cancelled = true; };
  // candlesLoadedCount re-runs this effect after loadCandles rebuilds the time
  // map (the gate above returns early until the map matches these params).
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chartIndicators, instrument, granularity, candleCount, fromDate, toDate, candlesLoadedCount]);

  // Track hovered edge for cursor changes
  const [hoveredEdge, setHoveredEdge] = useState<{ zoneId: string; edge: 'upper' | 'lower' } | null>(null);

  // Handle chart mouse events for S/R zone drawing and edge detection
  const handleChartMouseMove = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    if (!candleSeriesRef.current || !chartContainerRef.current) return;

    const rect = chartContainerRef.current.getBoundingClientRect();
    const y = e.clientY - rect.top;
    const price = candleSeriesRef.current.coordinateToPrice(y);

    // Handle edge resizing drag
    if (srZones.resizingEdge && price !== null) {
      const zone = srZones.srZones.find(z => z.id === srZones.resizingEdge!.zoneId);
      if (zone) {
        const upperY = candleSeriesRef.current.priceToCoordinate(
          srZones.resizingEdge.edge === 'upper' ? price : zone.upper_price
        );
        const lowerY = candleSeriesRef.current.priceToCoordinate(
          srZones.resizingEdge.edge === 'lower' ? price : zone.lower_price
        );
        if (upperY !== null && lowerY !== null) {
          srZones.setPreviewZone({ upperY, lowerY });
        }
      }
      return;
    }

    // Check for edge hover (only on custom zones without labels, not in editing mode)
    if (!srZones.srEditingMode && showSRTools) {
      const EDGE_THRESHOLD = 6; // pixels
      let foundEdge: { zoneId: string; edge: 'upper' | 'lower' } | null = null;

      for (const zone of srZones.srZones) {
        // Skip pivot zones (they have labels)
        if (zone.label) continue;

        const upperY = candleSeriesRef.current.priceToCoordinate(zone.upper_price);
        const lowerY = candleSeriesRef.current.priceToCoordinate(zone.lower_price);

        if (upperY !== null && Math.abs(y - upperY) <= EDGE_THRESHOLD) {
          foundEdge = { zoneId: zone.id, edge: 'upper' };
          break;
        }
        if (lowerY !== null && Math.abs(y - lowerY) <= EDGE_THRESHOLD) {
          foundEdge = { zoneId: zone.id, edge: 'lower' };
          break;
        }
      }

      setHoveredEdge(foundEdge);
    }

    // Handle zone drawing preview
    if (!srZones.srEditingMode || !chartRef.current) return;

    if (price === null || srZones.secondBoundary !== null) return;

    // Validate price is within main chart's visible range (not on oscillator panes)
    const visibleRange = chartRef.current.timeScale().getVisibleLogicalRange();
    if (visibleRange) {
      const data = candleSeriesRef.current.data();
      if (data.length > 0) {
        const startIdx = Math.max(0, Math.floor(visibleRange.from));
        const endIdx = Math.min(data.length - 1, Math.ceil(visibleRange.to));
        let minPrice = Infinity, maxPrice = -Infinity;
        for (let i = startIdx; i <= endIdx; i++) {
          const candle = data[i] as { low: number; high: number };
          if (candle.low < minPrice) minPrice = candle.low;
          if (candle.high > maxPrice) maxPrice = candle.high;
        }
        const range = maxPrice - minPrice;
        const buffer = range * 0.2;
        if (price < minPrice - buffer || price > maxPrice + buffer) {
          // Mouse is outside price range - likely on an oscillator pane
          // Clear any existing preview line
          if (srZones.previewLineRef.current) {
            candleSeriesRef.current.removePriceLine(srZones.previewLineRef.current);
            srZones.previewLineRef.current = null;
          }
          return;
        }
      }
    }

    // Update preview line
    if (srZones.previewLineRef.current) {
      candleSeriesRef.current.removePriceLine(srZones.previewLineRef.current);
    }
    srZones.previewLineRef.current = candleSeriesRef.current.createPriceLine({
      price,
      color: '#60a5fa',
      lineWidth: 1,
      lineStyle: 2,
      axisLabelVisible: true,
      title: srZones.pendingZoneBoundary === null ? 'Click to set first boundary' : 'Click to set second boundary',
    });
  }, [srZones.srEditingMode, srZones.pendingZoneBoundary, srZones.secondBoundary, srZones.previewLineRef, srZones.resizingEdge, srZones.srZones, srZones.setPreviewZone, showSRTools]);

  // Handle mouse down for edge resize
  const handleChartMouseDown = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    if (!hoveredEdge || !candleSeriesRef.current) return;

    const zone = srZones.srZones.find(z => z.id === hoveredEdge.zoneId);
    if (!zone) return;

    e.preventDefault();
    const startPrice = hoveredEdge.edge === 'upper' ? zone.upper_price : zone.lower_price;
    srZones.setResizingEdge({ zoneId: hoveredEdge.zoneId, edge: hoveredEdge.edge, startPrice });
  }, [hoveredEdge, srZones.srZones, srZones.setResizingEdge]);

  // Handle mouse up to commit edge resize
  const handleChartMouseUp = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    if (!srZones.resizingEdge || !candleSeriesRef.current || !chartContainerRef.current) return;

    const rect = chartContainerRef.current.getBoundingClientRect();
    const y = e.clientY - rect.top;
    const price = candleSeriesRef.current.coordinateToPrice(y);

    if (price !== null) {
      const zone = srZones.srZones.find(z => z.id === srZones.resizingEdge!.zoneId);
      if (zone) {
        const precision = getInstrumentPrecision(instrument);
        if (srZones.resizingEdge.edge === 'upper') {
          // Ensure upper stays above lower
          const newUpper = Math.max(price, zone.lower_price + 0.00001);
          srZones.handleUpdateZone(zone.id, { upper_price: newUpper.toFixed(precision) });
        } else {
          // Ensure lower stays below upper
          const newLower = Math.min(price, zone.upper_price - 0.00001);
          srZones.handleUpdateZone(zone.id, { lower_price: newLower.toFixed(precision) });
        }
      }
    }

    srZones.setResizingEdge(null);
    srZones.setPreviewZone(null);
  }, [srZones.resizingEdge, srZones.srZones, srZones.handleUpdateZone, srZones.setResizingEdge, srZones.setPreviewZone, instrument]);

  // Global mouseup to handle edge resize when mouse released outside chart
  useEffect(() => {
    if (!srZones.resizingEdge) return;

    const handleGlobalMouseUp = () => {
      srZones.setResizingEdge(null);
      srZones.setPreviewZone(null);
    };

    window.addEventListener('mouseup', handleGlobalMouseUp);
    return () => window.removeEventListener('mouseup', handleGlobalMouseUp);
  }, [srZones.resizingEdge, srZones.setResizingEdge, srZones.setPreviewZone]);

  const handleChartClick = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    if (!candleSeriesRef.current || !chartContainerRef.current || !chartRef.current) return;

    const rect = chartContainerRef.current.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    const price = candleSeriesRef.current.coordinateToPrice(y);

    // Handle indicator selection - if hovering over an indicator, select it on click
    // Otherwise, clear the selected indicator
    if (hoveredIndicatorRef.current) {
      setSelectedIndicator(hoveredIndicatorRef.current);
      return;
    } else if (selectedIndicator) {
      setSelectedIndicator(null);
      return;
    }

    // Handle zone selection/edit button clicks (only on live charts)
    if (srZones.selectedZoneId && showSRTools) {
      const selectedZone = srZones.srZones.find(z => z.id === srZones.selectedZoneId);
      if (selectedZone) {
        const upperY = candleSeriesRef.current.priceToCoordinate(selectedZone.upper_price);
        const lowerY = candleSeriesRef.current.priceToCoordinate(selectedZone.lower_price);

        if (upperY !== null && lowerY !== null) {
          const minY = Math.min(upperY, lowerY);
          const maxY = Math.max(upperY, lowerY);
          const rawHeight = maxY - minY;
          const height = Math.max(rawHeight, 10);
          const drawMinY = rawHeight < 10 ? minY - 5 : minY;

          const chartWidth = chartRef.current.timeScale().width();
          const buttonSize = 20;
          const deleteButtonX = chartWidth - buttonSize - 10;
          const editButtonX = deleteButtonX - buttonSize - 6;
          const buttonCenterY = drawMinY + (height / 2);

          // Check edit button
          const editDistance = Math.sqrt(Math.pow(x - (editButtonX + buttonSize / 2), 2) + Math.pow(y - buttonCenterY, 2));
          if (editDistance <= buttonSize / 2) {
            srZones.handleEditZone(selectedZone);
            return;
          }

          // Check delete button
          const deleteDistance = Math.sqrt(Math.pow(x - (deleteButtonX + buttonSize / 2), 2) + Math.pow(y - buttonCenterY, 2));
          if (deleteDistance <= buttonSize / 2) {
            srZones.handleDeleteSRZone(srZones.selectedZoneId);
            srZones.setSelectedZoneId(null);
            return;
          }
        }
      }
    }

    // Check if clicking on a zone (for selection)
    if (!srZones.srEditingMode && price !== null && showSRTools) {
      const clickedZone = srZones.srZones.find(zone => price >= zone.lower_price && price <= zone.upper_price);
      if (clickedZone) {
        // Toggle selection: if already selected, deselect; otherwise select
        srZones.setSelectedZoneId(srZones.selectedZoneId === clickedZone.id ? null : clickedZone.id);
      } else {
        srZones.setSelectedZoneId(null);
      }
      return;
    }

    // Handle zone drawing
    if (!srZones.srEditingMode || price === null) return;

    // Validate price is within main chart's visible range (not on oscillator panes)
    const visibleRange = chartRef.current.timeScale().getVisibleLogicalRange();
    if (visibleRange && candleSeriesRef.current) {
      const data = candleSeriesRef.current.data();
      if (data.length > 0) {
        // Get price range from visible candles
        const startIdx = Math.max(0, Math.floor(visibleRange.from));
        const endIdx = Math.min(data.length - 1, Math.ceil(visibleRange.to));
        let minPrice = Infinity, maxPrice = -Infinity;
        for (let i = startIdx; i <= endIdx; i++) {
          const candle = data[i] as { low: number; high: number };
          if (candle.low < minPrice) minPrice = candle.low;
          if (candle.high > maxPrice) maxPrice = candle.high;
        }
        // Add 20% buffer
        const range = maxPrice - minPrice;
        const buffer = range * 0.2;
        if (price < minPrice - buffer || price > maxPrice + buffer) {
          // Click is outside price range - likely on an oscillator pane
          return;
        }
      }
    }

    if (e.shiftKey && srZones.pendingZoneBoundary === null) {
      // Shift+click: Create single-line zone
      srZones.setPendingZoneBoundary(price);
      srZones.setSecondBoundary(price);

      if (srZones.firstBoundaryLineRef.current) {
        candleSeriesRef.current.removePriceLine(srZones.firstBoundaryLineRef.current);
      }
      srZones.firstBoundaryLineRef.current = candleSeriesRef.current.createPriceLine({
        price,
        color: '#3b82f6',
        lineWidth: 2,
        lineStyle: 0,
        axisLabelVisible: true,
        title: 'S/R Level',
      });

      if (srZones.previewLineRef.current) {
        candleSeriesRef.current.removePriceLine(srZones.previewLineRef.current);
        srZones.previewLineRef.current = null;
      }

      const upperY = candleSeriesRef.current.priceToCoordinate(price);
      if (upperY !== null) {
        srZones.setPreviewZone({ upperY, lowerY: upperY });
      }
    } else if (srZones.pendingZoneBoundary === null) {
      // First click
      srZones.setPendingZoneBoundary(price);

      if (srZones.firstBoundaryLineRef.current) {
        candleSeriesRef.current.removePriceLine(srZones.firstBoundaryLineRef.current);
      }
      srZones.firstBoundaryLineRef.current = candleSeriesRef.current.createPriceLine({
        price,
        color: '#3b82f6',
        lineWidth: 2,
        lineStyle: 0,
        axisLabelVisible: true,
        title: 'Zone boundary 1',
      });
    } else if (srZones.secondBoundary === null) {
      // Second click
      srZones.setSecondBoundary(price);

      if (srZones.secondBoundaryLineRef.current) {
        candleSeriesRef.current.removePriceLine(srZones.secondBoundaryLineRef.current);
      }
      srZones.secondBoundaryLineRef.current = candleSeriesRef.current.createPriceLine({
        price,
        color: '#3b82f6',
        lineWidth: 2,
        lineStyle: 0,
        axisLabelVisible: true,
        title: 'Zone boundary 2',
      });

      if (srZones.previewLineRef.current) {
        candleSeriesRef.current.removePriceLine(srZones.previewLineRef.current);
        srZones.previewLineRef.current = null;
      }

      const upperY = candleSeriesRef.current.priceToCoordinate(srZones.pendingZoneBoundary);
      const lowerY = candleSeriesRef.current.priceToCoordinate(price);
      if (upperY !== null && lowerY !== null) {
        srZones.setPreviewZone({ upperY, lowerY });
      }
    }
  }, [srZones, showSRTools, selectedIndicator]);

  // Cancel zone edit handler
  const handleCancelZoneEdit = useCallback(() => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    srZones.clearPreviewLines(candleSeriesRef.current as any);
    srZones.setPendingZoneBoundary(null);
    srZones.setSecondBoundary(null);
    srZones.setSrEditingMode(false);
  }, [srZones]);

  // Save zone handler
  const handleSaveZone = useCallback(async () => {
    await srZones.handleSaveZone();
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    srZones.clearPreviewLines(candleSeriesRef.current as any);
  }, [srZones]);

  return (
    <div className="min-h-screen bg-[var(--color-bg-page)] text-[var(--color-text-primary)] flex flex-col relative">
      <WindowHeader
        title="Charting"
        currentWindow="charting"
        fullWidth
        settingsOpen={settingsOpen}
        onSettingsChange={setSettingsOpen}
        terminalContextProvider={() => {
          // Get last 50 candles for potential chart analysis
          const recentCandles = candlesDataRef.current.slice(-50).map(c => ({
            time: c.time,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
          }));
          // Extract current (latest) indicator values from loaded data
          const indicatorValues: Record<string, string> = {};
          for (const series of indicatorDataRef.current) {
            const lastPoint = series.data[series.data.length - 1];
            if (lastPoint) {
              for (const [output, value] of Object.entries(lastPoint.values)) {
                const key = series.type === output ? output : `${series.type}_${output}`;
                indicatorValues[key] = value;
              }
            }
          }
          // Parse strategy risk settings if available
          let riskSettings: Record<string, unknown> | undefined;
          if (strategy?.risk_settings) {
            try {
              riskSettings = JSON.parse(strategy.risk_settings);
            } catch (e) { console.warn('[ChartApp] Failed to parse strategy risk_settings:', e); }
          }
          return buildChartingContext({
            instrument,
            granularity,
            strategyName: strategy?.name,
            strategyId: strategy?.id,
            strategyRiskSettings: riskSettings,
            indicators: chartIndicators.map((ind) => formatIndicatorLabel(ind)),
            indicatorValues: Object.keys(indicatorValues).length > 0 ? indicatorValues : undefined,
            currentPrice: priceStreaming.currentPrice?.bid,
            signalDirection: signalDirection ?? undefined,
            recentCandles,
          });
        }}
        terminalHeader={getTerminalWelcome('charting', { instrument }).header}
        terminalHeaderDescription={getTerminalWelcome('charting', { instrument }).description}
        terminalWelcomeContent={getTerminalWelcome('charting', { instrument }).content}
        subHeader={
          <ChartHeader
          instrument={instrument}
          granularity={granularity}
          loading={loading}
          onInstrumentChange={setInstrument}
          onGranularityChange={setGranularity}
          isHistoricalView={isHistoricalView}
          hoveredCandle={hoveredCandle}
          streaming={priceStreaming.streaming}
          currentPrice={priceStreaming.currentPrice}
          isMainChart={showSRTools}
          srEditingMode={srZones.srEditingMode}
          pendingZoneBoundary={srZones.pendingZoneBoundary}
          secondBoundary={srZones.secondBoundary}
          srMenuOpen={srZones.srMenuOpen}
          zoneCount={srZones.srZones.length}
          importingPivots={srZones.importingPivots}
          confirmClearAll={srZones.confirmClearAll}
          onSaveZone={handleSaveZone}
          onCancelZoneEdit={handleCancelZoneEdit}
          onSrMenuToggle={() => srZones.setSrMenuOpen(!srZones.srMenuOpen)}
          onSrMenuClose={() => srZones.setSrMenuOpen(false)}
          onDrawZone={() => { srZones.setSrEditingMode(true); srZones.setSrMenuOpen(false); }}
          onImportPivots={srZones.handleImportPivots}
          onClearAllZones={srZones.handleClearAllZones}
          onConfirmClearAll={() => srZones.setConfirmClearAll(true)}
          onCancelClearAll={() => srZones.setConfirmClearAll(false)}
          indicatorMenuOpen={indicatorMenuOpen}
          indicators={chartIndicators}
          onIndicatorMenuToggle={() => setIndicatorMenuOpen(!indicatorMenuOpen)}
          onIndicatorMenuClose={() => setIndicatorMenuOpen(false)}
          onAddIndicator={(type: IndicatorType, params: Record<string, number>, colors?: Record<string, string>) => addIndicator(type, params, colors)}
          onUpdateIndicator={(id: string, params: Record<string, number>, colors?: Record<string, string>) => updateIndicator(id, params, colors)}
          onRemoveIndicator={(id: string) => {
            // Clear selected indicator if it's the one being removed
            if (selectedIndicator?.id === id) {
              setSelectedIndicator(null);
            }
            removeIndicator(id);
          }}
          signalDirection={signalDirection}
          strategyId={strategyId}
          />
        }
      />
      <FirstRunTour windowType="chart" steps={chartTourSteps} />

      {/* Error */}
      {error && (
        <div className="px-4 py-2 bg-[var(--color-sell)]/20 border-l-2 border-[var(--color-sell)] text-[var(--color-text-secondary)] text-sm">
          {error}
        </div>
      )}

      {/* Chart and legends container */}
      <div className="flex-1 flex flex-col overflow-hidden">
        <div
          ref={chartContainerRef}
          data-tour="chart-canvas"
          className={`flex-1 overflow-hidden relative ${
            srZones.srEditingMode ? 'cursor-crosshair' :
            (hoveredEdge || srZones.resizingEdge) ? 'cursor-ns-resize' : ''
          }`}
          onMouseMove={handleChartMouseMove}
          onMouseDown={handleChartMouseDown}
          onMouseUp={handleChartMouseUp}
          onMouseLeave={() => {
            if (srZones.resizingEdge) {
              srZones.setResizingEdge(null);
              srZones.setPreviewZone(null);
            }
            setHoveredEdge(null);
          }}
          onClick={handleChartClick}
        >
          {/* Indicator label - shows selected (sticky) or hovered, updated via ref for performance */}
          <div
            ref={indicatorLabelRef}
            onClick={(e) => {
              e.stopPropagation();
              const indicatorToEdit = selectedIndicatorRef.current || hoveredIndicatorRef.current;
              if (indicatorToEdit) {
                setEditingIndicatorFromLabel(indicatorToEdit.id);
              }
            }}
            className="absolute top-5 left-2 z-[5] px-2 py-1 bg-[var(--color-bg-elevated)]/90 border border-[var(--color-border)] rounded text-xs text-[var(--color-text-secondary)] cursor-pointer hover:border-[var(--color-info)]"
            style={{ display: selectedIndicator ? 'block' : 'none' }}
          >
            {selectedIndicator?.label}
          </div>
          <SRZoneOverlay
            chart={chartRef.current}
            series={candleSeriesRef.current}
            zones={srZones.srZones}
            previewZone={showSRTools ? srZones.previewZone : null}
            selectedZoneId={showSRTools ? srZones.selectedZoneId : null}
            showEditButton={showSRTools}
            containerRef={chartContainerRef}
          />
        </div>

        {effectiveTrades && <TradeLegend trades={effectiveTrades} />}

        {/* Test Parameters overlay - shows optimized params from walk-forward */}
        {parameterOverrides && Object.keys(parameterOverrides).length > 0 && (
          <div className="flex-shrink-0 bg-purple-900/30 border-t border-purple-700/50 px-4 py-2 text-sm flex items-center gap-2">
            <span className="text-purple-400 font-medium">Test Parameters:</span>
            <span className="font-mono text-gray-200">
              {Object.entries(parameterOverrides)
                .map(([key, value]) => `${key}=${value}`)
                .join(', ')}
            </span>
          </div>
        )}

      </div>

      {/* S/R Zone Editor Modal */}
      {showSRTools && (
        <SRZoneEditor
          zone={srZones.editingZone}
          instrument={instrument}
          onSave={srZones.handleUpdateZone}
          onClose={() => srZones.setEditingZone(null)}
        />
      )}

      {/* Indicator Config Modal - opened from label click */}
      {editingIndicatorFromLabel && (() => {
        const indicator = chartIndicators.find(ind => ind.id === editingIndicatorFromLabel);
        if (!indicator) return null;
        return (
          <IndicatorConfigModal
            isOpen={true}
            indicator={indicator}
            onSave={(_type, params, colors) => {
              updateIndicator(editingIndicatorFromLabel, params, colors);
              setEditingIndicatorFromLabel(null);
              // Update the selected indicator label to reflect new params
              const updatedConfig = { ...indicator, params, colors };
              const newLabel = formatIndicatorLabel(updatedConfig);
              setSelectedIndicator({ id: editingIndicatorFromLabel, label: newLabel });
            }}
            onClose={() => setEditingIndicatorFromLabel(null)}
          />
        );
      })()}
    </div>
  );
};
