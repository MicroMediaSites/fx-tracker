# Strategy Monitor Interfaces

## Tauri Commands Exposed

All commands are registered in `src-tauri/src/main.rs` invoke_handler.

### Legacy Single-Instrument Watcher

| Command | Parameters | Returns | File |
|---------|-----------|---------|------|
| `start_strategy_watcher` | `config_id`, `strategy_json`, `instrument`, `timeframe`, `user_id`, `mode`, `sr_zones_json?` | `()` | `commands/watcher.rs:48` |
| `start_strategy_watcher_auto` | Same as above | `()` | `commands/watcher.rs:167` |
| `stop_strategy_watcher` | `config_id` | `()` | `commands/watcher.rs:275` |
| `get_active_watchers` | (none) | `Vec<WatcherStatusInfo>` | `commands/watcher.rs:301` |
| `is_watcher_running` | `config_id` | `bool` | `commands/watcher.rs:322` |

### Multi-Instrument Watcher

| Command | Parameters | Returns | File |
|---------|-----------|---------|------|
| `start_multi_watcher` | `strategy_id`, `strategy_name`, `strategy_json`, `timeframe`, `instruments: Vec<String>`, `user_id`, `mode`, `signal_filters_json?`, `sr_zones_json?` | `String` (watcher_id) | `commands/watcher.rs:357` |
| `stop_multi_watcher` | `watcher_id` | `()` | `commands/watcher.rs:592` |
| `add_watcher_instrument` | `watcher_id`, `instrument`, `sr_zones_json?`, `signal_filter?` | `()` | `commands/watcher.rs:518` |
| `remove_watcher_instrument` | `watcher_id`, `instrument` | `()` | `commands/watcher.rs:555` |
| `update_watcher_signal_filter` | `watcher_id`, `instrument`, `signal_filter` | `()` | `commands/watcher.rs:573` |
| `get_active_multi_watchers` | (none) | `Vec<MultiWatcherStatusInfo>` | `commands/watcher.rs:618` |
| `get_multi_watcher_instruments` | `watcher_id` | `Vec<String>` | `commands/watcher.rs:648` |
| `stop_all_watchers` | (none) | `u32` (count stopped) | `commands/watcher.rs:667` |

### Pending Match Store

| Command | Parameters | Returns | File |
|---------|-----------|---------|------|
| `get_pending_matches` | (none) | `Vec<serde_json::Value>` | `main.rs:155` |
| `clear_pending_matches` | (none) | `()` | `main.rs:161` |
| `remove_pending_match` | `match_id: String` | `()` | `main.rs:167` |

### Related Commands (Other Domains)

| Command | Parameters | Returns | File |
|---------|-----------|---------|------|
| `broadcast_match_executed` | match execution details | `()` | `commands/analysis.rs:550` |
| `place_market_order` | order details | order result | (oanda-trading domain) |

## Tauri Events Emitted

Events flow from the Rust backend to the frontend via `app_handle.emit()`.

### From Watcher to Frontend

| Event Name | Payload Type | Emitter | Description |
|------------|-------------|---------|-------------|
| `pattern-matched` | `PatternMatchEvent` | `watcher.rs:851`, `multi_watcher.rs:1099` | Strategy conditions matched; contains the full `PatternMatch` plus strategy name and timeframe |
| `strategy-status` | `StrategyStatusEvent` | `watcher.rs:861`, `multi_watcher.rs:1111` | Watcher status change (Running/Stopped/Error) |
| `strategy-error` | `StrategyErrorEvent` | `watcher.rs:871`, `multi_watcher.rs:1121` | Watcher error (transient or fatal) |
| `match-status-update` | `MatchStatusUpdateEvent` | `watcher.rs:682`, `multi_watcher.rs:1138` | Match expired or invalidated by conflicting signal |
| `watcher-tick` | `WatcherTickEvent` | `watcher.rs:558`, `multi_watcher.rs:894` | Debug event: candle processed with signal result |

### From Notifications to Frontend

| Event Name | Payload Type | Description |
|------------|-------------|-------------|
| `notification-clicked` | `NotificationClickedPayload` | User clicked a desktop notification; contains match_id, instrument, timeframe, strategy info for deep linking to chart window |

## Event Payload Structures

### PatternMatchEvent
```typescript
{
  pattern_match: {
    id: string,              // UUID
    user_id: string,
    config_id: string,       // strategy_id for multi-watcher
    instrument: string,      // e.g., "EUR_USD"
    match_type: 'entry' | 'exit' | 'partial_exit',
    direction?: 'long' | 'short',
    entry_price?: string,    // Decimal as string
    stop_loss?: string,      // Decimal as string
    take_profit?: string,    // Decimal as string
    position_size?: string,  // Decimal as string (only in single watcher)
    close_percent?: number,  // For partial exits (f64)
    reason: string,
    status: 'pending' | 'executed' | 'dismissed' | 'expired',
    indicator_snapshot?: Record<string, Record<string, string>>,
    has_existing_position: boolean,
    executed_at?: string,    // ISO 8601 timestamp
    created_at: string,      // ISO 8601 timestamp
  },
  strategy_name: string,
  timeframe: string,         // e.g., "H4"
}
```

### StrategyStatusEvent
```typescript
{
  config_id: string,    // watcher_id for multi-watcher
  status: 'running' | 'stopped' | 'error',
  message?: string,
}
```

### StrategyErrorEvent
```typescript
{
  config_id: string,    // For multi-watcher instrument errors: "{watcher_id}_{instrument}"
  error_type: string,   // 'warmup_failed' | 'transient_error' | 'tick_error' | 'start_failed' | 'warmup_retry'
  message: string,
}
```

### MatchStatusUpdateEvent
```typescript
{
  match_id: string,
  config_id: string,
  new_status: 'pending' | 'executed' | 'dismissed' | 'expired',
  reason: string,
}
```

### WatcherTickEvent
```typescript
{
  config_id: string,    // For multi-watcher: "{watcher_id}_{instrument}"
  instrument: string,
  timeframe: string,
  candle_time: string,  // RFC 3339
  close_price: string,
  signal_result: string, // 'Hold' | 'Entry Long' | 'Entry Short' | 'Exit' | 'PartialExit'
}
```

## Interfaces Consumed from Other Domains

### From backtest-core (`src-tauri/src/backtest/rules_engine.rs`)

```rust
// Types consumed
pub struct RulesEngine { ... }
pub struct StrategyDefinition { ... }  // Deserialized from strategy JSON
pub enum RulesSignal { Hold, Entry { direction, stop_loss, take_profit, ... }, Exit { reason, ... }, PartialExit { reason, close_percent, ... } }
pub enum PositionDirection { Long, Short }
pub struct SRZone { id: String, upper_price: Decimal, lower_price: Decimal }

// Methods called by watcher
RulesEngine::new(strategy: StrategyDefinition) -> Result<Self, String>
RulesEngine::warmup_candle(&mut self, candle: &Candle)          // Indicator-only, no rule eval
RulesEngine::on_candle_live(&mut self, candle: &Candle, position: Option<PositionDirection>) -> RulesSignal
RulesEngine::get_indicator_snapshot(&self) -> HashMap<String, HashMap<String, String>>
RulesEngine::calculate_position_size(&self, balance: Decimal, entry: Decimal, sl: Decimal, direction: PositionDirection) -> Option<Decimal>
RulesEngine::set_sr_zones(&mut self, zones: Vec<SRZone>)
RulesEngine::set_pip_value_for_instrument(&mut self, instrument: &str)
```

### From oanda-trading (`src-tauri/src/oanda/`)

```rust
// Types consumed
pub struct OandaClient { ... }         // Cloned into watcher threads
pub enum Granularity { S5, M1, H1, H4, D, ... }  // Candle timeframe

// Functions called
get_candles(&client, &instrument, timeframe, count, from, to) -> Result<Vec<Candle>>
get_open_positions(&client) -> Result<Vec<Position>>
get_account(&client) -> Result<Account>    // For position sizing (single watcher only)

// Integration point
PriceStreamer.set_candle_boundary_service(Arc<CandleBoundaryService>)
// PriceStreamer calls CandleBoundaryService.on_price_tick() for each streaming tick
```

### From shared (`src-tauri/src/models/`)

```rust
pub struct Candle {
    pub time: DateTime<Utc>,
    pub mid: CandlestickData,  // { open, high, low, close: Decimal }
    pub complete: bool,
    pub volume: i64,
}
```

### From notifications (`src-tauri/src/notifications.rs`)

```rust
pub struct NotificationClickedPayload {
    pub match_id: String,
    pub instrument: String,
    pub timeframe: String,
    pub strategy_id: String,
    pub strategy_name: String,
    pub direction: String,
    pub entry_price: String,
    pub stop_loss: String,
    pub take_profit: String,
    pub match_time: i64,
}

pub fn send_pattern_match_notification(app_handle: AppHandle, payload: NotificationClickedPayload)
```

## Frontend Hook Interface

### useStrategyWatcher() Return Value

```typescript
{
  // State (read-only)
  activeWatchers: ActiveWatcher[],         // Legacy single-instrument
  multiWatchers: MultiWatcher[],           // Multi-instrument (primary)
  pendingMatches: PatternMatch[],          // Matches awaiting user action
  matchHistory: PatternMatch[],            // All matches (last 200)
  lastError: StrategyErrorEvent | null,

  // Legacy single-instrument actions
  startWatcher: (params: StartWatcherParams) => Promise<boolean>,
  stopWatcher: (configId: string) => Promise<boolean>,

  // Multi-watcher actions
  startMultiWatcher: (params: StartMultiWatcherParams) => Promise<string | null>,  // Returns watcher_id
  stopMultiWatcher: (watcherId: string) => Promise<boolean>,
  addInstrument: (watcherId: string, instrument: string, srZones?: SRZoneData[], signalFilter?: string) => Promise<boolean>,
  removeInstrument: (watcherId: string, instrument: string) => Promise<boolean>,
  updateInstrumentSignalFilter: (watcherId: string, instrument: string, signalFilter: string) => Promise<boolean>,
  updateMultiWatcherSignalFilter: (watcherId: string, signalFilter: SignalFilter) => void,

  // Match actions
  updateMatchStatus: (matchId: string, status: MatchStatus) => void,  // Also syncs to Rust backend store

  // Helpers
  isWatcherRunning: (configId: string) => boolean,
  isMultiWatcherRunning: (watcherId: string) => boolean,
  hasActiveWatchers: boolean,
  hasActiveMultiWatchers: boolean,
  getMultiWatcher: (watcherId: string) => MultiWatcher | undefined,
}
```

## Zero Queries Used

| Query | Table | Filter | Used In |
|-------|-------|--------|---------|
| `myPromotedStrategies` | `strategy` | `user_id = currentUser AND is_promoted = true` | `StrategyWatcherApp.tsx` - to build strategy JSON for watcher |
| `myActiveWatchers` | `strategy_watcher` | `user_id = currentUser AND is_active = true` | `StrategyWatcherApp.tsx` - to auto-start watchers on mount |
| `mySRZones` | `sr_zone` | `user_id = currentUser` | `StrategyWatcherApp.tsx` - to pass S/R zones for zone-based triggers |

### strategy_watcher Table Schema

```typescript
{
  id: string,              // {strategy_id}-{instrument}-{timeframe}
  user_id: string,
  strategy_id: string,     // FK to strategy table
  strategy_name: string?,  // Cached for display
  instrument: string,      // e.g., "EUR_USD"
  timeframe: string,       // e.g., "H1"
  mode: string,            // 'signal_only' | 'confirm_execute' | 'auto_execute'
  signal_filter: string,   // Legacy: 'all'|'entries'|... or JSON: '{"long":"all","short":"entry"}'
  is_active: boolean,      // Whether to auto-start on app load
  created_at: number,      // Unix timestamp ms
  updated_at: number,      // Unix timestamp ms
}
```

## AppState Fields (main.rs)

The strategy-monitor domain uses these fields on the shared `AppState`:

```rust
pub struct AppState {
    pub watcher_handles: Arc<Mutex<HashMap<String, WatcherHandle>>>,         // Legacy single-instrument
    pub multi_watcher_handles: Arc<Mutex<HashMap<String, MultiWatcherHandle>>>, // Multi-instrument (primary)
    pub candle_boundary_service: Arc<CandleBoundaryService>,                 // Shared boundary detection
    pub client: Arc<RwLock<OandaClient>>,                                    // Cloned into watcher threads
    pub streamer: Arc<Mutex<PriceStreamer>>,                                 // For subscribing instruments
    // ... other fields owned by other domains
}
```

### WatcherHandle (Legacy)
```rust
pub struct WatcherHandle {
    pub config_id: String,
    pub instrument: String,
    pub timeframe: String,
    pub stop_signal: Arc<AtomicBool>,
}
```

### MultiWatcherHandle
```rust
pub struct MultiWatcherHandle {
    pub watcher_id: String,
    pub strategy_id: String,
    pub strategy_name: String,
    pub timeframe: String,
    pub instruments: Arc<RwLock<Vec<String>>>,
    pub command_tx: mpsc::Sender<WatcherCommand>,
    pub stop_signal: Arc<AtomicBool>,
}
```
