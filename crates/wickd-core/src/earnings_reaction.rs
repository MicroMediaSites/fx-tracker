//! How a stock reacted to an earnings report, read from its daily bars.
//!
//! The post-earnings-drift play trades the reaction, not the report: a
//! stock that gaps up on a report, on heavy volume, and closes near the top
//! of its day is the one worth following. This module turns "reported on
//! this date, before or after the market" plus daily bars into those few
//! figures. It is pure arithmetic over bars already fetched; nothing here
//! talks to a network.
//!
//! ## The reaction day
//!
//! - reported **before the open**: the report day itself;
//! - reported **after the close**: the next trading day;
//! - **no time given**: the report day (the feed's own date is the best
//!   guess; a report that actually came after the close will read a day
//!   early, which is why the session is shown beside the figures).
//!
//! "Trading day" is whatever day the bars have: a holiday simply has no bar,
//! so the next bar is used. A bar more than [`MAX_WAIT_DAYS`] after the
//! expected day is not treated as the reaction (the data has a hole).
//!
//! Figures are ratios of exchange prices (`f64`, like the bars), rounded
//! for display by the caller. They are observations, never order inputs.

use chrono::{DateTime, Duration, NaiveDate, Timelike, Utc};
use serde::Serialize;

use crate::earnings_store::Session;
use crate::equities::{eastern_offset_hours, Bar};

/// The furthest after the expected day a bar still counts as the reaction.
pub const MAX_WAIT_DAYS: i64 = 5;

/// Sessions averaged for the volume comparison.
pub const VOLUME_SESSIONS: usize = 20;

/// The fewest prior sessions a volume comparison is made from.
const MIN_VOLUME_SESSIONS: usize = 5;

/// A stock's first trading day after a report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reaction {
    /// The reaction day (US/Eastern market date).
    pub day: NaiveDate,
    /// False while that day's session is still open: the figures are "so
    /// far".
    pub complete: bool,
    /// The close of the session before the reaction day.
    pub prior_close: f64,
    pub open: f64,
    pub close: f64,
    /// Open against the prior close, in percent: the jump on the news.
    pub gap_pct: f64,
    /// Close against the prior close, in percent: the whole first day.
    pub change_pct: f64,
    /// Where the close sits in the day's range: 0 = on the low, 100 = on
    /// the high. `None` when the day had no range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_in_range: Option<f64>,
    /// The day's volume as a multiple of the average of the
    /// [`VOLUME_SESSIONS`] sessions before it. `None` with too little
    /// history. While the session is open this understates the final figure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel_volume: Option<f64>,
}

/// The US/Eastern date and hour of an instant.
fn eastern(t: DateTime<Utc>) -> chrono::NaiveDateTime {
    (t + Duration::hours(eastern_offset_hours(t))).naive_utc()
}

/// The market date of a daily bar (Alpaca stamps daily bars at the session
/// date's midnight US/Eastern).
pub fn bar_date(b: &Bar) -> NaiveDate {
    eastern(b.time).date()
}

/// The first day a report on `report` in `session` can be reacted to.
pub fn earliest_reaction_day(report: NaiveDate, session: Session) -> NaiveDate {
    match session {
        Session::AfterHours => report + Duration::days(1),
        Session::PreMarket | Session::Unknown => report,
    }
}

/// The reaction to a report on `report` in `session`, from `bars` (daily,
/// oldest first). `None` when the reaction day hasn't traded yet, the bars
/// don't reach it, or there is no session before it to compare with.
pub fn reaction(
    bars: &[Bar],
    report: NaiveDate,
    session: Session,
    now: DateTime<Utc>,
) -> Option<Reaction> {
    let earliest = earliest_reaction_day(report, session);
    let i = bars.iter().position(|b| bar_date(b) >= earliest)?;
    let bar = &bars[i];
    let day = bar_date(bar);
    if (day - earliest).num_days() > MAX_WAIT_DAYS {
        return None;
    }
    let prior = bars.get(i.checked_sub(1)?)?;
    if prior.close <= 0.0 {
        return None;
    }
    let pct = |x: f64| (x - prior.close) / prior.close * 100.0;
    let range = bar.high - bar.low;
    let before = &bars[i.saturating_sub(VOLUME_SESSIONS)..i];
    let avg = before.iter().map(|b| b.volume).sum::<f64>() / before.len().max(1) as f64;
    let now_et = eastern(now);
    Some(Reaction {
        day,
        complete: day < now_et.date() || (day == now_et.date() && now_et.hour() >= 16),
        prior_close: prior.close,
        open: bar.open,
        close: bar.close,
        gap_pct: pct(bar.open),
        change_pct: pct(bar.close),
        close_in_range: (range > 0.0).then(|| (bar.close - bar.low) / range * 100.0),
        rel_volume: (before.len() >= MIN_VOLUME_SESSIONS && avg > 0.0).then(|| bar.volume / avg),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// A daily bar stamped the way Alpaca stamps them: midnight Eastern
    /// (05:00 UTC in winter).
    fn bar(date: &str, open: f64, high: f64, low: f64, close: f64, volume: f64) -> Bar {
        Bar {
            time: format!("{date}T05:00:00Z").parse().unwrap(),
            open,
            high,
            low,
            close,
            volume,
            vwap: None,
        }
    }

    /// Ten quiet sessions at 100 on a million shares, ending Fri Nov 13.
    fn quiet() -> Vec<Bar> {
        [
            "2026-11-02",
            "2026-11-03",
            "2026-11-04",
            "2026-11-05",
            "2026-11-06",
            "2026-11-09",
            "2026-11-10",
            "2026-11-11",
            "2026-11-12",
            "2026-11-13",
        ]
        .iter()
        .map(|day| bar(day, 100.0, 101.0, 99.0, 100.0, 1_000_000.0))
        .collect()
    }

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn a_report_before_the_open_is_reacted_to_that_day() {
        let mut bars = quiet();
        // Mon Nov 16: gaps to 108, runs to 112, closes at 111 on 3M shares.
        bars.push(bar("2026-11-16", 108.0, 112.0, 107.0, 111.0, 3_000_000.0));
        bars.push(bar("2026-11-17", 111.0, 113.0, 110.0, 112.0, 1_500_000.0));
        let r = reaction(
            &bars,
            d("2026-11-16"),
            Session::PreMarket,
            at("2026-11-18T15:00:00Z"),
        )
        .unwrap();
        assert_eq!(r.day, d("2026-11-16"));
        assert!(r.complete);
        assert_eq!((r.prior_close, r.open, r.close), (100.0, 108.0, 111.0));
        assert!((r.gap_pct - 8.0).abs() < 1e-9);
        assert!((r.change_pct - 11.0).abs() < 1e-9);
        // (111 - 107) / (112 - 107) = 80% of the way up the day's range.
        assert!((r.close_in_range.unwrap() - 80.0).abs() < 1e-9);
        assert!((r.rel_volume.unwrap() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_report_after_the_close_is_reacted_to_the_next_trading_day() {
        let mut bars = quiet();
        // Reported Fri Nov 13 after the close; Monday is the reaction.
        bars.push(bar("2026-11-16", 94.0, 95.0, 90.0, 91.0, 2_000_000.0));
        let r = reaction(
            &bars,
            d("2026-11-13"),
            Session::AfterHours,
            at("2026-11-17T15:00:00Z"),
        )
        .unwrap();
        assert_eq!(r.day, d("2026-11-16"), "across the weekend");
        assert_eq!(r.prior_close, 100.0, "against Friday's close");
        assert!((r.gap_pct + 6.0).abs() < 1e-9);
        assert!((r.change_pct + 9.0).abs() < 1e-9);
        assert!((r.close_in_range.unwrap() - 20.0).abs() < 1e-9);
        // The same report read as "before the open" would use Friday itself.
        let fri = reaction(
            &bars,
            d("2026-11-13"),
            Session::PreMarket,
            at("2026-11-17T15:00:00Z"),
        )
        .unwrap();
        assert_eq!(fri.day, d("2026-11-13"));
        // No time given: the report day.
        let unknown = reaction(
            &bars,
            d("2026-11-13"),
            Session::Unknown,
            at("2026-11-17T15:00:00Z"),
        )
        .unwrap();
        assert_eq!(unknown.day, d("2026-11-13"));
    }

    #[test]
    fn a_day_that_has_not_traded_yet_has_no_reaction() {
        let bars = quiet(); // ends Fri Nov 13
        assert_eq!(
            reaction(
                &bars,
                d("2026-11-13"),
                Session::AfterHours,
                at("2026-11-14T15:00:00Z")
            ),
            None,
            "after Friday's close, before Monday"
        );
        assert_eq!(
            reaction(
                &bars,
                d("2026-11-18"),
                Session::PreMarket,
                at("2026-11-17T15:00:00Z")
            ),
            None,
            "a report still to come"
        );
        assert_eq!(
            reaction(
                &[],
                d("2026-11-13"),
                Session::PreMarket,
                at("2026-11-14T15:00:00Z")
            ),
            None
        );
    }

    #[test]
    fn todays_unfinished_session_is_marked_so_far() {
        let mut bars = quiet();
        bars.push(bar("2026-11-16", 105.0, 106.0, 104.0, 105.5, 400_000.0));
        // 15:00 UTC on the 16th = 10:00 Eastern: the session is open.
        let open = reaction(
            &bars,
            d("2026-11-16"),
            Session::PreMarket,
            at("2026-11-16T15:00:00Z"),
        )
        .unwrap();
        assert!(!open.complete);
        assert!((open.rel_volume.unwrap() - 0.4).abs() < 1e-9, "so far");
        // 21:00 UTC = 16:00 Eastern: closed.
        let closed = reaction(
            &bars,
            d("2026-11-16"),
            Session::PreMarket,
            at("2026-11-16T21:00:00Z"),
        )
        .unwrap();
        assert!(closed.complete);
    }

    #[test]
    fn missing_history_leaves_out_what_cannot_be_measured() {
        // The reaction day is the first bar there is: nothing to compare with.
        let only = vec![bar("2026-11-16", 108.0, 112.0, 107.0, 111.0, 3_000_000.0)];
        assert_eq!(
            reaction(
                &only,
                d("2026-11-16"),
                Session::PreMarket,
                at("2026-11-18T15:00:00Z")
            ),
            None
        );
        // Two prior sessions: the move is known, the volume comparison isn't.
        let mut few = quiet()[8..].to_vec();
        few.push(bar("2026-11-16", 108.0, 108.0, 108.0, 108.0, 3_000_000.0));
        let r = reaction(
            &few,
            d("2026-11-16"),
            Session::PreMarket,
            at("2026-11-18T15:00:00Z"),
        )
        .unwrap();
        assert!((r.gap_pct - 8.0).abs() < 1e-9);
        assert_eq!(r.rel_volume, None);
        // A day with no range has no "where in the range".
        assert_eq!(r.close_in_range, None);
        // A prior close of zero can't be a base.
        let mut bad = quiet();
        bad.last_mut().unwrap().close = 0.0;
        bad.push(bar("2026-11-16", 108.0, 112.0, 107.0, 111.0, 3_000_000.0));
        assert_eq!(
            reaction(
                &bad,
                d("2026-11-16"),
                Session::PreMarket,
                at("2026-11-18T15:00:00Z")
            ),
            None
        );
    }

    #[test]
    fn a_hole_in_the_bars_is_not_mistaken_for_the_reaction() {
        let mut bars = quiet();
        // Nothing for two weeks after the report, then a bar.
        bars.push(bar("2026-11-30", 120.0, 121.0, 119.0, 120.0, 1_000_000.0));
        assert_eq!(
            reaction(
                &bars,
                d("2026-11-16"),
                Session::PreMarket,
                at("2026-12-01T15:00:00Z")
            ),
            None
        );
        // A holiday gap of a few days is fine.
        let mut bars = quiet();
        bars.push(bar("2026-11-18", 103.0, 104.0, 102.0, 103.0, 1_000_000.0));
        let r = reaction(
            &bars,
            d("2026-11-16"),
            Session::PreMarket,
            at("2026-11-19T15:00:00Z"),
        )
        .unwrap();
        assert_eq!(r.day, d("2026-11-18"));
    }

    #[test]
    fn the_volume_average_uses_only_the_twenty_sessions_before() {
        // 25 sessions: five old ones on huge volume, then twenty at 1M.
        let mut bars: Vec<Bar> = (1..=5)
            .map(|i| {
                bar(
                    &format!("2026-10-{:02}", i + 4),
                    100.0,
                    101.0,
                    99.0,
                    100.0,
                    50_000_000.0,
                )
            })
            .collect();
        bars.extend((0..20).map(|i| {
            let day = d("2026-10-12") + Duration::days(i);
            bar(&day.to_string(), 100.0, 101.0, 99.0, 100.0, 1_000_000.0)
        }));
        bars.push(bar("2026-11-02", 110.0, 112.0, 109.0, 111.0, 2_000_000.0));
        let r = reaction(
            &bars,
            d("2026-11-02"),
            Session::PreMarket,
            at("2026-11-03T15:00:00Z"),
        )
        .unwrap();
        assert!(
            (r.rel_volume.unwrap() - 2.0).abs() < 1e-9,
            "the old sessions don't count"
        );
    }
}
