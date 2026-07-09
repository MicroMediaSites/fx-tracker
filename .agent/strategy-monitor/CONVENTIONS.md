# Strategy Monitor Conventions

## Naming Patterns

### Rust

- **Watcher ID format**: `{strategy_id}_{timeframe}` (e.g., `abc123_H4`) for multi-watchers, raw `config_id` for legacy single watchers
- **Event names**: kebab-case strings matching frontend `listen()` calls: `"pattern-matched"`, `"strategy-status"`, `"strategy-error"`, `"match-status-update"`, `"watcher-tick"`, `"notification-clicked"`
- **Type aliases**: During the migration from "signal" to "match" terminology, aliases exist: `StrategySignal = PatternMatch`, `SignalType = MatchType`, etc. New code should use the `Match`/`Pattern` names.
- **Config ID**: For `StrategyWatcher`, this is the `strategy_config` database ID. For `MultiWatcherHandle.watcher_id`, it is `{strategy_id}_{timeframe}`.
- **Instrument error config_id**: Multi-watcher instrument errors use `{watcher_id}_{instrument}` format (e.g., `abc123_H4_EUR_USD`)

### TypeScript

- **Store types**: Mirror Rust types with snake_case field names (e.g., `match_type`, `entry_price`, `config_id`) since they arrive from Tauri serialization
- **Component props**: PascalCase interfaces with `Props` suffix: `MatchCardProps`, `WatcherControlsProps`
- **Hook**: Single hook `useStrategyWatcher` wraps all watcher operations
- **Store**: `useWatcherStore` via Zustand `create()`
- **Signal filter types**: `SignalFilter` (legacy string union), `DirectionFilter` (per-direction), `InstrumentSignalFilters` (combined `{long, short}` object)

## Error Handling Patterns

### Transient vs Non-Transient Errors

The watcher distinguishes between:

**Transient** (retriable): 502/503/504 gateway errors, connection refused/reset, timeouts, rate limits, network errors. Handled with exponential backoff (5s base, 5min cap). Error events suppressed until 3 consecutive failures.

**Non-transient** (logged immediately): Parse errors, invalid strategy definitions, unknown instruments. These emit error events on first occurrence.

Pattern:
```rust
match self.process_tick(&app_handle).await {
    Ok(()) => {
        // Reset consecutive errors, re-emit Running status if recovering
        if self.consecutive_errors > 0 {
            self.consecutive_errors = 0;
            self.emit_status(&app_handle, WatcherStatus::Running, None);
        }
    }
    Err(e) => {
        self.consecutive_errors += 1;
        if Self::is_transient_error(&error_msg) {
            let backoff = self.calculate_backoff();
            if self.consecutive_errors >= 3 {
                self.emit_error(...);
            }
            sleep(backoff).await;
            continue; // Skip normal poll interval
        } else {
            self.emit_error(...);
        }
    }
}
```

### Warmup Failure

Multi-watcher marks the instrument as initialized even on warmup failure (to prevent infinite retry loops) and logs the error:
```rust
Err(e) => {
    state.initialized = true; // Prevent infinite loop
    self.emit_instrument_error(...);
}
```

### Command Channel Errors

Fire-and-forget commands in `MultiWatcherHandle` use `oneshot::channel().0` as a dummy response sender since the handle doesn't wait for responses. If the channel is closed (watcher stopped), the error is returned to the caller:
```rust
.map_err(|_| "Failed to send add command - watcher may have stopped".to_string())
```

## How to Add a New Execution Mode

1. **Add variant to `ExecutionMode`** in `src-tauri/src/strategy/watcher.rs`:
   ```rust
   pub enum ExecutionMode {
       SignalOnly,
       ConfirmExecute,
       AutoExecute,
       NewMode, // Add here
   }
   ```

2. **Update `FromStr` impl** in the same file to parse the new string value

3. **Update command handlers** in `src-tauri/src/commands/watcher.rs` -- the mode is already passed as a string and parsed, so no changes needed unless the new mode requires different initialization

4. **Add frontend support** in `useStrategyWatcher.ts` -- update the `StartMultiWatcherParams.mode` type union

5. **Add behavior**: If the mode changes signal creation or emission, modify `filter_signal` (single watcher) or `create_signal` + `emit_signal` (multi-watcher)

## How to Add a New Watcher Feature

The typical pattern for adding functionality to the live watcher:

1. **If it affects per-instrument state**: Add the field to `InstrumentState` in `multi_watcher.rs` and the constructor `InstrumentState::new()`

2. **If it needs dynamic updates**: Add a `WatcherCommand` variant and handle it in `process_commands()`. Add a method to `MultiWatcherHandle` for the command handler to call.

3. **If it needs a Tauri command**: Add to `src-tauri/src/commands/watcher.rs` and register in `main.rs` invoke_handler

4. **If it needs frontend state**: Add to `WatcherState` in `watcherStore.ts`, add the action, expose through `useStrategyWatcher` hook

5. **If it needs persistence**: Add column to `strategy_watcher` in `shared/schema.ts` with a migration in `queries-service/src/migrate.ts`

## Frontend Patterns

### Event Listener Setup

The `useStrategyWatcher` hook uses a single `useEffect([], [])` with no dependencies to set up all Tauri event listeners once. It uses `isMountedRef` (a `useRef`) rather than a local variable to handle React StrictMode's double-mount behavior:

```typescript
const isMountedRef = useRef(true);

useEffect(() => {
    isMountedRef.current = true;
    const setup = async () => {
        if (!isMountedRef.current) return;
        matchUnlisten = await listen('pattern-matched', (event) => {
            if (!isMountedRef.current) return;
            // process event
        });
    };
    setup();
    return () => { isMountedRef.current = false; matchUnlisten?.(); };
}, []);
```

### Match Status Sync

When the frontend updates a match status (dismiss, execute), it must also remove the match from the Rust pending store to prevent re-delivery on window reopen:

```typescript
const updateMatchStatus = useCallback(
    (matchId: string, status: MatchStatus) => {
        storeUpdateMatchStatus(matchId, status);
        if (status !== 'pending') {
            invoke('remove_pending_match', { matchId });
        }
    },
    [storeUpdateMatchStatus]
);
```

### Auto-Start from Zero

`StrategyWatcherApp` groups saved `strategy_watcher` rows by `{strategy_id}_{timeframe}` and starts one multi-watcher per group. Per-instrument signal filters are passed as a `signalFiltersMap` JSON. The auto-start runs once on mount via a `useRef` guard (`autoStartedRef`).

### Pending Match Persistence

Pending matches are persisted to `localStorage` (key: `candlesight_pending_matches`) so they survive page reloads within the Tauri window. On store changes, `savePendingMatches()` writes the current pending list. On store initialization, `loadPendingMatches()` restores only matches with `status === 'pending'`.

## Anti-Patterns

1. **Never use `f64` for financial values**: All prices, position sizes, stop losses, and take profits must use `rust_decimal::Decimal`. The only exception is `close_percent` (a percentage, not a financial value).

2. **Never evaluate rules during warmup**: Call `rules_engine.warmup_candle(candle)` (not `on_candle_live`) during indicator initialization. Evaluating rules on historical data would generate spurious signals.

3. **Never hold AppState locks across await points**: The command handlers clone handles from `AppState` before doing async work:
   ```rust
   let handle = {
       let handles = state.multi_watcher_handles.lock().await;
       handles.get(&watcher_id).cloned()
   };
   // Now use handle without holding the lock
   handle.add_instrument(...).await
   ```

4. **Never use check-then-set for the running flag**: Always use `compare_exchange` to atomically check and set:
   ```rust
   if self.running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
       return Ok(()); // Already running
   }
   ```

5. **Never block on watcher commands from the frontend**: The `MultiWatcherHandle` methods are fire-and-forget. Don't convert them to request/response -- the watcher may be blocked on a network call for seconds.

6. **Never add database migrations to Rust backend code**: Watcher config persistence is in Zero. If you need new columns, add migrations to `queries-service/src/migrate.ts` and update `shared/schema.ts`.

7. **Never skip the candle completeness filter**: Always check `candle.complete == true` before processing. Incomplete candles have partial OHLC data that would corrupt indicator calculations.

8. **Never emit signals to the frontend without checking the signal filter**: The multi-watcher's `emit_signal` method checks `should_emit_signal` before emitting. Bypassing this would flood users who have configured filters.
