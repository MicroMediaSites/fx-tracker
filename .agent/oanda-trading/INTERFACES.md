# OANDA Trading Interfaces

## OandaClient (`oanda/client.rs`)

The HTTP client wrapper that handles authentication and URL construction. Stored in `AppState` behind an `RwLock`.

```rust
pub struct OandaClient {
    // Fields: client, base_url, api_key, account_id, environment
}

impl OandaClient {
    // Constructors
    pub fn new(config: &Config) -> Result<Self>;
    pub fn for_environment(config: &Config, environment: OandaEnvironment) -> Result<Self>;
    pub fn with_credentials(api_key: &str, account_id: &str, environment: OandaEnvironment) -> Result<Self>;
    pub fn with_base_url(base_url: &str, api_key: &str, account_id: &str) -> Result<Self>;  // #[doc(hidden)] for tests

    // Accessors
    pub fn account_id(&self) -> &str;
    pub fn base_url(&self) -> &str;
    pub fn environment(&self) -> OandaEnvironment;
    pub fn has_credentials(&self) -> bool;

    // Request builders (add bearer auth automatically)
    pub fn get(&self, url: &str) -> reqwest::RequestBuilder;
    pub fn post(&self, url: &str) -> reqwest::RequestBuilder;  // also sets Content-Type: application/json
    pub fn put(&self, url: &str) -> reqwest::RequestBuilder;   // also sets Content-Type: application/json

    // URL helper
    pub fn url(&self, path: &str) -> String;  // format!("{}{}", self.base_url, path)
}
```

## REST Endpoints (`oanda/endpoints.rs`)

All functions take `&OandaClient` as their first parameter and return `Result<T>`.

### Account & Instruments

```rust
pub async fn get_account(client: &OandaClient) -> Result<OandaAccount>;
pub async fn get_instruments(client: &OandaClient) -> Result<Vec<OandaInstrument>>;
```

### Trades

```rust
pub async fn get_trades(client: &OandaClient, count: Option<u32>, instrument: Option<&str>, state: Option<&str>) -> Result<Vec<Trade>>;
pub async fn get_trade_history(client: &OandaClient, count: Option<u32>, instrument: Option<&str>) -> Result<Vec<Trade>>;
// get_trade_history is a convenience wrapper that calls get_trades with state="CLOSED"
```

### Positions

```rust
pub async fn get_positions(client: &OandaClient) -> Result<Vec<Position>>;
pub async fn get_open_positions(client: &OandaClient) -> Result<Vec<Position>>;
```

### Orders

```rust
pub async fn get_orders(client: &OandaClient) -> Result<Vec<Order>>;
// Note: enriches STOP_LOSS/TAKE_PROFIT orders with instrument from associated trade
pub async fn place_market_order(client: &OandaClient, instrument: &str, units: i64) -> Result<OrderCreateResponse>;
pub async fn place_market_order_with_sl_tp(client: &OandaClient, instrument: &str, units: i64, stop_loss: Option<&str>, take_profit: Option<&str>) -> Result<OrderCreateResponse>;
pub async fn close_position(client: &OandaClient, instrument: &str, is_long: bool) -> Result<ClosePositionResponse>;
```

### Candles

```rust
// Default alignment (dailyAlignment=3, UTC)
pub async fn get_candles(client: &OandaClient, instrument: &str, granularity: Granularity, count: Option<u32>, from: Option<&str>, to: Option<&str>) -> Result<Vec<Candle>>;

// Custom alignment (for special cases)
pub async fn get_candles_with_alignment(client: &OandaClient, instrument: &str, granularity: Granularity, count: Option<u32>, from: Option<&str>, to: Option<&str>, alignment_timezone: &str, daily_alignment: u8) -> Result<Vec<Candle>>;

// Auto-pagination for large date ranges (>5000 candles)
pub async fn get_candles_paginated(client: &OandaClient, instrument: &str, granularity: Granularity, from: &str, to: &str) -> Result<Vec<Candle>>;
```

### Autochartist

```rust
pub async fn get_autochartist_signals(client: &OandaClient, instrument: &str) -> Result<AutochartistResponse>;
```

### Granularity Enum

```rust
pub enum Granularity {
    S5, S10, S15, S30,
    M1, M2, M4, M5, M10, M15, M30,
    H1, H2, H3, H4, H6, H8, H12,
    D, W, M,
}
// Implements: Display, FromStr, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash
```

### Constants

```rust
pub const DEFAULT_ALIGNMENT_TIMEZONE: &str = "UTC";
pub const DEFAULT_DAILY_ALIGNMENT: u8 = 3;
```

## Streaming Interface (`oanda/streaming.rs`)

### PriceStreamManager

```rust
pub struct PriceStreamManager { /* ... */ }
pub type PriceStreamer = PriceStreamManager;  // Backwards-compatible alias

impl PriceStreamManager {
    pub fn new(api_key: &str, account_id: &str, environment: &OandaEnvironment) -> Self;

    // Subscription management
    pub async fn subscribe(&mut self, instrument: String, app_handle: AppHandle) -> Result<()>;
    pub async fn unsubscribe(&mut self, instrument: String) -> Result<()>;
    pub fn stop(&self);

    // State queries
    pub fn is_running(&self) -> bool;
    pub fn is_healthy(&self) -> bool;
    pub fn has_credentials(&self) -> bool;
    pub fn has_account(&self) -> bool;
    pub fn subscribed_instruments(&self) -> Vec<String>;
    pub fn subscriber_count(&self, instrument: &str) -> usize;
    pub fn health_status(&self) -> StreamHealthStatus;

    // Configuration
    pub fn set_candle_boundary_service(&mut self, service: Arc<CandleBoundaryService>);
    pub fn set_spread_stats_collector(&mut self, collector: Arc<Mutex<SpreadStatsCollector>>);
    pub fn spread_stats_collector(&self) -> Option<Arc<Mutex<SpreadStatsCollector>>>;
    pub fn start_health_monitor(&self, app_handle: AppHandle);

    // Deprecated
    #[deprecated] pub async fn start(&mut self, instruments: Vec<String>, app_handle: AppHandle) -> Result<()>;
}
```

### SpreadStatsCollector

```rust
pub struct SpreadStatsCollector { /* ... */ }

impl SpreadStatsCollector {
    pub fn new(server_url: String) -> Self;
    pub fn on_price_tick(&mut self, instrument: &str, spread: Decimal);
    pub async fn maybe_send_batch(&mut self) -> bool;
}
```

### Emitted Types

```rust
pub struct PriceUpdate {
    pub instrument: String,
    pub bid: String,
    pub ask: String,
    pub spread: String,
    pub time: String,
    pub tradeable: bool,
}

pub struct StreamError {
    pub error_type: StreamErrorType,  // ParseError, ConnectionLost, StreamEnded, Reconnecting, MaxReconnectsExceeded
    pub message: String,
}

pub struct StreamHealthStatus {
    pub healthy: bool,
    pub seconds_since_heartbeat: u64,
    pub subscribed_instruments: usize,
    pub running: bool,
}
```

## Domain Models (`models/`)

### Trade

```rust
pub enum TradeState { Open, Closed, CloseWhenTradeable }

pub struct Trade {
    pub id: String,
    pub instrument: String,
    pub open_price: Decimal,
    pub open_time: DateTime<Utc>,
    pub units: Decimal,              // positive = long, negative = short
    pub realized_pl: Decimal,
    pub unrealized_pl: Option<Decimal>,
    pub state: TradeState,
    pub close_time: Option<DateTime<Utc>>,
    pub close_price: Option<Decimal>,
}

impl Trade {
    pub fn is_long(&self) -> bool;
    pub fn is_short(&self) -> bool;
    pub fn is_open(&self) -> bool;
    pub fn total_pl(&self) -> Decimal;  // realized + unrealized
}
```

### Candle / Ohlc

```rust
pub struct Ohlc {
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
}

pub struct Candle {
    pub time: DateTime<Utc>,
    pub mid: Ohlc,
    pub volume: i32,
    pub complete: bool,
}

impl Candle {
    pub fn is_bullish(&self) -> bool;   // close > open
    pub fn is_bearish(&self) -> bool;   // close < open
    pub fn range(&self) -> Decimal;     // high - low
    pub fn body_size(&self) -> Decimal; // |close - open|
}
```

### Position

```rust
pub struct Position {
    pub instrument: String,
    pub units: Decimal,           // positive = net long, negative = net short, zero = flat
    pub average_price: Decimal,
    pub unrealized_pl: Decimal,
    pub realized_pl: Decimal,
}

impl Position {
    pub fn flat(instrument: &str) -> Self;  // constructor for a flat (zero) position
    pub fn is_flat(&self) -> bool;
    pub fn is_long(&self) -> bool;
    pub fn is_short(&self) -> bool;
}
```

### Order

```rust
pub enum OrderType { Market, Limit, Stop, MarketIfTouched, TakeProfit, StopLoss, TrailingStopLoss }
pub enum OrderState { Pending, Filled, Triggered, Cancelled }

pub struct Order {
    pub id: String,
    pub instrument: String,       // "N/A" if not available (e.g., stop-loss without trade lookup)
    pub order_type: OrderType,
    pub units: Decimal,
    pub price: Option<Decimal>,
    pub state: OrderState,
    pub create_time: DateTime<Utc>,
}
```

## Tauri Commands

All commands are registered in `main.rs` and callable from the frontend via `invoke()`.

### Trading Commands (`commands/trading.rs`)

| Command | Parameters | Returns |
|---------|-----------|---------|
| `get_account` | (none) | `Account { id, currency, balance, nav, unrealized_pl, open_trade_count }` |
| `get_positions` | (none) | `Vec<Position>` (string fields) |
| `get_orders` | (none) | `Vec<Order>` (string fields) |
| `get_trade_history` | `count?: u32, instrument?: String` | `Vec<HistoricalTrade>` |
| `place_order` | `instrument: String, units: i64, stop_loss?: String, take_profit?: String` | `OrderConfirmation { filled, price, units, instrument, realized_pl, trade_id, error }` |
| `close_position` | `instrument: String, is_long: bool` | `CloseConfirmation { closed, instrument, units, price, realized_pl, error }` |

### Data Commands (`commands/data.rs`)

| Command | Parameters | Returns |
|---------|-----------|---------|
| `get_candles` | `instrument, granularity, count?, from?, to?` | `Vec<CandleData>` |
| `sync_trades` | `user_id, count?, data_source?` | `SyncStarted` (non-blocking) |
| `is_sync_enabled` | (none) | `bool` |
| `fetch_instruments` | (none) | `Vec<InstrumentInfo>` |
| `fetch_autochartist_signals` | `instrument` | `Vec<AutochartistZone>` |
| `calculate_pivot_points` | `instrument, timeframe?` | `Vec<PivotLevel>` |
| `get_indicator_data` | `instrument, granularity, count?, from?, to?, indicators_json` | `Vec<IndicatorSeries>` |

### Streaming Commands (`commands/streaming.rs`)

| Command | Parameters | Returns |
|---------|-----------|---------|
| `subscribe_to_prices` | `instrument` | `()` |
| `unsubscribe_from_prices` | `instrument` | `()` |
| `start_price_stream` | `instruments: Vec<String>` | `()` (deprecated) |
| `stop_price_stream` | (none) | `()` |
| `is_streaming` | (none) | `bool` |
| `get_stream_health` | (none) | `StreamHealthStatus` |

### OANDA Environment Commands (`commands/oanda.rs`)

| Command | Parameters | Returns |
|---------|-----------|---------|
| `switch_oanda_environment` | `environment, account_id, use_practice_url?` | `String` (status message) |
| `get_oanda_environment` | (none) | `String` ("practice" or "live") |
| `get_oanda_credentials` | (none) | `OandaCredentials { api_key_preview, account_id, account_alias, environment, is_configured }` |
| `save_oanda_credentials` | `api_key, account_id, environment` | `String` (confirmation) |

## Tauri Events

Events emitted by the backend, consumed by the frontend via `listen()`.

| Event Name | Payload Type | Emitter | Description |
|-----------|-------------|---------|-------------|
| `price-update` | `PriceUpdate` | PriceStreamManager | Real-time price tick for an instrument |
| `stream-error` | `StreamError` | PriceStreamManager | Parse errors, connection issues, reconnection status |
| `stream-health` | `StreamHealthStatus` | Health monitor task | Emitted on health status changes (healthy <-> unhealthy) |
| `sync-progress` | `SyncProgress { stage, message, current, total }` | sync_trades background task | Trade sync progress updates |
| `sync-complete` | `SyncResult { synced_count, open_trades, closed_trades, deleted_count }` | sync_trades background task | Trade sync completion |

## Database Queries Interface (`db.rs`)

Trade-related queries owned by this domain:

```rust
impl Database {
    pub async fn upsert_trade(&self, user_id: &str, trade: &Trade) -> Result<()>;
    pub async fn upsert_trades(&self, user_id: &str, trades: &[Trade]) -> Result<usize>;  // batch via UNNEST
    pub async fn delete_user_trades(&self, user_id: &str) -> Result<usize>;
    pub async fn get_trades(&self, user_id: &str, state_filter: Option<&str>, limit: Option<i32>) -> Result<Vec<TradeRow>>;
    pub async fn ensure_user(&self, user_id: &str, email: &str, display_name: &str) -> Result<()>;
}
```

## Queries-Service HTTP Interface (`queries_service.rs`)

```rust
pub struct QueriesServiceClient { /* client, base_url */ }

impl QueriesServiceClient {
    pub fn new(base_url: &str) -> Self;

    // Trade sync
    pub async fn sync_trades(&self, auth_token: &str, request: SyncTradesRequest) -> Result<SyncTradesResponse, String>;

    // Job tracking
    pub async fn start_job(&self, auth_token: &str, job_id: &str) -> Result<(), String>;
    pub async fn update_job_progress(&self, auth_token: &str, job_id: &str, progress: i32, detail: Option<&str>) -> Result<(), String>;
    pub async fn complete_job(&self, auth_token: &str, job_id: &str, result_json: &str) -> Result<(), String>;
    pub async fn fail_job(&self, auth_token: &str, job_id: &str, error_message: &str) -> Result<(), String>;
    pub async fn cancel_job(&self, auth_token: &str, job_id: &str) -> Result<(), String>;

    // AI context (strategies, notes, trades, calendar, jobs)
    pub async fn get_strategies(&self, auth_token: &str) -> Result<Vec<StrategyInfo>, String>;
    pub async fn get_strategy_by_id(&self, auth_token: &str, strategy_id: &str) -> Result<Option<StrategyDetail>, String>;
    pub async fn create_strategy(&self, auth_token: &str, strategy: StrategyRequest) -> Result<(), String>;
    pub async fn update_strategy(&self, auth_token: &str, strategy: StrategyRequest) -> Result<(), String>;
    pub async fn get_notes(&self, auth_token: &str, limit: Option<u32>) -> Result<Vec<NoteInfo>, String>;
    pub async fn get_notes_by_trade(&self, auth_token: &str, trade_id: &str) -> Result<Vec<NoteInfo>, String>;
    pub async fn get_notes_by_strategy(&self, auth_token: &str, strategy_id: &str) -> Result<Vec<NoteInfo>, String>;
    pub async fn get_trades(&self, auth_token: &str, state_filter: Option<&str>, limit: Option<i32>) -> Result<Vec<TradeInfo>, String>;
    pub async fn get_calendar_events(&self, auth_token: &str, currency: Option<&str>, impact: Option<&str>, limit: Option<u32>) -> Result<Vec<CalendarEventInfo>, String>;
    pub async fn get_backtest_job(&self, auth_token: &str, job_id: &str) -> Result<Option<BacktestJobInfo>, String>;
    pub async fn list_backtest_jobs(&self, auth_token: &str, strategy_id: Option<&str>, status: Option<&str>, limit: Option<u32>) -> Result<Vec<BacktestJobInfo>, String>;
}
```

## Candle Interface (Consumed by Other Domains)

The `get_candles`, `get_candles_with_alignment`, and `get_candles_paginated` functions are consumed by:

- **backtest-core** -- Fetches historical candles for backtesting (typically via `get_candles_paginated` for large date ranges)
- **strategy-monitor** -- Fetches recent candles to check strategy entry/exit conditions
- **charting** -- Fetches candles for chart display (typically via `get_candles` with count)

All consumers MUST use the default alignment unless they have a specific, documented reason to deviate.

## Frontend Interfaces

### Zustand Price Store (`stores/priceStore.ts`)

```typescript
interface PriceState {
  prices: Record<string, PriceUpdate>;  // instrument -> latest price
  streaming: boolean;
  error: StreamError | null;
  updatePrice: (price: PriceUpdate) => void;
  setStreaming: (streaming: boolean) => void;
  setError: (error: StreamError | null) => void;
  clearPrices: () => void;
}
```

### usePriceStream Hook (`hooks/usePriceStream.ts`)

```typescript
function usePriceStream(instruments?: string[]): {
  streaming: boolean;
  startStream: () => Promise<void>;
  stopStream: () => Promise<void>;
}
// Default instruments: EUR_USD, GBP_USD, USD_JPY, AUD_USD, USD_CAD
```

### usePriceStreaming Hook (`hooks/usePriceStreaming.ts`)

```typescript
interface UsePriceStreamingOptions {
  instrument: string;
  granularity: string;
  isHistoricalView: boolean;
  timeMapStateRef: React.RefObject<TimeMapState>;
  candleSeriesRef: React.RefObject<any>;
}

function usePriceStreaming(options: UsePriceStreamingOptions): {
  streaming: boolean;
  currentPrice: PriceUpdate | null;
  currentCandleRef: React.RefObject<CandlestickData | null>;
  updateCurrentCandleRef: React.RefObject<UpdateCandleCallback | null>;
}
```
