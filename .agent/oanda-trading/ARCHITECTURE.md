# OANDA Trading Architecture

## System Overview

The OANDA trading domain provides the bridge between the OANDA broker and the CandleSight desktop application. Data flows through three channels:

1. **REST API** (request/response) -- account info, candles, trade execution, trade history
2. **SSE Streaming** (long-lived connection) -- real-time price ticks and heartbeats
3. **Trade Sync** (periodic batch) -- OANDA trades pushed to queries-service for PostgreSQL persistence and Zero sync to the frontend

```
                    OANDA REST API                OANDA Streaming API
                    (api-fxpractice/               (stream-fxpractice/
                     api-fxtrade.oanda.com)         stream-fxtrade.oanda.com)
                         |                              |
                         v                              v
                   +-----------+               +-------------------+
                   | endpoints |               | PriceStreamManager|
                   |   .rs     |               |    streaming.rs   |
                   +-----------+               +-------------------+
                         |                         |           |
                    OandaClient                    |      SpreadStats
                    (client.rs)                    |      Collector
                         |                         |           |
               +---------+----------+              |    queries-service
               |         |         |               |    /spread-stats/submit
            Domain    Tauri      DB               Tauri
            Models    Commands   Queries          Events
            (models/) (commands/) (db.rs)          |
                         |                    +----+----+
                         |                    |         |
                     Frontend             price-update  stream-error
                     (invoke)             stream-health
                                              |
                                         Zustand Store
                                         (priceStore.ts)
                                              |
                                    usePriceStream.ts (dashboard)
                                    usePriceStreaming.ts (chart)
```

## Two-Layer Type System

All OANDA data passes through a deliberate two-layer conversion. This is the foundational design decision of the domain.

### Layer 1: Raw OANDA Types (`src-tauri/src/oanda/types.rs`)

These types mirror the exact JSON structure returned by OANDA's API. All numeric fields are `String` because that is how OANDA serializes them.

- `OandaTrade` -- prices, units, PL as `String`; times as RFC3339 `String`; state as `String`
- `OandaCandle` -- OHLC in `OandaCandleData` with `o`, `h`, `l`, `c` as `String`
- `OandaPosition` -- long/short sides with `String` units and prices
- `OandaOrder` -- all optional fields, `String` types throughout
- `StreamPrice` -- bid/ask as `Vec<PriceBucket>` where price is `String`
- `MarketOrderRequest` / `ClosePositionRequest` -- outbound request types

**WHY strings?** OANDA returns all numbers as strings in their JSON. Deserializing directly to numeric types would require custom deserializers for every field and risk precision loss. By accepting strings first, we defer the decision about precision to the domain model layer.

### Layer 2: Domain Models (`src-tauri/src/models/`)

These are the rich types used throughout the application. Financial values use `rust_decimal::Decimal` (never f64). Timestamps use `chrono::DateTime<Utc>`. States use type-safe enums.

- `Trade` -- `open_price: Decimal`, `open_time: DateTime<Utc>`, `state: TradeState` enum
- `Candle` -- `mid: Ohlc` (with `Decimal` OHLC fields), `time: DateTime<Utc>`
- `Position` -- `units: Decimal`, `average_price: Decimal`, `unrealized_pl: Decimal`
- `Order` -- `order_type: OrderType` enum, `state: OrderState` enum, `price: Option<Decimal>`

Conversion is done via `From<OandaType> for DomainType` implementations. Each conversion:
- Parses strings to `Decimal` with `unwrap_or_default()` (zero on parse failure -- safe for financial display)
- Parses RFC3339 strings to `DateTime<Utc>` with fallback to `Utc::now()` on failure
- Maps string state values to enums via case-insensitive matching with safe defaults

**WHY Decimal?** f64 cannot represent `1.08500` exactly. In a trading application, even a sub-pip rounding error can cause incorrect PL calculations, wrong stop-loss placement, or UI display bugs. `Decimal` guarantees exact representation of the values OANDA returns.

## Candle Fetching and Alignment

All candle fetching uses two constants defined in `endpoints.rs`:

```rust
pub const DEFAULT_ALIGNMENT_TIMEZONE: &str = "UTC";
pub const DEFAULT_DAILY_ALIGNMENT: u8 = 3;
```

This means H4 candles align at 03:00, 07:00, 11:00, 15:00, 19:00, 23:00 UTC, matching OANDA's default chart boundaries. The `get_candles()` function always passes these alignment parameters. The `get_candles_with_alignment()` variant allows custom alignment for special use cases.

**WHY dailyAlignment=3?** OANDA's default candle boundaries use this alignment. If we changed it, candle OHLC values would differ from what users see on OANDA's own platform, TradingView with OANDA data, and other charting tools. This would cause confusion when validating backtests or comparing chart patterns.

### Pagination

OANDA limits candle requests to 5000 per call. `get_candles_paginated()` handles larger date ranges by:

1. Fetching 5000 candles from the `from` date (NOT passing `to` to avoid OANDA's date-range rejection)
2. Client-side filtering against the target `to` date
3. Using the last candle's timestamp as the new `from` for the next chunk
4. Deduplicating overlapping candles at chunk boundaries
5. Stopping when a candle exceeds `to` or fewer than 5000 candles are returned

**WHY not pass `to`?** OANDA rejects requests where the date range would exceed 5000 candles. By omitting `to`, we let OANDA return up to 5000 candles from the start point and handle the windowing ourselves.

## Price Streaming Architecture

### PriceStreamManager (`streaming.rs`)

The `PriceStreamManager` maintains a **single SSE connection** to OANDA for all subscribed instruments. It uses a pub/sub pattern:

1. **Subscribe** -- Component calls `subscribe(instrument)`. If the instrument is new, the stream restarts to include it. If already subscribed, the ref count increments.
2. **Unsubscribe** -- Decrements ref count. When it hits zero, removes the instrument and restarts the stream without it.
3. **Stream loop** -- Reads SSE lines, parses `StreamMessage` (Price or Heartbeat), emits Tauri events.

### Reconnection with Exponential Backoff

When the stream disconnects:
- Up to `MAX_RECONNECT_ATTEMPTS` (10) retries
- Initial delay: 1 second, doubles each attempt up to 60 seconds
- Resets to 0 attempts on successful reconnection
- Emits `stream-error` with `Reconnecting` or `MaxReconnectsExceeded` type

### Health Monitoring

A background tokio task checks stream health every 10 seconds:
- **Healthy**: stream is running AND received data within last 30 seconds
- Emits `stream-health` events only on health status **changes** (avoids spamming)
- Uses `AtomicU64` for lock-free timestamp updates from the stream task

### Spread Statistics

The `SpreadStatsCollector` samples spreads every 5 seconds per instrument and batches them to queries-service every 60 seconds via `POST /spread-stats/submit`. This is best-effort -- failed submissions are silently discarded because the data converges over time.

### Frontend Price Flow

Two React hooks consume streaming prices:

1. **`usePriceStream`** (dashboard) -- Subscribes to multiple instruments, stores all prices in a Zustand store (`priceStore.ts`). Used by the main dashboard for position/account displays.

2. **`usePriceStreaming`** (chart) -- Subscribes to a single instrument, directly updates the lightweight-charts candlestick series in real-time. Handles candle boundary detection (new candle vs. update existing) using time-map alignment. Skips streaming when viewing historical data.

Both hooks:
- Set up event listeners BEFORE subscribing (avoids race conditions)
- Use `cancelled` flags for cleanup during async operations
- Call `unsubscribe_from_prices` on unmount

## Trade Sync Pipeline

Trade sync is a non-blocking background operation:

1. **Frontend** calls `sync_trades` Tauri command with user_id, count, data_source
2. **Command** validates auth token and queries-service URL, then spawns a tokio task
3. **Background task**:
   - Emits `sync-progress` events at each stage
   - Fetches open trades from OANDA (`state=OPEN`)
   - Fetches closed trade history from OANDA (`state=CLOSED`)
   - Converts `Trade` domain models to `TradePayload` DTOs (includes OANDA account_id)
   - Sends batch to queries-service via `POST /sync/trades`
   - Emits `sync-complete` on success
4. **Queries-service** handles upserts to PostgreSQL
5. **Zero** syncs PostgreSQL to frontend clients

**WHY queries-service instead of direct DB?** Eliminates `DATABASE_URL` from the desktop binary, improving security. The desktop app never holds database credentials -- all mutations go through the authenticated queries-service.

## Environment Switching (Practice <-> Live)

`switch_oanda_environment` in `commands/oanda.rs`:

1. Reads API key from the credential vault (separate keys for practice/live)
2. Creates a new `OandaClient` with the target environment's base URL
3. Stops all active strategy watchers (they hold stale data for the old environment)
4. Replaces the shared `OandaClient` in `AppState` (behind `RwLock`)

The stream URL also differs:
- Practice: `https://stream-fxpractice.oanda.com`
- Live: `https://stream-fxtrade.oanda.com`

## db.rs Role

`db.rs` provides PostgreSQL query functions. It does NOT manage schema -- all `CREATE TABLE`, `ALTER TABLE`, `ADD COLUMN`, and `CREATE INDEX` statements live in `queries-service/src/migrate.ts`.

Trade-related queries in db.rs:
- `upsert_trade` / `upsert_trades` -- single and batch trade persistence using `ON CONFLICT` upserts
- `delete_user_trades` -- used when switching data sources
- `get_trades` -- returns `TradeRow` structs for AI tools

Trade IDs are user-scoped (`{user_id}:{oanda_trade_id}`) to ensure complete isolation between users even if they connect the same OANDA account.

The batch upsert (`upsert_trades`) uses PostgreSQL `UNNEST` to insert all trades in a single query, dramatically reducing network round-trips for remote databases.

## queries_service.rs Role

`QueriesServiceClient` is an HTTP client that routes mutations through queries-service instead of direct database connections. It provides:

- `sync_trades` -- Bulk trade sync with JWT authentication
- Job tracking (start, progress, complete, fail, cancel)
- AI context queries (strategies, notes, trades, calendar events)

All requests use bearer token authentication and enforce TLS 1.2 minimum.

## Key Invariants

1. **All financial values use `rust_decimal::Decimal`** -- never f64 for prices, amounts, or PL
2. **Candle alignment is dailyAlignment=3, alignmentTimezone=UTC** -- changing this breaks chart consistency
3. **TLS 1.2 minimum** on all HTTP clients (OandaClient, QueriesServiceClient, SpreadStatsCollector)
4. **Single stream** to OANDA -- PriceStreamManager ensures one SSE connection regardless of subscriber count
5. **Trade IDs are user-scoped** -- `{user_id}:{oanda_trade_id}` format in PostgreSQL
6. **NO migrations in db.rs** -- only queries; schema changes go in queries-service/src/migrate.ts
7. **Instrument format** -- Always `XXX_YYY` (e.g., `EUR_USD`), validated by `is_valid_instrument()`
8. **Price precision** -- JPY pairs use 3 decimal places, all others use 5 (`format_price_for_oanda`)

## Known Technical Debt

1. **`format_price_for_oanda` uses f64** -- The price formatting helper in `types.rs` parses to f64 for formatting. This is a small precision risk for the `StopLossOnFill`/`TakeProfitOnFill` prices. Should use `Decimal` with explicit scale setting.

2. **`HistoricalTrade.is_winner` uses f64** -- In `commands/trading.rs`, the `is_winner` field is calculated by parsing `realized_pl` through `f64`. Should compare `Decimal` directly to `Decimal::ZERO`.

3. **Legacy `start()` method** -- `PriceStreamManager::start()` is deprecated but still used via `start_price_stream` command. Should be fully migrated to `subscribe()`/`unsubscribe()`.

4. **Pivot point calculation uses f64** -- `calculate_pivot_points` in `commands/data.rs` converts Decimal to f64 for pivot math. Should use Decimal arithmetic.

5. **Duplicate type definitions** -- `db.rs` and `queries_service.rs` both define `StrategyInfo`, `NoteInfo`, `CalendarEventInfo`, etc. These should be consolidated into a shared module.

6. **`println!` in production** -- `get_oanda_credentials` in `commands/oanda.rs` uses `println!` instead of `tracing`.
