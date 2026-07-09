//! Human-readable terminal feed for alert fires (AGT-619).
//!
//! Both alert sources — price-level alerts (`wickd alert run`, via
//! [`crate::sink::AlertSink`]) and strategy-signal alerts (`wickd watch`, via
//! [`crate::signal_alert::AlertSink`]) — emit structured NDJSON by default so
//! machine consumers stay unaffected. This module adds the *human* rendering
//! variant, selected by `--format human` on either command: one clear line per
//! fire, tagged with the alert type (`[price-level]` vs `[strategy-signal]`)
//! and carrying the instrument, direction, and a timestamp (AC2).
//!
//! The line builders are **pure** — they take the fire's fields plus a
//! pre-rendered timestamp string and return a single line — so the sinks stay a
//! thin wiring layer and the formatting is unit-tested synthetically (mirroring
//! how [`crate::sink`]/[`crate::alert`] test the render path without stdout).

use clap::ValueEnum;

use crate::alert::{Direction, Fired};

/// Output format for the two alert-delivery commands (`wickd alert run`,
/// `wickd watch`). NDJSON is the default so nothing machine-facing breaks;
/// `human` swaps to the live terminal feed built here.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[value(rename_all = "kebab-case")]
pub enum Format {
    /// One JSON object per line (default) — machine-readable, unchanged behavior.
    #[default]
    Ndjson,
    /// One human-readable line per alert fire — the live terminal feed (AC1).
    Human,
}

/// Kebab-case label for a price-level cross direction, matching the on-disk
/// alert schema (`cross-up` | `cross-down` | `either`).
fn direction_label(direction: Direction) -> &'static str {
    match direction {
        Direction::CrossUp => "cross-up",
        Direction::CrossDown => "cross-down",
        Direction::Either => "either",
    }
}

/// Render one price-level fire as a terminal-feed line (AC1/AC2). `time` is the
/// triggering tick's RFC3339 timestamp (`PriceUpdate::time`).
pub fn price_level_line(instrument: &str, fired: &Fired, time: &str) -> String {
    // Pad the tag to the width of the longer `[strategy-signal]` so the
    // instrument column lines up across both feed sources.
    format!(
        "{time}  {tag:<17}  {instrument}  {dir}  @ {price} (level {level})",
        tag = "[price-level]",
        dir = direction_label(fired.direction),
        price = fired.price,
        level = fired.level,
    )
}

/// Render one strategy-signal fire as a terminal-feed line (AC1/AC2). `signal`
/// is the actionable direction ("buy"/"sell", from
/// [`crate::signal_alert::AlertSignal::as_str`]); `time` is the pattern match's
/// RFC3339 timestamp.
pub fn strategy_signal_line(
    instrument: &str,
    signal: &str,
    strategy: &str,
    timeframe: &str,
    time: &str,
) -> String {
    format!(
        "{time}  {tag:<17}  {instrument}  {signal}  ({strategy} {timeframe})",
        tag = "[strategy-signal]",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn fired(direction: Direction) -> Fired {
        Fired {
            alert_id: "a1".to_string(),
            level: dec!(1.0900),
            direction,
            price: dec!(1.0905),
        }
    }

    // AC2: a price-level fire line names its type, instrument, direction, and
    // time — and is a single line (feed invariant).
    #[test]
    fn price_level_line_carries_type_instrument_direction_time() {
        let line = price_level_line("EUR_USD", &fired(Direction::CrossUp), "2026-06-30T00:00:00Z");
        assert!(!line.contains('\n'));
        assert!(line.contains("[price-level]"));
        assert!(!line.contains("[strategy-signal]"), "must not be mistaken for a strategy signal");
        assert!(line.contains("EUR_USD"));
        assert!(line.contains("cross-up"));
        assert!(line.contains("2026-06-30T00:00:00Z"));
        assert!(line.contains("1.0905"), "shows the triggering price");
        assert!(line.contains("1.0900"), "shows the level");
    }

    // AC2: a strategy-signal fire line names its (different) type, instrument,
    // direction (buy/sell), and time.
    #[test]
    fn strategy_signal_line_carries_type_instrument_direction_time() {
        let line = strategy_signal_line("GBP_USD", "sell", "ma-crossover", "H1", "2026-06-30T12:00:00Z");
        assert!(!line.contains('\n'));
        assert!(line.contains("[strategy-signal]"));
        assert!(!line.contains("[price-level]"), "must not be mistaken for a price-level alert");
        assert!(line.contains("GBP_USD"));
        assert!(line.contains("sell"));
        assert!(line.contains("2026-06-30T12:00:00Z"));
        assert!(line.contains("ma-crossover"));
        assert!(line.contains("H1"));
    }

    // The two sources render distinguishable tags for the same instrument —
    // AC2's "clearly distinguishes alert type".
    #[test]
    fn the_two_alert_types_are_distinguishable() {
        let price = price_level_line("EUR_USD", &fired(Direction::CrossDown), "t");
        let signal = strategy_signal_line("EUR_USD", "buy", "rsi", "H1", "t");
        assert_ne!(price, signal);
        assert!(price.contains("[price-level]") && !signal.contains("[price-level]"));
        assert!(signal.contains("[strategy-signal]") && !price.contains("[strategy-signal]"));
    }

    #[test]
    fn direction_labels_are_kebab_case() {
        assert_eq!(direction_label(Direction::CrossUp), "cross-up");
        assert_eq!(direction_label(Direction::CrossDown), "cross-down");
        assert_eq!(direction_label(Direction::Either), "either");
    }

    // NDJSON stays the default so existing machine consumers are unaffected.
    #[test]
    fn format_defaults_to_ndjson() {
        assert_eq!(Format::default(), Format::Ndjson);
    }
}
