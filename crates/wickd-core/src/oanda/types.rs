use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderType {
    Market,
    Limit,
    Stop,
    MarketIfTouched,
    TakeProfit,
    StopLoss,
    TrailingStopLoss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TimeInForce {
    GTC,
    GTD,
    GFD,
    FOK,
    IOC,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PositionFill {
    Default,
    OpenOnly,
    ReduceFirst,
    ReduceOnly,
}

/// Which price component triggers a resting Limit/Stop entry order (AGT-612,
/// AC2). OANDA defaults to `DEFAULT` (the "natural" side for the order's
/// direction); the others let a caller pin the trigger to a specific book side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TriggerCondition {
    Default,
    Inverse,
    Bid,
    Ask,
    Mid,
}

/// The two *resting* entry order kinds [`EntryOrderRequest`] can build (AGT-612,
/// AC1/AC2). `Market` is intentionally excluded here — a market entry is always
/// immediate-FOK and goes through the dedicated [`MarketOrderRequest`] path, so
/// the single guarded place sequence branches Market → market builder,
/// Limit/Stop → this builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryOrderType {
    Limit,
    Stop,
}

/// Optional parameters for a resting Limit/Stop *entry* order (AGT-612, AC2).
/// Everything here is optional: `time_in_force` defaults to `GTC` when `None`,
/// and the rest are omitted from the POST body when `None`. Owned `String`s
/// (rather than borrows) keep the guarded place path free of lifetime plumbing.
#[derive(Debug, Clone, Default)]
pub struct EntryOptions {
    /// Time-in-force; `None` → `GTC` (a resting order lives until cancelled).
    pub time_in_force: Option<TimeInForce>,
    /// RFC3339 good-till-date; only meaningful with a `GTD` time-in-force.
    pub gtd_time: Option<String>,
    /// Worst price a Stop order may fill at (protects against slippage on a gap).
    pub price_bound: Option<String>,
    /// Which book side triggers the order; `None` → OANDA `DEFAULT`.
    pub trigger_condition: Option<TriggerCondition>,
    /// Stop-loss to attach on fill (formatted to the instrument's precision).
    pub stop_loss: Option<String>,
    /// Take-profit to attach on fill.
    pub take_profit: Option<String>,
    /// Strategy to attribute the order to (AGT-630, AC1). When set, the POST
    /// body carries OANDA `clientExtensions` built via
    /// [`ClientExtensions::for_strategy`]; when `None` the field is omitted.
    pub strategy: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TradesResponse {
    pub trades: Vec<OandaTrade>,
    #[serde(rename = "lastTransactionID")]
    pub last_transaction_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OandaTrade {
    pub id: String,
    pub instrument: String,
    pub price: String,
    pub open_time: String,
    pub initial_units: String,
    pub current_units: String,
    #[serde(rename = "realizedPL")]
    pub realized_pl: String,
    #[serde(default, rename = "unrealizedPL")]
    pub unrealized_pl: Option<String>,
    pub state: String,
    #[serde(default)]
    pub financing: Option<String>,
    #[serde(default)]
    pub close_time: Option<String>,
    #[serde(default)]
    pub average_close_price: Option<String>,
    #[serde(default)]
    pub initial_margin_required: Option<String>,
    #[serde(default, rename = "closingTransactionIDs")]
    pub closing_transaction_ids: Vec<String>,
    #[serde(default)]
    pub dividend_adjustment: Option<String>,
    /// OANDA echoes the placing order's `clientExtensions` onto the trade
    /// (AGT-630 attribution). `trade report` (AGT-631) reads `tag` off this to
    /// group closed-trade realized P&L by strategy; absent on manual/legacy
    /// trades → the trade reads back unattributed.
    #[serde(default, rename = "clientExtensions")]
    pub client_extensions: Option<ClientExtensions>,
}

#[derive(Debug, Deserialize)]
pub struct CandlesResponse {
    pub instrument: String,
    pub granularity: String,
    pub candles: Vec<OandaCandle>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OandaCandle {
    pub time: String,
    pub bid: Option<OandaCandleData>,
    pub ask: Option<OandaCandleData>,
    pub mid: Option<OandaCandleData>,
    pub volume: i32,
    pub complete: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OandaCandleData {
    pub o: String,
    pub h: String,
    pub l: String,
    pub c: String,
}

#[derive(Debug, Deserialize)]
pub struct AccountResponse {
    pub account: OandaAccount,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OandaAccount {
    pub id: String,
    #[serde(default)]
    pub alias: Option<String>,
    pub currency: String,
    pub balance: String,
    #[serde(rename = "NAV")]
    pub nav: String,
    #[serde(default)]
    #[serde(rename = "unrealizedPL")]
    pub unrealized_pl: String,
    #[serde(default)]
    pub open_trade_count: i32,
}

#[derive(Debug, Deserialize)]
pub struct PositionsResponse {
    pub positions: Vec<OandaPosition>,
    #[serde(rename = "lastTransactionID")]
    pub last_transaction_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OandaPosition {
    pub instrument: String,
    #[serde(rename = "pl")]
    pub realized_pl: String,
    #[serde(rename = "unrealizedPL")]
    pub unrealized_pl: String,
    pub long: OandaPositionSide,
    pub short: OandaPositionSide,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OandaPositionSide {
    #[serde(default)]
    pub units: String,
    #[serde(default)]
    pub average_price: Option<String>,
    #[serde(default)]
    #[serde(rename = "pl")]
    pub realized_pl: Option<String>,
    #[serde(default)]
    #[serde(rename = "unrealizedPL")]
    pub unrealized_pl: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OrdersResponse {
    pub orders: Vec<OandaOrder>,
    #[serde(rename = "lastTransactionID")]
    pub last_transaction_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OandaOrder {
    pub id: String,
    pub create_time: String,
    #[serde(rename = "type")]
    pub order_type: String,
    #[serde(default)]
    pub instrument: Option<String>,
    #[serde(default)]
    pub units: Option<String>,
    pub state: String,
    #[serde(default)]
    pub price: Option<String>,
    #[serde(default)]
    pub time_in_force: Option<String>,
    #[serde(default)]
    pub trigger_condition: Option<String>,
    #[serde(default, rename = "tradeID")]
    pub trade_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketOrderRequest {
    pub order: MarketOrderSpec,
}

/// OANDA `clientExtensions` — client-supplied attribution attached to an order
/// and echoed back on its transaction record (AGT-630, AC1). This is how a
/// filled trade stays attributable to the strategy that placed it *at the
/// broker*, not just in the local ledger: OANDA stores the tag/comment on the
/// order transactions, so the transaction history can be joined back to a
/// strategy by name.
///
/// Each member is serialized only when present (`skip_serializing_if`), so an
/// unattributed order's POST body stays byte-identical to the pre-AGT-630
/// shape. OANDA's ClientExtensions `id` member is deliberately omitted: it must
/// be unique per order and wickd has no client-order-id scheme — `tag` and
/// `comment` are the attribution carriers.
// `Deserialize` (AGT-631): OANDA echoes the order's `clientExtensions` back on
// the resulting *trade* record, so a closed trade fetched from `/trades` still
// names the strategy that placed it. `trade report` reads `tag` off each closed
// trade to attribute realized P&L by strategy. `#[serde(default)]` on each
// member keeps deserialization tolerant of OANDA omitting either field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientExtensions {
    /// Tag associated with the order (the strategy name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Human-readable comment (`wickd strategy=<name>`), naming the placer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

impl ClientExtensions {
    /// Attribution extensions for a strategy-placed order: the strategy name as
    /// the `tag`, plus a self-describing `comment` naming wickd as the placer.
    pub fn for_strategy(strategy: &str) -> Self {
        Self {
            tag: Some(strategy.to_string()),
            comment: Some(format!("wickd strategy={strategy}")),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketOrderSpec {
    #[serde(rename = "type")]
    pub order_type: OrderType,
    pub instrument: String,
    pub units: String,
    pub time_in_force: TimeInForce,
    pub position_fill: PositionFill,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_loss_on_fill: Option<StopLossOnFill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take_profit_on_fill: Option<TakeProfitOnFill>,
    /// Strategy attribution (AGT-630, AC1); omitted when no strategy is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_extensions: Option<ClientExtensions>,
}

/// Stop loss order to create when the trade is filled
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopLossOnFill {
    pub price: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<TimeInForce>,
}

/// Take profit order to create when the trade is filled
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TakeProfitOnFill {
    pub price: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_in_force: Option<TimeInForce>,
}

/// Number of decimal places OANDA expects for an instrument's price (AGT-612,
/// AC2). This is the per-instrument precision rule the limit/stop entry path
/// relies on: submitting a trigger price with too many decimals is rejected by
/// OANDA with `PRICE_PRECISION_EXCEEDED`, which is the mis-precisioned-price
/// reject bug this fixes.
///
/// The precision is keyed off the **quote** currency (the half after `_`),
/// which is more robust than a bare `contains("JPY")` substring check:
/// - JPY-quoted pairs (`USD_JPY`, `EUR_JPY`): 3 dp.
/// - Spot gold (`XAU_*`): 3 dp — the naive substring rule mis-precisioned this
///   to 5 dp because "XAU_USD" contains no "JPY", producing a rejected order.
/// - Everything else: 5 dp.
///
/// When a caller already knows the instrument's exact `displayPrecision` (from
/// the OANDA instruments endpoint), prefer [`format_price_with_precision`] to
/// honour it directly rather than re-deriving it here.
pub fn price_precision(instrument: &str) -> usize {
    let quote = instrument.split('_').nth(1).unwrap_or(instrument);
    if quote == "JPY" || instrument.starts_with("XAU") {
        3
    } else {
        5
    }
}

/// Format a price string to an explicit number of decimal places. Used when the
/// instrument's `displayPrecision` is known directly; [`format_price_for_oanda`]
/// wraps this with the [`price_precision`] heuristic.
pub fn format_price_with_precision(price: &str, decimals: usize) -> String {
    // Parse the price as f64 for formatting; an unparseable price → 0 so a
    // malformed input can never silently smuggle through extra precision.
    let price_val: f64 = price.parse().unwrap_or(0.0);
    format!("{price_val:.decimals$}")
}

/// Format a price string to the correct precision for an OANDA instrument
/// (AGT-612, AC2). Delegates to [`price_precision`] so JPY pairs and spot gold
/// get 3 dp and most pairs get 5 dp — the fix for the mis-precisioned-price
/// reject.
pub fn format_price_for_oanda(instrument: &str, price: &str) -> String {
    format_price_with_precision(price, price_precision(instrument))
}

impl MarketOrderRequest {
    pub fn new(instrument: &str, units: i64) -> Self {
        Self {
            order: MarketOrderSpec {
                order_type: OrderType::Market,
                instrument: instrument.to_string(),
                units: units.to_string(),
                time_in_force: TimeInForce::FOK,
                position_fill: PositionFill::Default,
                stop_loss_on_fill: None,
                take_profit_on_fill: None,
                client_extensions: None,
            },
        }
    }

    /// Create a market order with stop loss and take profit
    pub fn with_sl_tp(
        instrument: &str,
        units: i64,
        stop_loss: Option<&str>,
        take_profit: Option<&str>,
    ) -> Self {
        Self {
            order: MarketOrderSpec {
                order_type: OrderType::Market,
                instrument: instrument.to_string(),
                units: units.to_string(),
                time_in_force: TimeInForce::FOK,
                position_fill: PositionFill::Default,
                stop_loss_on_fill: stop_loss.map(|price| StopLossOnFill {
                    price: format_price_for_oanda(instrument, price),
                    time_in_force: Some(TimeInForce::GTC),
                }),
                take_profit_on_fill: take_profit.map(|price| TakeProfitOnFill {
                    price: format_price_for_oanda(instrument, price),
                    time_in_force: Some(TimeInForce::GTC),
                }),
                client_extensions: None,
            },
        }
    }

    /// Attach strategy attribution as OANDA `clientExtensions` (AGT-630, AC1).
    /// `None` leaves the order unattributed (the field is omitted from the POST
    /// body), so callers can thread an optional strategy straight through.
    pub fn with_strategy(mut self, strategy: Option<&str>) -> Self {
        self.order.client_extensions = strategy.map(ClientExtensions::for_strategy);
        self
    }
}

/// A resting Limit or Stop *entry* order POST body (AGT-612, AC2). Mirrors
/// [`MarketOrderRequest`]'s single-field wrapper shape; the guarded place path
/// builds this for the Limit/Stop kinds and POSTs it to the same `/orders`
/// endpoint the market path uses.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryOrderRequest {
    pub order: EntryOrderSpec,
}

/// The order body of an [`EntryOrderRequest`]. A Limit/Stop order carries a
/// required trigger `price` (unlike a market order), a default `GTC`
/// time-in-force, and the optional `gtdTime`/`priceBound`/`triggerCondition`
/// fields that OANDA only accepts on resting orders.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryOrderSpec {
    #[serde(rename = "type")]
    pub order_type: OrderType,
    pub instrument: String,
    pub units: String,
    /// Trigger price, formatted to the instrument's precision.
    pub price: String,
    pub time_in_force: TimeInForce,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gtd_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_bound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_condition: Option<TriggerCondition>,
    pub position_fill: PositionFill,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_loss_on_fill: Option<StopLossOnFill>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take_profit_on_fill: Option<TakeProfitOnFill>,
    /// Strategy attribution (AGT-630, AC1); omitted when no strategy is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_extensions: Option<ClientExtensions>,
}

impl EntryOrderRequest {
    /// Build a resting Limit or Stop entry order (AGT-612, AC2). `price` is the
    /// trigger price; it and any attached SL/TP or price bound are formatted to
    /// the instrument's precision via [`format_price_for_oanda`]. The
    /// time-in-force defaults to `GTC` when `opts.time_in_force` is `None` (a
    /// resting order lives until cancelled), and `gtdTime`/`priceBound`/
    /// `triggerCondition` are omitted from the body when unset.
    pub fn new(
        kind: EntryOrderType,
        instrument: &str,
        units: i64,
        price: &str,
        opts: &EntryOptions,
    ) -> Self {
        let order_type = match kind {
            EntryOrderType::Limit => OrderType::Limit,
            EntryOrderType::Stop => OrderType::Stop,
        };
        Self {
            order: EntryOrderSpec {
                order_type,
                instrument: instrument.to_string(),
                units: units.to_string(),
                price: format_price_for_oanda(instrument, price),
                time_in_force: opts.time_in_force.unwrap_or(TimeInForce::GTC),
                gtd_time: opts.gtd_time.clone(),
                price_bound: opts
                    .price_bound
                    .as_deref()
                    .map(|p| format_price_for_oanda(instrument, p)),
                trigger_condition: opts.trigger_condition,
                position_fill: PositionFill::Default,
                stop_loss_on_fill: opts.stop_loss.as_deref().map(|price| StopLossOnFill {
                    price: format_price_for_oanda(instrument, price),
                    time_in_force: Some(TimeInForce::GTC),
                }),
                take_profit_on_fill: opts.take_profit.as_deref().map(|price| {
                    TakeProfitOnFill {
                        price: format_price_for_oanda(instrument, price),
                        time_in_force: Some(TimeInForce::GTC),
                    }
                }),
                client_extensions: opts
                    .strategy
                    .as_deref()
                    .map(ClientExtensions::for_strategy),
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum StreamMessage {
    #[serde(rename = "PRICE")]
    Price(StreamPrice),
    #[serde(rename = "HEARTBEAT")]
    Heartbeat(StreamHeartbeat),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamPrice {
    pub instrument: String,
    pub time: String,
    pub tradeable: bool,
    pub bids: Vec<PriceBucket>,
    pub asks: Vec<PriceBucket>,
    #[serde(default)]
    pub close_out_bid: Option<String>,
    #[serde(default)]
    pub close_out_ask: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PriceBucket {
    pub price: String,
    pub liquidity: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamHeartbeat {
    pub time: String,
}

#[derive(Debug, Deserialize)]
pub struct OrderCreateResponse {
    #[serde(rename = "orderCreateTransaction")]
    pub order_create_transaction: Option<OrderTransaction>,
    #[serde(rename = "orderFillTransaction")]
    pub order_fill_transaction: Option<OrderFillTransaction>,
    #[serde(rename = "orderCancelTransaction")]
    pub order_cancel_transaction: Option<OrderCancelTransaction>,
    /// AGT-612 (AC3): a *hard* reject — OANDA refused to even create the order
    /// (e.g. a malformed/precision-exceeded body) and returned an
    /// `ORDER_REJECT` transaction instead of an `orderCreateTransaction`. This
    /// is distinct from an `orderCancelTransaction` (the order was created then
    /// cancelled) and lets the classifier tell a hard reject apart from a
    /// resting order.
    #[serde(rename = "orderRejectTransaction")]
    pub order_reject_transaction: Option<OrderRejectTransaction>,
    #[serde(default, rename = "relatedTransactionIDs")]
    pub related_transaction_ids: Vec<String>,
    #[serde(rename = "lastTransactionID")]
    pub last_transaction_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrderTransaction {
    pub id: String,
    pub time: String,
    #[serde(rename = "type")]
    pub transaction_type: String,
    pub instrument: String,
    pub units: String,
    #[serde(rename = "timeInForce")]
    pub time_in_force: String,
    #[serde(rename = "positionFill")]
    pub position_fill: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrderFillTransaction {
    pub id: String,
    pub time: String,
    #[serde(rename = "type")]
    pub transaction_type: String,
    pub instrument: String,
    pub units: String,
    pub price: String,
    #[serde(default)]
    pub pl: String,
    #[serde(default)]
    pub financing: String,
    #[serde(default)]
    pub commission: String,
    #[serde(default, rename = "accountBalance")]
    pub account_balance: String,
    #[serde(default, rename = "tradeOpened")]
    pub trade_opened: Option<TradeOpened>,
    #[serde(default, rename = "tradeReduced")]
    pub trade_reduced: Option<TradeReduced>,
    #[serde(default, rename = "tradesClosed")]
    pub trades_closed: Vec<TradeClosed>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TradeOpened {
    #[serde(rename = "tradeID")]
    pub trade_id: String,
    pub units: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TradeReduced {
    #[serde(rename = "tradeID")]
    pub trade_id: String,
    pub units: String,
    #[serde(rename = "realizedPL")]
    pub realized_pl: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TradeClosed {
    #[serde(rename = "tradeID")]
    pub trade_id: String,
    pub units: String,
    #[serde(rename = "realizedPL")]
    pub realized_pl: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrderCancelTransaction {
    pub id: String,
    pub time: String,
    #[serde(rename = "type")]
    pub transaction_type: String,
    #[serde(rename = "orderID")]
    pub order_id: String,
    pub reason: String,
}

/// A hard-reject transaction (AGT-612, AC3). OANDA emits this — in place of an
/// `orderCreateTransaction` — when it refuses to create the order at all. The
/// reject reason lives in `reject_reason` (OANDA's `rejectReason`); `reason` is
/// carried too for the rare shapes that use it, so the classifier always has a
/// human-readable cause to record on the audit row.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OrderRejectTransaction {
    pub id: String,
    pub time: String,
    #[serde(rename = "type")]
    pub transaction_type: String,
    #[serde(default, rename = "rejectReason")]
    pub reject_reason: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

impl OrderRejectTransaction {
    /// The best available human-readable reject cause, preferring OANDA's
    /// `rejectReason` and falling back to `reason` then a generic label.
    pub fn cause(&self) -> String {
        self.reject_reason
            .clone()
            .or_else(|| self.reason.clone())
            .unwrap_or_else(|| "order rejected".to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosePositionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_units: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_units: Option<String>,
}

impl ClosePositionRequest {
    pub fn close_long() -> Self {
        Self {
            long_units: Some("ALL".to_string()),
            short_units: None,
        }
    }

    pub fn close_short() -> Self {
        Self {
            long_units: None,
            short_units: Some("ALL".to_string()),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ClosePositionResponse {
    #[serde(default, rename = "longOrderFillTransaction")]
    pub long_order_fill_transaction: Option<OrderFillTransaction>,
    #[serde(default, rename = "shortOrderFillTransaction")]
    pub short_order_fill_transaction: Option<OrderFillTransaction>,
    #[serde(default, rename = "relatedTransactionIDs")]
    pub related_transaction_ids: Vec<String>,
    #[serde(rename = "lastTransactionID")]
    pub last_transaction_id: String,
}

// ============================================================================
// Order / Position Book Types (client sentiment snapshots)
// ============================================================================

/// `GET /v3/instruments/{instrument}/orderBook` response wrapper.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderBookResponse {
    pub order_book: OandaBook,
}

/// `GET /v3/instruments/{instrument}/positionBook` response wrapper.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionBookResponse {
    pub position_book: OandaBook,
}

/// One order- or position-book snapshot: OANDA's aggregate view of client
/// orders/positions bucketed by price. Snapshots are published on 20-minute
/// boundaries; historical snapshots are served via the `time` query parameter
/// (verified reachable back to ~2018). NOTE: `unixTime` is absent on older
/// historical snapshots, so it is not modelled — `time` (RFC3339) is the key.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OandaBook {
    pub instrument: String,
    /// Snapshot instant (RFC3339, always a 20-minute boundary).
    pub time: String,
    /// Instrument price at snapshot time (OANDA-precision string).
    pub price: String,
    /// Price width covered by each bucket (OANDA-precision string).
    pub bucket_width: String,
    #[serde(default)]
    pub buckets: Vec<BookBucket>,
}

/// One price bucket of a book: the percentage of all client orders/positions
/// (long and short) sitting in `[price, price + bucketWidth)`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookBucket {
    pub price: String,
    pub long_count_percent: String,
    pub short_count_percent: String,
}

// ============================================================================
// Instruments Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct InstrumentsResponse {
    pub instruments: Vec<OandaInstrument>,
    #[serde(rename = "lastTransactionID")]
    pub last_transaction_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OandaInstrument {
    pub name: String,
    #[serde(rename = "type")]
    pub instrument_type: String,
    pub display_name: String,
    #[serde(default)]
    pub pip_location: Option<i32>,
    #[serde(default)]
    pub display_precision: Option<i32>,
    #[serde(default)]
    pub trade_units_precision: Option<i32>,
    #[serde(default)]
    pub minimum_trade_size: Option<String>,
    #[serde(default)]
    pub maximum_trailing_stop_distance: Option<String>,
    #[serde(default)]
    pub minimum_trailing_stop_distance: Option<String>,
    #[serde(default)]
    pub maximum_position_size: Option<String>,
    #[serde(default)]
    pub maximum_order_units: Option<String>,
    #[serde(default)]
    pub margin_rate: Option<String>,
    /// Financing (swap/carry) terms — current annualized long/short rates
    /// plus the days-charged calendar (Wednesday triple-charges the weekend).
    /// OANDA returns this for every currency instrument; historical rates are
    /// NOT exposed anywhere in v20, so a time series requires sampling this.
    #[serde(default)]
    pub financing: Option<InstrumentFinancing>,
}

/// Per-instrument financing terms (`instrument.financing` in the OANDA
/// account-instruments response). Rates are annualized decimal fractions as
/// OANDA-precision strings (e.g. `"-0.0245"` = −2.45%/yr); long and short are
/// asymmetric — the gap is the broker's financing spread (measured ~1–2%/yr
/// round-trip on G10, wickd-lab STUDY-010).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentFinancing {
    pub long_rate: String,
    pub short_rate: String,
    #[serde(default)]
    pub financing_days_of_week: Vec<FinancingDayOfWeek>,
}

/// One weekday's financing charge multiplier (`daysCharged` 0 on weekends,
/// 3 on Wednesday for FX — the weekend roll).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancingDayOfWeek {
    pub day_of_week: String,
    pub days_charged: i32,
}

// ============================================================================
// Autochartist / ForexLabs Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct AutochartistResponse {
    #[serde(default)]
    pub signals: Vec<AutochartistSignal>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AutochartistSignal {
    pub instrument: String,
    pub data: AutochartistData,
    pub meta: AutochartistMeta,
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default, rename = "type")]
    pub signal_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AutochartistData {
    pub points: AutochartistPoints,
    #[serde(default)]
    pub patternendtime: Option<i64>,
    #[serde(default)]
    pub prediction: Option<AutochartistPrediction>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AutochartistPoints {
    pub support: AutochartistPricePoint,
    pub resistance: AutochartistPricePoint,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AutochartistPricePoint {
    pub y0: f64,
    pub y1: f64,
    #[serde(default)]
    pub x0: Option<i64>,
    #[serde(default)]
    pub x1: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AutochartistPrediction {
    #[serde(default)]
    pub pricelow: Option<f64>,
    #[serde(default)]
    pub pricehigh: Option<f64>,
    #[serde(default)]
    pub timefrom: Option<i64>,
    #[serde(default)]
    pub timeto: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AutochartistMeta {
    pub pattern: String,
    pub probability: f64,
    #[serde(default)]
    pub direction: Option<i32>,  // 1 = up, -1 = down
    #[serde(default)]
    pub completed: Option<i32>,
    #[serde(default)]
    pub interval: Option<i32>,
    #[serde(default)]
    pub trendtype: Option<String>,
    #[serde(default)]
    pub length: Option<i32>,
    #[serde(default)]
    pub scores: Option<AutochartistScores>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AutochartistScores {
    #[serde(default)]
    pub clarity: Option<i32>,
    #[serde(default)]
    pub breakout: Option<i32>,
    #[serde(default)]
    pub quality: Option<i32>,
    #[serde(default)]
    pub initialtrend: Option<i32>,
    #[serde(default)]
    pub uniformity: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_price_for_oanda_standard_pairs() {
        // EUR/USD - should be 5 decimals
        assert_eq!(format_price_for_oanda("EUR_USD", "1.085"), "1.08500");
        assert_eq!(format_price_for_oanda("EUR_USD", "1.08512345"), "1.08512");
        assert_eq!(format_price_for_oanda("EUR_USD", "1.1"), "1.10000");

        // GBP/USD
        assert_eq!(format_price_for_oanda("GBP_USD", "1.26789"), "1.26789");
    }

    #[test]
    fn test_format_price_for_oanda_jpy_pairs() {
        // USD/JPY - should be 3 decimals
        assert_eq!(format_price_for_oanda("USD_JPY", "150.5"), "150.500");
        assert_eq!(format_price_for_oanda("USD_JPY", "150.12345"), "150.123");

        // EUR/JPY
        assert_eq!(format_price_for_oanda("EUR_JPY", "162.789"), "162.789");
        assert_eq!(format_price_for_oanda("EUR_JPY", "162.7"), "162.700");
    }

    #[test]
    fn test_format_price_handles_edge_cases() {
        // Empty string defaults to 0
        assert_eq!(format_price_for_oanda("EUR_USD", ""), "0.00000");

        // Already correct precision
        assert_eq!(format_price_for_oanda("EUR_USD", "1.08500"), "1.08500");
        assert_eq!(format_price_for_oanda("USD_JPY", "150.500"), "150.500");
    }

    // AGT-612 (AC2): the per-instrument precision rule. The old
    // `contains("JPY")` heuristic mis-precisioned spot gold (XAU_USD) to 5 dp —
    // OANDA rejects that with PRICE_PRECISION_EXCEEDED. Precision is now keyed
    // off the quote half of the pair, plus a gold special-case.
    #[test]
    fn price_precision_is_per_instrument() {
        assert_eq!(price_precision("EUR_USD"), 5);
        assert_eq!(price_precision("GBP_USD"), 5);
        // JPY *quote* → 3 dp.
        assert_eq!(price_precision("USD_JPY"), 3);
        assert_eq!(price_precision("EUR_JPY"), 3);
        // Spot gold → 3 dp (the bug: no "JPY" substring, so it used to get 5).
        assert_eq!(price_precision("XAU_USD"), 3);
        assert_eq!(format_price_for_oanda("XAU_USD", "2015.5"), "2015.500");
        assert_eq!(format_price_for_oanda("XAU_USD", "2015.12345"), "2015.123");
    }

    // AGT-612 (AC2): an explicit-precision formatter for callers that already
    // hold the instrument's displayPrecision.
    #[test]
    fn format_price_with_explicit_precision() {
        assert_eq!(format_price_with_precision("1.234567", 5), "1.23457");
        assert_eq!(format_price_with_precision("150.5", 3), "150.500");
        assert_eq!(format_price_with_precision("not-a-number", 5), "0.00000");
    }

    // AGT-612 (AC2): a LIMIT entry serializes to the expected `/orders` body —
    // trigger price precision-formatted, default GTC TIF, and the optional
    // gtd/priceBound/trigger fields omitted when unset.
    #[test]
    fn limit_entry_defaults_gtc_and_formats_price() {
        let req = EntryOrderRequest::new(
            EntryOrderType::Limit,
            "EUR_USD",
            1000,
            "1.075", // under-precisioned on purpose
            &EntryOptions::default(),
        );
        let v = serde_json::to_value(&req).unwrap();
        let order = &v["order"];
        assert_eq!(order["type"], "LIMIT");
        assert_eq!(order["instrument"], "EUR_USD");
        assert_eq!(order["units"], "1000");
        assert_eq!(order["price"], "1.07500"); // formatted to 5 dp
        assert_eq!(order["timeInForce"], "GTC"); // default when unset
        assert_eq!(order["positionFill"], "DEFAULT");
        // Optional fields are omitted, not null.
        assert!(order.get("gtdTime").is_none());
        assert!(order.get("priceBound").is_none());
        assert!(order.get("triggerCondition").is_none());
        assert!(order.get("stopLossOnFill").is_none());
    }

    // AGT-612 (AC2): a STOP entry carries its optional fields through, and any
    // attached SL/TP + price bound are precision-formatted too.
    #[test]
    fn stop_entry_carries_optional_fields() {
        let opts = EntryOptions {
            time_in_force: Some(TimeInForce::GTD),
            gtd_time: Some("2026-07-01T00:00:00Z".to_string()),
            price_bound: Some("1.081".to_string()),
            trigger_condition: Some(TriggerCondition::Bid),
            stop_loss: Some("1.070".to_string()),
            take_profit: Some("1.099".to_string()),
            strategy: None,
        };
        let req = EntryOrderRequest::new(EntryOrderType::Stop, "EUR_USD", -500, "1.08", &opts);
        let v = serde_json::to_value(&req).unwrap();
        let order = &v["order"];
        assert_eq!(order["type"], "STOP");
        assert_eq!(order["units"], "-500");
        assert_eq!(order["price"], "1.08000");
        assert_eq!(order["timeInForce"], "GTD");
        assert_eq!(order["gtdTime"], "2026-07-01T00:00:00Z");
        assert_eq!(order["priceBound"], "1.08100");
        assert_eq!(order["triggerCondition"], "BID");
        assert_eq!(order["stopLossOnFill"]["price"], "1.07000");
        assert_eq!(order["takeProfitOnFill"]["price"], "1.09900");
    }

    // AGT-630 (AC1): a strategy-attributed market order carries OANDA
    // clientExtensions (tag = the strategy name, comment naming wickd as the
    // placer) — the attribution OANDA echoes back on the transaction record.
    #[test]
    fn market_order_with_strategy_serializes_client_extensions() {
        let req = MarketOrderRequest::with_sl_tp("EUR_USD", 1000, Some("1.0850"), None)
            .with_strategy(Some("ma-crossover"));
        let v = serde_json::to_value(&req).unwrap();
        let order = &v["order"];
        assert_eq!(order["clientExtensions"]["tag"], "ma-crossover");
        assert_eq!(
            order["clientExtensions"]["comment"],
            "wickd strategy=ma-crossover"
        );
        // The rest of the body is unchanged by attribution.
        assert_eq!(order["type"], "MARKET");
        assert_eq!(order["stopLossOnFill"]["price"], "1.08500");
    }

    // AGT-630 (AC1): with NO strategy, clientExtensions is omitted entirely —
    // the POST body stays byte-identical to the pre-AGT-630 shape.
    #[test]
    fn market_order_without_strategy_omits_client_extensions() {
        let plain = MarketOrderRequest::with_sl_tp("EUR_USD", 1000, None, None);
        let v = serde_json::to_value(&plain).unwrap();
        assert!(v["order"].get("clientExtensions").is_none());

        // Threading `None` through the builder is the same as never calling it.
        let threaded = MarketOrderRequest::with_sl_tp("EUR_USD", 1000, None, None)
            .with_strategy(None);
        let v = serde_json::to_value(&threaded).unwrap();
        assert!(v["order"].get("clientExtensions").is_none());
    }

    // AGT-630 (AC1): a resting limit/stop entry order carries the same
    // attribution via EntryOptions.strategy — and omits it when unset.
    #[test]
    fn entry_order_strategy_attribution_via_options() {
        let opts = EntryOptions {
            strategy: Some("rsi-reversion".to_string()),
            ..EntryOptions::default()
        };
        let req = EntryOrderRequest::new(EntryOrderType::Limit, "EUR_USD", 1000, "1.075", &opts);
        let v = serde_json::to_value(&req).unwrap();
        let order = &v["order"];
        assert_eq!(order["clientExtensions"]["tag"], "rsi-reversion");
        assert_eq!(
            order["clientExtensions"]["comment"],
            "wickd strategy=rsi-reversion"
        );

        let bare = EntryOrderRequest::new(
            EntryOrderType::Stop,
            "EUR_USD",
            1000,
            "1.09",
            &EntryOptions::default(),
        );
        let v = serde_json::to_value(&bare).unwrap();
        assert!(v["order"].get("clientExtensions").is_none());
    }

    // AGT-612 (AC3): a hard `orderRejectTransaction` deserializes and exposes a
    // human-readable cause preferring OANDA's `rejectReason`.
    #[test]
    fn order_reject_transaction_deserializes_with_cause() {
        let resp: OrderCreateResponse = serde_json::from_value(serde_json::json!({
            "orderRejectTransaction": {
                "id": "42",
                "time": "2026-07-01T00:00:00.000000000Z",
                "type": "MARKET_ORDER_REJECT",
                "rejectReason": "PRICE_PRECISION_EXCEEDED"
            },
            "lastTransactionID": "42"
        }))
        .unwrap();
        let reject = resp.order_reject_transaction.expect("reject txn present");
        assert_eq!(reject.cause(), "PRICE_PRECISION_EXCEEDED");
        // A reject with neither field still yields a generic cause.
        let bare = OrderRejectTransaction {
            id: "1".into(),
            time: "t".into(),
            transaction_type: "ORDER_REJECT".into(),
            reject_reason: None,
            reason: None,
        };
        assert_eq!(bare.cause(), "order rejected");
    }
}
