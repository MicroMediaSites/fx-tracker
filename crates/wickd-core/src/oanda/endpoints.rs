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

/// One page of closed trades older than `before_id` (OANDA `beforeID`), newest
/// first. `None` starts at the newest trade. This is the paging primitive
/// behind [`get_closed_trades_since`].
///
/// `before_id` is a `u64`, not a string, deliberately: it is interpolated into
/// the query string, and a numeric type makes query injection impossible by
/// construction rather than by the caller remembering to sanitise. OANDA trade
/// ids are numeric, so nothing is lost.
pub async fn get_trades_before(
    client: &OandaClient,
    count: Option<u32>,
    before_id: Option<u64>,
) -> Result<Vec<Trade>> {
    let mut url = format!(
        "{}/v3/accounts/{}/trades",
        client.base_url(),
        client.account_id()
    );

    let mut query_parts = vec!["state=CLOSED".to_string()];
    if let Some(c) = count {
        query_parts.push(format!("count={}", c));
    }
    if let Some(b) = before_id {
        query_parts.push(format!("beforeID={}", b));
    }
    url.push('?');
    url.push_str(&query_parts.join("&"));

    let response = client.get(&url).send().await?.error_for_status()?;
    let trades_response: TradesResponse = response.json().await?;
    Ok(trades_response.trades.into_iter().map(Trade::from).collect())
}

/// Outcome of a paged history walk.
#[derive(Debug, Clone)]
pub struct PagedHistory {
    /// Closed trades, newest first, across every page fetched.
    pub trades: Vec<Trade>,
    /// How many requests were issued (1 per page).
    pub pages: usize,
    /// True when the walk stopped at `max_pages` with history still unread —
    /// i.e. the result does NOT reach back to `oldest_wanted`. Callers must
    /// surface this rather than let a partial history read as complete.
    pub truncated: bool,
}

/// Walk closed-trade history backwards until `oldest_wanted` is covered.
///
/// OANDA caps `/trades?count` at 500 and returns newest-first, so a single
/// request cannot reach an old baseline on a busy account (a 30-trades/day
/// watcher outgrows 500 in under three weeks). This pages with `beforeID`
/// until one of three stops:
///
///  1. a page contains a trade closed at/before `oldest_wanted` — covered;
///  2. a short or empty page — the account's history is exhausted;
///  3. `max_pages` — reported as `truncated`, never silently.
///
/// Trades are returned unfiltered (the caller applies the exact window), so the
/// final page may reach slightly past `oldest_wanted`.
pub async fn get_closed_trades_since(
    client: &OandaClient,
    oldest_wanted: Option<chrono::DateTime<chrono::Utc>>,
    page_size: u32,
    max_pages: usize,
) -> Result<PagedHistory> {
    let mut all: Vec<Trade> = Vec::new();
    let mut before_id: Option<u64> = None;
    let mut pages = 0usize;

    while pages < max_pages {
        let page = get_trades_before(client, Some(page_size), before_id).await?;
        pages += 1;
        let page_len = page.len();

        // The next page starts below the lowest id seen. Ids arrive as numeric
        // strings; compare them as NUMBERS so "99" doesn't sort above "100"
        // (which would re-request the same page forever). An unparseable id is
        // skipped rather than paged from.
        let min_id = page.iter().filter_map(|t| t.id.parse::<u64>().ok()).min();

        // Does this page already reach past the window?
        let reached = match oldest_wanted {
            Some(cut) => page
                .iter()
                .any(|t| t.close_time.map(|ct| ct <= cut).unwrap_or(false)),
            None => false,
        };

        all.extend(page);

        if reached {
            return Ok(PagedHistory { trades: all, pages, truncated: false });
        }
        // A short page means OANDA has nothing older — history exhausted.
        if page_len < page_size as usize {
            return Ok(PagedHistory { trades: all, pages, truncated: false });
        }
        match min_id {
            Some(id) => before_id = Some(id),
            // No usable id to page from: stop rather than loop on the same page.
            None => return Ok(PagedHistory { trades: all, pages, truncated: true }),
        }
    }

    // Ran out of page budget with history still unread.
    Ok(PagedHistory { trades: all, pages, truncated: true })
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
    close_position_units(client, instrument, is_long, super::types::CloseUnits::All).await
}

/// Close `units` of one side of a position, rather than all of it (AGT-783).
///
/// This is what a strategy's `PartialExit` executes as. It goes against the
/// *position* rather than a trade id ([`close_trade`]) because that is the unit
/// the auto-executor tracks and the unit a partial exit is expressed in — "take
/// 40% off" is a statement about the position.
///
/// A non-positive count is refused rather than sent: OANDA would reject it, but
/// only after the attempt is on the wire, and a caller asking to close zero
/// units has a bug worth surfacing where it happened.
pub async fn close_position_units(
    client: &OandaClient,
    instrument: &str,
    is_long: bool,
    units: super::types::CloseUnits,
) -> Result<ClosePositionResponse> {
    if let super::types::CloseUnits::Partial(count) = units {
        if count <= rust_decimal::Decimal::ZERO {
            return Err(Error::InvalidArgument(format!(
                "cannot close {count} units of the {} position on {instrument}: \
                 the amount must be positive",
                if is_long { "long" } else { "short" }
            )));
        }
    }

    let url = format!(
        "{}/v3/accounts/{}/positions/{}/close",
        client.base_url(),
        client.account_id(),
        instrument
    );
    let close_request = if is_long {
        ClosePositionRequest::close_long_units(units)
    } else {
        ClosePositionRequest::close_short_units(units)
    };
    let response = client.put(&url).json(&close_request).send().await?;
    let text = response.text().await?;
    parse_response(&text)
}

/// Fetch one trade by its OANDA id (`GET /trades/{tradeID}`).
///
/// The per-trade close needs the trade's current size to validate against, and
/// `/trades` (the list) is a needlessly wide read for one id.
pub async fn get_trade(client: &OandaClient, trade_id: &str) -> Result<Trade> {
    let url = format!(
        "{}/v3/accounts/{}/trades/{}",
        client.base_url(),
        client.account_id(),
        trade_id
    );
    let response = client.get(&url).send().await?.error_for_status()?;
    let text = response.text().await?;
    let parsed: super::types::TradeResponse = parse_response(&text)?;
    Ok(Trade::from(parsed.trade))
}

/// Close one trade — all of it, or `units` of it (AGT-780).
///
/// This is the precise counterpart to [`close_position`], which closes by
/// instrument and side and therefore cannot express "half of trade 4001", nor
/// distinguish one trade from another on the same side. It is also the
/// prerequisite for partial exits existing at all: `ClosePositionRequest`
/// hardcodes `"ALL"`.
///
/// Takes the `Trade` rather than a bare id so the requested amount is validated
/// against what is actually open **before** anything is submitted — a close
/// larger than the trade cannot reach OANDA and land an attempt in the audit
/// log on its way to being rejected.
pub async fn close_trade(
    client: &OandaClient,
    trade: &Trade,
    units: super::types::CloseUnits,
) -> Result<OrderCreateResponse> {
    units.validate_against(trade)?;

    let url = format!(
        "{}/v3/accounts/{}/trades/{}/close",
        client.base_url(),
        client.account_id(),
        trade.id
    );
    let body = super::types::CloseTradeRequest::new(units);
    let response = client.put(&url).json(&body).send().await?;
    let text = response.text().await?;
    parse_response(&text)
}

// ============================================================================
// The transaction feed (AGT-779)
// ============================================================================

/// Ids per `/transactions/idrange` request. OANDA rejects a range wider than
/// this outright ("The number of Transactions requested exceeds the maximum
/// allowed"), so a wider walk is chunked rather than sent and refused.
pub const TRANSACTIONS_MAX_IDS_PER_REQUEST: u64 = 1000;

/// Fetch the account's transactions with ids in `from_id..=to_id`, oldest
/// first.
///
/// This is what makes a multi-exit trade decomposable: `/trades` reports only
/// the blended `averageClosePrice`, while the individual fills behind it are
/// here (AGT-779, issue #13).
///
/// Ranges wider than [`TRANSACTIONS_MAX_IDS_PER_REQUEST`] are fetched in
/// consecutive chunks and concatenated — the caller gets the whole range or an
/// error, never a silently truncated prefix. Both bounds are `u64` rather than
/// strings, matching [`get_trades_before`], so a query string cannot be
/// injected through them by construction.
///
/// Ids OANDA has no transaction for (gaps are normal) simply do not appear.
pub async fn get_transactions_idrange(
    client: &OandaClient,
    from_id: u64,
    to_id: u64,
) -> Result<Vec<super::types::Transaction>> {
    if from_id > to_id {
        return Err(Error::InvalidArgument(format!(
            "transaction id range is inverted: from={from_id} is past to={to_id}"
        )));
    }

    let mut out = Vec::new();
    let mut chunk_start = from_id;
    while chunk_start <= to_id {
        // -1 because the range is inclusive on both ends: [1, 1000] is 1000 ids.
        let chunk_end = to_id.min(chunk_start + TRANSACTIONS_MAX_IDS_PER_REQUEST - 1);
        let page = fetch_transactions_page(
            client,
            "idrange",
            &[("from", chunk_start.to_string()), ("to", chunk_end.to_string())],
        )
        .await?;
        out.extend(page.transactions);

        // Guard the wrap at u64::MAX rather than overflowing back to 0.
        match chunk_end.checked_add(1) {
            Some(next) => chunk_start = next,
            None => break,
        }
    }
    Ok(out)
}

/// Fetch every transaction after `since_id`, oldest first.
///
/// OANDA's `sinceid` returns at most one page, so a caller that is far behind
/// would get a truncated answer with nothing marking it as truncated. The
/// response's `lastTransactionID` is the account's newest id, so this reads the
/// first page and then walks any remainder through
/// [`get_transactions_idrange`] until the feed is caught up.
pub async fn get_transactions_since_id(
    client: &OandaClient,
    since_id: u64,
) -> Result<Vec<super::types::Transaction>> {
    let page =
        fetch_transactions_page(client, "sinceid", &[("id", since_id.to_string())]).await?;

    // The account's newest id. Without it there is no way to know whether the
    // page was everything, so the page is all that can honestly be returned.
    let Some(newest) = page.last_transaction_id.as_deref().and_then(|s| s.parse::<u64>().ok())
    else {
        return Ok(page.transactions);
    };

    let mut out = page.transactions;
    let highest_seen = out
        .iter()
        .filter_map(|t| t.id().and_then(|id| id.parse::<u64>().ok()))
        .max()
        .unwrap_or(since_id);

    if highest_seen < newest {
        let rest = get_transactions_idrange(client, highest_seen + 1, newest).await?;
        out.extend(rest);
    }
    Ok(out)
}

/// Fetch every transaction in a UTC time window, oldest first.
///
/// OANDA's time-based `/transactions` endpoint returns no transactions — only
/// the `/transactions/idrange` page URLs that cover the window. This reads
/// that index, takes the covering id span, and walks it through
/// [`get_transactions_idrange`] (which chunks requests itself).
pub async fn get_transactions_window(
    client: &OandaClient,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<super::types::Transaction>> {
    let url = format!(
        "{}/v3/accounts/{}/transactions?from={}&to={}&pageSize=1000",
        client.base_url(),
        client.account_id(),
        // to_rfc3339 can emit '+00:00', which must be %-escaped in a query
        // string; use the 'Z' form instead.
        from.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        to.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    );
    let response = client.get(&url).send().await?.error_for_status()?;
    let text = response.text().await?;
    let window: super::types::TransactionsWindowResponse = parse_response(&text)?;

    // The pages jointly cover [min from, max to]; one idrange walk over that
    // span fetches the same ids in the same number of requests.
    let mut span: Option<(u64, u64)> = None;
    for page in &window.pages {
        if let Some((f, t)) = parse_idrange_page_url(page) {
            span = Some(match span {
                None => (f, t),
                Some((lo, hi)) => (lo.min(f), hi.max(t)),
            });
        }
    }
    match span {
        Some((from_id, to_id)) => get_transactions_idrange(client, from_id, to_id).await,
        None => Ok(Vec::new()),
    }
}

/// The `from`/`to` ids off one of the page URLs the time-window index returns,
/// e.g. `…/transactions/idrange?from=447&to=476`. `None` when either id is
/// missing or non-numeric — that page is skipped rather than guessed at.
fn parse_idrange_page_url(url: &str) -> Option<(u64, u64)> {
    let query = url.split_once('?')?.1;
    let mut from = None;
    let mut to = None;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("from", v)) => from = v.parse::<u64>().ok(),
            Some(("to", v)) => to = v.parse::<u64>().ok(),
            _ => {}
        }
    }
    Some((from?, to?))
}

/// Closed trades rebuilt from the transaction ledger for a time window
/// (issue #16), including the backfill of entries that predate the window.
#[derive(Debug, Clone, Default)]
pub struct LedgerClosed {
    /// The rebuilt closed trades. Feed order; the caller sorts and filters.
    pub trades: Vec<Trade>,
    /// Trades that closed in the window but could not be fully reconstructed
    /// even after backfill (entry unfetchable, or exits that predate the
    /// window and exceeded the backfill budget). Non-zero means the rebuilt
    /// list is incomplete and the caller must say so rather than present it
    /// as everything.
    pub unresolved: usize,
}

/// How many out-of-window opening fills [`closed_trades_from_ledger`] will
/// fetch individually. A trade's id is its opening transaction's id, so each
/// costs exactly one precise idrange request; the cap only guards against a
/// pathological window where hundreds of long-held trades all closed at once.
const LEDGER_ENTRY_BACKFILL_MAX: usize = 20;

/// Rebuild the closed trades of `[from, to]` from the transaction ledger —
/// the fallback for OANDA's stale closed-trades index (issue #16).
///
/// `/trades?state=CLOSED` on the practice environment stopped covering trades
/// closed after ~2026-08-11 (and `/trades/{id}` 404s them) while the ledger
/// kept every fill, so this path derives what `/trades` should have said:
/// window fetch → [`crate::trade_fills::rebuild_closed_trades`] → one precise
/// fetch per entry that predates the window → rebuild again.
pub async fn closed_trades_from_ledger(
    client: &OandaClient,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
) -> Result<LedgerClosed> {
    let mut transactions = get_transactions_window(client, from, to).await?;
    let first = crate::trade_fills::rebuild_closed_trades(&transactions);
    if first.missing_entries.is_empty() {
        return Ok(LedgerClosed { trades: first.trades, unresolved: 0 });
    }

    // Entries older than the window: each trade id is the exact transaction
    // id of its opening fill, so fetch precisely those — never a spanning
    // range, which could drag in an unbounded stretch of unrelated feed.
    let mut budget = LEDGER_ENTRY_BACKFILL_MAX;
    for trade_id in &first.missing_entries {
        if budget == 0 {
            break;
        }
        if let Ok(id) = trade_id.parse::<u64>() {
            budget -= 1;
            let opening = get_transactions_idrange(client, id, id).await?;
            transactions.extend(opening);
        }
    }

    let mut second = crate::trade_fills::rebuild_closed_trades(&transactions);
    // A backfilled opening fill can itself close *other* trades (a netting
    // fill), whose closes predate the window — drop those rather than let a
    // widened fetch smuggle out-of-window rows into the result.
    second
        .trades
        .retain(|t| t.close_time.is_some_and(|ct| ct >= from && ct <= to));
    Ok(LedgerClosed {
        unresolved: second.missing_entries.len(),
        trades: second.trades,
    })
}

/// One `/transactions/<sub>` request with numeric query params already
/// stringified by the caller. Shared by the idrange and sinceid walks so both
/// decode and error-check identically.
async fn fetch_transactions_page(
    client: &OandaClient,
    sub_path: &str,
    query: &[(&str, String)],
) -> Result<super::types::TransactionsResponse> {
    let mut url = format!(
        "{}/v3/accounts/{}/transactions/{}",
        client.base_url(),
        client.account_id(),
        sub_path
    );
    let query_parts: Vec<String> =
        query.iter().map(|(key, value)| format!("{key}={value}")).collect();
    url.push('?');
    url.push_str(&query_parts.join("&"));

    let response = client.get(&url).send().await?.error_for_status()?;
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
    use wiremock::matchers::{body_json, method, path, query_param};
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

    // ── paged closed-trade history (beforeID walk) ────────────────────────

    /// One CLOSED trade page. `ids` are returned newest-first, as OANDA does.
    fn closed_page(ids: &[u64], close_day: u32) -> serde_json::Value {
        let trades: Vec<serde_json::Value> = ids
            .iter()
            .map(|id| {
                json!({
                    "id": id.to_string(),
                    "instrument": "EUR_USD",
                    "price": "1.08500",
                    "openTime": format!("2026-07-{:02}T09:00:00.000000000Z", close_day),
                    "initialUnits": "1000",
                    "currentUnits": "0",
                    "realizedPL": "1.0000",
                    "state": "CLOSED",
                    "closeTime": format!("2026-07-{:02}T10:00:00.000000000Z", close_day),
                    "averageClosePrice": "1.08600",
                    "closingTransactionIDs": [format!("{}", id + 1)]
                })
            })
            .collect();
        json!({ "trades": trades, "lastTransactionID": "99999" })
    }

    #[tokio::test]
    async fn paged_history_stops_on_a_short_page() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // A page smaller than the page size means OANDA has nothing older.
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .respond_with(ResponseTemplate::new(200).set_body_json(closed_page(&[300, 299], 20)))
            .expect(1)
            .mount(&mock_server)
            .await;

        let out = get_closed_trades_since(&client, None, 5, 10).await.unwrap();

        assert_eq!(out.trades.len(), 2);
        assert_eq!(out.pages, 1);
        assert!(!out.truncated, "a short page is exhaustion, not truncation");
    }

    #[tokio::test]
    async fn paged_history_stops_once_the_window_is_covered() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // Page 1: full, all closed Jul 20 (inside the window).
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .and(query_param("count", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(closed_page(&[300, 299], 20)))
            .up_to_n_times(1)
            .expect(1)
            .mount(&mock_server)
            .await;
        // Page 2: reaches Jul 10, at/before the cutoff → walk stops here.
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .and(query_param("beforeID", "299"))
            .respond_with(ResponseTemplate::new(200).set_body_json(closed_page(&[298, 297], 10)))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cut = chrono::DateTime::parse_from_rfc3339("2026-07-15T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let out = get_closed_trades_since(&client, Some(cut), 2, 10).await.unwrap();

        assert_eq!(out.pages, 2, "should stop as soon as the window is covered");
        assert_eq!(out.trades.len(), 4);
        assert!(!out.truncated);
    }

    #[tokio::test]
    async fn paged_history_reports_truncation_at_the_page_cap() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // Every page is full and never reaches the cutoff, so the walk runs out
        // of budget. That MUST be reported, not silently returned as complete.
        //
        // Three mocks, each answering once, consumed in registration order —
        // so successive requests get DISTINCT descending ids the way a real
        // endpoint would. A single always-the-same-page mock would satisfy the
        // pages/truncated assertions while quietly filling `trades` with
        // duplicates, which the id-uniqueness assertion below would miss.
        for ids in [[300u64, 299], [298, 297], [296, 295]] {
            Mock::given(method("GET"))
                .and(path("/v3/accounts/test-account-123/trades"))
                .respond_with(ResponseTemplate::new(200).set_body_json(closed_page(&ids, 24)))
                .up_to_n_times(1)
                .mount(&mock_server)
                .await;
        }

        let cut = chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let out = get_closed_trades_since(&client, Some(cut), 2, 3).await.unwrap();

        assert_eq!(out.pages, 3, "stops at max_pages");
        assert!(out.truncated, "hitting the page cap must surface as truncated");
        // Every page advanced: no id appears twice. This is what makes the
        // mock a faithful stand-in rather than a loop that merely satisfies
        // the counters above.
        let ids: Vec<&str> = out.trades.iter().map(|t| t.id.as_str()).collect();
        let unique: std::collections::HashSet<&str> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "paging must not re-request a page: {ids:?}");
    }

    // ── closing one trade by id (AGT-780) ─────────────────────────────────

    fn open_trade_json(id: &str, units: &str) -> serde_json::Value {
        json!({
            "id": id,
            "instrument": "EUR_USD",
            "price": "1.08000",
            "openTime": "2026-07-20T10:00:00.000000000Z",
            "initialUnits": units,
            "currentUnits": units,
            "realizedPL": "0.0000",
            "unrealizedPL": "1.5000",
            "state": "OPEN",
            "financing": "0.0000"
        })
    }

    #[tokio::test]
    async fn a_trade_is_fetched_by_id() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades/4001"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "trade": open_trade_json("4001", "1000") })),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let trade = get_trade(&client, "4001").await.unwrap();

        assert_eq!(trade.id, "4001");
        assert_eq!(trade.units, "1000".parse::<rust_decimal::Decimal>().unwrap());
    }

    // AC2: the partial close reduces the trade rather than closing it, and the
    // fill says so — `tradeReduced`, not `tradesClosed`.
    #[tokio::test]
    async fn a_partial_close_sends_the_units_and_reduces_the_trade() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("PUT"))
            .and(path("/v3/accounts/test-account-123/trades/4001/close"))
            .and(body_json(json!({ "units": "400" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "orderFillTransaction": {
                    "id": "4050", "time": "2026-07-20T12:00:00Z", "type": "ORDER_FILL",
                    "reason": "MARKET_ORDER_TRADE_CLOSE", "instrument": "EUR_USD",
                    "units": "-400", "price": "1.08500", "pl": "4.0000",
                    "tradeReduced": { "tradeID": "4001", "units": "-400", "realizedPL": "4.0000" }
                },
                "lastTransactionID": "4050"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let trade = get_trade_fixture("4001", "1000");
        let out = close_trade(&client, &trade, super::super::types::CloseUnits::Partial(
            "400".parse().unwrap(),
        ))
        .await
        .unwrap();

        let fill = out.order_fill_transaction.expect("filled");
        assert_eq!(fill.trade_reduced.as_ref().unwrap().units, "-400");
        assert!(fill.trades_closed.is_empty(), "the trade is reduced, not closed");
    }

    #[tokio::test]
    async fn a_full_close_sends_all() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("PUT"))
            .and(path("/v3/accounts/test-account-123/trades/4001/close"))
            .and(body_json(json!({ "units": "ALL" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "orderFillTransaction": {
                    "id": "4090", "time": "2026-07-20T15:00:00Z", "type": "ORDER_FILL",
                    "instrument": "EUR_USD", "units": "-1000", "price": "1.09000", "pl": "10.0000",
                    "tradesClosed": [
                        { "tradeID": "4001", "units": "-1000", "realizedPL": "10.0000" }
                    ]
                },
                "lastTransactionID": "4090"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let trade = get_trade_fixture("4001", "1000");
        let out = close_trade(&client, &trade, super::super::types::CloseUnits::All)
            .await
            .unwrap();

        assert_eq!(out.order_fill_transaction.unwrap().trades_closed.len(), 1);
    }

    // AC3: an oversized close never reaches OANDA. No mock is mounted, so any
    // request would fail the call for the wrong reason.
    #[tokio::test]
    async fn an_oversized_close_is_rejected_without_calling_out() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let trade = get_trade_fixture("4001", "1000");
        let err = close_trade(
            &client,
            &trade,
            super::super::types::CloseUnits::Partial("2500".parse().unwrap()),
        )
        .await
        .unwrap_err();

        assert!(format!("{err}").contains("only 1000 are open"), "{err}");
    }

    /// A `Trade` straight off the fixture JSON, so the tests exercise the same
    /// conversion the live path uses.
    fn get_trade_fixture(id: &str, units: &str) -> Trade {
        let oanda: super::super::types::OandaTrade =
            serde_json::from_value(open_trade_json(id, units)).expect("fixture decodes");
        Trade::from(oanda)
    }

    // ── the transaction feed (AGT-779) ────────────────────────────────────

    /// A page of `count` ORDER_FILL transactions with consecutive ids from
    /// `first_id`, plus the account's newest id.
    fn transaction_page(first_id: u64, count: u64, newest: u64) -> serde_json::Value {
        let transactions: Vec<serde_json::Value> = (0..count)
            .map(|n| {
                json!({
                    "id": (first_id + n).to_string(),
                    "time": "2026-07-20T14:00:00.000000000Z",
                    "type": "ORDER_FILL",
                    "reason": "MARKET_ORDER",
                    "instrument": "EUR_USD",
                    "units": "1000",
                    "price": "1.08500",
                    "tradeOpened": { "tradeID": (first_id + n).to_string(), "units": "1000" }
                })
            })
            .collect();
        json!({ "transactions": transactions, "lastTransactionID": newest.to_string() })
    }

    #[tokio::test]
    async fn idrange_returns_the_requested_window() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions/idrange"))
            .and(query_param("from", "10"))
            .and(query_param("to", "12"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transaction_page(10, 3, 12)))
            .expect(1)
            .mount(&mock_server)
            .await;

        let out = get_transactions_idrange(&client, 10, 12).await.unwrap();

        assert_eq!(out.len(), 3);
        assert_eq!(out[0].id(), Some("10"));
        assert_eq!(out[2].id(), Some("12"));
    }

    // AC3: a range past OANDA's per-request cap is chunked, not truncated and
    // not sent as one request OANDA would refuse.
    #[tokio::test]
    async fn idrange_chunks_a_range_wider_than_the_request_cap() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // 1..=1500 → [1, 1000] then [1001, 1500]. The bounds are asserted, so
        // an off-by-one that re-requested or skipped an id fails here.
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions/idrange"))
            .and(query_param("from", "1"))
            .and(query_param("to", "1000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transaction_page(1, 2, 1500)))
            .expect(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions/idrange"))
            .and(query_param("from", "1001"))
            .and(query_param("to", "1500"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transaction_page(1001, 2, 1500)))
            .expect(1)
            .mount(&mock_server)
            .await;

        let out = get_transactions_idrange(&client, 1, 1500).await.unwrap();

        let ids: Vec<&str> = out.iter().filter_map(|t| t.id()).collect();
        assert_eq!(ids, ["1", "2", "1001", "1002"], "both chunks, in order");
    }

    #[tokio::test]
    async fn idrange_rejects_an_inverted_range_without_calling_out() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;
        // No mock is mounted: any request at all would 404 and fail the call
        // for the wrong reason, so this also proves nothing was sent.

        let err = get_transactions_idrange(&client, 99, 12).await.unwrap_err();

        assert!(
            format!("{err}").contains("inverted"),
            "should name the inverted range: {err}"
        );
    }

    #[tokio::test]
    async fn sinceid_returns_the_page_when_it_is_already_caught_up() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // The page reaches lastTransactionID, so there is nothing to walk.
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions/sinceid"))
            .and(query_param("id", "40"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transaction_page(41, 2, 42)))
            .expect(1)
            .mount(&mock_server)
            .await;

        let out = get_transactions_since_id(&client, 40).await.unwrap();

        assert_eq!(out.len(), 2);
        assert_eq!(out[1].id(), Some("42"));
    }

    // AC3 again, on the other endpoint: `sinceid` returns one page, so a caller
    // far behind must be caught up rather than handed a silent prefix.
    #[tokio::test]
    async fn sinceid_walks_the_remainder_when_the_page_falls_short() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // Page stops at 42 while the account is at 44 → the rest comes from
        // idrange [43, 44].
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions/sinceid"))
            .and(query_param("id", "40"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transaction_page(41, 2, 44)))
            .expect(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions/idrange"))
            .and(query_param("from", "43"))
            .and(query_param("to", "44"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transaction_page(43, 2, 44)))
            .expect(1)
            .mount(&mock_server)
            .await;

        let out = get_transactions_since_id(&client, 40).await.unwrap();

        let ids: Vec<&str> = out.iter().filter_map(|t| t.id()).collect();
        assert_eq!(ids, ["41", "42", "43", "44"], "the feed is caught up, in order");
    }

    #[tokio::test]
    async fn sinceid_on_an_empty_feed_returns_nothing_and_walks_nowhere() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // Nothing since 44, and 44 is the newest — no idrange follow-up.
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions/sinceid"))
            .and(query_param("id", "44"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({ "transactions": [], "lastTransactionID": "44" }),
            ))
            .expect(1)
            .mount(&mock_server)
            .await;

        assert!(get_transactions_since_id(&client, 44).await.unwrap().is_empty());
    }

    // ── the time-window walk and the ledger rebuild (issue #16) ─────────────

    #[test]
    fn idrange_page_urls_parse_and_junk_ones_do_not() {
        assert_eq!(
            parse_idrange_page_url(
                "https://api-fxpractice.oanda.com/v3/accounts/x/transactions/idrange?from=447&to=476"
            ),
            Some((447, 476))
        );
        assert_eq!(parse_idrange_page_url("https://x/transactions/idrange?to=476"), None);
        assert_eq!(parse_idrange_page_url("https://x/idrange?from=nope&to=476"), None);
        assert_eq!(parse_idrange_page_url("no-query-at-all"), None);
    }

    fn window_ts(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&chrono::Utc)
    }

    #[tokio::test]
    async fn a_time_window_is_read_via_its_page_index_then_one_idrange_walk() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // The index returns two pages; the walk covers their joint span in one
        // idrange request (chunked internally if it were wide).
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "pages": [
                    "https://api/v3/accounts/x/transactions/idrange?from=10&to=11",
                    "https://api/v3/accounts/x/transactions/idrange?from=12&to=12"
                ],
                "count": 3
            })))
            .expect(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions/idrange"))
            .and(query_param("from", "10"))
            .and(query_param("to", "12"))
            .respond_with(ResponseTemplate::new(200).set_body_json(transaction_page(10, 3, 12)))
            .expect(1)
            .mount(&mock_server)
            .await;

        let out = get_transactions_window(
            &client,
            window_ts("2026-08-19T00:00:00Z"),
            window_ts("2026-08-20T00:00:00Z"),
        )
        .await
        .unwrap();

        assert_eq!(out.len(), 3);
    }

    #[tokio::test]
    async fn an_empty_time_window_fetches_no_idranges() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // Only the index is mocked: an idrange request would 404 and fail the
        // call, so success proves none was made.
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "pages": [], "count": 0 })),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let out = get_transactions_window(
            &client,
            window_ts("2026-08-19T00:00:00Z"),
            window_ts("2026-08-20T00:00:00Z"),
        )
        .await
        .unwrap();

        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn the_ledger_rebuild_backfills_an_entry_older_than_the_window() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // The window holds only the closing fill of trade 453 — the entry is
        // older. The rebuild must fetch exactly transaction 453 (a trade's id
        // is its opening transaction's id) and produce the full row.
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "pages": ["https://api/x/transactions/idrange?from=462&to=462"],
                "count": 1
            })))
            .expect(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions/idrange"))
            .and(query_param("from", "462"))
            .and(query_param("to", "462"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "transactions": [{
                    "id": "462", "time": "2026-08-19T17:15:00Z", "type": "ORDER_FILL",
                    "reason": "MARKET_ORDER_POSITION_CLOSEOUT", "instrument": "USD_JPY",
                    "units": "2000", "price": "157.800",
                    "tradesClosed": [
                        { "tradeID": "453", "units": "2000", "realizedPL": "7.7071" }
                    ]
                }],
                "lastTransactionID": "477"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions/idrange"))
            .and(query_param("from", "453"))
            .and(query_param("to", "453"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "transactions": [{
                    "id": "453", "time": "2026-08-19T06:30:00Z", "type": "ORDER_FILL",
                    "reason": "MARKET_ORDER", "instrument": "USD_JPY",
                    "units": "-2000", "price": "158.400",
                    "tradeOpened": {
                        "tradeID": "453", "units": "-2000", "price": "158.400",
                        "clientExtensions": { "tag": "rahagod" }
                    }
                }],
                "lastTransactionID": "477"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let out = closed_trades_from_ledger(
            &client,
            window_ts("2026-08-19T12:00:00Z"),
            window_ts("2026-08-20T00:00:00Z"),
        )
        .await
        .unwrap();

        assert_eq!(out.unresolved, 0);
        assert_eq!(out.trades.len(), 1);
        let t = &out.trades[0];
        assert_eq!(t.id, "453");
        assert_eq!(t.instrument, "USD_JPY");
        assert_eq!(t.strategy.as_deref(), Some("rahagod"));
        assert_eq!(t.realized_pl.to_string(), "7.7071");
    }

    #[tokio::test]
    async fn a_transactions_page_surfaces_an_oanda_error_rather_than_decoding_it() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/transactions/idrange"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "errorMessage": "Invalid transaction ID range specified"
            })))
            .mount(&mock_server)
            .await;

        let err = get_transactions_idrange(&client, 1, 5).await.unwrap_err();

        assert!(
            format!("{err}").contains("Invalid transaction ID range"),
            "OANDA's message should reach the caller: {err}"
        );
    }

    #[tokio::test]
    async fn paged_history_pages_from_the_lowest_id_numerically() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        // Ids 100 and 99: a string comparison would page from "99" (wrong, it
        // is the larger string) — the walk must page from 99 as a NUMBER.
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .and(query_param("count", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(closed_page(&[100, 99], 24)))
            .up_to_n_times(1)
            .expect(1)
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .and(query_param("beforeID", "99"))
            .respond_with(ResponseTemplate::new(200).set_body_json(closed_page(&[98], 24)))
            .expect(1)
            .mount(&mock_server)
            .await;

        let out = get_closed_trades_since(&client, None, 2, 10).await.unwrap();

        assert_eq!(out.trades.len(), 3);
        assert_eq!(out.pages, 2);
    }

    #[tokio::test]
    async fn paged_history_handles_an_empty_account() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/trades"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "trades": [], "lastTransactionID": "1" })),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let out = get_closed_trades_since(&client, None, 500, 10).await.unwrap();

        assert!(out.trades.is_empty());
        assert!(!out.truncated);
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
                    },
                    // The trade-level twin is the load-bearing one: only this
                    // survives onto the trade record read back from `/trades`.
                    // The original test asserted only the order-level field, so
                    // it passed for months while every closed trade came back
                    // unattributed. Both are required now.
                    "tradeClientExtensions": {
                        "tag": "ma-crossover",
                        "comment": "wickd strategy=ma-crossover"
                    }
                }
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(order_response))
            .expect(1)
            .mount(&mock_server)
            .await;

        // Without BOTH extensions in the body, the matcher above would not
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

    // Financing terms ride along on the instruments response (carry research
    // needs them); older/omitted payloads parse with financing = None.
    #[tokio::test]
    async fn test_get_instruments_parses_financing() {
        let mock_server = MockServer::start().await;
        let client = setup_mock_client(&mock_server).await;

        let body = json!({
            "instruments": [
                {
                    "name": "EUR_USD",
                    "type": "CURRENCY",
                    "displayName": "EUR/USD",
                    "financing": {
                        "longRate": "-0.0245",
                        "shortRate": "0.0042",
                        "financingDaysOfWeek": [
                            {"dayOfWeek": "MONDAY", "daysCharged": 1},
                            {"dayOfWeek": "WEDNESDAY", "daysCharged": 3}
                        ]
                    }
                },
                {
                    "name": "XAU_USD",
                    "type": "METAL",
                    "displayName": "Gold"
                }
            ],
            "lastTransactionID": "99999"
        });
        Mock::given(method("GET"))
            .and(path("/v3/accounts/test-account-123/instruments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&mock_server)
            .await;

        let instruments = get_instruments(&client).await.unwrap();
        assert_eq!(instruments.len(), 2);
        let fin = instruments[0].financing.as_ref().expect("financing present");
        assert_eq!(fin.long_rate, "-0.0245");
        assert_eq!(fin.short_rate, "0.0042");
        assert_eq!(fin.financing_days_of_week[1].days_charged, 3);
        assert!(instruments[1].financing.is_none());
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
