//! Live dashboard state model for `wickd dashboard` (AGT-616).
//!
//! The terminal UI (`crate::commands::dashboard`) is thin: it attaches to the
//! `wickd stream` socket hub (`~/.wickd/stream.sock`, AGT-615) and feeds every
//! NDJSON line it reads into a [`DashboardState`], then renders that state as a
//! watchlist table. Splitting the *state* out from the render loop keeps the
//! interesting logic — parsing an NDJSON line into a bid/ask/spread row and
//! folding it into the current view — unit-testable without standing up a real
//! terminal (a full ratatui render loop is not).
//!
//! Only `price-update` lines populate rows; `stream-error` / `stream-health`
//! lines update the status banner. Anything else (or a malformed line) is
//! ignored rather than crashing the TUI — a live stream must not be taken down
//! by one garbage line.

use std::collections::BTreeMap;

use serde::Deserialize;

/// One watchlist row: the latest quote seen for a single instrument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub instrument: String,
    pub bid: String,
    pub ask: String,
    pub spread: String,
    pub tradeable: bool,
    /// OANDA event timestamp of the last tick for this instrument (may be empty
    /// if the stream line carried none).
    pub time: String,
}

/// The `price-update` NDJSON payload we care about (see
/// `wickd_core::oanda::streaming::PriceUpdate` and `crate::sink`). Every
/// field but `instrument`/`bid`/`ask` is optional so a slightly different line
/// shape (or a missing `spread`) degrades gracefully instead of dropping the
/// tick.
#[derive(Debug, Deserialize)]
struct PriceLine {
    instrument: String,
    bid: String,
    ask: String,
    #[serde(default)]
    spread: Option<String>,
    #[serde(default)]
    time: Option<String>,
    #[serde(default)]
    tradeable: Option<bool>,
}

/// The full dashboard view: one row per instrument (sorted by symbol so the
/// table doesn't jump around as ticks arrive), plus a running update count and
/// the latest stream status.
#[derive(Debug, Default)]
pub struct DashboardState {
    rows: BTreeMap<String, Row>,
    /// Total `price-update` lines folded in — a cheap "is data flowing?" signal
    /// for the status banner.
    pub updates: u64,
    /// Last `stream-error` message seen, if any.
    pub last_error: Option<String>,
    /// Last `stream-health` `healthy` flag seen, if any.
    pub healthy: Option<bool>,
}

impl DashboardState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one NDJSON line into the state. Returns `true` if the visible view
    /// changed (so the render loop can skip redundant redraws). Malformed lines
    /// and unrecognized events are ignored (return `false`) rather than
    /// propagating an error — a live TUI must survive junk on the wire.
    pub fn apply_line(&mut self, line: &str) -> bool {
        let line = line.trim();
        if line.is_empty() {
            return false;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return false,
        };

        match value.get("event").and_then(|e| e.as_str()) {
            Some("price-update") => {
                let Ok(p) = serde_json::from_value::<PriceLine>(value) else {
                    return false;
                };
                let spread = p
                    .spread
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| compute_spread(&p.bid, &p.ask));
                let row = Row {
                    instrument: p.instrument.clone(),
                    bid: p.bid,
                    ask: p.ask,
                    spread,
                    tradeable: p.tradeable.unwrap_or(true),
                    time: p.time.unwrap_or_default(),
                };
                self.rows.insert(p.instrument, row);
                self.updates += 1;
                true
            }
            Some("stream-error") => {
                self.last_error = value
                    .get("message")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string());
                true
            }
            Some("stream-health") => {
                let healthy = value.get("healthy").and_then(|b| b.as_bool());
                let changed = healthy != self.healthy;
                self.healthy = healthy;
                changed
            }
            _ => false,
        }
    }

    /// Rows in stable (symbol-sorted) order for rendering.
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.values()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Compute `ask - bid` as a fallback when a `price-update` line carries no
/// explicit `spread`. Returns an empty string if either side isn't a decimal.
fn compute_spread(bid: &str, ask: &str) -> String {
    use std::str::FromStr;

    use rust_decimal::Decimal;

    match (Decimal::from_str(bid), Decimal::from_str(ask)) {
        (Ok(b), Ok(a)) => (a - b).to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price_line(instrument: &str, bid: &str, ask: &str, spread: &str) -> String {
        format!(
            r#"{{"event":"price-update","instrument":"{instrument}","bid":"{bid}","ask":"{ask}","spread":"{spread}","time":"2026-06-30T00:00:00Z","tradeable":true}}"#
        )
    }

    // A price-update line becomes exactly one row with the parsed quote.
    #[test]
    fn price_update_line_populates_a_row() {
        let mut state = DashboardState::new();
        assert!(state.apply_line(&price_line("EUR_USD", "1.0850", "1.0852", "0.0002")));
        assert_eq!(state.len(), 1);
        let row = state.rows().next().unwrap();
        assert_eq!(row.instrument, "EUR_USD");
        assert_eq!(row.bid, "1.0850");
        assert_eq!(row.ask, "1.0852");
        assert_eq!(row.spread, "0.0002");
        assert!(row.tradeable);
        assert_eq!(state.updates, 1);
    }

    // A later tick for the same instrument replaces the row in place — one row,
    // latest quote — rather than appending a duplicate.
    #[test]
    fn later_tick_updates_row_in_place() {
        let mut state = DashboardState::new();
        state.apply_line(&price_line("EUR_USD", "1.0850", "1.0852", "0.0002"));
        state.apply_line(&price_line("EUR_USD", "1.0860", "1.0863", "0.0003"));
        assert_eq!(state.len(), 1, "same instrument stays a single row");
        let row = state.rows().next().unwrap();
        assert_eq!(row.bid, "1.0860");
        assert_eq!(row.ask, "1.0863");
        assert_eq!(state.updates, 2);
    }

    // Rows render in stable, symbol-sorted order regardless of arrival order, so
    // the table doesn't reshuffle as ticks stream in.
    #[test]
    fn rows_are_sorted_by_instrument() {
        let mut state = DashboardState::new();
        state.apply_line(&price_line("USD_JPY", "157.10", "157.12", "0.02"));
        state.apply_line(&price_line("EUR_USD", "1.0850", "1.0852", "0.0002"));
        state.apply_line(&price_line("GBP_USD", "1.2500", "1.2503", "0.0003"));
        let order: Vec<&str> = state.rows().map(|r| r.instrument.as_str()).collect();
        assert_eq!(order, ["EUR_USD", "GBP_USD", "USD_JPY"]);
    }

    // A missing/empty spread is computed from ask - bid so the column is never
    // blank when we can derive it.
    #[test]
    fn missing_spread_is_computed_from_bid_ask() {
        let mut state = DashboardState::new();
        let line = r#"{"event":"price-update","instrument":"EUR_USD","bid":"1.0850","ask":"1.0853"}"#;
        assert!(state.apply_line(line));
        let row = state.rows().next().unwrap();
        assert_eq!(row.spread, "0.0003");
    }

    // Malformed / non-JSON lines are ignored and never add rows or panic.
    #[test]
    fn malformed_line_is_ignored() {
        let mut state = DashboardState::new();
        assert!(!state.apply_line("not json at all"));
        assert!(!state.apply_line("{ partial"));
        assert!(!state.apply_line(""));
        assert!(state.is_empty());
    }

    // Non-price events don't create rows: health updates the banner flag,
    // errors set the last-error message.
    #[test]
    fn non_price_events_do_not_add_rows() {
        let mut state = DashboardState::new();
        assert!(state.apply_line(r#"{"event":"stream-health","healthy":true}"#));
        assert_eq!(state.healthy, Some(true));
        assert!(state.is_empty());

        assert!(state
            .apply_line(r#"{"event":"stream-error","message":"connection lost"}"#));
        assert_eq!(state.last_error.as_deref(), Some("connection lost"));
        assert!(state.is_empty(), "error/health lines never add price rows");
    }
}
