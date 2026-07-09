import { create } from 'zustand';
import { persist } from 'zustand/middleware';

export type AIModel = 'haiku' | 'sonnet' | 'opus';
// AGT-652: surface reduced — account/ticket/tradeanalysis windows are gone.
export type StartupWindow = 'charting' | 'backtesting' | 'watcher';

/**
 * Data source for trading operations:
 * - 'demo': OANDA practice account (real market data, paper trading)
 * - 'live': OANDA live account (real money)
 */
export type DataSource = 'demo' | 'live';

interface SettingsState {
  // Hydration status
  _hasHydrated: boolean;
  setHasHydrated: (state: boolean) => void;

  // Data source: demo or live
  dataSource: DataSource;

  // Dev-only: use practice OANDA URL when in "live" mode (for testing with demo account)
  devUsePracticeUrlForLive: boolean;

  // Startup windows to open
  startupWindows: StartupWindow[];

  // AI model: 'haiku' (fastest), 'sonnet' (balanced), 'opus' (most capable)
  aiModel: AIModel;

  // User's custom symbol list
  mySymbols: string[];

  // Desktop notifications for pattern matches
  desktopNotifications: boolean;

  // Per-window tour completion tracking
  completedTours: Record<string, boolean>;

  // Actions
  setDataSource: (source: DataSource) => void;
  setDevUsePracticeUrlForLive: (value: boolean) => void;
  setStartupWindows: (windows: StartupWindow[]) => void;
  toggleStartupWindow: (window: StartupWindow) => void;
  setAIModel: (model: AIModel) => void;
  addSymbol: (symbol: string) => void;
  removeSymbol: (symbol: string) => void;
  setSymbols: (symbols: string[]) => void;
  setDesktopNotifications: (enabled: boolean) => void;
  setTourCompleted: (windowType: string) => void;
}

// Default symbols to start with
const DEFAULT_SYMBOLS = ['EUR_USD', 'GBP_USD', 'USD_JPY', 'AUD_USD', 'USD_CAD'];

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      _hasHydrated: false,
      setHasHydrated: (state) => set({ _hasHydrated: state }),

      dataSource: 'demo', // Default to demo for safety
      devUsePracticeUrlForLive: false,
      startupWindows: ['watcher'],
      aiModel: 'opus', // Hardcoded to Opus for best quality user-facing analysis
      mySymbols: DEFAULT_SYMBOLS,
      desktopNotifications: false, // Default to off - user must opt in
      completedTours: {}, // Per-window tour completion tracking

      setDataSource: (source) => set({ dataSource: source }),
      setDevUsePracticeUrlForLive: (value) => set({ devUsePracticeUrlForLive: value }),

      setStartupWindows: (windows) => set({ startupWindows: windows }),

      toggleStartupWindow: (window) =>
        set((state) => {
          const isRemoving = state.startupWindows.includes(window);
          const newWindows = isRemoving
            ? state.startupWindows.filter((w) => w !== window)
            : [...state.startupWindows, window];

          // Ensure at least one window is always selected
          if (newWindows.length === 0) {
            return state; // Don't allow removing the last window
          }

          return { startupWindows: newWindows };
        }),

      setAIModel: (model) => set({ aiModel: model }),

      addSymbol: (symbol) =>
        set((state) => ({
          mySymbols: state.mySymbols.includes(symbol)
            ? state.mySymbols
            : [...state.mySymbols, symbol],
        })),

      removeSymbol: (symbol) =>
        set((state) => ({
          mySymbols: state.mySymbols.filter((s) => s !== symbol),
        })),

      setSymbols: (symbols) => set({ mySymbols: symbols }),

      setDesktopNotifications: (enabled) => set({ desktopNotifications: enabled }),

      setTourCompleted: (windowType) => set((state) => ({
        completedTours: { ...state.completedTours, [windowType]: true },
      })),
    }),
    {
      name: 'candlesight-settings',
      version: 2,
      // Exclude aiModel from persistence - it's hardcoded to 'opus' and not user-configurable
      partialize: (state) => ({
        dataSource: state.dataSource,
        devUsePracticeUrlForLive: state.devUsePracticeUrlForLive,
        startupWindows: state.startupWindows,
        mySymbols: state.mySymbols,
        desktopNotifications: state.desktopNotifications,
        completedTours: state.completedTours,
      }),
      migrate: (persistedState: unknown, version: number) => {
        const state = { ...(persistedState as Record<string, unknown>) };
        if (version < 2) {
          // Migrate hasCompletedTour (boolean) → completedTours (per-window record)
          // Only migrate if completedTours isn't already set (e.g., from E2E fixtures)
          if (!state.completedTours) {
            state.completedTours = state.hasCompletedTour ? { account: true } : {};
          }
          delete state.hasCompletedTour;
        }
        return state as unknown as SettingsState;
      },
      onRehydrateStorage: () => (state) => {
        // Migrate from old localStorage keys if needed
        if (typeof window !== 'undefined') {
          const newKey = 'candlesight-settings';
          const oldKeys = ['trade-lab-settings', 'fx-tracker-settings'];

          // If new key is empty, try migrating from old keys
          if (!localStorage.getItem(newKey)) {
            for (const oldKey of oldKeys) {
              const oldData = localStorage.getItem(oldKey);
              if (oldData) {
                localStorage.setItem(newKey, oldData);
                break;
              }
            }
          }

          // Clean up old keys after migration
          if (localStorage.getItem(newKey)) {
            for (const oldKey of oldKeys) {
              localStorage.removeItem(oldKey);
            }
          }
        }
        state?.setHasHydrated(true);
      },
    }
  )
);
