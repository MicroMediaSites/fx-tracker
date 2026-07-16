import { create } from 'zustand';

export interface PriceUpdate {
  instrument: string;
  bid: string;
  ask: string;
  spread: string;
  time: string;
  tradeable: boolean;
}

export interface StreamError {
  errorType:
    | 'parse_error'
    | 'connection_lost'
    | 'stream_ended'
    | 'reconnecting'
    | 'max_reconnects_exceeded';
  message: string;
}

/** Payload of the backend's `stream-health` event (camelCase serde). */
export interface StreamHealth {
  healthy: boolean;
  secondsSinceHeartbeat: number;
  subscribedInstruments: number;
  running: boolean;
}

interface PriceState {
  prices: Record<string, PriceUpdate>;
  streaming: boolean;
  error: StreamError | null;
  /** Last `stream-health` report from the backend (null until first report). */
  streamHealth: StreamHealth | null;
  /** Wall-clock ms of the last price flush — drives the UI staleness badge
   *  even when no health event arrives (e.g. an attached hub going silent). */
  lastTickAtMs: number | null;

  updatePrice: (price: PriceUpdate) => void;
  setStreaming: (streaming: boolean) => void;
  setError: (error: StreamError | null) => void;
  setStreamHealth: (health: StreamHealth) => void;
  clearPrices: () => void;
}

// --- Batched price update mechanism ---
// Instead of calling Zustand set() on every tick (which triggers a React re-render
// per tick * number of subscribed components), we buffer price updates and flush
// them at most once per animation frame. With 28+ instruments streaming M1 data,
// this reduces re-renders from hundreds/second to ~60/second max.
let pendingPriceUpdates: Record<string, PriceUpdate> = {};
let flushScheduled = false;
let flushStore: ((updater: (state: PriceState) => Partial<PriceState>) => void) | null = null;

function schedulePriceFlush() {
  if (flushScheduled) return;
  flushScheduled = true;
  requestAnimationFrame(() => {
    flushScheduled = false;
    const updates = pendingPriceUpdates;
    pendingPriceUpdates = {};
    if (Object.keys(updates).length === 0) return;
    flushStore?.((state) => ({
      prices: { ...state.prices, ...updates },
      lastTickAtMs: Date.now(),
    }));
  });
}

export const usePriceStore = create<PriceState>((set) => {
  // Capture the set function so the batching mechanism can use it
  flushStore = set;

  return {
    prices: {},
    streaming: false,
    error: null,
    streamHealth: null,
    lastTickAtMs: null,

    updatePrice: (price) => {
      // Buffer the update instead of immediately calling set()
      pendingPriceUpdates[price.instrument] = price;
      schedulePriceFlush();
    },

    setStreaming: (streaming) => set({ streaming }),

    setError: (error) => set({ error }),

    setStreamHealth: (health) =>
      set((state) => ({
        streamHealth: health,
        // A healthy report means a reconnect-class outage self-healed —
        // clear its banner rather than leaving a stale error on screen.
        error:
          health.healthy &&
          (state.error?.errorType === 'reconnecting' ||
            state.error?.errorType === 'max_reconnects_exceeded')
            ? null
            : state.error,
      })),

    clearPrices: () => set({ prices: {} }),
  };
});
