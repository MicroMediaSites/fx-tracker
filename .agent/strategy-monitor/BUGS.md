# Strategy Monitor Bugs

<!-- Template for new entries:

## [BUG-SMxx] Short description
- **Status**: Open | In Progress | Fixed
- **Severity**: Critical | High | Medium | Low
- **Filed**: YYYY-MM-DD
- **Fixed**: YYYY-MM-DD (if applicable)
- **Symptoms**: What the user sees
- **Root Cause**: Why it happens
- **Fix**: What was done (or needs to be done)
- **Files**: Affected files
-->

## [BUG-050] Notification row disappears when trade executed from chart window

- **Status**: Fixed
- **Severity**: Medium
- **Filed**: 2026-01-25
- **Fixed**: 2026-03-01
- **Symptoms**: When a pattern match notification appeared in the Live Monitor, executing the trade via "Open in Chart" (clicking the match row, then executing from the chart window) caused the notification row to disappear entirely. Using the "Order" button directly in the row worked correctly -- the row stayed visible with a "BOUGHT/SOLD" confirmation.
- **Root Cause**: Two execution paths existed with different behaviors:
  1. **"Order" button (correct)**: `MatchCard.handleExecute` calls `executeMatch` in `StrategyWatcherApp`, gets back an `OrderResult`, sets local React state `confirmation`, and the row renders the confirmation text. The match stays in `pendingMatches` with `status === 'pending'`.
  2. **Chart window (buggy)**: `useTradeExecution.executeTrade` calls `invoke('broadcast_match_executed')` which emits a `match-status-update` Tauri event with `new_status: "executed"`. The watcher hook's event listener calls `updateMatchStatus(matchId, 'executed')`. In `watcherStore.ts`, `updateMatchStatus` removed ALL non-pending matches from `pendingMatches` (`state.pendingMatches.filter(m => m.id !== matchId)`), causing the row to disappear.
  The root issue was that `updateMatchStatus` treated 'executed' the same as 'dismissed'/'expired' -- all were filtered out.
- **Fix**: Three changes:
  1. `watcherStore.ts` `updateMatchStatus`: Keep 'executed' matches in `pendingMatches` with updated status (only filter out 'dismissed' and 'expired').
  2. `watcherStore.ts` `loadPendingMatches`: Restore both 'pending' and 'executed' matches from localStorage on reload.
  3. `MatchCard.tsx`: When `match.status === 'executed'` (and no local `confirmation` state), show "order placed" text and apply `opacity-60` visual treatment, matching the behavior of the direct Order button path.
- **Prevention**: When adding new match status transitions, check both the "in-row execution" path (local React state in MatchCard) and the "cross-window execution" path (Tauri event -> store update). Both must keep the row visible with appropriate confirmation text.
- **Files**: `src/stores/watcherStore.ts`, `src/components/watcher/MatchCard.tsx`, `e2e/tests/bug-050-notification-persist.spec.ts` (new test)

## [BUG-062] Live Monitor ignores Long/Short alert toggle when set to OFF

- **Status**: Fixed
- **Severity**: High
- **Filed**: 2026-01-28
- **Fixed**: 2026-03-01
- **Symptoms**: When adding a monitor with an instrument's long or short alert set to OFF, notifications still fire for that direction. The OFF setting is not respected.
- **Root Cause**: Multiple contributing factors:
  1. **Backend filter check too late:** In `multi_watcher.rs`, both `process_instrument_tick` and `evaluate_instrument_initial` added signals to `pending_signals` (for TTL tracking and conflict detection) BEFORE calling `emit_signal()` which performed the filter check. This meant filtered-out signals were still tracked as pending, potentially blocking future signals via conflict detection.
  2. **Race condition:** Commands to the watcher (AddInstrument, UpdateSignalFilter) are processed asynchronously via an mpsc channel at the start of each main loop iteration. If the user changes a filter while the watcher is processing instruments in the tick loop, the filter update waits until the next iteration. Signals fired during this window use the stale filter value.
  3. **No frontend-side filtering:** The `pattern-matched` event listener in `useStrategyWatcher.ts` added ALL received matches to the store without checking signal filters. If the backend had a stale filter for any reason (race condition, pending matches stored before filter update), signals passed through to the user.
- **Fix**:
  1. **Backend: Early filter check** - Moved signal filter check BEFORE `add_pending_signal` in both `process_instrument_tick` and `evaluate_instrument_initial`. Filtered signals are never tracked as pending and never emitted. The existing filter check inside `emit_signal` remains as a safety net.
  2. **Frontend: Defense-in-depth filtering** - Added `shouldPassFrontendFilter()` function in `useStrategyWatcher.ts` that mirrors the backend's `should_emit_signal` logic. Both the live `pattern-matched` event handler and the pending-match-on-mount loader check the frontend filter before calling `addMatch`. Filter state is synced from Zero data via a ref updated by `StrategyWatcherApp`.
- **Prevention**: Signal filtering should always be checked at the earliest possible point (before any tracking or persistence), not just at the emission boundary. Multi-layer filtering (backend + frontend) provides defense-in-depth against async timing issues.
- **Files**: `src-tauri/src/strategy/multi_watcher.rs`, `src/hooks/useStrategyWatcher.ts`, `src/StrategyWatcherApp.tsx`

## [BUG-051] Live Monitor UI freezes when adding all FX symbols on M1 timeframe

- **Status**: Fixed
- **Severity**: High
- **Filed**: 2026-01-25
- **Fixed**: 2026-03-01
- **Symptoms**: Adding all 28+ FX instruments on M1 timeframe causes the Live Monitor window to freeze and become unresponsive. UI stops updating and user cannot interact with the application.
- **Root Cause**: Two compounding bottlenecks:
  1. **Backend: N position API calls per tick cycle.** `process_instrument_tick` called `check_open_position(instrument)` for every instrument individually, each making an HTTP call to `get_open_positions()` on the OANDA API. With 28 instruments on M1 (1-second poll interval in streaming mode), this meant 28 concurrent API calls per cycle, overwhelming the network stack and causing cascading delays.
  2. **Frontend: unbatched price store updates.** The `priceStore.updatePrice()` called Zustand's `set()` on every price tick, creating a new state object with spread syntax. With 28+ streaming instruments producing potentially hundreds of ticks/second, each tick triggered a full React re-render cycle across all components consuming the price store, saturating the UI thread.
- **Fix**:
  1. **Backend position cache:** Added `cached_positions: Option<(Instant, HashMap<String, PositionDirection>)>` to `MultiInstrumentWatcher`. Positions are fetched once per tick cycle via `refresh_position_cache()` (5-second TTL), reducing API calls from N-per-cycle to 1-per-cycle. The cache is invalidated at the start of each main loop iteration.
  2. **Frontend batched price updates:** Replaced synchronous `set()` calls in `priceStore.updatePrice()` with a `requestAnimationFrame`-based batching mechanism. Price updates are buffered in a plain object and flushed to Zustand at most once per frame (~60fps), reducing re-renders from hundreds/second to at most 60/second regardless of tick volume.
- **Prevention**: When adding per-instrument operations that make network calls (API, WebSocket), always check if the data can be fetched once and shared across instruments. The position cache pattern (`refresh_position_cache` + `get_cached_position`) should be used for any shared API data. For high-frequency frontend events (price ticks, streaming data), always batch updates before committing to React state.
- **Files**: `src-tauri/src/strategy/multi_watcher.rs`, `src/stores/priceStore.ts`

## [BUG-071] Live Monitor trades use hardcoded position size instead of risk settings

- **Status**: Fixed
- **Severity**: High
- **Filed**: 2026-02-05
- **Fixed**: 2026-03-01
- **Symptoms**: Trades executed from the multi-instrument live monitor always use 1000 units regardless of the strategy's risk settings (risk method, risk value, risk percent, R:R ratio, etc.). SL/TP price levels are correct but position sizing is wrong.
- **Root Cause**: `MultiInstrumentWatcher.create_signal()` always passed `None` for `position_size` in `PatternMatch::entry()`. The single-instrument `StrategyWatcher` has a `calculate_position_size_for_signal()` method that fetches account balance from OANDA and delegates to `RulesEngine::calculate_position_size()`, but this was never implemented for the multi-watcher. On the frontend, when `position_size` is `None`, `executeMatch()` in `StrategyWatcherApp.tsx` falls back to a hardcoded 1000 units.
- **Fix**: Added `calculate_position_size_for_signal()` async method to `MultiInstrumentWatcher` (mirrors the single-watcher implementation). Made `create_signal()` async and calls the new method for entry signals. Position size is now computed using the strategy's risk settings and current account balance, then passed through to the frontend via the `PatternMatch` event payload.
- **Files**: `src-tauri/src/strategy/multi_watcher.rs`
