use crate::error::{Error, Result};
use crate::models::{Trade, Position, Order, Candle};
use super::client::OandaClient;
use super::types::{TradesResponse, PositionsResponse, OrdersResponse, MarketOrderRequest, EntryOrderRequest, OrderCreateResponse, ClosePositionRequest, ClosePositionResponse, CandlesResponse, AutochartistResponse, InstrumentsResponse, OandaInstrument, OandaBook, OrderBookResponse, PositionBookResponse};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OandaErrorResponse {
    error_message: Option<String>,
    error_code: Option<String>,
}

fn parse_response<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    if let Ok(error) = serde_json::from_str::<OandaErrorResponse>(text) {
        if error.error_message.is_some() || error.error_code.is_some() {
            let msg = error.error_message.unwrap_or_else(|| {
                error.error_code.unwrap_or_else(|| "Unknown API error".to_string())
            });
            return Err(Error::OandaApi(msg));
        }
    }
    serde_json::from_str(text).map_err(Into::into)
}

pub async fn get_trades(
    client: &OandaClient,
    count: Option<u32>,
    instrument: Option<&str>,
    state: Option<&str>,
) -> Result<Vec<Trade>> {
    let mut url = format!(
        "{}/v3/accounts/{}/trades",
        client.base_url(),
        client.account_id()
    );

    let mut query_parts = Vec::new();
    if let Some(c) = count {
        query_parts.push(format!("count={}", c));
    }
    if let Some(inst) = instrument {
        query_parts.push(format!("instrument={}", inst));
    }
    if let Some(s) = state {
        query_parts.push(format!("state={}", s));
    }
    if !query_parts.is_empty() {
        url.push('?');
        url.push_str(&query_parts.join("&"));
    }

    let response = client.get(&url).send().await?.error_for_status()?;
    let trades_response: TradesResponse = response.json().await?;
    Ok(trades_response.trades.into_iter().map(Trade::from).collect())
}

pub async fn get_trade_history(
    client: &OandaClient,
    count: Option<u32>,
    instrument: Option<&str>,
) -> Result<Vec<Trade>> {
    get_trades(client, count, instrument, Some("CLOSED")).await
}

pub async fn get_account(client: &OandaClient) -> Result<super::types::OandaAccount> {
    let url = format!("{}/v3/accounts/{}", client.base_url(), client.account_id());
    let response = client.get(&url).send().await?.error_for_status()?;
    let account_response: super::types::AccountResponse = response.json().await?;
    Ok(account_response.account)
}

/// Fetch all tradeable instruments for the account
pub async fn get_instruments(client: &OandaClient) -> Result<Vec<OandaInstrument>> {
    let url = format!(
        "{}/v3/accounts/{}/instruments",
        client.base_url(),
        client.account_id()
    );
    let response = client.get(&url).send().await?.error_for_status()?;
    let instruments_response: InstrumentsResponse = response.json().await?;
    Ok(instruments_response.instruments)
}

pub async fn get_positions(client: &OandaClient) -> Result<Vec<Position>> {
    let url = format!(
        "{}/v3/accounts/{}/positions",
        client.base_url(),
        client.account_id()
    );
    let response = client.get(&url).send().await?.error_for_status()?;
    let positions_response: PositionsResponse = response.json().await?;
    Ok(positions_response.positions.into_iter().map(Position::from).collect())
}

pub async fn get_open_positions(client: &OandaClient) -> Result<Vec<Position>> {
    let url = format!(
        "{}/v3/accounts/{}/openPositions",
        client.base_url(),
        client.account_id()
    );
    let response = client.get(&url).send().await?.error_for_status()?;
    let positions_response: PositionsResponse = response.json().await?;
    Ok(positions_response.positions.into_iter().map(Position::from).collect())
}

pub async fn get_orders(client: &OandaClient) -> Result<Vec<Order>> {
    // Fetch orders
    let url = format!(
        "{}/v3/accounts/{}/orders",
        client.base_url(),
        client.account_id()
    );
    let response = client.get(&url).send().await?.error_for_status()?;
    let orders_response: OrdersResponse = response.json().await?;

    // Fetch open trades to build trade_id -> instrument lookup
    // This is needed because STOP_LOSS and TAKE_PROFIT orders don't have
    // an instrument field - they have trade_id instead
    let trades = get_trades(client, None, None, Some("OPEN")).await?;
    let trade_instrument_map: std::collections::HashMap<String, String> = trades
        .into_iter()
        .map(|t| (t.id.clone(), t.instrument.clone()))
        .collect();

    // Convert orders, enriching with trade instrument when needed
    let orders = orders_response.orders.into_iter().map(|oanda_order| {
        // If instrument is missing but trade_id is present, look it up
        let resolved_instrument = oanda_order.instrument.clone().or_else(|| {
            oanda_order.trade_id.as_ref().and_then(|tid| {
                trade_instrument_map.get(tid).cloned()
            })
        });

        // Create enriched order with resolved instrument
        let enriched = super::types::OandaOrder {
            instrument: resolved_instrument,
            ..oanda_order
        };
        Order::from(enriched)
    }).collect();

    Ok(orders)
}

pub async fn place_market_order(
    client: &OandaClient,
    instrument: &str,
    units: i64,
) -> Result<OrderCreateResponse> {
    place_market_order_with_sl_tp(client, instrument, units, None, None).await
}

pub async fn place_market_order_with_sl_tp(
    client: &OandaClient,
    instrument: &str,
    units: i64,
    stop_loss: Option<&str>,
    take_profit: Option<&str>,
) -> Result<OrderCreateResponse> {
    place_market_order_attributed(client, instrument, units, stop_loss, take_profit, None).await
}

/// Place a market order with SL/TP and optional strategy attribution
/// (AGT-630, AC1). When `strategy` is `Some`, the POST body carries OANDA
/// `clientExtensions` (tag = the strategy name) so the broker's transaction
/// record itself names the strategy that placed the order; `None` produces a
/// body identical to [`place_market_order_with_sl_tp`] (which delegates here).
pub async fn place_market_order_attributed(
    client: &OandaClient,
    instrument: &str,
    units: i64,
    stop_loss: Option<&str>,
    take_profit: Option<&str>,
    strategy: Option<&str>,
) -> Result<OrderCreateResponse> {
    let url = format!(
        "{}/v3/accounts/{}/orders",
        client.base_url(),
        client.account_id()
    );
    let order_request = MarketOrderRequest::with_sl_tp(instrument, units, stop_loss, take_profit)
        .with_strategy(strategy);
    let response = client.post(&url).json(&order_request).send().await?;
    let text = response.text().await?;
    parse_response(&text)
}

/// Place a resting Limit or Stop *entry* order (AGT-612, AC2). POSTs to the
/// same `/orders` endpoint the market path uses — the only difference is the
/// request body ([`EntryOrderRequest`]), which carries a trigger `price`, a
/// default `GTC` time-in-force, and the optional `gtdTime`/`priceBound`/
/// `triggerCondition` fields. The caller (the guarded `execute_place` path)
/// builds the fully-formed request so all price fields are already precision-
/// formatted before they reach OANDA.
pub async fn place_entry_order(
    client: &OandaClient,
    request: &EntryOrderRequest,
) -> Result<OrderCreateResponse> {
    let url = format!(
        "{}/v3/accounts/{}/orders",
        client.base_url(),
        client.account_id()
    );
    let response = client.post(&url).json(request).send().await?;
    let text = response.text().await?;
    parse_response(&text)
}

pub async fn close_position(
    client: &OandaClient,
    instrument: &str,
    is_long: bool,
) -> Result<ClosePositionResponse> {
    let url = format!(
        "{}/v3/accounts/{}/positions/{}/close",
        client.base_url(),
        client.account_id(),
        instrument
    );
    let close_request = if is_long {
        ClosePositionRequest::close_long()
    } else {
        ClosePositionRequest::close_short()
    };
    let response = client.put(&url).json(&close_request).send().await?;
    let text = response.text().await?;
    parse_response(&text)
}

/// Granularity options for candles
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Granularity {
    S5, S10, S15, S30,
    M1, M2, M4, M5, M10, M15, M30,
    H1, H2, H3, H4, H6, H8, H12,
    D, W, M,
}

impl std::fmt::Display for Granularity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Granularity::S5 => write!(f, "S5"),
            Granularity::S10 => write!(f, "S10"),
            Granularity::S15 => write!(f, "S15"),
            Granularity::S30 => write!(f, "S30"),
            Granularity::M1 => write!(f, "M1"),
            Granularity::M2 => write!(f, "M2"),
            Granularity::M4 => write!(f, "M4"),
            Granularity::M5 => write!(f, "M5"),
            Granularity::M10 => write!(f, "M10"),
            Granularity::M15 => write!(f, "M15"),
            Granularity::M30 => write!(f, "M30"),
            Granularity::H1 => write!(f, "H1"),
            Granularity::H2 => write!(f, "H2"),
            Granularity::H3 => write!(f, "H3"),
            Granularity::H4 => write!(f, "H4"),
            Granularity::H6 => write!(f, "H6"),
            Granularity::H8 => write!(f, "H8"),
            Granularity::H12 => write!(f, "H12"),
            Granularity::D => write!(f, "D"),
            Granularity::W => write!(f, "W"),
            Granularity::M => write!(f, "M"),
        }
    }
}

impl std::str::FromStr for Granularity {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "S5" => Ok(Granularity::S5),
            "S10" => Ok(Granularity::S10),
            "S15" => Ok(Granularity::S15),
            "S30" => Ok(Granularity::S30),
            "M1" => Ok(Granularity::M1),
            "M2" => Ok(Granularity::M2),
            "M4" => Ok(Granularity::M4),
            "M5" => Ok(Granularity::M5),
            "M10" => Ok(Granularity::M10),
            "M15" => Ok(Granularity::M15),
            "M30" => Ok(Granularity::M30),
            "H1" => Ok(Granularity::H1),
            "H2" => Ok(Granularity::H2),
            "H3" => Ok(Granularity::H3),
            "H4" => Ok(Granularity::H4),
            "H6" => Ok(Granularity::H6),
            "H8" => Ok(Granularity::H8),
            "H12" => Ok(Granularity::H12),
            "D" => Ok(Granularity::D),
            "W" => Ok(Granularity::W),
            "M" => Ok(Granularity::M),
            _ => Err(Error::InvalidArgument(format!("Invalid granularity: {}", s))),
        }
    }
}

/// Default timezone for candle alignment
pub const DEFAULT_ALIGNMENT_TIMEZONE: &str = "UTC";

/// Default daily alignment hour (2 = 2am UTC)
/// This gives H4 candles at 02:00, 06:00, 10:00, 14:00, 18:00, 22:00 UTC
/// Matches OANDA's platform candle boundaries.
pub const DEFAULT_DAILY_ALIGNMENT: u8 = 2;

/// Fetch historical candles for an instrument
///
/// # Arguments
/// * `instrument` - The currency pair (e.g., "EUR_USD")
/// * `granularity` - The time period for each candle
/// * `count` - Number of candles to fetch (max 5000)
/// * `from` - Start time (RFC3339 format, optional)
/// * `to` - End time (RFC3339 format, optional)
pub async fn get_candles(
    client: &OandaClient,
    instrument: &str,
    granularity: Granularity,
    count: Option<u32>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<Candle>> {
    get_candles_with_alignment(
        client,
        instrument,
        granularity,
        count,
        from,
        to,
        DEFAULT_ALIGNMENT_TIMEZONE,
        DEFAULT_DAILY_ALIGNMENT,
    ).await
}

/// Fetch historical candles for an instrument with custom timezone alignment
///
/// # Arguments
/// * `instrument` - The currency pair (e.g., "EUR_USD")
/// * `granularity` - The time period for each candle
/// * `count` - Number of candles to fetch (max 5000)
/// * `from` - Start time (RFC3339 format, optional)
/// * `to` - End time (RFC3339 format, optional)
/// * `alignment_timezone` - Timezone for candle alignment (e.g., "America/New_York", "UTC")
/// * `daily_alignment` - Hour of day (0-23) for daily alignment in the specified timezone
pub async fn get_candles_with_alignment(
    client: &OandaClient,
    instrument: &str,
    granularity: Granularity,
    count: Option<u32>,
    from: Option<&str>,
    to: Option<&str>,
    alignment_timezone: &str,
    daily_alignment: u8,
) -> Result<Vec<Candle>> {
    // URL-encode the timezone (America/New_York -> America%2FNew_York)
    let encoded_tz = alignment_timezone.replace('/', "%2F");
    let mut url = format!(
        "{}/v3/instruments/{}/candles?granularity={}&price=M&alignmentTimezone={}&dailyAlignment={}",
        client.base_url(),
        instrument,
        granularity,
        encoded_tz,
        daily_alignment
    );

    tracing::debug!("Fetching candles with URL: {}", url);

    // OANDA API rules:
    // - If both from and to are specified, don't include count
    // - If only from is specified, count limits results forward from that date
    // - If only count is specified, returns most recent N candles
    let has_from_and_to = from.is_some() && to.is_some();

    if let Some(c) = count {
        if !has_from_and_to {
            url.push_str(&format!("&count={}", c.min(5000)));
        }
        // When both from and to are set, skip count - the date range defines the data
    }
    if let Some(f) = from {
        url.push_str(&format!("&from={}", f));
    }
    if let Some(t) = to {
        url.push_str(&format!("&to={}", t));
    }

    let response = client.get(&url).send().await?.error_for_status()?;
    let candles_response: CandlesResponse = response.json().await?;
    Ok(candles_response.candles.into_iter().map(Candle::from).collect())
}

/// Fetch historical candles with automatic pagination for large date ranges.
/// OANDA limits requests to 5000 candles, so this function fetches in chunks.
///
/// NOTE: We don't pass `to` to the OANDA API because OANDA rejects requests
/// where the date range would exceed 5000 candles. Instead, we fetch 5000
/// candles at a time from `from` and filter client-side.
pub async fn get_candles_paginated(
    client: &OandaClient,
    instrument: &str,
    granularity: Granularity,
    from: &str,
    to: &str,
) -> Result<Vec<Candle>> {
    const MAX_CANDLES_PER_REQUEST: u32 = 5000;
    let mut all_candles: Vec<Candle> = Vec::new();
    let mut current_from = from.to_string();

    // Parse the end date for client-side filtering
    let to_datetime = chrono::DateTime::parse_from_rfc3339(to)
        .map_err(|e| Error::InvalidArgument(format!("Invalid 'to' date format: {}", e)))?
        .with_timezone(&chrono::Utc);

    loop {
        tracing::info!(
            "[Pagination] Fetching candles from {} (target end: {})",
            current_from, to
        );

        // Fetch a chunk - don't pass 'to' to avoid OANDA's date range rejection
        let chunk = get_candles(
            client,
            instrument,
            granularity,
            Some(MAX_CANDLES_PER_REQUEST),
            Some(&current_from),
            None, // Don't pass 'to' - filter client-side
        ).await?;

        if chunk.is_empty() {
            break;
        }

        let chunk_len = chunk.len();
        let mut reached_end = false;

        // Filter and add candles that are within our target range
        for candle in chunk {
            // Skip duplicates from overlapping requests
            if let Some(last) = all_candles.last() {
                if candle.time == last.time {
                    continue;
                }
            }

            // Stop if we've passed the end date
            if candle.time > to_datetime {
                tracing::info!(
                    "[Pagination] Reached end date at candle {}",
                    candle.time
                );
                reached_end = true;
                break;
            }

            all_candles.push(candle);
        }

        // If we hit a candle past the end date, we're done
        if reached_end {
            break;
        }

        // If we got fewer than max, we've reached the end of available data
        if chunk_len < MAX_CANDLES_PER_REQUEST as usize {
            break;
        }

        // Use the last candle's time as the new 'from' for next request
        if let Some(last) = all_candles.last() {
            current_from = last.time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        } else {
            break;
        }
    }

    tracing::info!(
        "[Pagination] Fetched {} total candles for {} from {} to {}",
        all_candles.len(), instrument, from, to
    );

    Ok(all_candles)
}

/// Shared fetch for the two instrument book endpoints. OANDA publishes book
/// snapshots on 20-minute boundaries; `time` (RFC3339) selects a historical
/// snapshot, `None` returns the most recent one. A `time` that is not an
/// exact snapshot boundary, or predates retention (~2018), comes back as an
/// OANDA "snapshot does not exist" error via [`parse_response`].
async fn get_book(
    client: &OandaClient,
    instrument: &str,
    kind: &str,
    time: Option<&str>,
) -> Result<String> {
    let mut url = format!(
        "{}/v3/instruments/{}/{}",
        client.base_url(),
        instrument,
        kind
    );
    if let Some(t) = time {
        url.push_str(&format!("?time={}", t));
    }
    // No error_for_status(): OANDA answers a missing/misaligned snapshot with
    // 404/400 plus an errorMessage body — parse_response surfaces that message
    // instead of a bare HTTP status.
    let response = client.get(&url).send().await?;
    Ok(response.text().await?)
}

/// Fetch the client **order book** (pending orders bucketed by price) for an
/// instrument — current snapshot, or the one at `time` when given.
pub async fn get_order_book(
    client: &OandaClient,
    instrument: &str,
    time: Option<&str>,
) -> Result<OandaBook> {
    let text = get_book(client, instrument, "orderBook", time).await?;
    let parsed: OrderBookResponse = parse_response(&text)?;
    Ok(parsed.order_book)
}

/// Fetch the client **position book** (open positions bucketed by price) for
/// an instrument — current snapshot, or the one at `time` when given.
pub async fn get_position_book(
    client: &OandaClient,
    instrument: &str,
    time: Option<&str>,
) -> Result<OandaBook> {
    let text = get_book(client, instrument, "positionBook", time).await?;
    let parsed: PositionBookResponse = parse_response(&text)?;
    Ok(parsed.position_book)
}

/// Fetch Autochartist support/resistance signals from ForexLabs API
///
/// # Arguments
/// * `client` - The OANDA client
/// * `instrument` - The currency pair (e.g., "EUR_USD")
///
/// # Returns
/// Autochartist response containing detected patterns with support/resistance levels
pub async fn get_autochartist_signals(
    client: &OandaClient,
    instrument: &str,
) -> Result<AutochartistResponse> {
    // ForexLabs API uses a slightly different URL structure
    // The base URL should be the same as the REST API base
    let base_url = client.base_url();
    let url = format!(
        "{}/labs/v1/signal/autochartist?instrument={}",
        base_url.trim_end_matches("/v3"),
        instrument
    );

    tracing::info!("[Autochartist] base_url={}, final_url={}", base_url, url);

    let response = client.get(&url).send().await?.error_for_status()?;
    let text = response.text().await?;
    parse_response(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oanda::types::{EntryOptions, EntryOrderType};
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path, query_param};
    use serde_json::json;

    async fn setup_mock_client(mock_server: &MockServer) -> OandaClient {
        OandaClient::with_base_url(&mock_server.uri(), "test-api-key", "test-account-123")
            .expect("Failed to create test client")
    }

    fn mock_trades_response() -> serde_json::Value {
        json!({
            "trades": [
                {
                    "id": "12345",
                    "instrument": "EUR_USD",
                    "price": "1.08500",
                    "openTime": "2024-01-15T10:30:00.000000000Z",
                    "initialUnits": "1000",
                    "currentUnits": "1000",
                    "realizedPL": "0.0000",
                    "unrealizedPL": "25.5000",
                    "state": "OPEN",
                    "financing": "-0.5000"
                },
                {
                    "id": "12346",
                    "instrument": "GBP_USD",
                    "price": "1.26000",
                    "openTime": "2024-01-14T09:00:00.000000000Z",
                    "initialUnits": "-500",
                    "currentUnits": "-500",
                    "realizedPL": "0.0000",
                    "unrealizedPL": "-10.2500",
                    "state": "OPEN",
                    "financing": "-0.2500"
                }
            ],
            "lastTransactionID": "99999"
        })
    }

    fn mock_positions_response() -> serde_json::Value {
        json!({
            "positions": [
                {
                    "instrument": "EUR_USD",
                    "pl": "150.2500",
                    "unrealizedPL": "25.0000",
                    "long": {
                        "units": "5000",
                        "averagePrice": "1.08500",
                        "pl": "150.2500",
                        "unrealizedPL": "25.0000"
                    },
                    "short": {
                        "units": "0"
                    }
                }
            ],
            "lastTransactionID": "99999"
        })
    }

    fn mock_orders_response() -> serde_json::Value {
        json!({
            "orders": [
                {
                    "id": "54321",
                    "createTime": "2024-01-15T12:00:00.000000000Z",
                    "type": "LIMIT",
                    "instrument": "EUR_USD",
                    "units": "2000",
                    "state": "PENDING",
                    "price": "1.08000",
                    "timeInForce": "GTC",
                    "triggerCondition": "DEFAULT"
                }
            ],
            "lastTransactionID": "99999"
        })
    }

    fn mock_account_response() -> serde_json::Value {
        json!({
            "account": {
                "id": "test-account-123",
                "currency": "USD",
                "balance": "10000.0000",
                "NAV": "10025.5000",
                "unrealizedPL": "25.5000",
                "openTradeCount": 2
            }
        })
    }

    #[tokio::test]
    async fn test_get_trades_success() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_trades_response()))
            .mount(&mock_server)
            .await;

        let trades = get_trades(&client, None, None, None).await.unwrap();

        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].id, "12345");
        assert_eq!(trades[0].instrument, "EUR_USD");
        assert_eq!(trades[1].id, "12346");
        assert_eq!(trades[1].instrument, "GBP_USD");
    }

    #[tokio::test]
    async fn test_get_trades_with_count() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .and(query_param("count", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_trades_response()))
            .mount(&mock_server)
            .await;

        let trades = get_trades(&client, Some(10), None, None).await.unwrap();
        assert_eq!(trades.len(), 2);
    }

    #[tokio::test]
    async fn test_get_trades_with_instrument_filter() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let single_trade = json!({
            "trades": [{
                "id": "12345",
                "instrument": "EUR_USD",
                "price": "1.08500",
                "openTime": "2024-01-15T10:30:00.000000000Z",
                "initialUnits": "1000",
                "currentUnits": "1000",
                "realizedPL": "0.0000",
                "unrealizedPL": "25.5000",
                "state": "OPEN"
            }],
            "lastTransactionID": "99999"
        });

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .and(query_param("instrument", "EUR_USD"))
            .respond_with(ResponseTemplate::new(200).set_body_json(single_trade))
            .mount(&mock_server)
            .await;

        let trades = get_trades(&client, None, Some("EUR_USD"), None).await.unwrap();
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].instrument, "EUR_USD");
    }

    #[tokio::test]
    async fn test_get_trade_history_uses_closed_state() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let closed_trades = json!({
            "trades": [{
                "id": "12346",
                "instrument": "GBP_USD",
                "price": "1.26000",
                "openTime": "2024-01-14T09:00:00.000000000Z",
                "initialUnits": "-500",
                "currentUnits": "0",
                "realizedPL": "-15.2500",
                "state": "CLOSED",
                "closeTime": "2024-01-15T14:00:00.000000000Z",
                "averageClosePrice": "1.26305"
            }],
            "lastTransactionID": "99999"
        });

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .and(query_param("state", "CLOSED"))
            .respond_with(ResponseTemplate::new(200).set_body_json(closed_trades))
            .mount(&mock_server)
            .await;

        let trades = get_trade_history(&client, None, None).await.unwrap();
        assert_eq!(trades.len(), 1);
    }

    #[tokio::test]
    async fn test_get_positions_success() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/positions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_positions_response()))
            .mount(&mock_server)
            .await;

        let positions = get_positions(&client).await.unwrap();

        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].instrument, "EUR_USD");
    }

    #[tokio::test]
    async fn test_get_open_positions_success() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/openPositions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_positions_response()))
            .mount(&mock_server)
            .await;

        let positions = get_open_positions(&client).await.unwrap();

        assert_eq!(positions.len(), 1);
    }

    #[tokio::test]
    async fn test_get_orders_success() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // Mock the orders endpoint
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/orders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_orders_response()))
            .mount(&mock_server)
            .await;

        // Mock the trades endpoint (needed for SL/TP instrument lookup)
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .and(query_param("state", "OPEN"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_trades_response()))
            .mount(&mock_server)
            .await;

        let orders = get_orders(&client).await.unwrap();

        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].id, "54321");
    }

    #[tokio::test]
    async fn test_get_orders_enriches_sl_tp_with_trade_instrument() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // Orders response with a STOP_LOSS order that has trade_id but no instrument
        let orders_with_sl = json!({
            "orders": [
                {
                    "id": "54322",
                    "createTime": "2024-01-15T12:30:00.000000000Z",
                    "type": "STOP_LOSS",
                    "state": "PENDING",
                    "price": "1.07500",
                    "timeInForce": "GTC",
                    "triggerCondition": "DEFAULT",
                    "tradeID": "12345"
                }
            ],
            "lastTransactionID": "99999"
        });

        // Trades response with the trade that the stop loss is attached to
        let trades_with_matching_id = json!({
            "trades": [{
                "id": "12345",
                "instrument": "EUR_USD",
                "price": "1.08500",
                "openTime": "2024-01-15T10:30:00.000000000Z",
                "initialUnits": "1000",
                "currentUnits": "1000",
                "realizedPL": "0.0000",
                "unrealizedPL": "25.5000",
                "state": "OPEN"
            }],
            "lastTransactionID": "99999"
        });

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/orders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(orders_with_sl))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .and(query_param("state", "OPEN"))
            .respond_with(ResponseTemplate::new(200).set_body_json(trades_with_matching_id))
            .mount(&mock_server)
            .await;

        let orders = get_orders(&client).await.unwrap();

        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].id, "54322");
        // The instrument should be resolved from the trade
        assert_eq!(orders[0].instrument, "EUR_USD");
    }

    #[tokio::test]
    async fn test_get_account_success() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_account_response()))
            .mount(&mock_server)
            .await;

        let account = get_account(&client).await.unwrap();

        assert_eq!(account.id, "test-account-123");
        assert_eq!(account.currency, "USD");
        assert_eq!(account.balance, "10000.0000");
    }

    #[tokio::test]
    async fn test_place_market_order_success() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let order_response = json!({
            "orderCreateTransaction": {
                "id": "12347",
                "time": "2024-01-15T13:00:00.000000000Z",
                "type": "MARKET_ORDER",
                "instrument": "EUR_USD",
                "units": "1000",
                "timeInForce": "FOK",
                "positionFill": "DEFAULT"
            },
            "orderFillTransaction": {
                "id": "12348",
                "time": "2024-01-15T13:00:00.000000000Z",
                "type": "ORDER_FILL",
                "instrument": "EUR_USD",
                "units": "1000",
                "price": "1.08550",
                "pl": "0.0000",
                "financing": "0.0000",
                "commission": "0.0000",
                "accountBalance": "10000.0000",
                "tradeOpened": {
                    "tradeID": "12349",
                    "units": "1000"
                }
            },
            "relatedTransactionIDs": ["12347", "12348"],
            "lastTransactionID": "12348"
        });

        Mock::given(method("POST"))
            .and(path("/v3/accounts/test-account-123/orders"))
            .respond_with(ResponseTemplate::new(201).set_body_json(order_response))
            .mount(&mock_server)
            .await;

        let result = place_market_order(&client, "EUR_USD", 1000).await.unwrap();

        assert!(result.order_fill_transaction.is_some());
        let fill = result.order_fill_transaction.unwrap();
        assert_eq!(fill.price, "1.08550");
    }

    // AGT-630 (AC1): the attributed market path POSTs a body whose
    // clientExtensions carry the strategy name — the mock matches on the
    // partial body, so this proves the tag/comment actually go over the wire.
    #[tokio::test]
    async fn test_place_market_order_attributed_sends_client_extensions() {
        use wiremock::matchers::body_partial_json;

        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let order_response = json!({
            "orderCreateTransaction": {
                "id": "14001",
                "time": "2026-07-05T13:00:00.000000000Z",
                "type": "MARKET_ORDER",
                "instrument": "EUR_USD",
                "units": "1000",
                "timeInForce": "FOK",
                "positionFill": "DEFAULT"
            },
            "lastTransactionID": "14001"
        });

        Mock::given(method("POST"))
            .and(path("/v3/accounts/test-account-123/orders"))
            .and(body_partial_json(json!({
                "order": {
                    "clientExtensions": {
                        "tag": "ma-crossover",
                        "comment": "wickd strategy=ma-crossover"
                    }
                }
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(order_response))
            .expect(1)
            .mount(&mock_server)
            .await;

        // Without the clientExtensions in the body, the matcher above would not
        // match and this call would fail — so a passing unwrap IS the assertion.
        let result = place_market_order_attributed(
            &client,
            "EUR_USD",
            1000,
            None,
            None,
            Some("ma-crossover"),
        )
        .await
        .unwrap();
        assert!(result.order_create_transaction.is_some());
    }

    #[tokio::test]
    async fn test_place_market_order_error() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let error_response = json!({
            "errorMessage": "Insufficient funds",
            "errorCode": "INSUFFICIENT_MARGIN"
        });

        Mock::given(method("POST"))
            .and(path("/v3/accounts/test-account-123/orders"))
            .respond_with(ResponseTemplate::new(400).set_body_json(error_response))
            .mount(&mock_server)
            .await;

        let result = place_market_order(&client, "EUR_USD", 1000000).await;
        assert!(result.is_err());
    }

    // AGT-612 (AC2/AC3): a resting LIMIT entry POSTs to /orders and comes back
    // with only an orderCreateTransaction (no fill) — the "accepted, working"
    // shape a limit/stop order returns before its trigger is hit.
    #[tokio::test]
    async fn test_place_entry_order_limit_rests() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let resting_response = json!({
            "orderCreateTransaction": {
                "id": "13001",
                "time": "2026-07-01T13:00:00.000000000Z",
                "type": "LIMIT_ORDER",
                "instrument": "EUR_USD",
                "units": "1000",
                "timeInForce": "GTC",
                "positionFill": "DEFAULT"
            },
            "relatedTransactionIDs": ["13001"],
            "lastTransactionID": "13001"
        });

        Mock::given(method("POST"))
            .and(path("/v3/accounts/test-account-123/orders"))
            .respond_with(ResponseTemplate::new(201).set_body_json(resting_response))
            .mount(&mock_server)
            .await;

        let req =
            EntryOrderRequest::new(EntryOrderType::Limit, "EUR_USD", 1000, "1.07500", &EntryOptions::default());
        let result = place_entry_order(&client, &req).await.unwrap();

        // Resting: created, but neither filled nor cancelled nor rejected.
        assert!(result.order_create_transaction.is_some());
        assert!(result.order_fill_transaction.is_none());
        assert!(result.order_cancel_transaction.is_none());
        assert!(result.order_reject_transaction.is_none());
    }

    // AGT-612 (AC3): a hard reject comes back as an orderRejectTransaction — the
    // new response field the classifier reads to distinguish a hard reject from
    // a resting order.
    #[tokio::test]
    async fn test_place_entry_order_hard_reject() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let reject_response = json!({
            "orderRejectTransaction": {
                "id": "13002",
                "time": "2026-07-01T13:01:00.000000000Z",
                "type": "LIMIT_ORDER_REJECT",
                "rejectReason": "PRICE_PRECISION_EXCEEDED"
            },
            "lastTransactionID": "13002"
        });

        Mock::given(method("POST"))
            .and(path("/v3/accounts/test-account-123/orders"))
            .respond_with(ResponseTemplate::new(201).set_body_json(reject_response))
            .mount(&mock_server)
            .await;

        let req =
            EntryOrderRequest::new(EntryOrderType::Stop, "EUR_USD", 1000, "1.09000", &EntryOptions::default());
        let result = place_entry_order(&client, &req).await.unwrap();

        let reject = result.order_reject_transaction.expect("hard reject txn present");
        assert_eq!(reject.cause(), "PRICE_PRECISION_EXCEEDED");
        assert!(result.order_fill_transaction.is_none());
        assert!(result.order_create_transaction.is_none());
    }

    fn mock_book_body(root: &str) -> serde_json::Value {
        json!({
            root: {
                "instrument": "EUR_USD",
                "time": "2026-07-11T18:00:00Z",
                "unixTime": "1783792800",
                "price": "1.14150",
                "bucketWidth": "0.00050",
                "buckets": [
                    {"price": "1.14100", "longCountPercent": "0.6722", "shortCountPercent": "0.5418"},
                    {"price": "1.14150", "longCountPercent": "0.1630", "shortCountPercent": "0.1505"}
                ]
            }
        })
    }

    #[tokio::test]
    async fn test_get_order_book_success() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/instruments/EUR_USD/orderBook"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_book_body("orderBook")))
            .mount(&mock_server)
            .await;

        let book = get_order_book(&client, "EUR_USD", None).await.unwrap();
        assert_eq!(book.instrument, "EUR_USD");
        assert_eq!(book.time, "2026-07-11T18:00:00Z");
        assert_eq!(book.bucket_width, "0.00050");
        assert_eq!(book.buckets.len(), 2);
        assert_eq!(book.buckets[0].long_count_percent, "0.6722");
    }

    #[tokio::test]
    async fn test_get_position_book_with_time_param() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/instruments/EUR_USD/positionBook"))
            .and(query_param("time", "2023-01-03T12:00:00Z"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_book_body("positionBook")))
            .mount(&mock_server)
            .await;

        let book = get_position_book(&client, "EUR_USD", Some("2023-01-03T12:00:00Z"))
            .await
            .unwrap();
        assert_eq!(book.instrument, "EUR_USD");
        assert_eq!(book.buckets.len(), 2);
    }

    // Older historical snapshots omit `unixTime`; the model must not require it.
    #[tokio::test]
    async fn test_get_order_book_historical_without_unix_time() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let body = json!({
            "orderBook": {
                "instrument": "EUR_USD",
                "time": "2018-06-01T12:00:00Z",
                "price": "1.16740",
                "bucketWidth": "0.00050",
                "buckets": []
            }
        });
        Mock::given(method("GET"))
            .and(path("/v3/instruments/EUR_USD/orderBook"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&mock_server)
            .await;

        let book = get_order_book(&client, "EUR_USD", None).await.unwrap();
        assert_eq!(book.time, "2018-06-01T12:00:00Z");
        assert!(book.buckets.is_empty());
    }

    // OANDA's "snapshot does not exist" error surfaces as Error::OandaApi.
    #[tokio::test]
    async fn test_get_order_book_missing_snapshot_error() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let body = json!({
            "errorMessage": "The snapshot for EUR_USD does not exist at the given time 2016-06-01T12:00:00Z."
        });
        Mock::given(method("GET"))
            .and(path("/v3/instruments/EUR_USD/orderBook"))
            .respond_with(ResponseTemplate::new(404).set_body_json(body))
            .mount(&mock_server)
            .await;

        let result = get_order_book(&client, "EUR_USD", Some("2016-06-01T12:00:00Z")).await;
        let err = result.unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[tokio::test]
    async fn test_close_position_success() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let close_response = json!({
            "longOrderFillTransaction": {
                "id": "12350",
                "time": "2024-01-15T14:00:00.000000000Z",
                "type": "ORDER_FILL",
                "instrument": "EUR_USD",
                "units": "-1000",
                "price": "1.08600",
                "pl": "5.0000",
                "financing": "-0.5000",
                "commission": "0.0000",
                "accountBalance": "10005.0000"
            },
            "relatedTransactionIDs": ["12350"],
            "lastTransactionID": "12350"
        });

        Mock::given(method("PUT"))
            .and(path("/v3/accounts/test-account-123/positions/EUR_USD/close"))
            .respond_with(ResponseTemplate::new(200).set_body_json(close_response))
            .mount(&mock_server)
            .await;

        let result = close_position(&client, "EUR_USD", true).await.unwrap();

        assert!(result.long_order_fill_transaction.is_some());
        let fill = result.long_order_fill_transaction.unwrap();
        assert_eq!(fill.pl, "5.0000");
    }

    #[tokio::test]
    async fn test_close_position_error() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let error_response = json!({
            "errorMessage": "No open position for EUR_USD",
            "errorCode": "NO_SUCH_POSITION"
        });

        Mock::given(method("PUT"))
            .and(path("/v3/accounts/test-account-123/positions/EUR_USD/close"))
            .respond_with(ResponseTemplate::new(400).set_body_json(error_response))
            .mount(&mock_server)
            .await;

        let result = close_position(&client, "EUR_USD", true).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_api_http_error() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let result = get_trades(&client, None, None, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_api_invalid_json_response() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/positions"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not valid json"))
            .mount(&mock_server)
            .await;

        let result = get_positions(&client).await;
        assert!(result.is_err());
    }
}
