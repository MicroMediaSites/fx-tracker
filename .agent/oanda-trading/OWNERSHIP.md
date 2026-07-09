# OANDA Trading Domain

## Domain Description

The `oanda-trading` domain owns the OANDA broker integration for the CandleSight trading application. This includes the REST API client for account data, candle fetching, and trade execution; the SSE price streaming subsystem with reconnection and health monitoring; the two-layer type system (raw OANDA JSON types and rich domain models with `Decimal`/`DateTime`); trade synchronization to PostgreSQL via queries-service; and environment switching between practice and live accounts.

## Owned Files

### Rust Backend - OANDA API Client

```
src-tauri/src/oanda/client.rs        # HTTP client wrapper, bearer auth, TLS 1.2 minimum
src-tauri/src/oanda/endpoints.rs      # REST endpoint functions (trades, candles, positions, orders, instruments, autochartist)
src-tauri/src/oanda/streaming.rs      # PriceStreamManager - SSE streaming, reconnection, health monitoring, spread stats
src-tauri/src/oanda/types.rs          # Raw OANDA API JSON types (OandaTrade, OandaCandle, OandaPosition, etc.)
src-tauri/src/oanda/mod.rs            # Module re-exports
```

### Rust Backend - Domain Models

```
src-tauri/src/models/trade.rs         # Trade domain model (Decimal prices, DateTime timestamps, TradeState enum)
src-tauri/src/models/candle.rs        # Candle/Ohlc domain model (Decimal OHLC, bullish/bearish/range helpers)
src-tauri/src/models/position.rs      # Position domain model (Decimal units/price/PL, flat/long/short helpers)
src-tauri/src/models/order.rs         # Order domain model (OrderType/OrderState enums, Decimal price/units)
src-tauri/src/models/mod.rs           # Model re-exports
```

### Rust Backend - Tauri Commands

```
src-tauri/src/commands/trading.rs     # Trade execution commands (place_order, close_position, get_account, get_positions, get_orders, get_trade_history)
src-tauri/src/commands/data.rs        # Data fetching commands (get_candles, sync_trades, get_indicator_data, fetch_instruments, calculate_pivot_points, fetch_autochartist_signals)
src-tauri/src/commands/streaming.rs   # Stream commands (subscribe_to_prices, unsubscribe_from_prices, start_price_stream, stop_price_stream, is_streaming, get_stream_health)
src-tauri/src/commands/oanda.rs       # Environment commands (switch_oanda_environment, get_oanda_environment, get_oanda_credentials, save_oanda_credentials)
```

### Rust Backend - Data Layer

```
src-tauri/src/queries_service.rs      # HTTP client for queries-service (trade sync, job tracking, AI context)
```

### Frontend - Price Streaming

```
src/stores/priceStore.ts              # Zustand store for live prices (PriceUpdate, StreamError)
src/hooks/usePriceStream.ts           # Multi-instrument price subscription hook (dashboard use)
src/hooks/usePriceStreaming.ts        # Single-instrument chart streaming hook (chart use, updates candlesticks)
```

## Shared Files (Coordination Required)

### db.rs - Shared with all domains

```
src-tauri/src/db.rs                   # PostgreSQL queries. This domain owns trade-related queries (upsert_trade, upsert_trades, delete_user_trades, get_trades).
                                      # Other domains own strategy, note, calendar_event, and backtest_job queries.
                                      # CRITICAL: NO MIGRATIONS - only queries. All schema changes go in queries-service/src/migrate.ts.
```

### config.rs - Shared

```
src-tauri/src/config.rs               # OandaEnvironment enum (Practice/Live) and Config struct.
                                      # This domain consumes it; changes must be coordinated with the auth and startup domains.
```

### error.rs - Shared

```
src-tauri/src/error.rs                # Error enum including Error::OandaApi variant used by endpoints.rs.
```

## Primary Stack

- **Language**: Rust (backend), TypeScript (frontend)
- **HTTP Client**: `reqwest` with TLS 1.2 minimum
- **Async Runtime**: `tokio`
- **Framework**: Tauri 2 (desktop app shell)
- **Serialization**: `serde` / `serde_json`
- **Financial Math**: `rust_decimal` (NEVER f64 for prices/amounts)
- **Date/Time**: `chrono` (DateTime<Utc>)
- **Streaming**: `futures_util::StreamExt`, `tokio_util::io::StreamReader`, `tokio::io::AsyncBufReadExt`
- **Database**: `sqlx` (PostgreSQL, queries only)
- **Frontend State**: Zustand (price store)
- **Frontend Events**: `@tauri-apps/api/event` (listen/emit)

## Key Dependencies

- `reqwest` - HTTP client for both OANDA API and queries-service
- `rust_decimal` + `rust_decimal_macros` - Decimal arithmetic for all financial values
- `chrono` - RFC3339 timestamp parsing and DateTime<Utc> representation
- `serde` + `serde_json` - JSON serialization/deserialization of OANDA API responses
- `tokio` - Async runtime for streaming and concurrent operations
- `futures_util` - Stream combinators for SSE processing
- `tokio_util` - StreamReader adapter for byte stream to line reader conversion
- `sqlx` - PostgreSQL async driver
- `tauri` - AppHandle for event emission, State for shared state
- `wiremock` - Mock HTTP server for endpoint tests
- `tracing` - Structured logging
