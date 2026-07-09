# Strategy Monitor Domain

## Description

The strategy-monitor domain owns the live strategy monitoring system in CandleSight. It watches real-time candle data against user-defined strategy rules, detects when entry/exit conditions are satisfied (pattern matches), manages multi-instrument watchers, handles candle boundary detection from streaming tick data, and delivers pattern match notifications to the frontend via Tauri events.

## Primary Stack

- **Backend**: Rust + Tauri 2 (candle polling/streaming, rule evaluation, signal emission)
- **Frontend**: React 19 + Zustand (watcher state management, match display, watcher lifecycle)
- **Persistence**: Zero sync for watcher configs (`strategy_watcher` table), localStorage for UI preferences (stale/expiry configs, pending matches)

## Owned Files

### Backend (Rust)

```
src-tauri/src/strategy/watcher.rs          # Single-instrument strategy watcher (legacy, still used for single watchers)
src-tauri/src/strategy/multi_watcher.rs    # Multi-instrument orchestrator (primary watcher type)
src-tauri/src/strategy/candle_source.rs    # CandleSource trait + OandaPollingSource + StreamingCandleSource
src-tauri/src/strategy/candle_boundary.rs  # CandleBoundaryDetector + CandleBoundaryService
src-tauri/src/strategy/pattern_match.rs    # PatternMatch, event types, status enums
src-tauri/src/strategy/pending_store.rs    # Global store for matches emitted while frontend may not be listening
src-tauri/src/strategy/mod.rs              # Module exports and public API surface
src-tauri/src/commands/watcher.rs          # All Tauri command handlers for watcher lifecycle
```

### Frontend (React/TypeScript)

```
src/components/watcher/**/*                # All watcher UI components
src/components/watcher/ActiveWatchersList.tsx
src/components/watcher/AddMonitorForm.tsx
src/components/watcher/InstrumentPriceCard.tsx
src/components/watcher/MatchList.tsx
src/components/watcher/MultiWatcherRow.tsx
src/components/watcher/WatcherControls.tsx
src/components/watcher/WatcherError.tsx
src/components/watcher/MatchCard.tsx
src/components/watcher/watcherHelpers.ts
src/components/watcher/utils.ts
src/components/watcher/index.ts
src/hooks/useStrategyWatcher.ts            # Core hook: Tauri event listeners + watcher lifecycle
src/stores/watcherStore.ts                 # Zustand store: matches, watchers, stale/expiry configs
src/StrategyWatcherApp.tsx                 # Watcher window entry point (auto-start, Zero integration)
```

## Shared Files (Coordination Required)

| File | Owner | What We Use |
|------|-------|-------------|
| `src-tauri/src/backtest/rules_engine.rs` | backtest-core | `RulesEngine`, `RulesSignal`, `PositionDirection`, `StrategyDefinition`, `SRZone` |
| `src-tauri/src/oanda/streaming.rs` | oanda-trading | `PriceStreamer` calls `CandleBoundaryService.on_price_tick()` for each tick |
| `src-tauri/src/oanda/endpoints.rs` | oanda-trading | `get_candles`, `get_open_positions`, `get_account`, `Granularity` |
| `src-tauri/src/oanda/client.rs` | oanda-trading | `OandaClient` cloned into watcher threads |
| `src-tauri/src/models/` | shared | `Candle` struct consumed by watcher |
| `src-tauri/src/notifications.rs` | notifications | `send_pattern_match_notification`, `NotificationClickedPayload` |
| `src-tauri/src/main.rs` | app-core | `AppState` fields: `watcher_handles`, `multi_watcher_handles`, `candle_boundary_service`; registered commands; window close cleanup |
| `shared/schema.ts` | shared | `strategy_watcher` table definition |
| `shared/src/lib.rs` | shared | `StrategyDefinition`, indicator/rule types |
| `src/queries.ts` | shared | `myPromotedStrategies`, `myActiveWatchers`, `mySRZones` |

## Key Dependencies

- `rust_decimal` - All financial values (prices, position sizes, stop losses)
- `chrono` - All timestamps and candle boundary calculations
- `tokio` - Async runtime, broadcast channels, mpsc for watcher commands
- `uuid` - Pattern match IDs
- `serde`/`serde_json` - Serialization for Tauri events and strategy parsing
- `tracing` - Structured logging throughout
- `zustand` (frontend) - Watcher state store
- `@tauri-apps/api` (frontend) - `invoke` for commands, `listen` for events
- `@rocicorp/zero` (frontend) - Watcher config persistence and strategy queries
