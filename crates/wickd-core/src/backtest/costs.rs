//! Per-instrument execution-cost defaults for backtesting.
//!
//! The engine simulates fills at mid ± `spread_pips × pip_value` per side
//! (see `BacktestEngine::spread_amount`), so `spread_pips` is a HALF-spread:
//! the round-trip cost of a trade is `2 × spread_pips` pips. Historically both
//! values were hardcoded (1 pip/side, pip 0.0001) which was roughly right for
//! non-JPY majors and ~100× too small for JPY-quoted pairs. These helpers give
//! each instrument a realistic default; the CLI exposes `--spread-pips` to
//! override.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Price value of one pip for the instrument: 0.01 for JPY-quoted pairs,
/// 0.0001 for everything else (standard FX convention).
pub fn pip_value_for(instrument: &str) -> Decimal {
    match instrument.split_once('_') {
        Some((_, quote)) if quote.eq_ignore_ascii_case("JPY") => dec!(0.01),
        _ => dec!(0.0001),
    }
}

/// Default HALF-spread (pips charged per side; round-trip = 2×) by instrument.
///
/// Values approximate OANDA's typical average spreads, rounded UP so
/// backtests err conservative. Unknown instruments keep the legacy
/// 1 pip/side default.
pub fn default_half_spread_pips(instrument: &str) -> Decimal {
    match instrument.to_ascii_uppercase().as_str() {
        // tightest majors (~1.2–1.4 pips round-trip)
        "EUR_USD" | "USD_JPY" | "AUD_USD" => dec!(0.7),
        "GBP_USD" => dec!(0.8),
        "USD_CHF" | "USD_CAD" => dec!(0.9),
        "NZD_USD" | "EUR_GBP" | "EUR_JPY" | "GBP_JPY" | "AUD_JPY" | "EUR_CHF" => dec!(1.0),
        // wide crosses
        "AUD_NZD" => dec!(1.8),
        // legacy default for anything unlisted
        _ => dec!(1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpy_quoted_pairs_use_jpy_pip() {
        assert_eq!(pip_value_for("USD_JPY"), dec!(0.01));
        assert_eq!(pip_value_for("EUR_JPY"), dec!(0.01));
        assert_eq!(pip_value_for("AUD_JPY"), dec!(0.01));
    }

    #[test]
    fn non_jpy_pairs_use_standard_pip() {
        assert_eq!(pip_value_for("EUR_USD"), dec!(0.0001));
        assert_eq!(pip_value_for("EUR_GBP"), dec!(0.0001));
        assert_eq!(pip_value_for("JPY_USD"), dec!(0.0001)); // JPY base, not quote
    }

    #[test]
    fn unknown_instrument_falls_back_to_legacy_defaults() {
        assert_eq!(pip_value_for("XAU_USD"), dec!(0.0001));
        assert_eq!(default_half_spread_pips("XAU_USD"), dec!(1.0));
        assert_eq!(pip_value_for(""), dec!(0.0001));
        assert_eq!(default_half_spread_pips(""), dec!(1.0));
    }

    #[test]
    fn per_instrument_spread_table() {
        assert_eq!(default_half_spread_pips("EUR_USD"), dec!(0.7));
        assert_eq!(default_half_spread_pips("eur_usd"), dec!(0.7)); // case-insensitive
        assert_eq!(default_half_spread_pips("AUD_NZD"), dec!(1.8));
    }
}
