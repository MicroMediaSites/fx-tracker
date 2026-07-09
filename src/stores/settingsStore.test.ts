import { describe, it, expect, beforeEach } from 'vitest';
import { useSettingsStore } from './settingsStore';

describe('settingsStore', () => {
  beforeEach(() => {
    // Reset store to initial state before each test
    useSettingsStore.setState({
      _hasHydrated: false,
      dataSource: 'demo',
      devUsePracticeUrlForLive: false,
      startupWindows: ['watcher'],
      aiModel: 'opus',
      mySymbols: ['EUR_USD', 'GBP_USD', 'USD_JPY', 'AUD_USD', 'USD_CAD'],
      desktopNotifications: false,
      completedTours: {},
    });
  });

  describe('initial state', () => {
    it('defaults to demo data source', () => {
      expect(useSettingsStore.getState().dataSource).toBe('demo');
    });

    it('defaults to opus AI model', () => {
      expect(useSettingsStore.getState().aiModel).toBe('opus');
    });

    it('defaults to the watcher startup window (AGT-652)', () => {
      expect(useSettingsStore.getState().startupWindows).toEqual(['watcher']);
    });

    it('defaults to desktop notifications off', () => {
      expect(useSettingsStore.getState().desktopNotifications).toBe(false);
    });

    it('defaults to no completed tours', () => {
      expect(useSettingsStore.getState().completedTours).toEqual({});
    });
  });

  describe('setDataSource', () => {
    it('changes data source to live', () => {
      const { setDataSource } = useSettingsStore.getState();
      setDataSource('live');

      expect(useSettingsStore.getState().dataSource).toBe('live');
    });

    it('changes data source back to demo', () => {
      const { setDataSource } = useSettingsStore.getState();
      setDataSource('live');
      setDataSource('demo');

      expect(useSettingsStore.getState().dataSource).toBe('demo');
    });
  });

  describe('toggleStartupWindow', () => {
    it('adds a window to startup windows', () => {
      const { toggleStartupWindow } = useSettingsStore.getState();
      toggleStartupWindow('charting');

      expect(useSettingsStore.getState().startupWindows).toContain('charting');
      expect(useSettingsStore.getState().startupWindows).toContain('watcher');
    });

    it('removes a window from startup windows', () => {
      const { toggleStartupWindow } = useSettingsStore.getState();
      // First add charting
      toggleStartupWindow('charting');
      expect(useSettingsStore.getState().startupWindows).toContain('charting');

      // Now remove it
      toggleStartupWindow('charting');
      expect(useSettingsStore.getState().startupWindows).not.toContain('charting');
    });

    it('prevents removing the last window', () => {
      const { toggleStartupWindow } = useSettingsStore.getState();
      // Try to remove the only window
      toggleStartupWindow('watcher');

      // Should still have at least one window
      expect(useSettingsStore.getState().startupWindows).toEqual(['watcher']);
    });
  });

  describe('setStartupWindows', () => {
    it('sets startup windows directly', () => {
      const { setStartupWindows } = useSettingsStore.getState();
      setStartupWindows(['charting', 'backtesting', 'watcher']);

      expect(useSettingsStore.getState().startupWindows).toEqual([
        'charting',
        'backtesting',
        'watcher',
      ]);
    });
  });

  describe('symbol management', () => {
    it('adds a new symbol', () => {
      const { addSymbol } = useSettingsStore.getState();
      addSymbol('NZD_USD');

      expect(useSettingsStore.getState().mySymbols).toContain('NZD_USD');
    });

    it('does not add duplicate symbols', () => {
      const { addSymbol } = useSettingsStore.getState();
      const initialLength = useSettingsStore.getState().mySymbols.length;
      addSymbol('EUR_USD'); // Already exists

      expect(useSettingsStore.getState().mySymbols.length).toBe(initialLength);
    });

    it('removes a symbol', () => {
      const { removeSymbol } = useSettingsStore.getState();
      removeSymbol('EUR_USD');

      expect(useSettingsStore.getState().mySymbols).not.toContain('EUR_USD');
    });

    it('sets symbols directly', () => {
      const { setSymbols } = useSettingsStore.getState();
      setSymbols(['XAU_USD', 'XAG_USD']);

      expect(useSettingsStore.getState().mySymbols).toEqual(['XAU_USD', 'XAG_USD']);
    });
  });

  describe('setAIModel', () => {
    it('changes AI model', () => {
      const { setAIModel } = useSettingsStore.getState();
      setAIModel('sonnet');

      expect(useSettingsStore.getState().aiModel).toBe('sonnet');
    });
  });

  describe('setDesktopNotifications', () => {
    it('enables desktop notifications', () => {
      const { setDesktopNotifications } = useSettingsStore.getState();
      setDesktopNotifications(true);

      expect(useSettingsStore.getState().desktopNotifications).toBe(true);
    });

    it('disables desktop notifications', () => {
      const { setDesktopNotifications } = useSettingsStore.getState();
      setDesktopNotifications(true);
      setDesktopNotifications(false);

      expect(useSettingsStore.getState().desktopNotifications).toBe(false);
    });
  });

  describe('setTourCompleted', () => {
    it('marks a specific window tour as completed', () => {
      const { setTourCompleted } = useSettingsStore.getState();
      setTourCompleted('account');

      expect(useSettingsStore.getState().completedTours).toEqual({ account: true });
    });

    it('can mark multiple window tours as completed', () => {
      const { setTourCompleted } = useSettingsStore.getState();
      setTourCompleted('account');
      setTourCompleted('chart');

      expect(useSettingsStore.getState().completedTours).toEqual({ account: true, chart: true });
    });
  });

  describe('hydration', () => {
    it('tracks hydration state', () => {
      expect(useSettingsStore.getState()._hasHydrated).toBe(false);

      const { setHasHydrated } = useSettingsStore.getState();
      setHasHydrated(true);

      expect(useSettingsStore.getState()._hasHydrated).toBe(true);
    });
  });
});
