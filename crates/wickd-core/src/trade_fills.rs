//! Per-fill decomposition of a closed trade (AGT-782).
//!
//! `/trades` reports a trade's exit as `averageClosePrice` — already blended
//! across every exit, so on a multi-exit trade it is not a price anything
//! filled at. [`crate::models::Trade::exit_count`] can *detect* that, which is
//! why the history marks such a trade `blended`, but it cannot undo it.
//!
//! The individual fills are in the transaction feed
//! ([`crate::oanda::endpoints::get_transactions_idrange`]): an `ORDER_FILL`
//! carries `tradeOpened` for the fill that opened a trade and
//! `tradeReduced` / `tradesClosed` for each fill that took units back out,
//! each with its own price, units and realized P&L. This module turns that
//! feed into per-trade entry/exit rows.
//!
//! **Pure**: no I/O. The caller fetches; this maps. That keeps the
//! reconciliation rules testable off fixtures.
//!
//! ## One entry per trade — scale-ins are several trades
//!
//! OANDA opens a trade with exactly one fill. Adding to a position does not
//! add an entry to an existing trade; it opens a *new* trade that the position
//! aggregates. So a decomposed trade has at most one entry and any number of
//! exits, and a scale-in appears in the history as multiple trades rather than
//! one trade with two entries. Grouping those trades back into a position is a
//! presentation decision and deliberately not made here.

use std::collections::HashMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::models::Trade;
use crate::oanda::types::{OrderFillTransaction, Transaction};

/// One fill against a trade — the entry that opened it or an exit that took
/// units out of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    /// The `ORDER_FILL` transaction this fill came from.
    pub transaction_id: String,
    /// RFC3339 fill time, as OANDA reported it.
    pub time: String,
    /// The price this fill actually happened at — the whole point of
    /// decomposing, as against the blended average.
    pub price: Decimal,
    /// Units moved, signed as OANDA signs them: positive when the fill added
    /// to a long (or removed from a short), negative the other way. An exit's
    /// sign is therefore opposite its trade's direction.
    pub units: Decimal,
    /// Realized P&L booked by this fill. Always zero on an entry — opening a
    /// trade realizes nothing.
    pub realized_pl: Decimal,
}

/// A trade's fills, plus whether they can be trusted to be the whole story.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeFills {
    /// The fill that opened the trade, when the feed covered it. `None` when
    /// the entry is older than the fetched range — the exits can still be
    /// complete and useful on their own.
    pub entry: Option<Fill>,
    /// Every fill that took units out of the trade, oldest first.
    pub exits: Vec<Fill>,
    /// Whether the exits account for the trade in full: their units sum to the
    /// trade's size and their realized P&L sums to the trade's realized P&L.
    ///
    /// A decomposition that does not reconcile is **not** shown as the trade's
    /// exits — a partial set of rows presented as the whole set is a worse lie
    /// than the blended average it replaces. Callers fall back to `blended`.
    pub reconciled: bool,
}

impl TradeFills {
    /// Whether this decomposition is safe to show in place of the blended
    /// average: it reconciles and there is at least one exit to show.
    pub fn is_showable(&self) -> bool {
        self.reconciled && !self.exits.is_empty()
    }
}

/// Decompose `trades` against the transaction feed in `transactions`,
/// returning fills keyed by trade id. Trades the feed does not cover are
/// absent from the map — the caller keeps its blended display for those.
///
/// `transactions` is expected in feed order (ascending id); exits are emitted
/// in the order encountered, so they read oldest-first like the feed.
pub fn decompose(trades: &[Trade], transactions: &[Transaction]) -> HashMap<String, TradeFills> {
    let wanted: HashMap<&str, &Trade> = trades.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut fills: HashMap<String, TradeFills> = HashMap::new();

    for fill in transactions.iter().filter_map(Transaction::order_fill) {
        // The fill that opened a trade. `units` here is the trade's size, not
        // the order's — they differ when one order opens and closes at once.
        if let Some(opened) = &fill.trade_opened {
            if wanted.contains_key(opened.trade_id.as_str()) {
                if let Some(entry) = build_fill(fill, &opened.units, "0") {
                    fills.entry(opened.trade_id.clone()).or_insert_with(empty_fills).entry =
                        Some(entry);
                }
            }
        }

        // A partial close of a trade that stayed open.
        if let Some(reduced) = &fill.trade_reduced {
            if wanted.contains_key(reduced.trade_id.as_str()) {
                if let Some(exit) = build_fill(fill, &reduced.units, &reduced.realized_pl) {
                    fills
                        .entry(reduced.trade_id.clone())
                        .or_insert_with(empty_fills)
                        .exits
                        .push(exit);
                }
            }
        }

        // Trades this fill closed outright. One fill can close several.
        for closed in &fill.trades_closed {
            if wanted.contains_key(closed.trade_id.as_str()) {
                if let Some(exit) = build_fill(fill, &closed.units, &closed.realized_pl) {
                    fills
                        .entry(closed.trade_id.clone())
                        .or_insert_with(empty_fills)
                        .exits
                        .push(exit);
                }
            }
        }
    }

    // Reconcile once, at the end, when every fill for a trade has been seen.
    for (trade_id, decomposed) in fills.iter_mut() {
        let Some(trade) = wanted.get(trade_id.as_str()) else {
            continue;
        };
        decomposed.reconciled = reconciles(trade, decomposed);
    }
    fills
}

fn empty_fills() -> TradeFills {
    TradeFills { entry: None, exits: Vec::new(), reconciled: false }
}

/// Build a [`Fill`] from the fill transaction plus the per-trade units and
/// realized P&L. Returns `None` if any of the numbers do not parse — an
/// unparseable fill makes the decomposition incomplete, which reconciliation
/// then catches, rather than being papered over with a zero.
fn build_fill(fill: &OrderFillTransaction, units: &str, realized_pl: &str) -> Option<Fill> {
    Some(Fill {
        transaction_id: fill.id.clone(),
        time: fill.time.clone(),
        price: fill.price.parse::<Decimal>().ok()?,
        units: units.parse::<Decimal>().ok()?,
        realized_pl: realized_pl.parse::<Decimal>().ok()?,
    })
}

/// Whether the exits account for the trade in full, on both units and money.
///
/// Units are compared by magnitude because an exit is signed against its
/// trade's direction: a 1000-unit long is closed by fills totalling -1000.
fn reconciles(trade: &Trade, decomposed: &TradeFills) -> bool {
    if decomposed.exits.is_empty() {
        return false;
    }
    let exited: Decimal = decomposed.exits.iter().map(|f| f.units.abs()).sum();
    let realized: Decimal = decomposed.exits.iter().map(|f| f.realized_pl).sum();
    exited == trade.units.abs() && realized == trade.realized_pl
}

/// The transaction id range that covers `trades` — the oldest trade's opening
/// transaction through the newest closing transaction — or `None` when the
/// trades carry no usable ids.
///
/// A trade's id *is* the id of the transaction that opened it, so the low
/// bound comes from the trade ids themselves and the high bound from
/// [`Trade::closing_transaction_ids`]. Bounding the fetch this tightly is what
/// keeps a decomposition from walking the account's whole transaction history.
pub fn covering_id_range(trades: &[Trade]) -> Option<(u64, u64)> {
    let opening = trades.iter().filter_map(|t| t.id.parse::<u64>().ok());
    let closing = trades
        .iter()
        .flat_map(|t| t.closing_transaction_ids.iter())
        .filter_map(|id| id.parse::<u64>().ok());

    let low = opening.clone().min()?;
    // The high bound must still be defined for a set with no closing ids at
    // all (all-open trades), where the opening ids are all there is.
    let high = closing.chain(opening).max()?;
    Some((low, high.max(low)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TradeState;
    use chrono::{DateTime, Utc};

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    /// A closed long of `units`, realizing `realized_pl`, closed by `closing`.
    fn trade(id: &str, units: &str, realized_pl: &str, closing: &[&str]) -> Trade {
        Trade {
            id: id.to_string(),
            instrument: "EUR_USD".to_string(),
            open_price: dec("1.08000"),
            open_time: ts("2026-07-20T10:00:00Z"),
            units: dec(units),
            realized_pl: dec(realized_pl),
            unrealized_pl: None,
            state: TradeState::Closed,
            close_time: Some(ts("2026-07-20T15:00:00Z")),
            close_price: Some(dec("1.09000")),
            strategy: None,
            exit_count: closing.len(),
            closing_transaction_ids: closing.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn fill_tx(value: serde_json::Value) -> Transaction {
        serde_json::from_value(value).expect("fixture decodes")
    }

    /// An ORDER_FILL that opened `trade_id` with `units`.
    fn opening(id: &str, trade_id: &str, units: &str, price: &str) -> Transaction {
        fill_tx(serde_json::json!({
            "id": id, "time": "2026-07-20T10:00:00Z", "type": "ORDER_FILL",
            "reason": "MARKET_ORDER", "instrument": "EUR_USD",
            "units": units, "price": price,
            "tradeOpened": { "tradeID": trade_id, "units": units }
        }))
    }

    /// An ORDER_FILL that reduced `trade_id` by `units`, realizing `pl`.
    fn reducing(id: &str, trade_id: &str, units: &str, price: &str, pl: &str) -> Transaction {
        fill_tx(serde_json::json!({
            "id": id, "time": "2026-07-20T12:00:00Z", "type": "ORDER_FILL",
            "reason": "MARKET_ORDER_TRADE_CLOSE", "instrument": "EUR_USD",
            "units": units, "price": price,
            "tradeReduced": { "tradeID": trade_id, "units": units, "realizedPL": pl }
        }))
    }

    /// An ORDER_FILL that closed `trade_id` outright.
    fn closing(id: &str, trade_id: &str, units: &str, price: &str, pl: &str) -> Transaction {
        fill_tx(serde_json::json!({
            "id": id, "time": "2026-07-20T15:00:00Z", "type": "ORDER_FILL",
            "reason": "MARKET_ORDER_TRADE_CLOSE", "instrument": "EUR_USD",
            "units": units, "price": price,
            "tradesClosed": [{ "tradeID": trade_id, "units": units, "realizedPL": pl }]
        }))
    }

    // AC1: a trade closed in two exits reports each with its own price, units
    // and realized P&L — not one blended average.
    #[test]
    fn a_two_exit_trade_decomposes_into_its_real_fills() {
        let trades = vec![trade("4001", "1000", "12.4000", &["4050", "4090"])];
        let feed = vec![
            opening("4001", "4001", "1000", "1.08000"),
            reducing("4050", "4001", "-400", "1.08500", "4.0000"),
            closing("4090", "4001", "-600", "1.09400", "8.4000"),
        ];

        let out = decompose(&trades, &feed);
        let fills = out.get("4001").expect("decomposed");

        assert_eq!(fills.exits.len(), 2);
        // Each exit carries the price it actually filled at, and neither is
        // the trade's blended 1.09000 close price.
        assert_eq!(fills.exits[0].price, dec("1.08500"));
        assert_eq!(fills.exits[0].units, dec("-400"));
        assert_eq!(fills.exits[0].realized_pl, dec("4.0000"));
        assert_eq!(fills.exits[1].price, dec("1.09400"));
        assert_eq!(fills.exits[1].realized_pl, dec("8.4000"));
        assert_eq!(fills.entry.as_ref().unwrap().price, dec("1.08000"));
        assert_eq!(fills.entry.as_ref().unwrap().realized_pl, Decimal::ZERO);

        assert!(fills.reconciled, "400 + 600 units and 4.0 + 8.4 P&L account for the trade");
        assert!(fills.is_showable());
    }

    #[test]
    fn exits_come_back_oldest_first() {
        let trades = vec![trade("4001", "900", "3.0000", &["4050", "4060", "4090"])];
        let feed = vec![
            reducing("4050", "4001", "-300", "1.08100", "1.0000"),
            reducing("4060", "4001", "-300", "1.08200", "1.0000"),
            closing("4090", "4001", "-300", "1.08300", "1.0000"),
        ];

        let fills = decompose(&trades, &feed).remove("4001").expect("decomposed");

        let ids: Vec<&str> = fills.exits.iter().map(|f| f.transaction_id.as_str()).collect();
        assert_eq!(ids, ["4050", "4060", "4090"]);
    }

    // AC3: a decomposition that does not add up is not presented as the truth.
    #[test]
    fn a_short_decomposition_does_not_reconcile() {
        // The feed covers only one of the two exits (the other predates the
        // fetched range), so 400 of a 1000-unit trade is all it can see.
        let trades = vec![trade("4001", "1000", "12.4000", &["4050", "4090"])];
        let feed = vec![reducing("4050", "4001", "-400", "1.08500", "4.0000")];

        let fills = decompose(&trades, &feed).remove("4001").expect("partially decomposed");

        assert_eq!(fills.exits.len(), 1);
        assert!(!fills.reconciled, "units do not account for the trade");
        assert!(!fills.is_showable(), "a partial set must not replace the blend");
    }

    #[test]
    fn units_that_add_up_but_money_that_does_not_still_fails() {
        // Same units, wrong P&L — the likelier real-world corruption, and the
        // one a units-only check would wave through.
        let trades = vec![trade("4001", "1000", "12.4000", &["4090"])];
        let feed = vec![closing("4090", "4001", "-1000", "1.09000", "9.9900")];

        let fills = decompose(&trades, &feed).remove("4001").expect("decomposed");

        assert!(!fills.reconciled);
    }

    #[test]
    fn a_short_trade_reconciles_against_its_positive_exit_units() {
        // A -1000 short is closed by +1000. Comparing signed sums would fail
        // here; magnitudes are what reconcile.
        let trades = vec![trade("4001", "-1000", "5.0000", &["4090"])];
        let feed = vec![closing("4090", "4001", "1000", "1.07500", "5.0000")];

        let fills = decompose(&trades, &feed).remove("4001").expect("decomposed");

        assert!(fills.reconciled);
        assert_eq!(fills.exits[0].units, dec("1000"));
    }

    #[test]
    fn one_fill_closing_several_trades_is_attributed_to_each() {
        // A close that swept two trades at once: both must see it, with their
        // own units and P&L, not one of them or a merged row.
        let trades = vec![
            trade("4001", "1000", "4.0000", &["4090"]),
            trade("4002", "500", "2.0000", &["4090"]),
        ];
        let feed = vec![fill_tx(serde_json::json!({
            "id": "4090", "time": "2026-07-20T15:00:00Z", "type": "ORDER_FILL",
            "reason": "MARKET_ORDER_TRADE_CLOSE", "instrument": "EUR_USD",
            "units": "-1500", "price": "1.09000",
            "tradesClosed": [
                { "tradeID": "4001", "units": "-1000", "realizedPL": "4.0000" },
                { "tradeID": "4002", "units": "-500", "realizedPL": "2.0000" }
            ]
        }))];

        let out = decompose(&trades, &feed);

        assert!(out["4001"].reconciled && out["4002"].reconciled);
        assert_eq!(out["4001"].exits[0].units, dec("-1000"));
        assert_eq!(out["4002"].exits[0].units, dec("-500"));
    }

    // AC4: a trade the feed does not cover is simply absent, so the caller
    // keeps its blended display rather than showing a gap.
    #[test]
    fn a_trade_the_feed_does_not_cover_is_absent_rather_than_empty() {
        let trades = vec![trade("4001", "1000", "12.4000", &["4090"])];
        let feed = vec![closing("4090", "9999", "-1000", "1.09000", "12.4000")];

        assert!(!decompose(&trades, &feed).contains_key("4001"));
    }

    #[test]
    fn non_fill_transactions_are_ignored() {
        let trades = vec![trade("4001", "1000", "12.4000", &["4090"])];
        let feed = vec![
            fill_tx(serde_json::json!({
                "id": "4002", "time": "2026-07-20T10:00:01Z", "type": "STOP_LOSS_ORDER",
                "tradeID": "4001", "price": "1.07000"
            })),
            fill_tx(serde_json::json!({
                "id": "4003", "time": "2026-07-20T21:00:00Z", "type": "DAILY_FINANCING",
                "financing": "-0.4500"
            })),
            closing("4090", "4001", "-1000", "1.09000", "12.4000"),
        ];

        let fills = decompose(&trades, &feed).remove("4001").expect("decomposed");

        assert_eq!(fills.exits.len(), 1, "only the fill counts as an exit");
        assert!(fills.reconciled);
    }

    #[test]
    fn an_unparseable_number_makes_the_decomposition_fail_reconciliation() {
        // Rather than being silently read as zero and reconciling wrongly.
        let trades = vec![trade("4001", "1000", "12.4000", &["4090"])];
        let feed = vec![fill_tx(serde_json::json!({
            "id": "4090", "time": "2026-07-20T15:00:00Z", "type": "ORDER_FILL",
            "instrument": "EUR_USD", "units": "-1000", "price": "not-a-price",
            "tradesClosed": [{ "tradeID": "4001", "units": "-1000", "realizedPL": "12.4000" }]
        }))];

        assert!(!decompose(&trades, &feed).contains_key("4001"));
    }

    // ── the fetch window ──────────────────────────────────────────────────

    #[test]
    fn the_covering_range_spans_the_oldest_entry_to_the_newest_exit() {
        let trades = vec![
            trade("4001", "1000", "1.0000", &["4090"]),
            trade("4300", "1000", "1.0000", &["4500", "4402"]),
        ];

        assert_eq!(covering_id_range(&trades), Some((4001, 4500)));
    }

    #[test]
    fn the_covering_range_of_trades_with_no_closes_is_the_trades_themselves() {
        let trades = vec![trade("4001", "1000", "1.0000", &[]), trade("4300", "1000", "1.0000", &[])];

        assert_eq!(covering_id_range(&trades), Some((4001, 4300)));
    }

    #[test]
    fn there_is_no_covering_range_without_trades() {
        assert_eq!(covering_id_range(&[]), None);
    }

    #[test]
    fn non_numeric_trade_ids_do_not_produce_a_range() {
        let mut odd = trade("not-a-number", "1000", "1.0000", &[]);
        odd.closing_transaction_ids = vec!["also-not".to_string()];

        assert_eq!(covering_id_range(&[odd]), None);
    }
}
