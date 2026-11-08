//! US equities market data — the stock-side counterpart to `oanda`.
//!
//! This is *display/research* data for the `wickd tui` ticker: quotes,
//! historical bars, a searchable symbol list and the market clock. Prices are
//! `f64` on purpose — nothing here feeds the order path or a P&L ledger (those
//! stay on `rust_decimal`), and charting wants floats anyway.
//!
//! **Do not reuse these types for strategy evaluation, backtests or
//! execution.** If equities ever feed a money path, convert to `Decimal` at
//! that boundary (or add separate domain types) — and any equities *trading*
//! is its own explicit operator decision, not an extension of this client.
//!
//! The only provider today is Alpaca's market-data API ([`alpaca`]). The types
//! below are provider-neutral so a second source can slot in behind the same
//! shapes later.

pub mod alpaca;

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// One OHLCV bar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    #[serde(rename = "t")]
    pub time: DateTime<Utc>,
    #[serde(rename = "o")]
    pub open: f64,
    #[serde(rename = "h")]
    pub high: f64,
    #[serde(rename = "l")]
    pub low: f64,
    #[serde(rename = "c")]
    pub close: f64,
    #[serde(rename = "v", default)]
    pub volume: f64,
    #[serde(rename = "vw", default)]
    pub vwap: Option<f64>,
}

/// Last trade print.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    #[serde(rename = "t")]
    pub time: DateTime<Utc>,
    #[serde(rename = "p")]
    pub price: f64,
    #[serde(rename = "s", default)]
    pub size: f64,
}

/// Top-of-book quote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quote {
    #[serde(rename = "t")]
    pub time: DateTime<Utc>,
    #[serde(rename = "bp", default)]
    pub bid: f64,
    #[serde(rename = "bs", default)]
    pub bid_size: f64,
    #[serde(rename = "ap", default)]
    pub ask: f64,
    #[serde(rename = "as", default)]
    pub ask_size: f64,
}

/// Point-in-time state of one symbol.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(default)]
    pub latest_trade: Option<Trade>,
    #[serde(default)]
    pub latest_quote: Option<Quote>,
    #[serde(default)]
    pub minute_bar: Option<Bar>,
    #[serde(default)]
    pub daily_bar: Option<Bar>,
    #[serde(default)]
    pub prev_daily_bar: Option<Bar>,
}

impl Snapshot {
    /// Best available "last" price: the latest trade, else today's close.
    pub fn last(&self) -> Option<f64> {
        self.latest_trade
            .as_ref()
            .map(|t| t.price)
            .or_else(|| self.daily_bar.as_ref().map(|b| b.close))
    }

    /// Reference close the change is measured against: the prior session.
    ///
    /// Before today's regular session opens, Alpaca's `dailyBar` is still the
    /// *previous* session's bar while the latest trade is a pre-market print.
    /// In that window the prior session's close is `dailyBar.close`, not
    /// `prevDailyBar.close` (which is two sessions back).
    pub fn prev_close(&self) -> Option<f64> {
        if let (Some(trade), Some(daily)) = (&self.latest_trade, &self.daily_bar) {
            if eastern_date(trade.time) > eastern_date(daily.time) {
                return Some(daily.close);
            }
        }
        self.prev_daily_bar.as_ref().map(|b| b.close)
    }

    /// Absolute change vs the prior session's close.
    pub fn change(&self) -> Option<f64> {
        Some(self.last()? - self.prev_close()?)
    }

    /// Percent change vs the prior session's close.
    pub fn change_pct(&self) -> Option<f64> {
        let prev = self.prev_close()?;
        if prev == 0.0 {
            return None;
        }
        Some(self.change()? / prev * 100.0)
    }
}

/// A tradeable symbol, for search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub symbol: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub exchange: String,
    #[serde(default)]
    pub tradable: bool,
}

/// Market open/closed state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketClock {
    pub timestamp: DateTime<Utc>,
    pub is_open: bool,
    pub next_open: DateTime<Utc>,
    pub next_close: DateTime<Utc>,
}

/// A history window for the symbol drill-down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HistoryRange {
    Day1,
    Day5,
    Month1,
    Month6,
    Year1,
    Year5,
    Max,
}

impl HistoryRange {
    pub const ALL: [HistoryRange; 7] = [
        HistoryRange::Day1,
        HistoryRange::Day5,
        HistoryRange::Month1,
        HistoryRange::Month6,
        HistoryRange::Year1,
        HistoryRange::Year5,
        HistoryRange::Max,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HistoryRange::Day1 => "1D",
            HistoryRange::Day5 => "5D",
            HistoryRange::Month1 => "1M",
            HistoryRange::Month6 => "6M",
            HistoryRange::Year1 => "1Y",
            HistoryRange::Year5 => "5Y",
            HistoryRange::Max => "MAX",
        }
    }

    /// Bar size requested for this range.
    pub fn timeframe(self) -> &'static str {
        match self {
            HistoryRange::Day1 => "5Min",
            HistoryRange::Day5 => "15Min",
            HistoryRange::Month1 => "1Hour",
            HistoryRange::Month6 | HistoryRange::Year1 => "1Day",
            HistoryRange::Year5 => "1Week",
            HistoryRange::Max => "1Month",
        }
    }

    /// True for ranges whose bars are intraday (time-of-day matters).
    pub fn intraday(self) -> bool {
        matches!(
            self,
            HistoryRange::Day1 | HistoryRange::Day5 | HistoryRange::Month1
        )
    }

    /// How far back to request. Intraday ranges over-fetch calendar days so a
    /// weekend or holiday still leaves enough sessions; [`Self::trim`] then
    /// keeps exactly the sessions the range promises.
    pub fn start(self, now: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            HistoryRange::Day1 => now - Duration::days(6),
            HistoryRange::Day5 => now - Duration::days(10),
            HistoryRange::Month1 => now - Duration::days(31),
            HistoryRange::Month6 => now - Duration::days(183),
            HistoryRange::Year1 => now - Duration::days(366),
            HistoryRange::Year5 => now - Duration::days(5 * 365 + 1),
            // Alpaca's history starts in 2016.
            HistoryRange::Max => Utc.with_ymd_and_hms(2016, 1, 1, 0, 0, 0).unwrap(),
        }
    }

    /// Keep only the most recent N trading sessions for the 1D/5D ranges
    /// (grouped by US/Eastern calendar date); other ranges pass through.
    pub fn trim(self, bars: Vec<Bar>) -> Vec<Bar> {
        let sessions = match self {
            HistoryRange::Day1 => 1,
            HistoryRange::Day5 => 5,
            _ => return bars,
        };
        let mut dates: Vec<NaiveDate> = bars.iter().map(|b| eastern_date(b.time)).collect();
        dates.dedup();
        if dates.len() <= sessions {
            return bars;
        }
        let cutoff = dates[dates.len() - sessions];
        bars.into_iter()
            .filter(|b| eastern_date(b.time) >= cutoff)
            .collect()
    }
}

/// UTC offset of US/Eastern at `t`, in hours (-4 during DST, else -5).
///
/// DST runs from 02:00 local on the second Sunday of March to 02:00 local on
/// the first Sunday of November (the US rule since 2007). Implemented by hand
/// to avoid pulling a full tz database in for one zone.
pub fn eastern_offset_hours(t: DateTime<Utc>) -> i64 {
    let year = t.year();
    let nth_sunday = |month: u32, n: u32| -> NaiveDate {
        let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
        let to_sunday = (7 - first.weekday().num_days_from_sunday()) % 7;
        first + Duration::days(i64::from(to_sunday) + 7 * i64::from(n - 1))
    };
    // 02:00 EST = 07:00 UTC; 02:00 EDT = 06:00 UTC.
    let dst_start = Utc.from_utc_datetime(&nth_sunday(3, 2).and_hms_opt(7, 0, 0).unwrap());
    let dst_end = Utc.from_utc_datetime(&nth_sunday(11, 1).and_hms_opt(6, 0, 0).unwrap());
    if t >= dst_start && t < dst_end {
        -4
    } else {
        -5
    }
}

/// The US/Eastern calendar date of `t` (the exchange's trading date).
pub fn eastern_date(t: DateTime<Utc>) -> NaiveDate {
    (t + Duration::hours(eastern_offset_hours(t))).date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(t: &str, c: f64) -> Bar {
        Bar {
            time: t.parse().unwrap(),
            open: c,
            high: c,
            low: c,
            close: c,
            volume: 1.0,
            vwap: None,
        }
    }

    #[test]
    fn eastern_offset_tracks_dst() {
        // 2026: DST starts Mar 8, ends Nov 1.
        assert_eq!(
            eastern_offset_hours("2026-03-08T06:59:00Z".parse().unwrap()),
            -5
        );
        assert_eq!(
            eastern_offset_hours("2026-03-08T07:00:00Z".parse().unwrap()),
            -4
        );
        assert_eq!(
            eastern_offset_hours("2026-07-01T12:00:00Z".parse().unwrap()),
            -4
        );
        assert_eq!(
            eastern_offset_hours("2026-11-01T05:59:00Z".parse().unwrap()),
            -4
        );
        assert_eq!(
            eastern_offset_hours("2026-11-01T06:00:00Z".parse().unwrap()),
            -5
        );
        assert_eq!(
            eastern_offset_hours("2026-12-15T15:00:00Z".parse().unwrap()),
            -5
        );
    }

    #[test]
    fn eastern_date_rolls_at_local_midnight() {
        // 2026-11-06 23:30 ET (EST) = 2026-11-07 04:30Z → still the 6th in ET.
        assert_eq!(
            eastern_date("2026-11-07T04:30:00Z".parse().unwrap()),
            NaiveDate::from_ymd_opt(2026, 11, 6).unwrap()
        );
        assert_eq!(
            eastern_date("2026-11-07T05:30:00Z".parse().unwrap()),
            NaiveDate::from_ymd_opt(2026, 11, 7).unwrap()
        );
    }

    #[test]
    fn trim_keeps_latest_sessions() {
        let bars = vec![
            bar("2026-11-04T15:00:00Z", 1.0),
            bar("2026-11-05T15:00:00Z", 2.0),
            bar("2026-11-05T20:00:00Z", 3.0),
            bar("2026-11-06T15:00:00Z", 4.0),
            bar("2026-11-06T20:55:00Z", 5.0),
        ];
        let d1 = HistoryRange::Day1.trim(bars.clone());
        assert_eq!(
            d1.iter().map(|b| b.close).collect::<Vec<_>>(),
            vec![4.0, 5.0]
        );
        // Fewer sessions than the range wants → everything passes.
        assert_eq!(HistoryRange::Day5.trim(bars.clone()).len(), 5);
        // Non-intraday ranges are untouched.
        assert_eq!(HistoryRange::Year1.trim(bars).len(), 5);
    }

    #[test]
    fn snapshot_change_math() {
        let s = Snapshot {
            latest_trade: Some(Trade {
                time: "2026-11-06T20:59:00Z".parse().unwrap(),
                price: 110.0,
                size: 10.0,
            }),
            prev_daily_bar: Some(bar("2026-11-05T05:00:00Z", 100.0)),
            ..Default::default()
        };
        assert_eq!(s.last(), Some(110.0));
        assert_eq!(s.change(), Some(10.0));
        assert!((s.change_pct().unwrap() - 10.0).abs() < 1e-9);
        // No trade → falls back to the daily bar close.
        let s2 = Snapshot {
            daily_bar: Some(bar("2026-11-06T05:00:00Z", 50.0)),
            ..Default::default()
        };
        assert_eq!(s2.last(), Some(50.0));
        assert_eq!(s2.change(), None);
    }

    #[test]
    fn premarket_change_is_vs_the_last_session_close() {
        // Mon 2026-11-09 08:00 ET pre-market print; dailyBar is still Friday's.
        let s = Snapshot {
            latest_trade: Some(Trade {
                time: "2026-11-09T13:00:00Z".parse().unwrap(),
                price: 105.0,
                size: 10.0,
            }),
            daily_bar: Some(bar("2026-11-06T05:00:00Z", 100.0)),
            prev_daily_bar: Some(bar("2026-11-05T05:00:00Z", 90.0)),
            ..Default::default()
        };
        assert_eq!(s.prev_close(), Some(100.0));
        assert_eq!(s.change(), Some(5.0));
    }
}
