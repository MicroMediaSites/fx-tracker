import { create } from 'zustand';
import type { IndicatorType } from '../types/strategy';
import { INDICATOR_DEFAULTS } from '../types/strategy';
import type { ChartIndicatorConfig } from '../components/charts/chartTypes';
import { generateIndicatorId } from '../components/charts/indicatorHelpers';
import { getChartConfig, setChartConfig } from '../lib/localStore';

// Legacy per-instrument localStorage keys (pre-AGT-646). Kept only for a
// one-time import into the local SQLite store.
function getLegacyStorageKey(instrument: string): string {
  return `candlesight-chart-indicators-${instrument}`;
}

/**
 * Load the persisted indicator config for an instrument from the local store
 * (~/.wickd/app.db, AGT-646). Falls back to — and imports — the legacy
 * localStorage value the first time an instrument has no row in the store.
 */
async function loadFromStorage(instrument: string): Promise<ChartIndicatorConfig[]> {
  try {
    const raw = await getChartConfig(instrument);
    if (raw !== null) {
      const parsed = JSON.parse(raw);
      return Array.isArray(parsed) ? parsed : [];
    }
    // One-time import of the pre-AGT-646 localStorage persistence.
    const legacy = localStorage.getItem(getLegacyStorageKey(instrument));
    if (legacy) {
      const parsed = JSON.parse(legacy);
      if (Array.isArray(parsed) && parsed.length > 0) {
        saveToStorage(instrument, parsed);
        return parsed;
      }
    }
    return [];
  } catch {
    return [];
  }
}

function saveToStorage(instrument: string, indicators: ChartIndicatorConfig[]): void {
  // Fire-and-forget: persistence failure must never break chart interaction.
  setChartConfig(instrument, JSON.stringify(indicators)).catch((err) => {
    console.warn('[chartIndicatorStore] Failed to persist chart config:', err);
  });
}

interface ChartIndicatorState {
  instrument: string;
  indicators: ChartIndicatorConfig[];

  /** Initialize store: use seed if provided, otherwise load from per-instrument persistence */
  initWithSeed: (instrument: string, seed: ChartIndicatorConfig[] | null) => void;
  /** Save current indicators and load indicators for a new instrument */
  switchInstrument: (newInstrument: string) => void;
  /** Bulk-replace indicators */
  setIndicators: (indicators: ChartIndicatorConfig[]) => void;

  addIndicator: (type: IndicatorType, params?: Record<string, number>, colors?: Record<string, string>) => void;
  updateIndicator: (id: string, params: Record<string, number>, colors?: Record<string, string>) => void;
  removeIndicator: (id: string) => void;
  clearAll: () => void;
}

export const useChartIndicatorStore = create<ChartIndicatorState>()((set, get) => ({
  instrument: '',
  indicators: [],

  initWithSeed: (instrument, seed) => {
    if (seed && seed.length > 0) {
      // Caller provided indicators (e.g., watcher passed strategy indicators).
      // Use for this session only — don't overwrite per-instrument persistence.
      set({ instrument, indicators: seed });
    } else {
      // No seed — load from per-instrument persistence (async: the local
      // store answers over IPC). Guard against the instrument changing while
      // the load is in flight.
      set({ instrument, indicators: [] });
      loadFromStorage(instrument).then((indicators) => {
        if (get().instrument === instrument && indicators.length > 0) {
          set({ indicators });
        }
      });
    }
  },

  switchInstrument: (newInstrument) => {
    const { instrument: oldInstrument, indicators: oldIndicators } = get();
    if (oldInstrument) {
      saveToStorage(oldInstrument, oldIndicators);
    }
    // Carry over the current set immediately; replace with the new
    // instrument's saved config when the (async) load resolves.
    set({ instrument: newInstrument, indicators: oldIndicators });
    loadFromStorage(newInstrument).then((savedIndicators) => {
      if (get().instrument === newInstrument && savedIndicators.length > 0) {
        set({ indicators: savedIndicators });
      }
    });
  },

  setIndicators: (indicators) => {
    const { instrument } = get();
    set({ indicators });
    if (instrument) {
      saveToStorage(instrument, indicators);
    }
  },

  addIndicator: (type, params, colors) =>
    set((state) => {
      const id = generateIndicatorId(type, state.indicators);
      const defaultParams = INDICATOR_DEFAULTS[type] ?? {};
      const hasParams = params && Object.keys(params).length > 0;
      const newIndicator: ChartIndicatorConfig = {
        id,
        type,
        params: hasParams ? params : { ...defaultParams },
        colors,
      };
      const newIndicators = [newIndicator, ...state.indicators];
      saveToStorage(state.instrument, newIndicators);
      return { indicators: newIndicators };
    }),

  updateIndicator: (id, params, colors) =>
    set((state) => {
      const newIndicators = state.indicators.map((ind) =>
        ind.id === id ? { ...ind, params, colors } : ind
      );
      saveToStorage(state.instrument, newIndicators);
      return { indicators: newIndicators };
    }),

  removeIndicator: (id) =>
    set((state) => {
      const newIndicators = state.indicators.filter((ind) => ind.id !== id);
      saveToStorage(state.instrument, newIndicators);
      return { indicators: newIndicators };
    }),

  clearAll: () =>
    set((state) => {
      saveToStorage(state.instrument, []);
      return { indicators: [] };
    }),
}));
