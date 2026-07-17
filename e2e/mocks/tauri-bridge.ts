/**
 * Tauri IPC Bridge for E2E tests
 *
 * This script is injected via page.addInitScript() before the app loads.
 * It sets up window.__E2E_TAURI_INVOKE__ which the @tauri-apps/api/core
 * mock reads from.
 *
 * Per-test overrides can be set via window.__E2E_TAURI_OVERRIDES__
 */

export function getTauriBridgeScript() {
  return `
    // In-memory stand-in for the wickd local store (~/.wickd/app.db, AGT-642).
    // Stateful so create/delete flows in the local window exercise real
    // re-renders. Tests can pre-seed via addInitScript before navigation.
    window.__E2E_LOCAL_STRATEGIES__ = window.__E2E_LOCAL_STRATEGIES__ || [];
    // Charting-domain datasets (AGT-646)
    window.__E2E_LOCAL_SR_ZONES__ = window.__E2E_LOCAL_SR_ZONES__ || [];
    window.__E2E_LOCAL_NOTES__ = window.__E2E_LOCAL_NOTES__ || [];
    window.__E2E_LOCAL_CHART_CONFIGS__ = window.__E2E_LOCAL_CHART_CONFIGS__ || {};
    // Trades/analysis domain datasets (AGT-647).
    window.__E2E_LOCAL_TRADES__ = window.__E2E_LOCAL_TRADES__ || [];
    window.__E2E_LOCAL_TRADE_SCORES__ = window.__E2E_LOCAL_TRADE_SCORES__ || [];

    // Strategies + backtests domain datasets (AGT-645).
    window.__E2E_LOCAL_BACKTESTS__ = window.__E2E_LOCAL_BACKTESTS__ || [];
    window.__E2E_LOCAL_JOBS__ = window.__E2E_LOCAL_JOBS__ || [];
    window.__E2E_LOCAL_PROMOTIONS__ = window.__E2E_LOCAL_PROMOTIONS__ || [];

    // Unified .rhai strategy store entries (~/.wickd/strategies/, AGT-651).
    // Read-only: entries may carry a 'source' field consumed by
    // store_read_strategy.
    window.__E2E_STORE_STRATEGIES__ = window.__E2E_STORE_STRATEGIES__ || [];

    // Watch-daemon client datasets (AGT-652): the wickd daemon's
    // client-visible stores as the app reads them.
    window.__E2E_DAEMON_STATUS__ = window.__E2E_DAEMON_STATUS__ || null;
    window.__E2E_DAEMON_QUEUE__ = window.__E2E_DAEMON_QUEUE__ || [];
    window.__E2E_DAEMON_PENDING__ = window.__E2E_DAEMON_PENDING__ || [];
    window.__E2E_HUB_STREAM__ = window.__E2E_HUB_STREAM__ || null;
    // Historical spread stats rows (~/.wickd/spreads.db, read-only).
    window.__E2E_SPREAD_STATS__ = window.__E2E_SPREAD_STATS__ || [];
    // Economic-calendar rows (~/.wickd/calendar, read-only).
    window.__E2E_ECONOMIC_CALENDAR__ = window.__E2E_ECONOMIC_CALENDAR__ || [];

    // Zero-removal sweep datasets (AGT-650).
    window.__E2E_LOCAL_LABELS__ = window.__E2E_LOCAL_LABELS__ || [];
    window.__E2E_LOCAL_TRADE_LABELS__ = window.__E2E_LOCAL_TRADE_LABELS__ || [];
    window.__E2E_LOCAL_STRATEGY_LABELS__ = window.__E2E_LOCAL_STRATEGY_LABELS__ || [];
    window.__E2E_LOCAL_STRATEGY_TRADES__ = window.__E2E_LOCAL_STRATEGY_TRADES__ || [];
    window.__E2E_LOCAL_STRATEGY_WATCHERS__ = window.__E2E_LOCAL_STRATEGY_WATCHERS__ || [];
    // Device credential row (null = onboarding). Default: a stored practice
    // credential so the credential gate goes straight to the app.
    if (window.__E2E_LOCAL_CREDENTIAL__ === undefined) {
      window.__E2E_LOCAL_CREDENTIAL__ = {
        id: 'e2e-device-001',
        device_id: 'e2e-device-001',
        practice_blob: 'e2e-ciphertext-practice',
        practice_account_id: '101-001-1234567-001',
        live_blob: null,
        live_account_id: null,
        created_at: 1700000000000,
        updated_at: 1700000000000,
      };
    }

    // Default command responses for the full credential gate chain
    const defaultResponses = {
      // Unified .rhai strategy store (AGT-651, read-only)
      'store_list_strategies': () =>
        [...window.__E2E_STORE_STRATEGIES__]
          .map(({ source, ...entry }) => entry)
          .sort((a, b) => a.name.localeCompare(b.name)),
      'store_read_strategy': (args) => {
        const entry = window.__E2E_STORE_STRATEGIES__.find((s) => s.name === args.name);
        if (!entry) throw new Error("no stored strategy '" + args.name + "'");
        return { ...entry, source: entry.source ?? '' };
      },
      // Local store commands (AGT-642)
      'local_store_path': '/e2e-home/.wickd/app.db',
      'local_list_strategies': () =>
        [...window.__E2E_LOCAL_STRATEGIES__].sort((a, b) => b.updated_at - a.updated_at),
      'local_get_strategy': (args) =>
        window.__E2E_LOCAL_STRATEGIES__.find((s) => s.id === args.id) ?? null,
      'local_save_strategy': (args) => {
        const idx = window.__E2E_LOCAL_STRATEGIES__.findIndex((s) => s.id === args.strategy.id);
        if (idx >= 0) window.__E2E_LOCAL_STRATEGIES__[idx] = args.strategy;
        else window.__E2E_LOCAL_STRATEGIES__.push(args.strategy);
        return null;
      },
      'local_delete_strategy': (args) => {
        const before = window.__E2E_LOCAL_STRATEGIES__.length;
        window.__E2E_LOCAL_STRATEGIES__ = window.__E2E_LOCAL_STRATEGIES__.filter((s) => s.id !== args.id);
        return window.__E2E_LOCAL_STRATEGIES__.length < before;
      },
      // Charting-domain local store commands (AGT-646)
      'local_list_sr_zones': (args) =>
        window.__E2E_LOCAL_SR_ZONES__
          .filter((z) => !args || args.instrument == null || z.instrument === args.instrument)
          .sort((a, b) => a.created_at - b.created_at),
      'local_save_sr_zone': (args) => {
        const idx = window.__E2E_LOCAL_SR_ZONES__.findIndex((z) => z.id === args.zone.id);
        if (idx >= 0) window.__E2E_LOCAL_SR_ZONES__[idx] = args.zone;
        else window.__E2E_LOCAL_SR_ZONES__.push(args.zone);
        return null;
      },
      'local_delete_sr_zone': (args) => {
        const before = window.__E2E_LOCAL_SR_ZONES__.length;
        window.__E2E_LOCAL_SR_ZONES__ = window.__E2E_LOCAL_SR_ZONES__.filter((z) => z.id !== args.id);
        return window.__E2E_LOCAL_SR_ZONES__.length < before;
      },
      'local_clear_sr_zones': (args) => {
        const before = window.__E2E_LOCAL_SR_ZONES__.length;
        window.__E2E_LOCAL_SR_ZONES__ = window.__E2E_LOCAL_SR_ZONES__.filter((z) => z.instrument !== args.instrument);
        return before - window.__E2E_LOCAL_SR_ZONES__.length;
      },
      'local_list_notes': (args) =>
        window.__E2E_LOCAL_NOTES__
          .filter((n) => {
            if (args && args.tradeId != null) return n.trade_id === args.tradeId;
            if (args && args.strategyId != null) return n.strategy_id === args.strategyId;
            return true;
          })
          .sort((a, b) => b.created_at - a.created_at),
      'local_save_note': (args) => {
        const idx = window.__E2E_LOCAL_NOTES__.findIndex((n) => n.id === args.note.id);
        if (idx >= 0) window.__E2E_LOCAL_NOTES__[idx] = args.note;
        else window.__E2E_LOCAL_NOTES__.push(args.note);
        return null;
      },
      'local_delete_note': (args) => {
        const before = window.__E2E_LOCAL_NOTES__.length;
        window.__E2E_LOCAL_NOTES__ = window.__E2E_LOCAL_NOTES__.filter((n) => n.id !== args.id);
        return window.__E2E_LOCAL_NOTES__.length < before;
      },
      'local_get_chart_config': (args) =>
        window.__E2E_LOCAL_CHART_CONFIGS__[args.instrument] ?? null,
      'local_set_chart_config': (args) => {
        window.__E2E_LOCAL_CHART_CONFIGS__[args.instrument] = args.indicators;
        return null;
      },
      // Trades/analysis local-store commands (AGT-647)
      'local_list_trades': () =>
        [...window.__E2E_LOCAL_TRADES__].sort((a, b) => b.open_time - a.open_time),
      'local_list_closed_trades_by_instrument': (args) =>
        window.__E2E_LOCAL_TRADES__
          .filter((t) => t.instrument === args.instrument && t.state === 'CLOSED')
          .sort((a, b) => b.open_time - a.open_time),
      'local_list_trade_scores': () =>
        [...window.__E2E_LOCAL_TRADE_SCORES__].sort((a, b) => b.created_at - a.created_at),
      'local_get_trade_score_by_trade': (args) =>
        window.__E2E_LOCAL_TRADE_SCORES__.find((s) => s.trade_id === args.tradeId) ?? null,
      'local_save_trade_score': (args) => {
        const idx = window.__E2E_LOCAL_TRADE_SCORES__.findIndex(
          (s) => s.trade_id === args.score.trade_id
        );
        if (idx >= 0) window.__E2E_LOCAL_TRADE_SCORES__[idx] = args.score;
        else window.__E2E_LOCAL_TRADE_SCORES__.push(args.score);
        return null;
      },
      // Strategies + backtests domain commands (AGT-645)
      'local_list_backtests': (args) =>
        window.__E2E_LOCAL_BACKTESTS__
          .filter((b) => b.strategy_id === args.strategyId)
          .sort((a, b) => a.created_at - b.created_at),
      'local_save_backtest': (args) => {
        const idx = window.__E2E_LOCAL_BACKTESTS__.findIndex((b) => b.id === args.backtest.id);
        if (idx >= 0) window.__E2E_LOCAL_BACKTESTS__[idx] = args.backtest;
        else window.__E2E_LOCAL_BACKTESTS__.push(args.backtest);
        return null;
      },
      'local_delete_backtests_for_strategy': (args) => {
        const before = window.__E2E_LOCAL_BACKTESTS__.length;
        window.__E2E_LOCAL_BACKTESTS__ = window.__E2E_LOCAL_BACKTESTS__.filter(
          (b) => b.strategy_id !== args.strategyId
        );
        return before - window.__E2E_LOCAL_BACKTESTS__.length;
      },
      'local_list_backtest_jobs': (args) =>
        window.__E2E_LOCAL_JOBS__
          .filter((j) => j.strategy_id === args.strategyId)
          .sort((a, b) => b.updated_at - a.updated_at),
      'local_get_backtest_job': (args) =>
        window.__E2E_LOCAL_JOBS__.find((j) => j.id === args.id) ?? null,
      'local_save_backtest_job': (args) => {
        const idx = window.__E2E_LOCAL_JOBS__.findIndex((j) => j.id === args.job.id);
        if (idx >= 0) window.__E2E_LOCAL_JOBS__[idx] = args.job;
        else window.__E2E_LOCAL_JOBS__.push(args.job);
        return null;
      },
      'local_record_promotion': (args) => {
        window.__E2E_LOCAL_PROMOTIONS__.push(args.audit);
        return null;
      },
      // Zero-removal sweep commands (AGT-650)
      'local_list_labels': () =>
        [...window.__E2E_LOCAL_LABELS__].sort((a, b) => a.name.localeCompare(b.name)),
      'local_save_label': (args) => {
        const idx = window.__E2E_LOCAL_LABELS__.findIndex((l) => l.id === args.label.id);
        if (idx >= 0) window.__E2E_LOCAL_LABELS__[idx] = args.label;
        else window.__E2E_LOCAL_LABELS__.push(args.label);
        return null;
      },
      'local_list_trade_labels': (args) =>
        window.__E2E_LOCAL_TRADE_LABELS__
          .filter((tl) => !args || args.tradeId == null || tl.trade_id === args.tradeId)
          .sort((a, b) => a.created_at - b.created_at),
      'local_add_trade_label': (args) => {
        window.__E2E_LOCAL_TRADE_LABELS__.push(args.tradeLabel);
        return null;
      },
      'local_delete_trade_label': (args) => {
        const before = window.__E2E_LOCAL_TRADE_LABELS__.length;
        window.__E2E_LOCAL_TRADE_LABELS__ = window.__E2E_LOCAL_TRADE_LABELS__.filter((tl) => tl.id !== args.id);
        return window.__E2E_LOCAL_TRADE_LABELS__.length < before;
      },
      'local_list_strategy_labels': (args) =>
        window.__E2E_LOCAL_STRATEGY_LABELS__
          .filter((sl) => !args || args.strategyId == null || sl.strategy_id === args.strategyId)
          .sort((a, b) => a.created_at - b.created_at),
      'local_add_strategy_label': (args) => {
        window.__E2E_LOCAL_STRATEGY_LABELS__.push(args.strategyLabel);
        return null;
      },
      'local_delete_strategy_label': (args) => {
        const before = window.__E2E_LOCAL_STRATEGY_LABELS__.length;
        window.__E2E_LOCAL_STRATEGY_LABELS__ = window.__E2E_LOCAL_STRATEGY_LABELS__.filter((sl) => sl.id !== args.id);
        return window.__E2E_LOCAL_STRATEGY_LABELS__.length < before;
      },
      'local_list_strategy_trades': (args) =>
        window.__E2E_LOCAL_STRATEGY_TRADES__
          .filter((st) => !args || args.strategyId == null || st.strategy_id === args.strategyId)
          .sort((a, b) => b.executed_at - a.executed_at),
      'local_save_strategy_trade': (args) => {
        const idx = window.__E2E_LOCAL_STRATEGY_TRADES__.findIndex((st) => st.id === args.strategyTrade.id);
        if (idx >= 0) window.__E2E_LOCAL_STRATEGY_TRADES__[idx] = args.strategyTrade;
        else window.__E2E_LOCAL_STRATEGY_TRADES__.push(args.strategyTrade);
        return null;
      },
      'local_list_strategy_watchers': () =>
        [...window.__E2E_LOCAL_STRATEGY_WATCHERS__].sort((a, b) => b.updated_at - a.updated_at),
      'local_save_strategy_watcher': (args) => {
        const idx = window.__E2E_LOCAL_STRATEGY_WATCHERS__.findIndex((w) => w.id === args.watcher.id);
        if (idx >= 0) window.__E2E_LOCAL_STRATEGY_WATCHERS__[idx] = args.watcher;
        else window.__E2E_LOCAL_STRATEGY_WATCHERS__.push(args.watcher);
        return null;
      },
      'local_delete_strategy_watcher': (args) => {
        const before = window.__E2E_LOCAL_STRATEGY_WATCHERS__.length;
        window.__E2E_LOCAL_STRATEGY_WATCHERS__ = window.__E2E_LOCAL_STRATEGY_WATCHERS__.filter((w) => w.id !== args.id);
        return window.__E2E_LOCAL_STRATEGY_WATCHERS__.length < before;
      },
      'local_get_credential': () => window.__E2E_LOCAL_CREDENTIAL__ ?? null,
      'local_save_credential': (args) => {
        window.__E2E_LOCAL_CREDENTIAL__ = args.credential;
        return null;
      },
      'local_delete_credentials': () => {
        const had = window.__E2E_LOCAL_CREDENTIAL__ ? 1 : 0;
        window.__E2E_LOCAL_CREDENTIAL__ = null;
        return had;
      },
      'get_device_id': 'e2e-device-001',
      'is_vault_unlocked': true,
      'has_practice_credentials': true,
      'has_live_credentials': false,
      'check_unlock_rate_limit': null,
      'get_oanda_credentials': {
        apiKeyPreview: 'abc...xyz',
        accountId: '101-001-1234567-001',
        accountAlias: 'E2E Practice',
        environment: 'practice',
        isConfigured: true,
      },
      'sync_trades': null,
      'set_desktop_notifications_enabled': null,
      'test_notification': true,
      'set_active_environment': null,
      'is_chat_enabled': false,
      'open_chart_window': null,
      'get_candles': [],
      'get_indicator_data': [],
      'open_startup_windows': null,
      'open_backtest_window': null,
      'fetch_instruments': [],
      // AGT-652: the app as a client of the wickd watch daemon. Seed via
      // window.__E2E_DAEMON_STATUS__ / __E2E_DAEMON_QUEUE__ / __E2E_DAEMON_PENDING__ /
      // __E2E_HUB_STREAM__ (addInitScript or the appPage.setLocalDataset fixture).
      'daemon_status': () =>
        window.__E2E_DAEMON_STATUS__ ?? {
          watchers: [],
          hub_socket_present: false,
          pending_count: window.__E2E_DAEMON_PENDING__?.length ?? 0,
          queue_len: window.__E2E_DAEMON_QUEUE__?.length ?? 0,
        },
      // Market-awareness feed reader (FeedOverlay). Seed per-test via
      // appPage.mockTauriCommand('feed_list', [...]).
      'feed_list': [],
      'daemon_queue_list': (args) => {
        const queue = [...(window.__E2E_DAEMON_QUEUE__ ?? [])].reverse();
        return queue.slice(0, args?.limit ?? 100);
      },
      'daemon_pending_list': () =>
        [...(window.__E2E_DAEMON_PENDING__ ?? [])]
          .filter((sig) => sig.status === 'pending')
          .reverse(),
      'hub_stream_status': () =>
        window.__E2E_HUB_STREAM__ ?? { mode: 'idle', observed: [], direct: [], last_line_ms: null },
      // Historical spread stats (~/.wickd/spreads.db, read-only). Empty by
      // default = the purple "no history" spread-bar fallback.
      'get_spread_stats': () => window.__E2E_SPREAD_STATS__ ?? [],
      // Economic-calendar store (~/.wickd/calendar, read-only). Empty by
      // default = the section's "no upcoming events" state.
      'get_economic_calendar': () => window.__E2E_ECONOMIC_CALENDAR__ ?? [],
    };

    // Set up the invoke handler
    window.__E2E_TAURI_INVOKE__ = function(cmd, args) {
      // Check per-test overrides first (set via mockTauriCommand)
      if (window.__E2E_TAURI_OVERRIDES__ && cmd in window.__E2E_TAURI_OVERRIDES__) {
        const override = window.__E2E_TAURI_OVERRIDES__[cmd];
        if (override instanceof Error) throw override;
        return typeof override === 'function' ? override(args) : override;
      }

      if (cmd in defaultResponses) {
        const response = defaultResponses[cmd];
        if (response instanceof Error) throw response;
        return typeof response === 'function' ? response(args) : response;
      }

      console.warn('[e2e-tauri-bridge] Unhandled command:', cmd, args);
      return null;
    };

    // Per-test override storage
    window.__E2E_TAURI_OVERRIDES__ = window.__E2E_TAURI_OVERRIDES__ || {};
  `;
}
