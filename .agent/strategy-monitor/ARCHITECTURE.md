# Strategy Monitor Architecture

## End-to-End Flow

The watcher system turns a user's strategy definition into live pattern match signals. The flow is:

1. **User promotes a strategy** in the main app (sets `is_promoted = true` in Zero)
2. **StrategyWatcherApp** reads promoted strategies and saved watcher configs from Zero on mount
3. **Auto-start** calls `start_multi_watcher` Tauri command for each saved `strategy_watcher` row
4. **Rust backend** creates a `MultiInstrumentWatcher` in a dedicated OS thread with its own tokio runtime
5. **Watcher polls or streams** for new candles, evaluates rules via `RulesEngine`, emits `PatternMatch` events
6. **Frontend** receives events via Tauri `listen()`, stores in Zustand, renders `MatchCard` components
7. **User acts** on matches (dismiss, execute via OANDA, or let them expire)

```
Zero DB (strategy_watcher rows)
         |
         v
StrategyWatcherApp (auto-start on mount)
         |
         v  invoke('start_multi_watcher', ...)
Tauri Command Handler (commands/watcher.rs)
         |
         v  std::thread::spawn + tokio runtime
MultiInstrumentWatcher (multi_watcher.rs)
    |-- InstrumentState per instrument
    |   |-- CandleSource (streaming or polling)
    |   |-- RulesEngine (one per instrument)
    |   |-- PendingSignal tracking
    |
    v  on new candle
RulesEngine.on_candle_live(candle, position_direction)
         |
         v  RulesSignal::Entry / Exit / Hold
PatternMatch created + emitted via app_handle.emit("pattern-matched")
         |
         v  also stored in pending_store for offline retrieval
Frontend listen("pattern-matched") -> Zustand store -> MatchCard UI
```

## Two Watcher Types

### StrategyWatcher (Single-Instrument, Legacy)

Defined in `watcher.rs`. Watches one instrument with one strategy. Created via `start_strategy_watcher` or `start_strategy_watcher_auto` commands. Still functional but the multi-watcher is preferred because:
- It calculates position size per signal (fetches OANDA account balance)
- It was the original implementation before multi-instrument support

### MultiInstrumentWatcher (Multi-Instrument, Primary)

Defined in `multi_watcher.rs`. Watches N instruments with one strategy + one timeframe. Each instrument gets its own `InstrumentState` containing:
- Its own `CandleSource` instance
- Its own `RulesEngine` instance (separate indicator state per instrument)
- Its own pending signal tracking
- Its own signal filter (per-instrument, supports `{"long":"all","short":"entry"}` JSON format)

The watcher ID format is `{strategy_id}_{timeframe}` (e.g., `abc123_H4`).

Dynamic instrument management happens via an `mpsc` command channel (`WatcherCommand`):
- `AddInstrument` - adds with warmup
- `RemoveInstrument` - removes immediately
- `UpdateSRZones` - updates zone-based triggers
- `UpdateSignalFilter` - changes per-instrument signal filter
- `Stop` - terminates the watcher loop

The `MultiWatcherHandle` is a `Clone`-able handle stored in `AppState.multi_watcher_handles`. It exposes fire-and-forget methods that send commands via the `mpsc` channel and optimistically update a local instruments list (`Arc<RwLock<Vec<String>>>`).

## Candle Source Abstraction

The `CandleSource` trait (`candle_source.rs`) abstracts how candle data arrives:

```rust
pub trait CandleSource: Send + Sync {
    fn get_latest_candle(&self) -> Pin<Box<dyn Future<Output = Result<Option<Candle>>> + Send + '_>>;
    fn get_candles(&self, count: u32) -> Pin<Box<dyn Future<Output = Result<Vec<Candle>>> + Send + '_>>;
    fn timeframe(&self) -> Granularity;
    fn instrument(&self) -> &str;
    fn poll_interval(&self) -> Duration;
}
```

### OandaPollingSource

- Polls the OANDA REST API (`get_candles`) on a timer
- Tracks `last_candle_time` via `Mutex<Option<DateTime<Utc>>>` to detect new candles
- Only returns complete candles (filters `c.complete == true`)
- Poll interval scales with timeframe: M1=10s, H1=5min, H4=10min, D=30min

### StreamingCandleSource

- Subscribes to `CandleBoundaryService` for candle close events
- When a close event arrives, fetches the official OHLC from the REST API (ensures data accuracy)
- Poll interval is 1 second (just checking the broadcast channel)
- Sub-second candle detection latency vs up to 10 minutes for polling
- Falls back gracefully if boundary service channel closes (logs warning, returns None)
- Handles `Lagged` broadcast errors by trying to receive the latest event

## Candle Boundary Detection

The boundary detection system (`candle_boundary.rs`) determines when candle periods close by observing incoming tick timestamps from the OANDA price stream.

### CandleBoundaryDetector

A stateful detector that maps `(instrument, Granularity)` pairs to their current candle start time. On each tick:

1. Calculates `next_candle_start` from the stored current start
2. If `tick_time >= next_candle_start`, the candle has closed
3. Emits a `CandleCloseEvent` with the closed candle's start and end times
4. Updates the stored start to `current_candle_start(tick_time, timeframe)` (handles gaps where multiple candles may have passed)

**OANDA Candle Alignment**: All calculations use `dailyAlignment=3` with `alignmentTimezone=UTC`:
- H4 candles: 03:00, 07:00, 11:00, 15:00, 19:00, 23:00 UTC
- H6 candles: 03:00, 09:00, 15:00, 21:00 UTC
- Daily candles: close at 21:00 UTC (NY close, 5pm ET)
- Hours 0-2 UTC wrap to the previous day's candle period for H4+ timeframes

**Forex Market Hours**: Skips processing when markets are closed (Friday 21:00 UTC to Sunday 21:00 UTC) via `is_forex_market_closed()`.

### CandleBoundaryService

A shared service (`Arc<Self>`) that wraps the detector with:
- `RwLock<CandleBoundaryDetector>` for thread-safe tick processing
- `RwLock<HashMap<(String, Granularity), broadcast::Sender<CandleCloseEvent>>>` for pub/sub
- Called by `PriceStreamer` (oanda-trading domain) via `on_price_tick(instrument, tick_time)`
- Subscribers (StreamingCandleSources) receive events via `broadcast::Receiver`

The service is created once in `main.rs`, stored in `AppState.candle_boundary_service`, and injected into both the `PriceStreamer` and each `MultiInstrumentWatcher`.

## Multi-Timeframe in Live Monitoring

The `MultiInstrumentWatcher` supports strategies with higher-timeframe indicators:

1. **At startup**: `extract_htf_timeframes()` identifies all non-primary timeframes from the strategy definition.
2. **Warmup**: For each HTF timeframe, fetches warmup candles via `get_candles_paginated()`, creates an `MtfCandleStore`, and calls `rules_engine.set_mtf_candle_store()`.
3. **Refresh**: Every 5 minutes (rate-limited), `refresh_htf_candles()` fetches the latest HTF candles and appends newly completed ones via `rules_engine.append_htf_candle()`.
4. **Advancement**: On each primary-timeframe candle, the rules engine advances HTF indicator engines to keep them in sync.

The 5-minute refresh rate is a compromise between API call frequency and HTF candle staleness. For daily indicators, this is more than sufficient. For H4 indicators, there may be up to a 5-minute delay in detecting a new H4 candle close.

## RulesEngine Usage: Live vs Backtest

The watcher reuses the same `RulesEngine` from the backtest domain but calls it in "live mode":

- **`warmup_candle(candle)`** - Feeds historical candles through indicators without evaluating rules. This primes indicator state (e.g., EMA values, ATR buffers) without generating spurious signals from historical data.

- **`on_candle_live(candle, position_direction)`** - Evaluates entry/exit rules against the current candle. Unlike backtest mode, it takes the actual OANDA position direction as input (checked via `get_open_positions`), rather than maintaining internal position state.

The key difference: in backtesting, `RulesEngine` tracks positions internally. In live mode, the watcher queries OANDA for the truth about whether a position is open, avoiding state drift between the app and the broker.

Each instrument in a `MultiInstrumentWatcher` gets its own `RulesEngine` instance because indicator state (moving averages, etc.) is per-instrument.

## Pattern Match Emission and Storage

When `RulesEngine` returns a non-Hold signal:

1. **Signal filtering**: Exit signals require an existing position. Entry signals always emit (even if a position exists -- `has_existing_position` flag lets the UI inform the user).

2. **Signal filter check** (`should_emit_signal`): Per-instrument filters support both legacy format (`"all"`, `"entries"`, `"exits"`, `"longs"`, `"shorts"`) and new JSON format (`{"long":"all","short":"entry"}`). The JSON format allows independent control of long vs short signal types.

3. **Desktop notification**: `send_pattern_match_notification` sends an OS-level notification with deep link payload (match ID, instrument, timeframe, strategy info).

4. **Tauri event**: `app_handle.emit("pattern-matched", PatternMatchEvent)` delivers the match to any listening frontend window.

5. **Pending store**: `add_pending_match(json)` stores the event in a global `Lazy<Mutex<Vec<serde_json::Value>>>` (max 100 entries). This handles the case where the Live Monitor window is closed when a match fires. On window open, `get_pending_matches` retrieves missed matches.

6. **Pending signal tracking**: The match is added to `InstrumentState.pending_signals` for TTL expiration (3 candles) and conflict detection.

### Signal Lifecycle

```
Created (Pending)
    |
    +-- User executes -> Executed
    +-- User dismisses -> Dismissed
    +-- 3 candles pass -> Expired (via update_pending_signals)
    +-- Opposite direction signal -> Expired (via check_signal_conflicts)
    +-- Exit signal invalidates entry -> Expired (via check_signal_conflicts)
```

Status updates emit via `app_handle.emit("match-status-update", MatchStatusUpdateEvent)`.

## Threading Model

Each watcher runs in a **dedicated OS thread** with its own single-threaded tokio runtime:

```rust
std::thread::spawn(move || {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime for multi-watcher");
    rt.block_on(async move { watcher.start(app_handle).await });
});
```

This design avoids `Send + Sync` requirements on `StrategyWatcher` internals (which use `Mutex`, not `tokio::Mutex`, for `last_candle_time`). The watcher owns everything within its thread.

**Why not `tokio::spawn`?** The `CandleBoundaryDetector` uses `std::sync::Mutex` for `candle_starts` (because it was designed before the streaming architecture was added). Running in a dedicated thread with a current-thread runtime avoids holding a std Mutex across await points.

## Stop Signal Mechanism

Watchers check two stop conditions via `should_stop()`:

1. **Internal**: `running: AtomicBool` set to false by `stop()`
2. **External**: `stop_signal: Arc<AtomicBool>` shared with the command handler

The `start()` method uses `compare_exchange(false, true, SeqCst, SeqCst)` to atomically prevent double-start.

On app window close, `main.rs` intercepts the close event and stops all watchers synchronously before the window is destroyed. This is more reliable than JS cleanup during unmount.

## Frontend State Management

### Zustand Store (`watcherStore.ts`)

The store manages:
- `activeWatchers: Map<string, ActiveWatcher>` - legacy single-instrument watchers
- `multiWatchers: Map<string, MultiWatcher>` - multi-instrument watchers
- `pendingMatches: PatternMatch[]` - matches awaiting user action (persisted to localStorage)
- `matchHistory: PatternMatch[]` - all matches for reference (last 200)
- `staleConfigs / expiryConfigs` - per-watcher threshold configs (persisted to localStorage)
- `keepMonitorInBackground` - whether the monitor window stays alive

### useStrategyWatcher Hook

The hook bridges Tauri events to the Zustand store:
- Sets up `listen()` for: `pattern-matched`, `strategy-status`, `strategy-error`, `match-status-update`, `notification-clicked`
- On mount, fetches pending matches from the Rust backend store (`get_pending_matches`)
- Wraps `updateMatchStatus` to also call `remove_pending_match` on the Rust backend
- Returns watcher lifecycle functions: `startMultiWatcher`, `stopMultiWatcher`, `addInstrument`, `removeInstrument`, `updateInstrumentSignalFilter`

### StrategyWatcherApp

The entry point for the watcher window:
- Queries promoted strategies and saved watcher configs from Zero
- Auto-starts watchers on mount by grouping `strategy_watcher` rows by `strategy_id + timeframe`
- Subscribes watched instruments to the price stream for candle boundary detection
- Manages add/remove monitor forms and settings

## Execution Modes

```rust
pub enum ExecutionMode {
    SignalOnly,       // Emit pattern matches for user to act on
    ConfirmExecute,   // Emit matches, user confirms, app places order
    AutoExecute,      // Automatic execution (future feature)
}
```

Currently `SignalOnly` is the default. `ConfirmExecute` is implemented on the frontend via `MatchCard.onExecute` which calls `invoke('place_market_order')`. `AutoExecute` is defined but not yet implemented.

## Error Handling and Resilience

### Exponential Backoff

On transient network errors (502, 503, timeout, connection refused, etc.):
- Backoff starts at 5 seconds, doubles each consecutive failure, caps at 5 minutes
- Error events only emit to the frontend after 3 consecutive failures (avoids noise from brief blips)
- On success after errors, emits a recovery status event to clear the error state in UI

### Warmup Retry

Indicator warmup retries up to 5 times with exponential backoff for transient errors. Non-transient errors fail immediately.

### Per-Instrument Error Isolation

In `MultiInstrumentWatcher`, each instrument tracks its own `consecutive_errors` count. One instrument failing does not stop others.

## Key Design Decisions

1. **Separate RulesEngine per instrument**: Because indicators (EMAs, ATR, etc.) are stateful and instrument-specific. Sharing would corrupt calculations.

2. **OANDA as position truth source**: Instead of tracking positions internally, the watcher queries `get_open_positions` on every candle. This prevents state drift between the app and broker (e.g., if user manually trades on OANDA's platform).

3. **Fire-and-forget command pattern**: `MultiWatcherHandle` methods send commands optimistically and update local state immediately. The watcher processes commands when it can (between instrument ticks). This prevents the frontend from blocking on network-bound watcher operations.

4. **Pending store for offline matches**: Matches emitted while the Live Monitor window is closed are stored globally and retrieved on window open. This prevents missed signals during window lifecycle transitions.

5. **Dedicated OS threads**: Watchers run in separate threads to avoid Send+Sync constraints and to isolate watcher failures from the main Tauri thread.

6. **Streaming preferred over polling**: When the price stream is active, `StreamingCandleSource` detects candle closes within 1 second vs up to 10 minutes for polling. The boundary service is shared across all watchers.

7. **No duplicate signal blocking**: Each candle that matches emits its own signal. Previous behavior blocked duplicates, but this was changed so users can see every match and decide individually.

## Invariants

- All financial values (`entry_price`, `stop_loss`, `take_profit`, `position_size`) use `rust_decimal::Decimal`, never `f64`
- Only complete candles (`candle.complete == true`) are processed for indicator calculation and rule evaluation
- Watcher IDs for multi-watchers follow the format `{strategy_id}_{timeframe}`
- `compare_exchange` is used for the running flag (not check-then-set) to prevent race conditions
- Candle alignment uses `dailyAlignment=3` + `alignmentTimezone=UTC` everywhere
- Exit signals are only generated when the user has an open position on that instrument
- The pending store is capped at 100 entries to prevent unbounded memory growth

## Known Technical Debt

1. **Dual watcher types**: Both `StrategyWatcher` (single) and `MultiInstrumentWatcher` exist. Position sizing logic parity was achieved in BUG-071 fix (both now call `calculate_position_size_for_signal()`), but they should still be consolidated into a single implementation.

2. **Type aliases for backwards compatibility**: `StrategySignal = PatternMatch`, `SignalType = MatchType`, etc. exist in `pattern_match.rs` and are used throughout `watcher.rs` and `multi_watcher.rs`. These should be migrated to the new names.

3. **Duplicated transient error detection**: `is_transient_error()` is defined identically in both `watcher.rs` and `multi_watcher.rs`. Should be extracted to a shared utility.

4. **Duplicated poll interval logic**: `recommended_poll_interval` in `OandaPollingSource` and `poll_interval` in `MultiInstrumentWatcher` define the same timeframe-to-duration mapping.

5. **Monthly candle duration is approximate**: `candle_duration(Granularity::M)` returns `Duration::days(30)`, which doesn't handle variable month lengths.

6. **`close_percent` on PatternMatch uses `f64`**: This is the one exception to the Decimal rule. It represents a percentage (0-100) for partial exits and doesn't need financial precision, but it breaks the consistency.

7. **Detector unsubscribe is a no-op**: `CandleBoundaryService::unsubscribe()` doesn't actually clean up the detector registration. The comment says overhead is minimal, but it means registrations accumulate over the app lifetime.
