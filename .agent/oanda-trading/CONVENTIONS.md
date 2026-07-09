# OANDA Trading Conventions

## Type Conversion Pattern

Every OANDA API response type has a corresponding domain model with a `From` implementation. This is the single place where string-to-rich-type conversion happens.

```rust
// In models/trade.rs
impl From<OandaTrade> for Trade {
    fn from(oanda: OandaTrade) -> Self {
        let open_price = oanda.price.parse().unwrap_or_default();
        let units: Decimal = oanda.initial_units.parse().unwrap_or_default();
        // ... parse all fields, fallback to safe defaults
        Trade { open_price, units, ... }
    }
}
```

**Rules**:
- Use `parse().unwrap_or_default()` for numeric fields -- returns `Decimal::ZERO` on failure, which is safe for display
- Use `DateTime::parse_from_rfc3339().unwrap_or_else(|_| Utc::now())` for timestamps -- never panic on bad dates
- Use case-insensitive matching for string-to-enum conversions with a safe default variant
- Use `initial_units` (not `current_units`) for trade direction -- `current_units` is 0 for closed trades, which would lose the long/short direction

### Adding a New Domain Model

1. Create the raw type in `oanda/types.rs` with all fields as `String` and `#[serde(rename_all = "camelCase")]`
2. Create the domain model in `models/` with `Decimal` for financials, `DateTime<Utc>` for times, enums for states
3. Implement `From<RawType> for DomainModel` with defensive parsing
4. Re-export from `models/mod.rs`
5. Write tests covering: normal conversion, missing optional fields, invalid strings defaulting to zero

## Adding a New Endpoint

All endpoints follow the same pattern in `endpoints.rs`:

```rust
pub async fn get_something(client: &OandaClient, param: &str) -> Result<DomainType> {
    let url = format!("{}/v3/accounts/{}/something?param={}",
        client.base_url(), client.account_id(), param);
    let response = client.get(&url).send().await?.error_for_status()?;
    let raw: RawResponseType = response.json().await?;
    Ok(raw.items.into_iter().map(DomainType::from).collect())
}
```

**Steps**:
1. Add the raw response type to `types.rs`
2. Add the endpoint function to `endpoints.rs`
3. Add a Tauri command in the appropriate `commands/` file
4. Add a wiremock test in the `endpoints.rs` test module
5. Register the command in `main.rs` invoke_handler

**Error handling**: For order placement (where the response body contains error details even on HTTP 200), use the `parse_response` helper instead of `.json()`:

```rust
let text = response.text().await?;
parse_response::<ResponseType>(&text)
```

This checks for OANDA's `errorMessage`/`errorCode` fields before attempting deserialization.

## Command Layer Conventions

Tauri commands in `commands/` have their own serializable types that convert domain models to frontend-friendly strings:

```rust
// Domain model uses Decimal
pub struct Position {
    pub units: Decimal,
    pub average_price: Decimal,
}

// Command response uses String
pub struct PositionResponse {
    pub units: String,         // p.units.to_string()
    pub average_price: String, // p.average_price.to_string()
}
```

**WHY?** Tauri serializes to JSON, which would lose Decimal precision if we used numeric types. By converting to strings, the frontend receives the exact value the backend computed.

**Input validation**: All commands validate instrument format using `is_valid_instrument()` before calling endpoints. This function checks for `XXX_YYY` format where X and Y are 3-character uppercase ASCII.

## Error Handling

- **Endpoint functions** return `Result<T>` using the crate's error type, which includes `Error::OandaApi(String)` for API-level errors
- **Tauri commands** map errors to `String` via `.map_err(|e| e.to_string())`
- **Streaming** emits `StreamError` events rather than returning errors, because the stream is long-lived
- **Trade sync** emits `sync-progress` events with `stage: "error"` for background task failures

## Price and Amount Naming

| Concept | Naming | Type |
|---------|--------|------|
| Entry price | `open_price` | `Decimal` |
| Exit price | `close_price` | `Option<Decimal>` |
| Current bid/ask | `bid`, `ask` | `String` (in PriceUpdate) |
| Spread | `spread` | `String` (computed as `ask - bid`) |
| Position size | `units` | `Decimal` (positive=long, negative=short) |
| Realized PL | `realized_pl` | `Decimal` |
| Unrealized PL | `unrealized_pl` | `Option<Decimal>` |
| Total PL | `total_pl()` | computed: `realized_pl + unrealized_pl.unwrap_or(0)` |

## Candle Alignment

All candle fetching passes `dailyAlignment=3` and `alignmentTimezone=UTC` to OANDA.

This produces H4 candle boundaries at:
- 03:00, 07:00, 11:00, 15:00, 19:00, 23:00 UTC

**Do NOT change these values.** They must match across:
- Charting (frontend candle display)
- Backtesting (backtest-core candle ingestion)
- Strategy monitoring (watcher candle detection)
- Manual analysis (comparing with OANDA's platform)

If a consumer needs custom alignment (rare), use `get_candles_with_alignment()` instead of `get_candles()`.

## Instrument Format

OANDA uses underscore-separated currency pairs: `EUR_USD`, `GBP_JPY`, `AUD_NZD`.

Validation is done by `is_valid_instrument()`:
- Exactly two parts separated by `_`
- Each part is exactly 3 characters
- All characters are uppercase ASCII

This validation runs in every Tauri command that accepts an instrument parameter.

## OANDA Price Formatting

When sending prices to OANDA (e.g., stop-loss or take-profit), they must be formatted with the correct decimal places:
- **JPY pairs**: 3 decimal places (e.g., `"150.500"`)
- **All others**: 5 decimal places (e.g., `"1.08500"`)

Use `format_price_for_oanda(instrument, price)` from `types.rs`.

## Anti-Patterns

### Never use f64 for financial values
```rust
// WRONG
let price: f64 = 1.08500;
let pl: f64 = trade.realized_pl.parse().unwrap();

// RIGHT
let price: Decimal = Decimal::from_str("1.08500").unwrap();
let pl: Decimal = trade.realized_pl; // already Decimal in domain model
```

### Never hardcode account IDs or API keys
```rust
// WRONG
let account_id = "101-001-12345678-001";

// RIGHT
let account_id = client.account_id(); // from OandaClient config
```

### Never add migrations to db.rs
```rust
// WRONG (in db.rs)
sqlx::query("CREATE TABLE IF NOT EXISTS new_table ...").execute(&self.pool).await?;

// RIGHT (in queries-service/src/migrate.ts)
await sql`CREATE TABLE IF NOT EXISTS new_table ...`.catch(() => {});
```

### Never skip TLS version enforcement
```rust
// WRONG
let client = Client::builder().build()?;

// RIGHT
let client = Client::builder()
    .min_tls_version(tls::Version::TLS_1_2)
    .build()?;
```

### Never use current_units for trade direction
```rust
// WRONG - current_units is 0 for closed trades
let is_long = trade.current_units.parse::<Decimal>().unwrap() > Decimal::ZERO;

// RIGHT - initial_units preserves the original direction
let is_long = trade.initial_units.parse::<Decimal>().unwrap() > Decimal::ZERO;
```

### Never bypass the PriceStreamManager
```rust
// WRONG - starting a direct connection to OANDA streaming
let response = client.get(&streaming_url).send().await?;

// RIGHT - use the centralized manager
streamer.subscribe(instrument, app_handle).await?;
```

## Testing Patterns

### Endpoint Tests

Use `wiremock::MockServer` to test endpoint functions without hitting OANDA:

```rust
#[tokio::test]
async fn test_get_something() {
    let mock_server = MockServer::start().await;
    let client = OandaClient::with_base_url(&mock_server.uri(), "key", "acct").unwrap();

    Mock::given(method("GET"))
        .and(path("/v3/accounts/acct/something"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({...})))
        .mount(&mock_server).await;

    let result = get_something(&client).await.unwrap();
    assert_eq!(result.len(), 1);
}
```

### Model Tests

Test `From` implementations with both valid and invalid input:

```rust
#[test]
fn test_valid_conversion() {
    let trade = Trade::from(make_valid_oanda_trade());
    assert_eq!(trade.open_price, dec!(1.08500));
}

#[test]
fn test_invalid_price_defaults_to_zero() {
    let mut oanda = make_valid_oanda_trade();
    oanda.price = "invalid".to_string();
    let trade = Trade::from(oanda);
    assert_eq!(trade.open_price, dec!(0));
}
```

Run all backend tests with: `npm run test:be`
