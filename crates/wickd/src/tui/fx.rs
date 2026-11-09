//! Forex symbols in the TUI (wickd-forex, AGT-2387).
//!
//! Pairs use OANDA names (`EUR_USD`); stock symbols never contain `_`
//! (Alpaca writes share classes as `BRK.B`), so the underscore is the market
//! test. History comes straight from OANDA's candles endpoint (practice
//! account, read-only, wickd-forex D-4/D-9) using the repo-wide alignment
//! (`dailyAlignment=2`, UTC); live prices come from the stream daemon
//! (AGT-2389). Candles are converted to the chart's `Bar` (f64 is fine here:
//! display only, nothing is priced or traded from these values).
//!
//! Range buttons mean the same as for stocks (stock-chart convention). A
//! forex "day" is the trading day that rolls at 17:00 New York, so 1D shows
//! the current trading day and 5D the last five.

use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use wickd_core::equities::{eastern_offset_hours, Asset, Bar, HistoryRange};
use wickd_core::models::candle::Candle as OandaCandle;
use wickd_core::oanda::endpoints::{self, Granularity};
use wickd_core::oanda::OandaClient;

use super::chart;

/// Pairs a new watchlist starts with (wickd-forex D-6).
pub const DEFAULT_PAIRS: [&str; 5] = ["EUR_USD", "GBP_USD", "USD_JPY", "AUD_USD", "USD_CAD"];

/// `exchange` label on the search entries built from OANDA's instrument list.
pub const FX_EXCHANGE: &str = "OANDA";

/// True for an OANDA instrument name (`EUR_USD`), false for a stock symbol.
pub fn is_fx(symbol: &str) -> bool {
    symbol.contains('_')
}

/// One pip: 0.01 for JPY-quoted pairs, else 0.0001 (OANDA's pipLocation for
/// currencies).
pub fn pip_size(symbol: &str) -> Decimal {
    if symbol.ends_with("_JPY") {
        dec!(0.01)
    } else {
        dec!(0.0001)
    }
}

/// Price decimals OANDA quotes: 3 for JPY-quoted pairs, else 5.
pub fn decimals(symbol: &str) -> u32 {
    if symbol.ends_with("_JPY") {
        3
    } else {
        5
    }
}

/// A price difference in pips.
pub fn pips(symbol: &str, delta: Decimal) -> Decimal {
    delta / pip_size(symbol)
}

/// A pair's quote for the watchlist: the latest bid/ask plus the current
/// trading day's open/high/low (exact, from OANDA's price strings) and its
/// closes as f64 for the sparkline only (5-minute bars since 17:00 New York).
#[derive(Debug, Clone, PartialEq)]
pub struct FxQuote {
    pub bid: Decimal,
    pub ask: Decimal,
    pub time: DateTime<Utc>,
    pub open: Option<Decimal>,
    pub high: Option<Decimal>,
    pub low: Option<Decimal>,
    pub closes: Vec<f64>,
}

impl FxQuote {
    pub fn mid(&self) -> Decimal {
        (self.bid + self.ask) / dec!(2)
    }

    /// Change since the trading day's open (mid prices).
    pub fn change(&self) -> Option<Decimal> {
        self.open.map(|o| self.mid() - o)
    }

    pub fn change_pct(&self) -> Option<Decimal> {
        self.open
            .filter(|o| !o.is_zero())
            .map(|o| (self.mid() - o) / o * dec!(100))
    }
}

/// The trading day so far for one pair: (open, high, low, closes).
type Day = (Option<Decimal>, Option<Decimal>, Option<Decimal>, Vec<f64>);

async fn trading_day(client: &OandaClient, symbol: &str, from: &str) -> Day {
    // A missing day (weekend, fetch error) only blanks the day columns.
    let candles: Vec<OandaCandle> =
        endpoints::get_candles(client, symbol, Granularity::M5, Some(5000), Some(from), None)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|c| c.mid.close > Decimal::ZERO)
            .collect();
    (
        candles.first().map(|c| c.mid.open),
        candles.iter().map(|c| c.mid.high).max(),
        candles.iter().map(|c| c.mid.low).min(),
        candles.iter().map(|c| c.mid.close.to_f64().unwrap_or(0.0)).collect(),
    )
}

/// Quotes for `pairs`: one pricing call for bid/ask, plus each pair's
/// 5-minute bars since the trading day's open (fetched concurrently).
pub async fn quotes(
    client: &OandaClient,
    pairs: &[String],
    now: DateTime<Utc>,
) -> Result<HashMap<String, FxQuote>, String> {
    if pairs.is_empty() {
        return Ok(HashMap::new());
    }
    let prices = endpoints::get_pricing(client, pairs)
        .await
        .map_err(|e| format!("OANDA pricing: {e}"))?;
    let from = trading_day_start(now).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let days = futures_util::future::join_all(
        prices.iter().map(|p| trading_day(client, &p.instrument, &from)),
    )
    .await;
    Ok(prices
        .into_iter()
        .zip(days)
        .map(|(p, (open, high, low, closes))| {
            (
                p.instrument,
                FxQuote {
                    bid: p.bid,
                    ask: p.ask,
                    time: p.time,
                    open,
                    high,
                    low,
                    closes,
                },
            )
        })
        .collect())
}

/// OANDA's instrument list as search entries (`EUR_USD`, "EUR/USD").
pub async fn instruments(client: &OandaClient) -> Result<Vec<Asset>, String> {
    let list = endpoints::get_instruments(client)
        .await
        .map_err(|e| format!("OANDA instruments: {e}"))?;
    let mut assets: Vec<Asset> = list
        .into_iter()
        .map(|i| Asset {
            symbol: i.name,
            name: i.display_name,
            exchange: FX_EXCHANGE.to_string(),
            tradable: true,
        })
        .collect();
    assets.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    Ok(assets)
}

/// OANDA granularity for each range (same bar sizes as the stock ranges).
pub fn granularity(range: HistoryRange) -> Granularity {
    match range {
        HistoryRange::Day1 => Granularity::M5,
        HistoryRange::Day5 => Granularity::M15,
        HistoryRange::Month1 => Granularity::H1,
        HistoryRange::Month6 | HistoryRange::Year1 => Granularity::D,
        HistoryRange::Year5 => Granularity::W,
        HistoryRange::Max => Granularity::M,
    }
}

/// Start (UTC) of the forex trading day containing `t`: 17:00 New York.
pub fn trading_day_start(t: DateTime<Utc>) -> DateTime<Utc> {
    let off = Duration::hours(eastern_offset_hours(t));
    let local = t + off;
    let five_pm = NaiveTime::from_hms_opt(17, 0, 0).unwrap();
    let day = if local.time() >= five_pm {
        local.date_naive()
    } else {
        local.date_naive() - Duration::days(1)
    };
    Utc.from_utc_datetime(&day.and_time(five_pm)) - off
}

/// Keep the last `n` trading days of `bars` (1D = 1, 5D = 5).
fn last_trading_days(bars: Vec<Bar>, n: usize) -> Vec<Bar> {
    let mut days: Vec<DateTime<Utc>> = bars.iter().map(|b| trading_day_start(b.time)).collect();
    days.dedup();
    if days.len() <= n {
        return bars;
    }
    let cutoff = days[days.len() - n];
    bars.into_iter().filter(|b| b.time >= cutoff).collect()
}

/// Split fetched bars into (warm-up, visible) for `range`: 1D/5D keep the
/// last 1/5 trading days, other ranges everything from the range start;
/// anything older is warm-up (MAs, Ichimoku).
pub fn split(range: HistoryRange, now: DateTime<Utc>, all: Vec<Bar>) -> (Vec<Bar>, Vec<Bar>) {
    let vis: Vec<Bar> = match range {
        HistoryRange::Day1 => last_trading_days(all.clone(), 1),
        HistoryRange::Day5 => last_trading_days(all.clone(), 5),
        _ => {
            let start = range.start(now);
            all.iter().filter(|b| b.time >= start).cloned().collect()
        }
    };
    let first = vis.first().map(|b| b.time);
    let warm = all
        .into_iter()
        .filter(|b| first.is_some_and(|f| b.time < f))
        .collect();
    (warm, vis)
}

/// An OANDA mid candle as a chart bar (tick volume as volume).
pub fn to_bar(c: &OandaCandle) -> Bar {
    let f = |d: rust_decimal::Decimal| d.to_f64().unwrap_or(0.0);
    Bar {
        time: c.time,
        open: f(c.mid.open),
        high: f(c.mid.high),
        low: f(c.mid.low),
        close: f(c.mid.close),
        volume: f64::from(c.volume),
        vwap: None,
    }
}

/// History for a pair: one candles request covering the range plus its
/// warm-up, split into (warm-up, visible).
pub async fn history(
    client: &OandaClient,
    symbol: &str,
    range: HistoryRange,
    now: DateTime<Utc>,
) -> Result<(Vec<Bar>, Vec<Bar>), String> {
    let from = range.start(now) - Duration::days(chart::warmup_days(range));
    let from = from.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let candles = endpoints::get_candles(
        client,
        symbol,
        granularity(range),
        Some(5000),
        Some(&from),
        None,
    )
    .await
    .map_err(|e| format!("OANDA candles for {symbol}: {e}"))?;
    let bars: Vec<Bar> = candles
        .iter()
        .filter(|c| c.mid.close > rust_decimal::Decimal::ZERO)
        .map(to_bar)
        .collect();
    Ok(split(range, now, bars))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn bar(time: &str) -> Bar {
        Bar {
            time: t(time),
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
            vwap: None,
        }
    }

    #[test]
    fn pips_decimals_and_change_are_exact() {
        assert_eq!(decimals("EUR_USD"), 5);
        assert_eq!(decimals("USD_JPY"), 3);
        assert_eq!(pips("EUR_USD", dec!(0.00123)), dec!(12.3));
        assert_eq!(pips("USD_JPY", dec!(0.25)), dec!(25));
        let q = FxQuote {
            bid: dec!(1.1154),
            ask: dec!(1.1156),
            time: t("2026-11-09T03:00:00Z"),
            open: Some(dec!(1.1145)),
            high: None,
            low: None,
            closes: vec![],
        };
        assert_eq!(pips("EUR_USD", q.change().unwrap()), dec!(10));
    }

    #[test]
    fn market_is_decided_by_the_underscore() {
        assert!(is_fx("EUR_USD") && is_fx("USD_JPY"));
        assert!(!is_fx("SPY") && !is_fx("BRK.B"));
    }

    #[test]
    fn trading_day_rolls_at_5pm_new_york() {
        // November (EST, UTC-5): 17:00 NY = 22:00 UTC.
        assert_eq!(
            trading_day_start(t("2026-11-09T21:59:00Z")),
            t("2026-11-08T22:00:00Z")
        );
        assert_eq!(
            trading_day_start(t("2026-11-09T22:00:00Z")),
            t("2026-11-09T22:00:00Z")
        );
        // July (EDT, UTC-4): 17:00 NY = 21:00 UTC.
        assert_eq!(
            trading_day_start(t("2026-07-15T03:00:00Z")),
            t("2026-07-14T21:00:00Z")
        );
    }

    #[test]
    fn day_ranges_keep_whole_trading_days_and_the_rest_is_warmup() {
        let all = vec![
            bar("2026-11-05T12:00:00Z"),
            bar("2026-11-06T12:00:00Z"),
            bar("2026-11-08T22:05:00Z"), // Sunday open = Monday's trading day
            bar("2026-11-09T10:00:00Z"),
        ];
        let now = t("2026-11-09T12:00:00Z");
        let (warm, vis) = split(HistoryRange::Day1, now, all.clone());
        assert_eq!((warm.len(), vis.len()), (2, 2));
        let (warm, vis) = split(HistoryRange::Day5, now, all);
        assert_eq!((warm.len(), vis.len()), (0, 4));
    }

    /// Live: `cargo test -p wickd live_fx_history -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_fx_history_every_range() {
        let client = super::super::oanda_client().expect("OANDA practice credentials");
        let now = Utc::now();
        for r in HistoryRange::ALL {
            let (warm, vis) = history(&client, "EUR_USD", r, now).await.expect("candles");
            println!(
                "{:?}: {} visible ({} .. {}), {} warm-up, last close {}",
                r,
                vis.len(),
                vis.first().map(|b| b.time.to_string()).unwrap_or_default(),
                vis.last().map(|b| b.time.to_string()).unwrap_or_default(),
                warm.len(),
                vis.last().map(|b| b.close).unwrap_or_default()
            );
            assert!(!vis.is_empty(), "{r:?}");
        }
    }

    #[test]
    fn granularity_matches_the_stock_bar_sizes() {
        assert_eq!(granularity(HistoryRange::Day1).to_string(), "M5");
        assert_eq!(granularity(HistoryRange::Month1).to_string(), "H1");
        assert_eq!(granularity(HistoryRange::Year1).to_string(), "D");
        assert_eq!(granularity(HistoryRange::Year5).to_string(), "W");
    }
}
