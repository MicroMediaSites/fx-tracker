//! `wickd quote SYM... [--days N]` — daily stock prices as JSON, for agents
//! (wickd-lists AGT-2407): the last daily bar, the previous close and the
//! change, so a plan can say "buy above 268.60" instead of "above Friday's
//! close".
//!
//!   wickd quote NET ABNB
//!   wickd quote NVDA --days 20        # also the last 20 daily bars
//!
//! One batched Alpaca request per call (consolidated SIP daily bars — fine
//! on the free plan, which only delays the *latest* 15 minutes). Prices are
//! exchange prices (f64 in, rounded to cents out); nothing here trades.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, NaiveDate, Timelike, Utc};
use clap::Args;
use serde::Serialize;

use wickd_core::equities::alpaca::{AlpacaClient, AlpacaError, QuoteFeed};
use wickd_core::equities::{eastern_offset_hours, Bar};

use crate::alpaca_creds;
use crate::output::{exit, Out};

#[derive(Args, Debug)]
pub struct QuoteArgs {
    /// Stock symbols (commas work too). Forex pairs aren't covered yet.
    #[arg(required = true)]
    pub symbols: Vec<String>,
    /// Also return the last N daily bars per symbol (max 400).
    #[arg(long)]
    pub days: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayBar {
    /// Trading date (US/Eastern).
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// False for today's bar while the session is still open.
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Quote {
    /// The newest daily bar (today's, partial, during the session).
    pub last: DayBar,
    /// The close before `last` (the reference for chg / chg%).
    pub prev_close: Option<f64>,
    pub chg: Option<f64>,
    pub chg_pct: Option<f64>,
    /// The newest *completed* daily bar — the one to quote levels from
    /// ("Friday's close/high/low" on a weekend or before the open).
    pub last_complete: Option<DayBar>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bars: Option<Vec<DayBar>>,
}

fn cents(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn eastern(t: DateTime<Utc>) -> chrono::NaiveDateTime {
    (t + Duration::hours(eastern_offset_hours(t))).naive_utc()
}

/// Alpaca stamps daily bars at the session date's midnight (US/Eastern,
/// expressed in UTC); a bar is complete once its date is past, or its date
/// is today and it's after the 16:00 close.
fn day_bar(b: &Bar, now: DateTime<Utc>) -> DayBar {
    let date = eastern(b.time).date();
    let now_et = eastern(now);
    let complete = date < now_et.date() || (date == now_et.date() && now_et.hour() >= 16);
    DayBar {
        date,
        open: cents(b.open),
        high: cents(b.high),
        low: cents(b.low),
        close: cents(b.close),
        volume: b.volume,
        complete,
    }
}

/// Build a quote from a symbol's daily bars (oldest first).
pub fn quote_from(bars: &[Bar], days: Option<usize>, now: DateTime<Utc>) -> Option<Quote> {
    let day: Vec<DayBar> = bars.iter().map(|b| day_bar(b, now)).collect();
    let last = day.last()?.clone();
    let prev_close = day.len().checked_sub(2).map(|i| day[i].close);
    let chg = prev_close.map(|p| cents(last.close - p));
    let chg_pct = prev_close
        .filter(|p| *p != 0.0)
        .map(|p| ((last.close - p) / p * 10_000.0).round() / 100.0);
    let last_complete = day.iter().rev().find(|d| d.complete).cloned();
    let bars = days.map(|n| day[day.len().saturating_sub(n)..].to_vec());
    Some(Quote {
        last,
        prev_close,
        chg,
        chg_pct,
        last_complete,
        bars,
    })
}

fn normalized(symbols: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    symbols
        .iter()
        .flat_map(|s| s.split(','))
        .map(crate::lists::normalize_symbol)
        .filter(|s| !s.is_empty() && seen.insert(s.clone()))
        .collect()
}

pub async fn run(args: QuoteArgs, out: Out) -> ! {
    let symbols = normalized(&args.symbols);
    if let Some(bad) = symbols.iter().find(|s| crate::info::validate(s).is_err() || s.contains('_')) {
        out.fail(
            exit::VALIDATION,
            "invalid_symbol",
            format!("'{bad}': give stock tickers (forex pairs aren't covered by quote yet)"),
        );
    }
    let days = args.days.map(|d| d.clamp(1, 400));
    let creds = match alpaca_creds::resolve() {
        Ok(Some((c, _))) => c,
        Ok(None) => out.fail(
            exit::AUTH,
            "no_alpaca_keys",
            "no Alpaca keys configured — run `wickd alpaca login`",
        ),
        Err(e) => out.fail(exit::AUTH, "alpaca_keys_unreadable", format!("{e:#}")),
    };
    let client = AlpacaClient::new(creds, QuoteFeed::Iex);
    let now = Utc::now();
    // Enough calendar days for N trading days (+ weekends/holidays) and a
    // previous close.
    let want = days.unwrap_or(1).max(2);
    let start = now - Duration::days((want as i64) * 7 / 5 + 10);
    match client.bars_on(&symbols, "1Day", start, "sip").await {
        Ok(map) => {
            let mut quotes = BTreeMap::new();
            let mut missing = Vec::new();
            for s in &symbols {
                match map.get(s).and_then(|b| quote_from(b, days, now)) {
                    Some(q) => {
                        quotes.insert(s.clone(), q);
                    }
                    None => missing.push(s.clone()),
                }
            }
            out.ok(&serde_json::json!({
                "as_of": now,
                "quotes": quotes,
                "missing": missing,
            }));
            std::process::exit(exit::OK);
        }
        Err(AlpacaError::Auth(_)) => out.fail(
            exit::AUTH,
            "alpaca_auth",
            "Alpaca rejected the keys — run `wickd alpaca login`",
        ),
        Err(e) => out.fail(exit::GENERIC, "quote_failed", format!("{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(t: &str, o: f64, h: f64, l: f64, c: f64) -> Bar {
        Bar {
            time: t.parse().unwrap(),
            open: o,
            high: h,
            low: l,
            close: c,
            volume: 1000.0,
            vwap: None,
        }
    }

    #[test]
    fn quote_uses_the_last_bar_and_the_close_before_it() {
        // Thu Nov 5, Fri Nov 6 (daily bars stamped 05:00Z = midnight EST).
        let bars = [
            bar("2026-11-05T05:00:00Z", 240.0, 250.0, 238.0, 244.904),
            bar("2026-11-06T05:00:00Z", 255.0, 270.12, 252.3, 268.6),
        ];
        // Sunday: Friday's bar is complete and is the one to quote from.
        let sun: DateTime<Utc> = "2026-11-08T23:00:00Z".parse().unwrap();
        let q = quote_from(&bars, None, sun).unwrap();
        assert_eq!(q.last.date, NaiveDate::from_ymd_opt(2026, 11, 6).unwrap());
        assert_eq!((q.last.high, q.last.low, q.last.close), (270.12, 252.3, 268.6));
        assert_eq!(q.prev_close, Some(244.9));
        assert_eq!(q.chg, Some(23.7));
        assert_eq!(q.chg_pct, Some(9.68));
        assert!(q.last.complete);
        assert_eq!(q.last_complete.as_ref().unwrap().date, q.last.date);
        assert!(q.bars.is_none());
    }

    #[test]
    fn during_the_session_today_is_partial_and_last_complete_is_yesterday() {
        let bars = [
            bar("2026-11-06T05:00:00Z", 255.0, 270.0, 252.0, 268.6),
            bar("2026-11-09T05:00:00Z", 269.0, 275.0, 266.0, 272.0),
        ];
        // Mon Nov 9, 11:00 EST.
        let mon: DateTime<Utc> = "2026-11-09T16:00:00Z".parse().unwrap();
        let q = quote_from(&bars, Some(5), mon).unwrap();
        assert!(!q.last.complete);
        assert_eq!(
            q.last_complete.unwrap().date,
            NaiveDate::from_ymd_opt(2026, 11, 6).unwrap()
        );
        assert_eq!(q.bars.unwrap().len(), 2);
        // After 16:00 ET the day is complete.
        let close: DateTime<Utc> = "2026-11-09T21:30:00Z".parse().unwrap();
        assert!(quote_from(&bars, None, close).unwrap().last.complete);
        assert!(quote_from(&[], None, close).is_none());
    }
}
