import { invoke } from '@tauri-apps/api/core';

/**
 * Opens the backtest window via Rust backend.
 * The backend handles URL creation correctly for both dev and production.
 */
export async function openBacktestWindow(): Promise<void> {
  try {
    await invoke('open_backtest_window');
  } catch (e) {
    console.error('[windows] Failed to open backtest window:', e);
  }
}

/**
 * Opens a chart window via Rust backend with optional parameters.
 */
export async function openChartWindow(params?: {
  instrument?: string;
  granularity?: string;
  count?: number;
  from?: string;
  to?: string;
  trades?: string;
  strategyId?: string;
  signalDirection?: string;
  signalId?: string;
  indicators?: string; // JSON array of indicator type IDs
}): Promise<void> {
  try {
    await invoke('open_chart_window', {
      instrument: params?.instrument,
      granularity: params?.granularity,
      count: params?.count,
      from: params?.from,
      to: params?.to,
      trades: params?.trades,
      strategyId: params?.strategyId,
      signalDirection: params?.signalDirection,
      signalId: params?.signalId,
      indicators: params?.indicators,
    });
  } catch (e) {
    console.error('[windows] Failed to open chart window:', e);
  }
}

/**
 * Opens the main account window via Rust backend.
 */
export async function openWatcherWindow(): Promise<void> {
  try {
    await invoke('open_startup_windows', { windows: ['watcher'] });
  } catch (e) {
    console.error('[windows] Failed to open watcher window:', e);
  }
}

/**
 * Opens the trade analysis window via Rust backend.
 */
