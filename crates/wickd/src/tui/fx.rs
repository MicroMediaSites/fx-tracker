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
pub fn pip_size(symbol: &str) -> f64 {
    if symbol.ends_with("_JPY") {
        0.01
    } else {
        0.0001
    }
}

/// Price decimals OANDA quotes: 3 for JPY-quoted pairs, else 5.
pub fn decimals(symbol: &str) -> usize {
    if symbol.ends_with("_JPY") {
        3
    } else {
        5
    }
}

/// A price difference in pips.
pub fn pips(symbol: &str, delta: f64) -> f64 {
    delta / pip_size(symbol)
}

/// A pair's quote for the watchlist: the latest bid/ask plus the current
/// trading day's open/high/low and closes (5-minute bars since 17:00 New
/// York), for the change columns and the sparkline.
#[derive(Debug, Clone, PartialEq)]
pub struct FxQuote {
    pub bid: f64,
    pub ask: f64,
    pub time: DateTime<Utc>,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub closes: Vec<f64>,
}

impl FxQuote {
    pub fn mid(&self) -> f64 {
        (self.bid + self.ask) / 2.0
    }

    /// Change since the trading day's open (mid prices).
    pub fn change(&self) -> Option<f64> {
        self.open.map(|o| self.mid() - o)
    }

    pub fn change_pct(&self) -> Option<f64> {
        self.open.filter(|o| *o != 0.0).map(|o| (self.mid() - o) / o * 100.0)
    }
}

/// Bid/ask for `pairs` from OANDA's pricing endpoint, as
/// `(instrument, bid, ask, time)`.
fn parse_prices(v: &serde_json::Value) -> Vec<(String, f64, f64, DateTime<Utc>)> {
    let num = |p: &serde_json::Value, side: &str| -> Option<f64> {
        p.get(side)?.get(0)?.get("price")?.as_str()?.parse().ok()
    };
    v.get("prices")
        .and_then(|p| p.as_array())
        .map(|prices| {
            prices
                .iter()
                .filter_map(|p| {
                    Some((
                        p.get("instrument")?.as_str()?.to_string(),
                        num(p, "bids")?,
                        num(p, "asks")?,
                        p.get("time")?.as_str()?.parse().ok()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Quotes for `pairs`: one pricing call for bid/ask, plus each pair's
/// 5-minute bars since the trading day's open.
pub async fn quotes(
    client: &OandaClient,
    pairs: &[String],
    now: DateTime<Utc>,
) -> Result<HashMap<String, FxQuote>, String> {
    if pairs.is_empty() {
        return Ok(HashMap::new());
    }
    let url = format!(
        "{}/v3/accounts/{}/pricing?instruments={}",
        client.base_url(),
        client.account_id(),
        pairs.join(",")
    );
    let body: serde_json::Value = client
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("OANDA pricing: {e}"))?
        .json()
        .await
        .map_err(|e| format!("OANDA pricing: {e}"))?;
    let from = trading_day_start(now).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut out = HashMap::new();
    for (sym, bid, ask, time) in parse_prices(&body) {
        // A missing day (weekend, fetch error) only blanks the day columns.
        let day: Vec<Bar> = endpoints::get_candles(client, &sym, Granularity::M5, Some(5000), Some(&from), None)
            .await
            .map(|c| c.iter().filter(|c| c.mid.close > rust_decimal::Decimal::ZERO).map(to_bar).collect())
            .unwrap_or_default();
        out.insert(
            sym,
            FxQuote {
                bid,
                ask,
                time,
                open: day.first().map(|b| b.open),
                high: day.iter().map(|b| b.high).reduce(f64::max),
                low: day.iter().map(|b| b.low).reduce(f64::min),
                closes: day.iter().map(|b| b.close).collect(),
            },
        );
    }
    Ok(out)
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
    fn pips_and_decimals_follow_the_quote_currency() {
        assert_eq!(decimals("EUR_USD"), 5);
        assert_eq!(decimals("USD_JPY"), 3);
        assert!((pips("EUR_USD", 0.00123) - 12.3).abs() < 1e-9);
        assert!((pips("USD_JPY", 0.25) - 25.0).abs() < 1e-9);
    }

    #[test]
    fn pricing_response_parses_bid_ask() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"prices":[{"instrument":"EUR_USD","time":"2026-11-09T03:00:00Z",
            "bids":[{"price":"1.11540"}],"asks":[{"price":"1.11548"}]},
            {"instrument":"BAD","time":"x","bids":[],"asks":[]}]}"#,
        )
        .unwrap();
        let p = parse_prices(&v);
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].0.as_str(), p[0].1, p[0].2), ("EUR_USD", 1.1154, 1.11548));
        let q = FxQuote {
            bid: 1.1154,
            ask: 1.1156,
            time: p[0].3,
            open: Some(1.1145),
            high: None,
            low: None,
            closes: vec![],
        };
        assert!((pips("EUR_USD", q.change().unwrap()) - 10.0).abs() < 1e-6);
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
