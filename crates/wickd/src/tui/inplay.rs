//! "In play": which stocks are worth watching today.
//!
//! The universe is today's top movers (gainers + losers), the most active
//! names by volume, and the watchlist. Each row gets:
//! - **chg%**: last vs the previous session's close (works pre-market, where
//!   it *is* the gap);
//! - **from open**: last vs today's open, once the regular session has opened;
//! - **rvol**: today's volume so far ÷ the 20-day average daily volume, both
//!   from the *same* feed (IEX vs IEX on the free plan), so the ratio means
//!   something even though IEX sees only a few percent of all trading;
//! - **news**: how recent the latest headline is (a catalyst hint).
//!
//! Everything here is pure; the TUI worker fetches the inputs.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc, Weekday};

use wickd_core::equities::alpaca::{MostActive, Movers, NewsItem};
use wickd_core::equities::{eastern_date, eastern_offset_hours, Bar, MarketClock, Snapshot};

/// Universe cap (keeps the snapshot/bars requests to two pages).
pub const MAX_UNIVERSE: usize = 150;
/// Days averaged for relative volume.
pub const RVOL_DAYS: usize = 20;
/// Default minimum price (filters out sub-$5 names, which are mostly
/// warrants, penny stocks and other untradeable noise for a beginner).
pub const DEFAULT_MIN_PRICE: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Gainer,
    Loser,
    Active,
    Watch,
}

impl Tag {
    pub fn glyph(self) -> &'static str {
        match self {
            Tag::Gainer => "▲",
            Tag::Loser => "▼",
            Tag::Active => "●",
            Tag::Watch => "★",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    /// Biggest move vs previous close, either direction.
    Change,
    /// Highest relative volume.
    RelVolume,
    /// Highest raw volume today.
    Volume,
}

impl SortBy {
    pub fn label(self) -> &'static str {
        match self {
            SortBy::Change => "biggest move",
            SortBy::RelVolume => "relative volume",
            SortBy::Volume => "volume",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub symbol: String,
    pub last: Option<f64>,
    pub chg_pct: Option<f64>,
    pub from_open_pct: Option<f64>,
    pub volume: Option<f64>,
    pub avg_volume: Option<f64>,
    pub rvol: Option<f64>,
    pub tags: Vec<Tag>,
    pub latest_news: Option<DateTime<Utc>>,
    pub news_count: usize,
}

/// Merge movers, most-actives and the watchlist into one tagged, de-duplicated
/// list (movers first, then actives, then the watchlist), capped.
pub fn universe(
    movers: Option<&Movers>,
    actives: &[MostActive],
    watchlist: &[String],
) -> Vec<(String, Vec<Tag>)> {
    let mut out: Vec<(String, Vec<Tag>)> = Vec::new();
    let mut add = |sym: &str, tag: Tag| {
        if let Some((_, tags)) = out.iter_mut().find(|(s, _)| s == sym) {
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        } else {
            out.push((sym.to_string(), vec![tag]));
        }
    };
    if let Some(m) = movers {
        for g in &m.gainers {
            add(&g.symbol, Tag::Gainer);
        }
        for l in &m.losers {
            add(&l.symbol, Tag::Loser);
        }
    }
    for a in actives {
        add(&a.symbol, Tag::Active);
    }
    for w in watchlist {
        add(w, Tag::Watch);
    }
    // Watchlist names always survive the cap.
    if out.len() > MAX_UNIVERSE {
        let (mut keep, rest): (Vec<_>, Vec<_>) =
            out.into_iter().partition(|(_, t)| t.contains(&Tag::Watch));
        let room = MAX_UNIVERSE.saturating_sub(keep.len());
        keep.extend(rest.into_iter().take(room));
        out = keep;
    }
    out
}

/// Average daily volume over the last [`RVOL_DAYS`] sessions before `today`
/// (ET date). `None` with fewer than 5 sessions.
pub fn avg_daily_volume(bars: &[Bar], today: NaiveDate) -> Option<f64> {
    let past: Vec<f64> = bars
        .iter()
        .filter(|b| eastern_date(b.time) < today)
        .map(|b| b.volume)
        .collect();
    let recent = &past[past.len().saturating_sub(RVOL_DAYS)..];
    (recent.len() >= 5).then(|| recent.iter().sum::<f64>() / recent.len() as f64)
}

/// Today's (ET) daily bar from a snapshot, if the session has one yet.
fn todays_bar(s: &Snapshot, today: NaiveDate) -> Option<&Bar> {
    s.daily_bar
        .as_ref()
        .filter(|b| eastern_date(b.time) == today)
}

pub fn build_rows(
    universe: &[(String, Vec<Tag>)],
    snapshots: &HashMap<String, Snapshot>,
    daily: &HashMap<String, Vec<Bar>>,
    news: &[NewsItem],
    now: DateTime<Utc>,
) -> Vec<Row> {
    let today = eastern_date(now);
    universe
        .iter()
        .map(|(sym, tags)| {
            let snap = snapshots.get(sym);
            let bar = snap.and_then(|s| todays_bar(s, today));
            let last = snap.and_then(Snapshot::last);
            let from_open_pct = match (bar, last) {
                (Some(b), Some(l)) if b.open > 0.0 => Some((l - b.open) / b.open * 100.0),
                _ => None,
            };
            let volume = bar.map(|b| b.volume);
            let avg_volume = daily.get(sym).and_then(|b| avg_daily_volume(b, today));
            let rvol = match (volume, avg_volume) {
                (Some(v), Some(a)) if a > 0.0 => Some(v / a),
                _ => None,
            };
            let mine: Vec<&NewsItem> = news
                .iter()
                .filter(|n| n.symbols.iter().any(|s| s == sym))
                .collect();
            Row {
                symbol: sym.clone(),
                last,
                chg_pct: snap.and_then(Snapshot::change_pct),
                from_open_pct,
                volume,
                avg_volume,
                rvol,
                tags: tags.clone(),
                latest_news: mine.iter().map(|n| n.created_at).max(),
                news_count: mine.len(),
            }
        })
        .collect()
}

/// Sort in place, best first; rows missing the sort key go last.
pub fn sort_rows(rows: &mut [Row], by: SortBy) {
    let key = |r: &Row| -> Option<f64> {
        match by {
            SortBy::Change => r.chg_pct.map(f64::abs),
            SortBy::RelVolume => r.rvol,
            SortBy::Volume => r.volume,
        }
    };
    rows.sort_by(|a, b| match (key(a), key(b)) {
        (Some(x), Some(y)) => y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.symbol.cmp(&b.symbol),
    });
}

/// Rows passing the price filter; watchlist names always pass.
pub fn visible(rows: &[Row], min_price: Option<f64>) -> Vec<&Row> {
    rows.iter()
        .filter(|r| match min_price {
            None => true,
            Some(m) => r.tags.contains(&Tag::Watch) || r.last.is_some_and(|p| p >= m),
        })
        .collect()
}

/// Session label for the header: pre-market / regular session / after hours
/// / closed, from the market clock and the ET time.
pub fn session_phase(clock: Option<&MarketClock>, now: DateTime<Utc>) -> &'static str {
    let Some(c) = clock else {
        return "…";
    };
    if c.is_open {
        return "regular session";
    }
    let et = now + Duration::hours(eastern_offset_hours(now));
    let minutes = et.hour() * 60 + et.minute();
    let today = eastern_date(now);
    let opens_today = eastern_date(c.next_open) == today;
    if opens_today && (4 * 60..9 * 60 + 30).contains(&minutes) {
        "pre-market"
    } else if !opens_today
        && (16 * 60..20 * 60).contains(&minutes)
        && !matches!(today.weekday(), Weekday::Sat | Weekday::Sun)
    {
        // After today's close, before 8 pm ET, on a weekday (holidays aside).
        "after hours"
    } else {
        "closed"
    }
}

/// "12m", "3h", "2d" since `t`.
pub fn age(t: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let m = (now - t).num_minutes().max(0);
    if m < 60 {
        format!("{m}m")
    } else if m < 48 * 60 {
        format!("{}h", m / 60)
    } else {
        format!("{}d", m / (24 * 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wickd_core::equities::alpaca::Mover;
    use wickd_core::equities::Trade;

    fn mover(s: &str, p: f64) -> Mover {
        Mover {
            symbol: s.into(),
            price: 10.0,
            percent_change: p,
        }
    }

    fn bar(t: &str, o: f64, c: f64, v: f64) -> Bar {
        Bar {
            time: t.parse().unwrap(),
            open: o,
            high: o.max(c),
            low: o.min(c),
            close: c,
            volume: v,
            vwap: None,
        }
    }

    #[test]
    fn universe_merges_tags_and_keeps_the_watchlist_under_the_cap() {
        let movers = Movers {
            gainers: vec![mover("AAA", 20.0)],
            losers: vec![mover("BBB", -15.0)],
        };
        let actives = vec![
            MostActive {
                symbol: "AAA".into(),
                volume: 1e8,
            },
            MostActive {
                symbol: "CCC".into(),
                volume: 9e7,
            },
        ];
        let u = universe(Some(&movers), &actives, &["SPY".into(), "CCC".into()]);
        let syms: Vec<&str> = u.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(syms, vec!["AAA", "BBB", "CCC", "SPY"]);
        assert_eq!(u[0].1, vec![Tag::Gainer, Tag::Active]);
        assert_eq!(u[2].1, vec![Tag::Active, Tag::Watch]);
        // Cap: 200 actives + a watchlist name → watchlist survives.
        let many: Vec<MostActive> = (0..200)
            .map(|i| MostActive {
                symbol: format!("S{i}"),
                volume: 1.0,
            })
            .collect();
        let u = universe(None, &many, &["ZZZ".into()]);
        assert_eq!(u.len(), MAX_UNIVERSE);
        assert!(u.iter().any(|(s, _)| s == "ZZZ"));
        // No movers (endpoint unavailable) is fine.
        assert_eq!(universe(None, &[], &["SPY".into()]).len(), 1);
    }

    #[test]
    fn rows_compute_change_from_open_rvol_and_news() {
        let now: DateTime<Utc> = "2026-11-06T16:00:00Z".parse().unwrap(); // 11:00 ET
        let snap = Snapshot {
            latest_trade: Some(Trade {
                time: now,
                price: 110.0,
                size: 1.0,
            }),
            daily_bar: Some(bar("2026-11-06T05:00:00Z", 105.0, 110.0, 3_000_000.0)),
            prev_daily_bar: Some(bar("2026-11-05T05:00:00Z", 98.0, 100.0, 1_000_000.0)),
            ..Default::default()
        };
        let snaps = HashMap::from([("AAA".to_string(), snap)]);
        let history: Vec<Bar> = (1..=25)
            .map(|d| bar(&format!("2026-10-{d:02}T05:00:00Z"), 1.0, 1.0, 1_000_000.0))
            .chain([bar("2026-11-06T05:00:00Z", 1.0, 1.0, 9e9)]) // today: excluded
            .collect();
        let daily = HashMap::from([("AAA".to_string(), history)]);
        let news = vec![NewsItem {
            id: 1,
            headline: "AAA beats".into(),
            summary: String::new(),
            created_at: "2026-11-06T12:00:00Z".parse().unwrap(),
            symbols: vec!["AAA".into()],
            source: "benzinga".into(),
        }];
        let u = vec![
            ("AAA".to_string(), vec![Tag::Gainer]),
            ("NOPE".to_string(), vec![Tag::Watch]),
        ];
        let rows = build_rows(&u, &snaps, &daily, &news, now);
        let r = &rows[0];
        assert_eq!(r.chg_pct.map(|p| p.round()), Some(10.0));
        assert!((r.from_open_pct.unwrap() - 4.7619).abs() < 1e-3);
        assert_eq!(r.avg_volume, Some(1_000_000.0));
        assert_eq!(r.rvol, Some(3.0));
        assert_eq!(
            (r.news_count, age(r.latest_news.unwrap(), now)),
            (1, "4h".to_string())
        );
        // A symbol with no snapshot still gets a row, with blanks.
        assert_eq!((rows[1].last, rows[1].rvol), (None, None));
    }

    #[test]
    fn premarket_has_change_but_no_open_or_rvol() {
        // 08:00 ET Monday; dailyBar is still Friday's.
        let now: DateTime<Utc> = "2026-11-09T13:00:00Z".parse().unwrap();
        let snap = Snapshot {
            latest_trade: Some(Trade {
                time: now,
                price: 105.0,
                size: 1.0,
            }),
            daily_bar: Some(bar("2026-11-06T05:00:00Z", 98.0, 100.0, 1e6)),
            prev_daily_bar: Some(bar("2026-11-05T05:00:00Z", 90.0, 95.0, 1e6)),
            ..Default::default()
        };
        let rows = build_rows(
            &[("AAA".to_string(), vec![])],
            &HashMap::from([("AAA".to_string(), snap)]),
            &HashMap::new(),
            &[],
            now,
        );
        assert_eq!(rows[0].chg_pct.map(|p| p.round()), Some(5.0)); // the gap vs Friday's close
        assert_eq!((rows[0].from_open_pct, rows[0].volume), (None, None));
    }

    #[test]
    fn sorting_and_price_filter() {
        let row = |s: &str, chg: Option<f64>, rvol: Option<f64>, last: f64, tags: Vec<Tag>| Row {
            symbol: s.into(),
            last: Some(last),
            chg_pct: chg,
            from_open_pct: None,
            volume: rvol.map(|r| r * 1e6),
            avg_volume: None,
            rvol,
            tags,
            latest_news: None,
            news_count: 0,
        };
        let mut rows = vec![
            row("A", Some(2.0), Some(1.0), 50.0, vec![]),
            row("B", Some(-9.0), Some(4.0), 3.0, vec![]),
            row("C", None, None, 20.0, vec![]),
            row("D", Some(5.0), Some(2.0), 2.0, vec![Tag::Watch]),
        ];
        sort_rows(&mut rows, SortBy::Change);
        assert_eq!(
            rows.iter().map(|r| r.symbol.as_str()).collect::<String>(),
            "BDAC"
        );
        sort_rows(&mut rows, SortBy::RelVolume);
        assert_eq!(rows[0].symbol, "B");
        // $5 filter drops B; D survives because it's on the watchlist.
        let v: String = visible(&rows, Some(5.0))
            .iter()
            .map(|r| r.symbol.as_str())
            .collect();
        assert_eq!(v, "DAC");
        assert_eq!(visible(&rows, None).len(), 4);
    }

    #[test]
    fn session_phases() {
        let clock = |open: bool, next_open: &str| MarketClock {
            timestamp: Utc::now(),
            is_open: open,
            next_open: next_open.parse().unwrap(),
            next_close: "2026-11-09T21:00:00Z".parse().unwrap(),
        };
        // Monday 08:00 ET, opens 09:30 today.
        let pre: DateTime<Utc> = "2026-11-09T13:00:00Z".parse().unwrap();
        assert_eq!(
            session_phase(Some(&clock(false, "2026-11-09T14:30:00Z")), pre),
            "pre-market"
        );
        assert_eq!(
            session_phase(Some(&clock(true, "2026-11-10T14:30:00Z")), pre),
            "regular session"
        );
        // Monday 17:00 ET, next open tomorrow.
        let after: DateTime<Utc> = "2026-11-09T22:00:00Z".parse().unwrap();
        assert_eq!(
            session_phase(Some(&clock(false, "2026-11-10T14:30:00Z")), after),
            "after hours"
        );
        // Sunday noon ET.
        let sun: DateTime<Utc> = "2026-11-08T17:00:00Z".parse().unwrap();
        assert_eq!(
            session_phase(Some(&clock(false, "2026-11-09T14:30:00Z")), sun),
            "closed"
        );
        // Saturday 17:00 ET is not "after hours".
        let sat: DateTime<Utc> = "2026-11-07T22:00:00Z".parse().unwrap();
        assert_eq!(
            session_phase(Some(&clock(false, "2026-11-09T14:30:00Z")), sat),
            "closed"
        );
        assert_eq!(session_phase(None, sun), "…");
    }
}
