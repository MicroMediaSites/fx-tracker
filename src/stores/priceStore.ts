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
  errorType: 'parse_error' | 'connection_lost' | 'stream_ended';
  message: string;
}

interface PriceState {
  prices: Record<string, PriceUpdate>;
  streaming: boolean;
  error: StreamError | null;

  updatePrice: (price: PriceUpdate) => void;
  setStreaming: (streaming: boolean) => void;
  setError: (error: StreamError | null) => void;
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

    updatePrice: (price) => {
      // Buffer the update instead of immediately calling set()
      pendingPriceUpdates[price.instrument] = price;
      schedulePriceFlush();
    },

    setStreaming: (streaming) => set({ streaming }),

    setError: (error) => set({ error }),

    clearPrices: () => set({ prices: {} }),
  };
});
