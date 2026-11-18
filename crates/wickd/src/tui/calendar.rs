//! The calendar screen's state (`g c`): the earnings watch.
//!
//! The play is post-earnings drift, and it trades the reaction, not the
//! report. So this screen is not a market calendar. By default it shows only
//! what was picked ahead of time:
//!
//! - the **slate** ([`crate::slate`]): companies chosen to watch through
//!   their reports, each with why and what is expected, written before the
//!   report;
//! - any **watchlist or list** symbol that reports;
//! - **high-impact US economic releases**, the few things that move
//!   everything.
//!
//! It looks back three weekdays so a report and the stock's reaction to it
//! ([`wickd_core::earnings_reaction`]) stay in view, and ahead six days.
//! `a` switches to browsing everything that reports, to add to the slate
//! from. Watch, then react: nothing here places a trade.
//!
//! Each day is one list of events in time order: a row says whether it is
//! an economic release or an earnings report, and nothing separates the two.
//!
//! Sources, all read from disk and never blocked on:
//!
//! - economic events from `~/.wickd/calendar/` (kept fresh by the
//!   `com.openthink.wickd-calendar` job; this screen only reads it), shown
//!   in the machine's local time;
//! - earnings from `~/.wickd/earnings/` ([`wickd_core::earnings_store`]),
//!   which this screen does refresh: opening it asks for a background fetch
//!   when a shown weekday has no file or the coming days' files are more
//!   than six hours old ([`CalendarState::needs_fetch`]), and `r` forces one;
//! - the slate from `~/.wickd/slate.json`, re-read when the file changes.
//!
//! [`build`] turns those into display rows and is a pure function of its
//! inputs (the clock and the time zone are passed in), so everything the
//! screen shows is unit-tested without a terminal or a network.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use chrono::{
    DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, Utc, Weekday,
};
use rust_decimal::Decimal;

use wickd_core::backtest::surprise::impact_rank;
use wickd_core::calendar_store::{self, CalendarEvent};
use wickd_core::earnings_headlines::{self, Headline, HeadlinesFile};
use wickd_core::earnings_reaction::{earliest_reaction_day, Reaction};
use wickd_core::earnings_results::{self, ResultOutcome, ResultRow, ResultsFile};
use wickd_core::earnings_store::{self, clean_text, DayFile, DayOutcome, Session};

use super::app::Mode;
use crate::slate::Entry;

/// Days ahead on the screen: today and the six after it.
pub const DAYS: i64 = 7;

/// Weekdays before today the watch keeps on screen, so a report and the
/// stock's reaction to it stay in view.
pub const LOOKBACK_WEEKDAYS: usize = 3;

/// Earnings files older than this are fetched again when the screen opens.
const STALE_HOURS: i64 = 6;

/// How long before a company's reported figures are asked for again when
/// the feed didn't have them: it can lag a report by a day, so there is no
/// point asking every minute.
const RESULTS_RETRY: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// How long after an automatic fetch before another may start, so a feed
/// that is down isn't asked again on every visit.
const AUTO_RETRY: std::time::Duration = std::time::Duration::from_secs(600);

/// The only currency this screen shows: it is a US stock calendar.
const CURRENCY: &str = "USD";

/// One economic release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconRow {
    /// The local day it falls on.
    pub date: NaiveDate,
    /// Local time, `6:30 AM`.
    pub time: String,
    /// Already happened.
    pub past: bool,
    /// `high` | `medium` | `low` | `holiday`, lower-case.
    pub impact: String,
    pub event: String,
    pub forecast: String,
    pub previous: String,
}

/// One company's report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EarnRow {
    pub symbol: String,
    /// The company's name; empty for a slate entry the earnings store
    /// doesn't have.
    pub name: String,
    /// The report's market date.
    pub date: NaiveDate,
    pub session: Session,
    pub eps_forecast: Option<Decimal>,
    pub last_year_eps: Option<Decimal>,
    pub market_cap: Option<Decimal>,
    /// On the watchlist or a `wickd list`.
    pub listed: bool,
    /// On the slate for this report.
    pub slate: bool,
    /// The slate's one-line reason.
    pub why: Option<String>,
    /// The slate's expectation, written before the report.
    pub expect: Option<String>,
    /// The slate's note on the result, written after the report.
    pub result: Option<String>,
    /// The report is already out ([`session_passed`]).
    pub past: bool,
}

/// One line of the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A day heading: `Today · Wed Nov 18`.
    Day(String),
    Econ(EconRow),
    Earn(EarnRow),
    /// A plain sentence: nothing stored, nothing reports, how many are hidden.
    Note(String),
    Blank,
}

impl Row {
    /// Rows the cursor can sit on.
    pub fn selectable(&self) -> bool {
        matches!(self, Row::Econ(_) | Row::Earn(_))
    }

    /// A release or report that has already happened.
    pub fn past(&self) -> bool {
        match self {
            Row::Econ(e) => e.past,
            Row::Earn(e) => e.past,
            _ => false,
        }
    }
}

/// Whether a report due on market day `date` in `session` is out by `now`.
/// Before-the-open reports are out once the market opens (9:30 Eastern);
/// after-the-close ones by the end of the evening session (8:00 PM
/// Eastern). A report with no time given only counts as out once its day
/// is over.
pub fn session_passed(date: NaiveDate, session: Session, now: DateTime<Utc>) -> bool {
    let (hour, minute) = match session {
        Session::PreMarket => (9, 30),
        Session::AfterHours => (20, 0),
        Session::Unknown => (23, 59),
    };
    eastern_instant(date, hour, minute).is_some_and(|t| now >= t)
}

/// `hour:minute` US/Eastern on `date`, as an instant.
fn eastern_instant(date: NaiveDate, hour: u32, minute: u32) -> Option<DateTime<Utc>> {
    // Eastern wall time → UTC: take the offset in force around that moment.
    let guess = date.and_hms_opt(hour, minute, 0)?.and_utc();
    Some(guess - Duration::hours(wickd_core::equities::eastern_offset_hours(guess)))
}

/// What an earnings row shows in the TIME column: `pre-open`, `post-close`,
/// or nothing when the feed gave no time.
pub fn session_text(session: Session) -> &'static str {
    match session {
        Session::PreMarket => "pre-open",
        Session::AfterHours => "post-close",
        Session::Unknown => "",
    }
}

/// The wall clock and time zone, replaceable in tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct Clock {
    /// `None` = the real time.
    pub now: Option<DateTime<Utc>>,
    /// `None` = the machine's zone.
    pub offset: Option<FixedOffset>,
}

impl Clock {
    pub fn now(&self) -> DateTime<Utc> {
        self.now.unwrap_or_else(Utc::now)
    }

    pub fn local(&self, t: DateTime<Utc>) -> NaiveDateTime {
        match self.offset {
            Some(o) => t.with_timezone(&o).naive_local(),
            None => t.with_timezone(&Local).naive_local(),
        }
    }

    pub fn today(&self) -> NaiveDate {
        self.local(self.now()).date()
    }

    /// A fixed moment in Mountain Standard Time (UTC−7), for tests.
    #[cfg(test)]
    pub fn mst(now: &str) -> Self {
        Self {
            now: Some(now.parse().expect("an RFC 3339 time")),
            offset: FixedOffset::west_opt(7 * 3600),
        }
    }
}

/// `Yesterday · Tue Nov 17`, `Today · Wed Nov 18`, `Tomorrow · Thu Nov 19`,
/// and plain `Fri Nov 20` for any other day.
pub fn day_label(day: NaiveDate, today: NaiveDate) -> String {
    let date = day.format("%a %b %-d");
    match (day - today).num_days() {
        -1 => format!("Yesterday · {date}"),
        0 => format!("Today · {date}"),
        1 => format!("Tomorrow · {date}"),
        _ => date.to_string(),
    }
}

/// `6:30 AM` (no leading zero).
fn clock_text(t: NaiveDateTime) -> String {
    t.format("%-I:%M %p").to_string()
}

/// A market value in words a person reads: `$3.76T`, `$72.8B`, `$475M`.
pub fn fmt_cap(cap: Decimal) -> String {
    use rust_decimal::prelude::ToPrimitive;
    let v = cap.to_f64().unwrap_or(0.0);
    let (n, unit) = if v >= 1e12 {
        (v / 1e12, "T")
    } else if v >= 1e9 {
        (v / 1e9, "B")
    } else if v >= 1e6 {
        (v / 1e6, "M")
    } else {
        return format!("${v:.0}");
    };
    if n >= 100.0 {
        format!("${n:.0}{unit}")
    } else if n >= 10.0 {
        format!("${n:.1}{unit}")
    } else {
        format!("${n:.2}{unit}")
    }
}

fn is_weekend(d: NaiveDate) -> bool {
    matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

/// A stock's next earnings report, when it is today or on the next weekday:
/// what the watchlist, the lists and the chart header warn about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Soon {
    pub date: NaiveDate,
    pub session: Session,
}

impl Soon {
    fn day(&self, today: NaiveDate) -> String {
        if self.date == today {
            "today".into()
        } else {
            self.date.format("%a").to_string()
        }
    }

    /// For a table cell: `today pre-open`, `Thu post-close`, `today`.
    pub fn short(&self, today: NaiveDate) -> String {
        let day = self.day(today);
        match self.session {
            Session::PreMarket => format!("{day} pre-open"),
            Session::AfterHours => format!("{day} post-close"),
            Session::Unknown => day,
        }
    }

    /// For a sentence: `reports today before the open`.
    pub fn long(&self, today: NaiveDate) -> String {
        let day = self.day(today);
        match self.session {
            Session::PreMarket => format!("reports {day} before the open"),
            Session::AfterHours => format!("reports {day} after the close"),
            Session::Unknown => format!("reports {day}"),
        }
    }
}

/// The weekday after `d` (Friday → Monday). Market holidays aren't known
/// here, so the day before one points at the holiday, where nobody reports.
pub fn next_weekday(d: NaiveDate) -> NaiveDate {
    let mut n = d + Duration::days(1);
    while is_weekend(n) {
        n += Duration::days(1);
    }
    n
}

/// Every symbol that reports today or on the next weekday, from the store
/// only. A symbol on both days keeps today's. A day that is missing or
/// unreadable contributes nothing: no marker is better than a wrong one.
pub fn reports_soon(dir: &Path, today: NaiveDate) -> std::collections::HashMap<String, Soon> {
    let mut out = std::collections::HashMap::new();
    for date in [today, next_weekday(today)] {
        // The file's own date must match: a stale or misnamed file is ignored.
        let Ok(Some(day)) = earnings_store::read_day(dir, date) else {
            continue;
        };
        if day.date != date {
            continue;
        }
        for r in day.rows {
            out.entry(r.symbol).or_insert(Soon {
                date,
                session: r.session,
            });
        }
    }
    out
}

/// What [`build`] reads.
pub struct Inputs<'a> {
    pub clock: Clock,
    /// Economic rows covering at least the window (any currency).
    pub econ: &'a [CalendarEvent],
    /// Stored earnings days inside the window.
    pub earnings: &'a [DayFile],
    /// The slate: companies picked to watch through their reports.
    pub slate: &'a [Entry],
    /// Symbols on the watchlist or a list.
    pub listed: &'a HashSet<String>,
    /// Browse everything that reports and every US release, instead of the
    /// watch.
    pub show_all: bool,
}

/// The first day on screen: [`LOOKBACK_WEEKDAYS`] weekdays before `today`,
/// so a report and the stock's reaction to it stay in view.
pub fn watch_start(today: NaiveDate) -> NaiveDate {
    let mut d = today;
    let mut back = 0;
    while back < LOOKBACK_WEEKDAYS {
        d -= Duration::days(1);
        if !is_weekend(d) {
            back += 1;
        }
    }
    d
}

/// The screen's rows: a heading per day, then everything on that day as one
/// list in time order, economic releases and earnings together. Each row
/// says which it is; there are no captions between them.
///
/// **The watch (default).** Only what was picked: companies on the slate,
/// any watchlist or list symbol that reports, and high-impact US releases.
/// Nothing is shown because of a company's size. Days with nothing on them
/// are left out (today always shows), and the days before today show only
/// the watched companies, so a report and its reaction stay in view.
///
/// **Everything (`show_all`).** From today on, every company that reports
/// and every US release, with notes saying what isn't stored.
///
/// Order within a day: reports due before the open, then releases by time
/// up to the 4:00 PM Eastern close, then reports due after the close, then
/// any later releases, then reports with no time given. Watched companies
/// come first within a session, then the rest largest first.
pub fn build(inp: &Inputs) -> Vec<Row> {
    let now = inp.clock.now();
    let today = inp.clock.today();
    let econ_all: Vec<(DateTime<Utc>, NaiveDateTime, &CalendarEvent)> = inp
        .econ
        .iter()
        .filter_map(|e| {
            let at = DateTime::<Utc>::from_timestamp(e.time_unix()?, 0)?;
            Some((at, inp.clock.local(at), e))
        })
        .collect();
    // How far the economic store reaches (any currency): a day past it is
    // "not published yet", not "nothing scheduled".
    let econ_through = econ_all.iter().map(|(_, t, _)| t.date()).max();

    let mut rows = Vec::new();
    if !inp.show_all && inp.slate.is_empty() {
        rows.push(Row::Note(
            "Your slate is empty. Press a to browse who reports and s to add a company, or use `wickd earnings slate add`."
                .into(),
        ));
        rows.push(Row::Blank);
    }
    let mut unstored = 0usize;
    let mut day = watch_start(today);
    let last = today + Duration::days(DAYS - 1);
    while day <= last {
        let this = day;
        day += Duration::days(1);
        let ahead = this >= today;

        let econ: Vec<&(DateTime<Utc>, NaiveDateTime, &CalendarEvent)> = econ_all
            .iter()
            .filter(|(_, t, e)| {
                ahead
                    && t.date() == this
                    && e.currency.eq_ignore_ascii_case(CURRENCY)
                    && (inp.show_all
                        || impact_rank(&e.impact) >= 3
                        || e.impact.eq_ignore_ascii_case("holiday"))
            })
            .collect();
        let file = inp.earnings.iter().find(|d| d.date == this);
        if ahead && file.is_none() && !is_weekend(this) {
            unstored += 1;
        }

        // (slot in the day, instant, watched first, size (largest first),
        // name) → row.
        type Key = (u8, i64, bool, std::cmp::Reverse<Option<Decimal>>, String);
        let mut items: Vec<(Key, Row)> = Vec::new();
        let close = eastern_instant(this, 16, 0);
        for (at, local, e) in &econ {
            let slot = if close.is_some_and(|c| *at >= c) {
                3
            } else {
                1
            };
            items.push((
                (
                    slot,
                    at.timestamp(),
                    false,
                    std::cmp::Reverse(None),
                    e.event.clone(),
                ),
                Row::Econ(EconRow {
                    date: this,
                    time: clock_text(*local),
                    past: *at < now,
                    impact: e.impact.to_lowercase(),
                    event: clean_text(&e.event, 80),
                    forecast: clean_text(&e.forecast, 12),
                    previous: clean_text(&e.previous, 12),
                }),
            ));
        }

        let on_slate = |sym: &str| inp.slate.iter().find(|s| s.symbol == sym && s.date == this);
        let mut earn = |symbol: &str,
                        name: &str,
                        session: Session,
                        eps: Option<Decimal>,
                        last_year: Option<Decimal>,
                        cap: Option<Decimal>| {
            let pick = on_slate(symbol);
            let slot = match session {
                Session::PreMarket => 0,
                Session::AfterHours => 2,
                Session::Unknown => 4,
            };
            let listed = inp.listed.contains(symbol);
            items.push((
                (
                    slot,
                    0,
                    !(pick.is_some() || listed),
                    std::cmp::Reverse(cap),
                    symbol.to_string(),
                ),
                Row::Earn(EarnRow {
                    symbol: symbol.to_string(),
                    name: name.to_string(),
                    date: this,
                    session,
                    eps_forecast: eps,
                    last_year_eps: last_year,
                    market_cap: cap,
                    listed,
                    slate: pick.is_some(),
                    why: pick.and_then(|p| p.why.clone()),
                    expect: pick.and_then(|p| p.expect.clone()),
                    result: pick.and_then(|p| p.result.clone()),
                    past: session_passed(this, session, now),
                }),
            ));
        };
        let stored = file.map_or(&[][..], |f| &f.rows[..]);
        let mut shown = 0usize;
        for r in stored {
            let watched = on_slate(&r.symbol).is_some() || inp.listed.contains(&r.symbol);
            if watched || (inp.show_all && ahead) {
                // The slate's own session wins: it may have been corrected.
                let session = on_slate(&r.symbol).map_or(r.session, |p| p.session);
                earn(
                    &r.symbol,
                    &r.name,
                    session,
                    r.eps_forecast,
                    r.last_year_eps,
                    r.market_cap,
                );
                shown += 1;
            }
        }
        // A slate entry the store doesn't have for this day (dated by hand,
        // or the day isn't stored) still shows.
        for p in inp.slate.iter().filter(|p| p.date == this) {
            if !stored.iter().any(|r| r.symbol == p.symbol) {
                earn(&p.symbol, "", p.session, None, None, None);
            }
        }

        // The watch leaves empty days out; today is always there.
        if items.is_empty() && !(this == today || (inp.show_all && ahead && !is_weekend(this))) {
            continue;
        }
        if inp.show_all && is_weekend(this) && items.is_empty() {
            continue;
        }
        rows.push(Row::Day(day_label(this, today)));
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let none = items.is_empty();
        rows.extend(items.into_iter().map(|(_, row)| row));

        if inp.show_all && ahead {
            // Browsing: say what isn't there.
            if econ.is_empty() {
                rows.push(Row::Note(match econ_through {
                    Some(through) if this <= through => "No US economic releases.".into(),
                    _ => "Economic events for this day aren't published yet (the feed covers the current week).".into(),
                }));
            }
            match file {
                None => rows.push(Row::Note(
                    "No earnings stored for this day yet. Press r to fetch.".into(),
                )),
                Some(f) if f.rows.is_empty() => rows.push(Row::Note("No company reports.".into())),
                Some(_) => {}
            }
        } else if none {
            rows.push(Row::Note("Nothing you're watching.".into()));
        } else if ahead && stored.len() > shown {
            let n = stored.len() - shown;
            rows.push(Row::Note(format!(
                "{n} other {} · a shows everything",
                if n == 1 {
                    "company reports"
                } else {
                    "companies report"
                }
            )));
        }
        rows.push(Row::Blank);
    }
    while rows.last() == Some(&Row::Blank) {
        rows.pop();
    }
    if !inp.show_all && unstored > 0 {
        rows.push(Row::Blank);
        rows.push(Row::Note(format!(
            "Earnings aren't stored for {unstored} of the coming weekdays, so a watched company could be missing. Press r to fetch."
        )));
    }
    rows
}

#[derive(Debug)]
pub struct CalendarState {
    /// `~/.wickd/calendar`; `None` when the data home couldn't be resolved.
    pub econ_dir: Option<PathBuf>,
    /// `~/.wickd/earnings`.
    pub earn_dir: Option<PathBuf>,
    /// `~/.wickd/slate.json`.
    pub slate_path: Option<PathBuf>,
    pub clock: Clock,
    pub rows: Vec<Row>,
    /// The row under the cursor (index into `rows`; always selectable when
    /// any row is).
    pub selected: usize,
    /// First row shown, as last settled by the renderer.
    pub scroll: Cell<usize>,
    /// Body rows on screen, as last measured by the renderer.
    pub page: Cell<usize>,
    /// Browse everything, instead of the watch.
    pub show_all: bool,
    /// An earnings fetch is in flight.
    pub loading: bool,
    /// Why a store couldn't be read.
    pub error: Option<String>,
    /// Why the last earnings fetch fell short (stored data is still shown).
    pub fetch_error: Option<String>,
    /// The oldest fetch time among the coming days' earnings files.
    pub earnings_as_of: Option<DateTime<Utc>>,
    /// When the economic store was last written.
    pub econ_as_of: Option<SystemTime>,
    /// A shown weekday has no earnings file.
    pub missing_days: bool,
    /// The slate as last read.
    pub slate: Vec<Entry>,
    /// When the slate file was last seen written.
    slate_seen: Option<SystemTime>,
    /// How each watched stock reacted, by symbol and report date.
    pub reactions: HashMap<(String, NaiveDate), Reaction>,
    /// A reaction fetch is in flight.
    pub reactions_loading: bool,
    /// Why the last reaction fetch failed (rows simply show no figures).
    pub reactions_error: Option<String>,
    /// The results headlines stored for each watched report that is out, by
    /// symbol and report date.
    pub headlines: HashMap<(String, NaiveDate), HeadlinesFile>,
    /// A headlines fetch is in flight.
    pub headlines_loading: bool,
    /// Why the last headlines fetch fell short.
    pub headlines_error: Option<String>,
    /// When each report's headlines were last asked for.
    headlines_tried: HashMap<(String, NaiveDate), Instant>,
    /// Set by `r` until everything it started has come back: what was
    /// already known then, so the screen can say what came in.
    pub updating: Option<HashSet<String>>,
    /// What each watched company that has reported actually reported, as
    /// stored (by symbol).
    pub reported: HashMap<String, ResultsFile>,
    /// A fetch of reported figures is in flight.
    pub results_loading: bool,
    /// Why the last fetch of reported figures fell short.
    pub results_error: Option<String>,
    /// When each company's figures were last asked for.
    results_tried: HashMap<String, Instant>,
    /// A slate entry with text on it that `s` was pressed on once: a second
    /// `s` removes it.
    pub pending_remove: Option<String>,
    /// Where `esc` returns to.
    pub back_to: Mode,
    /// When the last automatic fetch was started.
    last_auto: Option<Instant>,
}

/// One reaction to ask for: the symbol, its report date and session.
pub type ReactionAsk = (String, NaiveDate, Session);

impl CalendarState {
    pub fn new(
        econ_dir: Option<PathBuf>,
        earn_dir: Option<PathBuf>,
        slate_path: Option<PathBuf>,
    ) -> Self {
        Self {
            econ_dir,
            earn_dir,
            slate_path,
            clock: Clock::default(),
            rows: Vec::new(),
            selected: 0,
            scroll: Cell::new(0),
            page: Cell::new(20),
            show_all: false,
            loading: false,
            error: None,
            fetch_error: None,
            earnings_as_of: None,
            econ_as_of: None,
            missing_days: false,
            slate: Vec::new(),
            slate_seen: None,
            reactions: HashMap::new(),
            reactions_loading: false,
            reactions_error: None,
            headlines: HashMap::new(),
            headlines_loading: false,
            headlines_error: None,
            headlines_tried: HashMap::new(),
            updating: None,
            reported: HashMap::new(),
            results_loading: false,
            results_error: None,
            results_tried: HashMap::new(),
            pending_remove: None,
            back_to: Mode::Watchlist,
            last_auto: None,
        }
    }

    /// The real stores under the wickd data home.
    pub fn at_data_home() -> Self {
        Self::new(
            wickd_core::events::calendar_dir().ok(),
            earnings_store::earnings_dir().ok(),
            crate::slate::slate_path().ok(),
        )
    }

    /// The weekdays on screen, which are the days an earnings fetch asks
    /// for (the look-back days too: they carry the names of what reported).
    pub fn fetch_dates(&self) -> Vec<NaiveDate> {
        let today = self.clock.today();
        let start = watch_start(today);
        let span = (today - start).num_days() + DAYS - 1;
        earnings_store::weekdays_from(start, span as u32)
    }

    /// Whether the slate file changed since it was last read (checked by
    /// the event loop while the screen is open).
    pub fn slate_changed(&self) -> bool {
        self.slate_path.as_deref().and_then(file_mtime) != self.slate_seen
    }

    /// Read the stores and the slate, and rebuild the rows, keeping the
    /// cursor on the same row when it is still there.
    pub fn reload(&mut self, listed: &HashSet<String>) {
        let keep = self
            .rows
            .get(self.selected)
            .filter(|r| r.selectable())
            .map(row_key);
        let today = self.clock.today();
        let start = watch_start(today);
        let last = today + Duration::days(DAYS - 1);
        self.error = None;

        // Economic rows are stored in UTC: read a day either side so a
        // local day's first and last hours are covered.
        let econ = match &self.econ_dir {
            Some(dir) => {
                calendar_store::read_range(dir, today - Duration::days(1), last + Duration::days(1))
                    .unwrap_or_else(|e| {
                        self.error = Some(e);
                        Vec::new()
                    })
            }
            None => Vec::new(),
        };
        self.econ_as_of = self
            .econ_dir
            .as_deref()
            .and_then(|d| file_mtime(&d.join(format!("{}.csv", today.format("%Y-%m")))));

        let earnings = match &self.earn_dir {
            Some(dir) => earnings_store::read_days(dir, start, last).unwrap_or_else(|e| {
                self.error = Some(e);
                Vec::new()
            }),
            None => Vec::new(),
        };
        self.missing_days = self
            .fetch_dates()
            .iter()
            .any(|d| !earnings.iter().any(|f| f.date == *d));
        // Freshness is about what is still to come; a past day's file never
        // needs fetching again.
        self.earnings_as_of = earnings
            .iter()
            .filter(|f| f.date >= today)
            .map(|f| f.fetched_at)
            .min();

        self.slate_seen = self.slate_path.as_deref().and_then(file_mtime);
        self.slate = match &self.slate_path {
            Some(path) => match crate::slate::load_at(path) {
                Ok(file) => crate::slate::ordered(&file, today, false),
                Err(e) => {
                    self.error = Some(format!("{e:#}"));
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        if self.econ_dir.is_none() || self.earn_dir.is_none() || self.slate_path.is_none() {
            self.error
                .get_or_insert_with(|| "couldn't find the wickd data folder".into());
        }

        self.rows = build(&Inputs {
            clock: self.clock,
            econ: &econ,
            earnings: &earnings,
            slate: &self.slate,
            listed,
            show_all: self.show_all,
        });
        // Reactions for reports that have left the screen are dropped.
        self.reactions.retain(|(_, date), _| *date >= start);
        // The results headlines stored for the watched reports that are out
        // (disk).
        self.headlines.clear();
        if let Some(dir) = self
            .earn_dir
            .as_deref()
            .map(earnings_headlines::headlines_dir)
        {
            for row in &self.rows {
                let Row::Earn(e) = row else { continue };
                if !(e.slate || e.listed) || !e.past {
                    continue;
                }
                if let Ok(Some(file)) = earnings_headlines::read_headlines(&dir, &e.symbol, e.date)
                {
                    self.headlines.insert((e.symbol.clone(), e.date), file);
                }
            }
        }
        // What the watched companies that have reported did report (disk).
        self.reported.clear();
        if let Some(dir) = self.earn_dir.as_deref().map(earnings_results::results_dir) {
            for row in &self.rows {
                let Row::Earn(e) = row else { continue };
                if !(e.slate || e.listed) || !e.past || self.reported.contains_key(&e.symbol) {
                    continue;
                }
                if let Ok(Some(file)) = earnings_results::read_results(&dir, &e.symbol) {
                    self.reported.insert(e.symbol.clone(), file);
                }
            }
        }
        self.selected = keep
            .and_then(|k| self.rows.iter().position(|r| row_key(r) == k))
            .or_else(|| {
                self.nearest_selectable(self.selected.min(self.rows.len().saturating_sub(1)))
            })
            .unwrap_or(0);
    }

    /// Put the cursor on the next thing still to come (the first row that
    /// hasn't happened yet), or on the last row when everything has.
    pub fn go_next_upcoming(&mut self) {
        self.selected = (0..self.rows.len())
            .find(|i| self.rows[*i].selectable() && !self.rows[*i].past())
            .or_else(|| {
                (0..self.rows.len())
                    .rev()
                    .find(|i| self.rows[*i].selectable())
            })
            .unwrap_or(0);
    }

    /// Whether opening the screen should start a background fetch: a shown
    /// weekday has no file, or the oldest coming day's file is more than
    /// six hours old. Never while one is running, and not again within ten
    /// minutes of the last automatic one.
    pub fn needs_fetch(&self) -> bool {
        if self.loading || self.earn_dir.is_none() {
            return false;
        }
        if self.last_auto.is_some_and(|t| t.elapsed() < AUTO_RETRY) {
            return false;
        }
        self.missing_days
            || self
                .earnings_as_of
                .is_none_or(|t| self.clock.now() - t > Duration::hours(STALE_HOURS))
    }

    /// Mark a fetch as started (`auto` = not asked for with `r`).
    pub fn fetch_started(&mut self, auto: bool) {
        self.loading = true;
        if auto {
            self.last_auto = Some(Instant::now());
        }
    }

    /// Take a finished fetch: say what fell short, then re-read the store.
    pub fn fetch_finished(
        &mut self,
        result: Result<Vec<DayOutcome>, String>,
        listed: &HashSet<String>,
    ) {
        self.loading = false;
        self.fetch_error = match result {
            Err(e) => Some(format!("couldn't fetch earnings: {e}")),
            Ok(days) => {
                let failed: Vec<&DayOutcome> = days.iter().filter(|d| !d.ok()).collect();
                failed.first().map(|first| {
                    format!(
                        "couldn't fetch earnings for {} of {} days ({}); showing what is stored",
                        failed.len(),
                        days.len(),
                        first.error.as_deref().unwrap_or("unknown error")
                    )
                })
            }
        };
        self.reload(listed);
    }

    /// The watched reports whose first trading day afterwards has come (as
    /// far as the calendar can tell): the reactions worth asking for.
    pub fn reactions_wanted(&self) -> Vec<ReactionAsk> {
        let market_today = earnings_store::market_date(self.clock.now());
        let mut out: Vec<ReactionAsk> = Vec::new();
        for row in &self.rows {
            let Row::Earn(e) = row else { continue };
            if !(e.slate || e.listed) || earliest_reaction_day(e.date, e.session) > market_today {
                continue;
            }
            if !out.iter().any(|(s, d, _)| *s == e.symbol && *d == e.date) {
                out.push((e.symbol.clone(), e.date, e.session));
            }
        }
        out
    }

    /// Take a finished reaction fetch. A failure keeps the figures already
    /// held.
    pub fn reactions_finished(&mut self, result: Result<Vec<(ReactionAsk, Reaction)>, String>) {
        self.reactions_loading = false;
        match result {
            Ok(got) => {
                self.reactions_error = None;
                for ((symbol, date, _), r) in got {
                    self.reactions.insert((symbol, date), r);
                }
            }
            Err(e) => self.reactions_error = Some(format!("couldn't fetch reactions: {e}")),
        }
    }

    /// Put `symbol` on the slate for the report on `date` (locked write).
    /// An entry already there keeps its notes and takes the date and
    /// session.
    pub fn slate_add(&self, symbol: &str, date: NaiveDate, session: Session) -> Result<(), String> {
        let path = self
            .slate_path
            .as_deref()
            .ok_or("couldn't find the wickd data folder")?;
        let change = crate::slate::Change {
            date: Some(date),
            session: Some(session),
            ..Default::default()
        };
        crate::slate::update_at(path, |f| {
            crate::slate::add(f, &[symbol.to_string()], &change, |_| Ok(None), Utc::now())
        })
        .map(|_| ())
        .map_err(|e| format!("{e:#}"))
    }

    /// Take `symbol` off the slate (locked write).
    pub fn slate_remove(&self, symbol: &str) -> Result<(), String> {
        let path = self
            .slate_path
            .as_deref()
            .ok_or("couldn't find the wickd data folder")?;
        crate::slate::update_at(path, |f| Ok(crate::slate::remove(f, &[symbol.to_string()])))
            .map(|_| ())
            .map_err(|e| format!("{e:#}"))
    }

    /// The results headlines for a row's report, when any are stored.
    pub fn headlines_of(&self, e: &EarnRow) -> &[Headline] {
        self.headlines
            .get(&(e.symbol.clone(), e.date))
            .map_or(&[], |f| &f.picked[..])
    }

    /// The watched reports that are out and have no results headline
    /// stored: the ones worth asking the news feed about. Unless `force`d
    /// (the `r` key), a report asked about in the last half hour is left
    /// alone.
    pub fn headlines_wanted(&self, force: bool) -> Vec<ReactionAsk> {
        let mut out: Vec<ReactionAsk> = Vec::new();
        for row in &self.rows {
            let Row::Earn(e) = row else { continue };
            if !(e.slate || e.listed) || !e.past || !self.headlines_of(e).is_empty() {
                continue;
            }
            let key = (e.symbol.clone(), e.date);
            let recently = self
                .headlines_tried
                .get(&key)
                .is_some_and(|t| t.elapsed() < RESULTS_RETRY);
            if (force || !recently) && !out.iter().any(|(s, d, _)| *s == key.0 && *d == key.1) {
                out.push((e.symbol.clone(), e.date, e.session));
            }
        }
        out
    }

    /// Mark a headlines fetch as started for `asks`.
    pub fn headlines_started(&mut self, asks: &[ReactionAsk]) {
        self.headlines_loading = true;
        let now = Instant::now();
        for (s, d, _) in asks {
            self.headlines_tried.insert((s.clone(), *d), now);
        }
    }

    /// Take a finished headlines fetch (`failed` = how many reports
    /// couldn't be fetched, with the first reason), then re-read the store.
    pub fn headlines_finished(
        &mut self,
        result: Result<(usize, usize, Option<String>), String>,
        listed: &HashSet<String>,
    ) {
        self.headlines_loading = false;
        self.headlines_error = match result {
            Err(e) => Some(format!("couldn't fetch headlines: {e}")),
            Ok((_, 0, _)) => None,
            Ok((asked, failed, first)) => Some(format!(
                "couldn't fetch headlines for {failed} of {asked} reports ({})",
                first.as_deref().unwrap_or("unknown error")
            )),
        };
        self.reload(listed);
    }

    /// Everything known about the watched reports right now, one key per
    /// kind of thing per company: used to say what an update brought in.
    pub fn known(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for row in &self.rows {
            let Row::Earn(e) = row else { continue };
            if !(e.slate || e.listed) {
                continue;
            }
            if !self.headlines_of(e).is_empty() {
                out.insert(format!("{} headlines", e.symbol));
            }
            if self.reported_of(e).is_some() {
                out.insert(format!("{} figures", e.symbol));
            }
            if self.reaction_of(e).is_some() {
                out.insert(format!("{} reaction", e.symbol));
            }
        }
        out
    }

    /// Whether anything an update started is still out.
    pub fn busy(&self) -> bool {
        self.loading || self.reactions_loading || self.results_loading || self.headlines_loading
    }

    /// If an update (`r`) has fully come back, end it and say what it
    /// brought in: the companies with something new, in screen order.
    pub fn take_update_summary(&mut self) -> Option<String> {
        if self.busy() {
            return None;
        }
        let before = self.updating.take()?;
        let now = self.known();
        let mut new: Vec<String> = Vec::new();
        for row in &self.rows {
            let Row::Earn(e) = row else { continue };
            let gained = now
                .iter()
                .any(|k| k.starts_with(&format!("{} ", e.symbol)) && !before.contains(k));
            if gained && !new.contains(&e.symbol) {
                new.push(e.symbol.clone());
            }
        }
        Some(if new.is_empty() {
            "Updated. Nothing new came in.".into()
        } else {
            format!("Updated. New for {}.", new.join(", "))
        })
    }

    /// The watched companies whose report is out and whose reported figures
    /// aren't stored: the ones worth asking the results feed about. Unless
    /// `force`d (the `r` key), a company asked about in the last half hour
    /// is left alone, since the feed can lag a report by a day.
    pub fn results_wanted(&self, force: bool) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for row in &self.rows {
            let Row::Earn(e) = row else { continue };
            if !(e.slate || e.listed) || !e.past || self.reported_of(e).is_some() {
                continue;
            }
            let recently = self
                .results_tried
                .get(&e.symbol)
                .is_some_and(|t| t.elapsed() < RESULTS_RETRY);
            if (force || !recently) && !out.contains(&e.symbol) {
                out.push(e.symbol.clone());
            }
        }
        out
    }

    /// Mark a fetch of reported figures as started for `symbols`.
    pub fn results_started(&mut self, symbols: &[String]) {
        self.results_loading = true;
        let now = Instant::now();
        for s in symbols {
            self.results_tried.insert(s.clone(), now);
        }
    }

    /// Take a finished fetch of reported figures: say what fell short, then
    /// re-read the store.
    pub fn results_finished(
        &mut self,
        result: Result<Vec<ResultOutcome>, String>,
        listed: &HashSet<String>,
    ) {
        self.results_loading = false;
        self.results_error = match result {
            Err(e) => Some(format!("couldn't fetch reported figures: {e}")),
            Ok(outcomes) => {
                let failed: Vec<&ResultOutcome> = outcomes.iter().filter(|o| !o.ok()).collect();
                failed.first().map(|first| {
                    format!(
                        "couldn't fetch reported figures for {} of {} companies ({})",
                        failed.len(),
                        outcomes.len(),
                        first.error.as_deref().unwrap_or("unknown error")
                    )
                })
            }
        };
        self.reload(listed);
    }

    /// What a row's company reported for that report, when the figures are
    /// stored (matched by report date, never by being the newest).
    pub fn reported_of(&self, e: &EarnRow) -> Option<&ResultRow> {
        self.reported.get(&e.symbol)?.for_report(e.date)
    }

    /// How a row's stock reacted, when that is known.
    pub fn reaction_of(&self, e: &EarnRow) -> Option<&Reaction> {
        self.reactions.get(&(e.symbol.clone(), e.date))
    }

    fn nearest_selectable(&self, from: usize) -> Option<usize> {
        let after = (from..self.rows.len()).find(|i| self.rows[*i].selectable());
        after.or_else(|| (0..from).rev().find(|i| self.rows[*i].selectable()))
    }

    /// Move the cursor `delta` selectable rows (negative = up), stopping at
    /// the ends.
    pub fn step(&mut self, delta: isize) {
        let mut at = self.selected;
        for _ in 0..delta.unsigned_abs() {
            let next = if delta > 0 {
                (at + 1..self.rows.len()).find(|i| self.rows[*i].selectable())
            } else {
                (0..at).rev().find(|i| self.rows[*i].selectable())
            };
            match next {
                Some(n) => at = n,
                None => break,
            }
        }
        self.selected = at;
    }

    pub fn go_top(&mut self) {
        self.selected = self.nearest_selectable(0).unwrap_or(0);
    }

    pub fn go_end(&mut self) {
        self.selected = (0..self.rows.len())
            .rev()
            .find(|i| self.rows[*i].selectable())
            .unwrap_or(0);
    }

    /// The earnings row under the cursor.
    pub fn selected_earn(&self) -> Option<&EarnRow> {
        match self.rows.get(self.selected) {
            Some(Row::Earn(e)) => Some(e),
            _ => None,
        }
    }

    /// The ticker under the cursor, when it's an earnings row.
    pub fn selected_symbol(&self) -> Option<&str> {
        self.selected_earn().map(|e| e.symbol.as_str())
    }

    /// The first row to draw so the cursor is on screen in a body `page`
    /// rows tall. The first selectable row shows from the very top, so its
    /// day heading is never scrolled off, and the last one shows through to
    /// the end, so the notes under it aren't either.
    pub fn settle_scroll(&self, page: usize) -> usize {
        let page = page.max(1);
        let bottom = self.rows.len().saturating_sub(page);
        let last = (0..self.rows.len())
            .rev()
            .find(|i| self.rows[*i].selectable());
        let mut scroll = self.scroll.get();
        if self.nearest_selectable(0) == Some(self.selected) || self.rows.len() <= page {
            scroll = 0;
        } else if last == Some(self.selected) {
            // On the last row, show as much of what is under it as fits.
            scroll = bottom.min(self.selected);
        } else if self.selected < scroll {
            // Moving up: bring the row's day heading back with it when it
            // fits on the page.
            let head = (0..=self.selected)
                .rev()
                .find(|i| matches!(self.rows[*i], Row::Day(_)))
                .unwrap_or(0);
            scroll = if self.selected - head < page {
                head
            } else {
                self.selected
            };
        } else if self.selected >= scroll + page {
            scroll = self.selected + 1 - page;
        }
        scroll = scroll.min(self.rows.len().saturating_sub(page));
        self.scroll.set(scroll);
        self.page.set(page);
        scroll
    }
}

/// What makes a row "the same row" across a reload: an event's identity,
/// not its changeable details (a row marked for the slate is still that
/// row).
fn row_key(r: &Row) -> (u8, String, String) {
    match r {
        Row::Earn(e) => (0, e.symbol.clone(), e.date.to_string()),
        Row::Econ(e) => (1, e.event.clone(), format!("{} {}", e.date, e.time)),
        Row::Day(l) => (2, l.clone(), String::new()),
        Row::Note(n) => (3, n.clone(), String::new()),
        Row::Blank => (4, String::new(), String::new()),
    }
}

fn file_mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Fetch and store the results headlines for each of `asks` (one request
/// per report: the feed returns the newest or the oldest fifty for all the
/// symbols asked together, which would starve the quieter ones). Returns
/// how many were asked, how many failed, and the first failure's reason.
pub async fn fetch_headlines(
    client: &wickd_core::equities::alpaca::AlpacaClient,
    dir: &Path,
    asks: Vec<ReactionAsk>,
    now: DateTime<Utc>,
) -> (usize, usize, Option<String>) {
    let asked = asks.len();
    let mut failed = 0;
    let mut first: Option<String> = None;
    for (symbol, date, session) in asks {
        let r = match earnings_headlines::window(date, session) {
            Some((start, end)) => client
                .news_window(std::slice::from_ref(&symbol), start, end.min(now), 50)
                .await
                .map_err(|e| e.to_string())
                .and_then(|items| {
                    earnings_headlines::store_fetch(dir, &symbol, date, &items, now).map(|_| ())
                }),
            None => Err(format!("no time window for {symbol} on {date}")),
        };
        if let Err(e) = r {
            failed += 1;
            first.get_or_insert(e);
        }
    }
    (asked, failed, first)
}

/// Fetch daily bars for `asks` in one batched request and work out each
/// reaction. An ask whose reaction day hasn't traded yet is simply absent
/// from the answer.
pub async fn fetch_reactions(
    client: &wickd_core::equities::alpaca::AlpacaClient,
    asks: Vec<ReactionAsk>,
    now: DateTime<Utc>,
) -> Result<Vec<(ReactionAsk, Reaction)>, String> {
    let mut symbols: Vec<String> = Vec::new();
    for (s, _, _) in &asks {
        if !symbols.contains(s) {
            symbols.push(s.clone());
        }
    }
    let Some(oldest) = asks.iter().map(|a| a.1).min() else {
        return Ok(Vec::new());
    };
    // Far enough back for a prior close and the volume average.
    let back = (wickd_core::earnings_reaction::VOLUME_SESSIONS as i64 + 2) * 7 / 5 + 10;
    let start = oldest.and_time(chrono::NaiveTime::MIN).and_utc() - Duration::days(back);
    let bars = client
        .bars_on(&symbols, "1Day", start, "sip")
        .await
        .map_err(|e| e.to_string())?;
    Ok(asks
        .into_iter()
        .filter_map(|ask| {
            let r = wickd_core::earnings_reaction::reaction(bars.get(&ask.0)?, ask.1, ask.2, now)?;
            Some((ask, r))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use wickd_core::earnings_store::EarningsRow;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn mst(now: &str) -> Clock {
        Clock::mst(now)
    }

    fn ev(
        date: &str,
        time: &str,
        currency: &str,
        event: &str,
        impact: &str,
        forecast: &str,
    ) -> CalendarEvent {
        CalendarEvent {
            date: date.into(),
            time: time.into(),
            currency: currency.into(),
            event: event.into(),
            impact: impact.into(),
            actual: String::new(),
            forecast: forecast.into(),
            previous: "1.0".into(),
        }
    }

    fn er(date: &str, symbol: &str, session: Session, cap: Option<Decimal>) -> EarningsRow {
        EarningsRow {
            date: d(date),
            symbol: symbol.into(),
            name: format!("{symbol} Corp"),
            session,
            eps_forecast: Some(dec!(1.25)),
            estimates: Some(5),
            market_cap: cap,
            last_year_eps: None,
            fiscal_quarter: String::new(),
        }
    }

    fn day(date: &str, rows: Vec<EarningsRow>) -> DayFile {
        DayFile {
            date: d(date),
            fetched_at: "2026-11-18T00:30:00Z".parse().unwrap(),
            source: "nasdaq".into(),
            rows,
        }
    }

    fn pick(symbol: &str, date: &str, session: Session, why: &str, expect: &str) -> Entry {
        Entry {
            symbol: symbol.into(),
            date: d(date),
            session,
            why: (!why.is_empty()).then(|| why.to_string()),
            expect: (!expect.is_empty()).then(|| expect.to_string()),
            result: None,
            added: "2026-11-15T03:00:00Z".parse().unwrap(),
        }
    }

    fn econ() -> Vec<CalendarEvent> {
        vec![
            // 13:30 UTC = 6:30 AM MST.
            ev(
                "2026-11-18",
                "13:30",
                "USD",
                "Housing Starts",
                "low",
                "1.25M",
            ),
            ev(
                "2026-11-18",
                "19:00",
                "USD",
                "FOMC Meeting Minutes",
                "high",
                "",
            ),
            ev(
                "2026-11-18",
                "10:00",
                "EUR",
                "Final CPI y/y",
                "high",
                "2.1%",
            ),
            // 01:00 UTC on the 19th is still 6:00 PM on the 18th in Denver.
            ev(
                "2026-11-19",
                "01:00",
                "USD",
                "FOMC Member Speaks",
                "medium",
                "",
            ),
            ev(
                "2026-11-19",
                "13:30",
                "USD",
                "Unemployment Claims",
                "medium",
                "198K",
            ),
            // Yesterday's big release: behind us, not on the watch.
            ev(
                "2026-11-17",
                "13:30",
                "USD",
                "Retail Sales m/m",
                "high",
                "0.2%",
            ),
            // The store reaches Friday the 20th and no further.
            ev(
                "2026-11-20",
                "15:00",
                "USD",
                "Core PCE Price Index m/m",
                "high",
                "0.2%",
            ),
        ]
    }

    fn earnings() -> Vec<DayFile> {
        vec![
            day(
                "2026-11-17",
                vec![
                    er(
                        "2026-11-17",
                        "HD",
                        Session::PreMarket,
                        Some(dec!(347000000000)),
                    ),
                    er(
                        "2026-11-17",
                        "NVDA",
                        Session::AfterHours,
                        Some(dec!(3760000000000)),
                    ),
                    er(
                        "2026-11-17",
                        "DIS",
                        Session::PreMarket,
                        Some(dec!(205000000000)),
                    ),
                ],
            ),
            day(
                "2026-11-18",
                vec![
                    er(
                        "2026-11-18",
                        "TGT",
                        Session::PreMarket,
                        Some(dec!(72800000000)),
                    ),
                    er(
                        "2026-11-18",
                        "TJX",
                        Session::PreMarket,
                        Some(dec!(144000000000)),
                    ),
                    er(
                        "2026-11-18",
                        "SMOL",
                        Session::PreMarket,
                        Some(dec!(400000000)),
                    ),
                    er(
                        "2026-11-18",
                        "MINE",
                        Session::AfterHours,
                        Some(dec!(90000000)),
                    ),
                    er(
                        "2026-11-18",
                        "PANW",
                        Session::AfterHours,
                        Some(dec!(258000000000)),
                    ),
                    er("2026-11-18", "NOTIME", Session::Unknown, None),
                ],
            ),
            day(
                "2026-11-19",
                vec![er(
                    "2026-11-19",
                    "WMT",
                    Session::PreMarket,
                    Some(dec!(1020000000000)),
                )],
            ),
            day("2026-11-20", vec![]),
        ]
    }

    /// NVDA (yesterday), TGT (today) and a hand-dated company the store
    /// doesn't have.
    fn slate() -> Vec<Entry> {
        vec![
            pick(
                "NVDA",
                "2026-11-17",
                Session::AfterHours,
                "sets the tone for AI",
                "Beat and raise: +5%.",
            ),
            pick("TGT", "2026-11-18", Session::PreMarket, "consumer read", ""),
            pick("XYZ", "2026-11-19", Session::AfterHours, "", "A guess."),
        ]
    }

    fn built_with(show_all: bool, slate: &[Entry], listed: &[&str]) -> Vec<Row> {
        let listed: HashSet<String> = listed.iter().map(|s| s.to_string()).collect();
        build(&Inputs {
            // 15:00 UTC Wednesday = 8:00 AM in Denver, after the open.
            clock: mst("2026-11-18T15:00:00Z"),
            econ: &econ(),
            earnings: &earnings(),
            slate,
            listed: &listed,
            show_all,
        })
    }

    /// The rows as short text, one per line, for readable assertions.
    fn text(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                Row::Day(l) => format!("# {l}"),
                Row::Econ(e) => format!(
                    "{} {} {}{}",
                    e.time,
                    e.impact,
                    e.event,
                    if e.past { " (past)" } else { "" }
                ),
                Row::Earn(e) => format!(
                    "{} {}{}{}",
                    match session_text(e.session) {
                        "" => "no-time",
                        t => t,
                    },
                    match (e.slate, e.listed) {
                        (true, _) => "+",
                        (false, true) => "*",
                        _ => "",
                    },
                    e.symbol,
                    if e.past { " (past)" } else { "" }
                ),
                Row::Note(n) => format!("~ {n}"),
                Row::Blank => String::new(),
            })
            .collect()
    }

    #[test]
    fn the_watch_shows_only_what_was_picked_and_looks_back() {
        // Slate: NVDA, TGT, XYZ. On a list: MINE.
        let got = text(&built_with(false, &slate(), &["MINE"]));
        assert_eq!(
            got,
            [
                // Yesterday stays on screen for the reaction; only the watched
                // company, and no releases.
                "# Yesterday · Tue Nov 17",
                "post-close +NVDA (past)",
                "",
                "# Today · Wed Nov 18",
                "pre-open +TGT (past)",
                // High impact only: the low and medium releases and the EUR one are gone.
                "12:00 PM high FOMC Meeting Minutes",
                // Tiny, but on a list.
                "post-close *MINE",
                // TJX and PANW are far larger and not shown: size is not a reason.
                "~ 4 other companies report · a shows everything",
                "",
                "# Tomorrow · Thu Nov 19",
                // On the slate with a hand-given date the store doesn't have.
                "post-close +XYZ",
                "~ 1 other company reports · a shows everything",
                "",
                "# Fri Nov 20",
                "8:00 AM high Core PCE Price Index m/m",
                "",
                // Mon and Tue of next week have nothing watched: left out.
                "~ Earnings aren't stored for 2 of the coming weekdays, so a watched company could be missing. Press r to fetch.",
            ]
        );
        // No caption rows: every row is a day, an event, a note or a gap.
        assert!(built_with(false, &slate(), &[]).iter().all(|r| matches!(
            r,
            Row::Day(_) | Row::Econ(_) | Row::Earn(_) | Row::Note(_) | Row::Blank
        )));
    }

    #[test]
    fn slate_rows_carry_their_why_and_expectation() {
        let rows = built_with(false, &slate(), &["TGT"]);
        let earn = |sym: &str| {
            rows.iter()
                .find_map(|r| match r {
                    Row::Earn(e) if e.symbol == sym => Some(e.clone()),
                    _ => None,
                })
                .unwrap()
        };
        let nvda = earn("NVDA");
        assert_eq!(
            (
                nvda.slate,
                nvda.why.as_deref(),
                nvda.expect.as_deref(),
                nvda.date
            ),
            (
                true,
                Some("sets the tone for AI"),
                Some("Beat and raise: +5%."),
                d("2026-11-17")
            )
        );
        assert_eq!(nvda.name, "NVDA Corp", "the name comes from the store");
        // On the slate and on a list: both are recorded.
        let tgt = earn("TGT");
        assert!(tgt.slate && tgt.listed && tgt.expect.is_none());
        // Not in the store: no name or figures, the slate's own session.
        let xyz = earn("XYZ");
        assert_eq!(
            (
                xyz.name.as_str(),
                xyz.session,
                xyz.market_cap,
                xyz.eps_forecast
            ),
            ("", Session::AfterHours, None, None)
        );
        // A slate entry for another day doesn't mark the same symbol's other report.
        let other = [pick("WMT", "2026-11-25", Session::PreMarket, "", "")];
        let rows = built_with(true, &other, &[]);
        let wmt = rows
            .iter()
            .find_map(|r| match r {
                Row::Earn(e) if e.symbol == "WMT" => Some(e.clone()),
                _ => None,
            })
            .unwrap();
        assert!(!wmt.slate, "Thursday's report isn't the one on the slate");
    }

    #[test]
    fn the_slates_session_wins_over_the_stores() {
        // The store says PANW reports after the close; the slate was corrected.
        let fixed = [pick("PANW", "2026-11-18", Session::PreMarket, "", "")];
        let got = text(&built_with(false, &fixed, &[]));
        assert!(
            got.contains(&"pre-open +PANW (past)".to_string()),
            "{got:?}"
        );
    }

    #[test]
    fn an_empty_slate_says_how_to_start() {
        let got = text(&built_with(false, &[], &[]));
        assert_eq!(
            got[..5],
            [
                "~ Your slate is empty. Press a to browse who reports and s to add a company, or use `wickd earnings slate add`.",
                "",
                "# Today · Wed Nov 18",
                "12:00 PM high FOMC Meeting Minutes",
                "~ 6 other companies report · a shows everything",
            ]
        );
        // A day with nothing at all on it still shows when it is today.
        let listed = HashSet::new();
        let rows = build(&Inputs {
            clock: mst("2026-11-18T15:00:00Z"),
            econ: &[],
            earnings: &[day("2026-11-18", vec![])],
            slate: &[],
            listed: &listed,
            show_all: false,
        });
        let got = text(&rows);
        assert_eq!(
            got[2..4],
            ["# Today · Wed Nov 18", "~ Nothing you're watching."]
        );
    }

    #[test]
    fn everything_browses_the_full_feed_from_today_on() {
        let got = text(&built_with(true, &slate(), &[]));
        assert_eq!(
            &got[..15],
            [
                // Past days still show only what was watched.
                "# Yesterday · Tue Nov 17",
                "post-close +NVDA (past)",
                "",
                "# Today · Wed Nov 18",
                // Watched first within a session, then largest first.
                "pre-open +TGT (past)",
                "pre-open TJX (past)",
                "pre-open SMOL (past)",
                "6:30 AM low Housing Starts (past)",
                "12:00 PM high FOMC Meeting Minutes",
                "post-close PANW",
                "post-close MINE",
                "6:00 PM medium FOMC Member Speaks",
                // No time given: last in the day.
                "no-time NOTIME",
                "",
                "# Tomorrow · Thu Nov 19",
            ]
        );
        // Still US only, and yesterday's release isn't brought back.
        assert!(!got
            .iter()
            .any(|l| l.contains("Final CPI") || l.contains("Retail Sales")));
        // Browsing says what isn't stored, day by day.
        assert!(got.contains(&"~ No company reports.".to_string()));
        assert!(
            got.contains(&"~ No earnings stored for this day yet. Press r to fetch.".to_string())
        );
        assert!(got.contains(
            &"~ Economic events for this day aren't published yet (the feed covers the current week).".to_string()
        ));
        assert!(!got.iter().any(|l| l.contains("other compan")));
    }

    #[test]
    fn reports_count_as_out_after_their_session() {
        use Session::*;
        let day = d("2026-11-18");
        let at = |s: &str| -> DateTime<Utc> { s.parse().unwrap() };
        // 9:30 Eastern (EST) = 14:30 UTC.
        assert!(!session_passed(day, PreMarket, at("2026-11-18T14:29:00Z")));
        assert!(session_passed(day, PreMarket, at("2026-11-18T14:30:00Z")));
        // 8:00 PM Eastern = 01:00 UTC the next day.
        assert!(!session_passed(day, AfterHours, at("2026-11-19T00:59:00Z")));
        assert!(session_passed(day, AfterHours, at("2026-11-19T01:00:00Z")));
        // No time given: only once the day is over.
        assert!(!session_passed(day, Unknown, at("2026-11-19T04:00:00Z")));
        assert!(session_passed(day, Unknown, at("2026-11-19T05:00:00Z")));
        // Summer time: 9:30 Eastern (EDT) = 13:30 UTC.
        assert!(session_passed(
            d("2026-07-01"),
            PreMarket,
            at("2026-07-01T13:30:00Z")
        ));
        assert!(!session_passed(
            d("2026-07-01"),
            PreMarket,
            at("2026-07-01T13:29:00Z")
        ));
        // Tomorrow's reports are never out today.
        assert!(!session_passed(
            d("2026-11-19"),
            PreMarket,
            at("2026-11-19T01:00:00Z")
        ));
    }

    #[test]
    fn the_watch_starts_three_weekdays_back() {
        assert_eq!(watch_start(d("2026-11-18")), d("2026-11-13"), "Wed → Fri");
        assert_eq!(watch_start(d("2026-11-16")), d("2026-11-11"), "Mon → Wed");
        assert_eq!(watch_start(d("2026-11-21")), d("2026-11-18"), "Sat → Wed");
        assert_eq!(watch_start(d("2026-11-20")), d("2026-11-17"), "Fri → Tue");
    }

    #[test]
    fn labels_and_values_read_like_a_person_wrote_them() {
        let today = d("2026-11-18");
        assert_eq!(day_label(today, today), "Today · Wed Nov 18");
        assert_eq!(day_label(d("2026-11-19"), today), "Tomorrow · Thu Nov 19");
        assert_eq!(day_label(d("2026-11-17"), today), "Yesterday · Tue Nov 17");
        assert_eq!(day_label(d("2026-11-16"), today), "Mon Nov 16");
        assert_eq!(day_label(d("2026-12-01"), today), "Tue Dec 1");
        assert_eq!(fmt_cap(dec!(3762974100000)), "$3.76T");
        assert_eq!(fmt_cap(dec!(72832852000)), "$72.8B");
        assert_eq!(fmt_cap(dec!(258701689000)), "$259B");
        assert_eq!(fmt_cap(dec!(475001472)), "$475M");
        assert_eq!(fmt_cap(dec!(950000)), "$950000");
    }

    #[test]
    fn reports_soon_covers_today_and_the_next_weekday_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let write = |date: &str, rows: Vec<EarningsRow>| {
            earnings_store::write_day(dir, &day(date, rows)).unwrap();
        };
        // Friday the 20th: today, Monday the 23rd, and days that must not count.
        write(
            "2026-11-19",
            vec![er("2026-11-19", "OLD", Session::PreMarket, None)],
        );
        write(
            "2026-11-20",
            vec![
                er("2026-11-20", "FRI", Session::PreMarket, None),
                er("2026-11-20", "BOTH", Session::Unknown, None),
            ],
        );
        write(
            "2026-11-23",
            vec![
                er("2026-11-23", "MON", Session::AfterHours, None),
                er("2026-11-23", "BOTH", Session::PreMarket, None),
            ],
        );
        write(
            "2026-11-24",
            vec![er("2026-11-24", "TUE", Session::PreMarket, None)],
        );
        let today = d("2026-11-20");
        let soon = reports_soon(dir, today);
        let mut syms: Vec<&str> = soon.keys().map(String::as_str).collect();
        syms.sort_unstable();
        assert_eq!(syms, ["BOTH", "FRI", "MON"]);
        assert_eq!(soon["FRI"].short(today), "today pre-open");
        assert_eq!(soon["FRI"].long(today), "reports today before the open");
        // Across the weekend, by weekday name.
        assert_eq!(soon["MON"].short(today), "Mon post-close");
        assert_eq!(soon["MON"].long(today), "reports Mon after the close");
        // On both days: today's wins, and no time given reads plainly.
        assert_eq!(soon["BOTH"].short(today), "today");
        assert_eq!(soon["BOTH"].long(today), "reports today");
        assert_eq!(next_weekday(d("2026-11-18")), d("2026-11-19"));
        assert_eq!(next_weekday(d("2026-11-21")), d("2026-11-23"));

        // Nothing stored, an unreadable day, or a file holding another
        // day's rows: no markers, no panic.
        assert!(reports_soon(&dir.join("nope"), today).is_empty());
        std::fs::write(dir.join("2026-11-20.json"), b"not json").unwrap();
        let soon = reports_soon(dir, today);
        assert!(!soon.contains_key("FRI") && soon.contains_key("MON"));
        let wrong = serde_json::to_vec(&day(
            "2026-11-19",
            vec![er("2026-11-19", "OLD", Session::PreMarket, None)],
        ))
        .unwrap();
        std::fs::write(dir.join("2026-11-20.json"), wrong).unwrap();
        assert!(!reports_soon(dir, today).contains_key("OLD"));
    }

    /// A state reading temp stores and a temp slate, at 8:00 AM MST on Wed
    /// Nov 18.
    fn state(tmp: &Path) -> CalendarState {
        let econ_dir = tmp.join("calendar");
        let earn_dir = tmp.join("earnings");
        let slate_path = tmp.join("slate.json");
        std::fs::create_dir_all(&econ_dir).unwrap();
        calendar_store::merge_into_store(&econ_dir, econ()).unwrap();
        for f in earnings() {
            earnings_store::write_day(&earn_dir, &f).unwrap();
        }
        write_slate(&slate_path, slate());
        let mut s = CalendarState::new(Some(econ_dir), Some(earn_dir), Some(slate_path));
        s.clock = mst("2026-11-18T15:00:00Z");
        s
    }

    fn write_slate(path: &Path, entries: Vec<Entry>) {
        let file = crate::slate::SlateFile {
            version: 1,
            entries,
        };
        std::fs::write(path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn the_state_reads_the_stores_and_the_slate_and_the_cursor_skips_headings() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        s.reload(&none());
        assert!(s.error.is_none(), "{:?}", s.error);
        assert!(s.econ_as_of.is_some());
        assert_eq!(
            s.earnings_as_of,
            Some("2026-11-18T00:30:00Z".parse().unwrap())
        );
        assert_eq!(s.slate.len(), 3);
        // Rows: yesterday's NVDA, then today's TGT, minutes; tomorrow's XYZ; Friday's PCE.
        assert_eq!(s.selected, 1);
        assert_eq!(s.selected_symbol(), Some("NVDA"));
        s.step(1);
        assert_eq!(
            s.selected_symbol(),
            Some("TGT"),
            "over the gap and the heading"
        );
        s.step(1);
        assert_eq!(
            text(&s.rows[s.selected..=s.selected]),
            ["12:00 PM high FOMC Meeting Minutes"]
        );
        assert_eq!(s.selected_symbol(), None, "a release has no ticker");
        s.step(-10);
        assert_eq!(s.selected, 1, "stops at the first row");
        s.go_end();
        assert_eq!(
            text(&s.rows[s.selected..=s.selected]),
            ["8:00 AM high Core PCE Price Index m/m"]
        );
        s.step(5);
        assert!(
            matches!(s.rows[s.selected], Row::Econ(_)),
            "stops at the last row"
        );
        s.go_top();
        assert_eq!(s.selected, 1);
        // Opening lands on the next thing to come, not on yesterday.
        s.go_next_upcoming();
        assert_eq!(
            text(&s.rows[s.selected..=s.selected]),
            ["12:00 PM high FOMC Meeting Minutes"]
        );
    }

    #[test]
    fn toggling_everything_keeps_the_cursor_on_the_same_company() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        s.reload(&none());
        s.step(1);
        assert_eq!(s.selected_symbol(), Some("TGT"));
        s.show_all = true;
        s.reload(&none());
        assert_eq!(s.selected_symbol(), Some("TGT"));
        // A company only visible when browsing: toggling back moves to a neighbour.
        s.step(1);
        assert_eq!(s.selected_symbol(), Some("TJX"));
        s.show_all = false;
        s.reload(&none());
        assert!(s.rows[s.selected].selectable());
        assert_ne!(s.selected_symbol(), Some("TJX"));
    }

    #[test]
    fn a_change_to_the_slate_file_is_picked_up_and_keeps_the_cursor() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        s.reload(&none());
        assert!(!s.slate_changed());
        s.step(1); // TGT
                   // An agent adds PANW and writes TGT's expectation.
        let mut entries = slate();
        entries[1].expect = Some("Margins are the question.".into());
        entries.push(pick(
            "PANW",
            "2026-11-18",
            Session::AfterHours,
            "first software report",
            "",
        ));
        let path = s.slate_path.clone().unwrap();
        write_slate(&path, entries);
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(s.slate_changed());
        s.reload(&none());
        assert!(!s.slate_changed());
        // Same row under the cursor, though its text changed.
        let tgt = s.selected_earn().unwrap();
        assert_eq!(
            (tgt.symbol.as_str(), tgt.expect.as_deref()),
            ("TGT", Some("Margins are the question."))
        );
        assert!(text(&s.rows).contains(&"post-close +PANW".to_string()));
        // A slate that can't be read: said, and the screen still draws.
        std::fs::write(&path, b"not json").unwrap();
        s.reload(&none());
        assert!(
            s.error.as_deref().unwrap().contains("slate.json"),
            "{:?}",
            s.error
        );
        assert!(s.slate.is_empty());
    }

    #[test]
    fn in_the_evening_the_cursor_starts_on_the_next_thing_to_come() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        // 6:30 PM Mountain on the 18th: the minutes and today's timed
        // reports are behind us.
        s.clock = mst("2026-11-19T01:30:00Z");
        s.reload(&none());
        let got = text(&s.rows);
        for past in [
            "pre-open +TGT (past)",
            "12:00 PM high FOMC Meeting Minutes (past)",
        ] {
            assert!(got.contains(&past.to_string()), "{past} in {got:?}");
        }
        s.go_next_upcoming();
        assert_eq!(s.selected_symbol(), Some("XYZ"), "tomorrow's report");
        // When everything on screen has happened, the cursor rests on the last row.
        s.clock = mst("2026-11-21T03:00:00Z"); // Friday evening
        s.reload(&none());
        s.go_next_upcoming();
        assert!(s.rows[s.selected].selectable() && s.rows[s.selected].past());
        assert_eq!(
            s.nearest_selectable(s.selected + 1),
            Some(s.selected),
            "nothing after it"
        );
        // An empty screen is fine.
        let mut e = CalendarState::new(None, None, None);
        e.go_next_upcoming();
        assert_eq!(e.selected, 0);
    }

    #[test]
    fn a_fetch_is_wanted_for_missing_or_old_days_but_not_twice_in_a_row() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        s.reload(&none());
        // The look-back days and next week's have no file.
        assert!(s.missing_days && s.needs_fetch());
        assert_eq!(
            s.fetch_dates()
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>(),
            [
                "2026-11-13",
                "2026-11-16",
                "2026-11-17",
                "2026-11-18",
                "2026-11-19",
                "2026-11-20",
                "2026-11-23",
                "2026-11-24"
            ]
        );
        s.fetch_started(true);
        assert!(!s.needs_fetch(), "one is running");
        // It lands: every shown weekday is stored, fetched at 5:00 AM local
        // (three hours before "now").
        let dir = s.earn_dir.clone().unwrap();
        for date in s.fetch_dates() {
            let mut f = day(&date.to_string(), vec![]);
            f.fetched_at = "2026-11-18T12:00:00Z".parse().unwrap();
            earnings_store::write_day(&dir, &f).unwrap();
        }
        s.fetch_finished(Ok(vec![]), &none());
        assert!(!s.loading && !s.missing_days && s.fetch_error.is_none());
        s.last_auto = None;
        assert!(!s.needs_fetch(), "all days stored and under six hours old");
        // Noon the same day: seven hours after the fetch.
        s.clock = mst("2026-11-18T19:00:00Z");
        assert!(s.needs_fetch(), "more than six hours old");
        s.fetch_started(true);
        s.fetch_finished(Err("offline".into()), &none());
        assert_eq!(
            s.fetch_error.as_deref(),
            Some("couldn't fetch earnings: offline")
        );
        assert!(
            !s.needs_fetch(),
            "not again within ten minutes of an automatic try"
        );
    }

    #[test]
    fn an_old_file_for_a_past_day_does_not_make_the_data_stale() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        let dir = s.earn_dir.clone().unwrap();
        for date in s.fetch_dates() {
            let mut f = day(&date.to_string(), vec![]);
            // Past days were fetched a week ago; today onward an hour ago.
            f.fetched_at = if date < d("2026-11-18") {
                "2026-11-11T12:00:00Z".parse().unwrap()
            } else {
                "2026-11-18T14:00:00Z".parse().unwrap()
            };
            earnings_store::write_day(&dir, &f).unwrap();
        }
        s.reload(&none());
        assert_eq!(
            s.earnings_as_of,
            Some("2026-11-18T14:00:00Z".parse().unwrap())
        );
        assert!(!s.needs_fetch());
    }

    #[test]
    fn a_partial_fetch_says_how_many_days_fell_short() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        let ok = DayOutcome {
            date: d("2026-11-18"),
            rows: Some(6),
            written: true,
            kept_stored: false,
            error: None,
        };
        let bad = DayOutcome {
            date: d("2026-11-19"),
            rows: Some(1),
            written: false,
            kept_stored: false,
            error: Some("HTTP 503".into()),
        };
        s.fetch_finished(Ok(vec![ok, bad]), &none());
        assert_eq!(
            s.fetch_error.as_deref(),
            Some("couldn't fetch earnings for 1 of 2 days (HTTP 503); showing what is stored")
        );
        assert!(!s.rows.is_empty(), "the stored days are still shown");
    }

    fn reacted(day: &str, change: f64) -> Reaction {
        Reaction {
            day: d(day),
            complete: true,
            prior_close: 100.0,
            open: 100.0 + change / 2.0,
            close: 100.0 + change,
            gap_pct: change / 2.0,
            change_pct: change,
            close_in_range: Some(80.0),
            rel_volume: Some(2.5),
        }
    }

    #[test]
    fn reactions_are_asked_for_watched_reports_whose_day_has_come() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        // MINE (after today's close) is on a list.
        let listed: HashSet<String> = ["MINE".to_string()].into_iter().collect();
        s.reload(&listed);
        // 8:00 AM Wednesday. NVDA reported Tuesday after the close: its
        // reaction day is today. TGT reported this morning: today. MINE and
        // XYZ report after a close still to come.
        assert_eq!(
            s.reactions_wanted(),
            vec![
                ("NVDA".to_string(), d("2026-11-17"), Session::AfterHours),
                ("TGT".to_string(), d("2026-11-18"), Session::PreMarket),
            ]
        );
        // Browsing everything doesn't ask about companies nobody picked.
        s.show_all = true;
        s.reload(&listed);
        assert_eq!(s.reactions_wanted().len(), 2);
        s.show_all = false;
        s.reload(&listed);

        s.reactions_loading = true;
        let nvda = ("NVDA".to_string(), d("2026-11-17"), Session::AfterHours);
        s.reactions_finished(Ok(vec![(nvda.clone(), reacted("2026-11-18", 4.0))]));
        assert!(!s.reactions_loading && s.reactions_error.is_none());
        let row = s.selected_earn().cloned().unwrap();
        assert_eq!(row.symbol, "NVDA");
        assert_eq!(s.reaction_of(&row).map(|r| r.change_pct), Some(4.0));
        // A failed fetch keeps what is held and says so.
        s.reactions_finished(Err("rate limited".into()));
        assert_eq!(
            s.reactions_error.as_deref(),
            Some("couldn't fetch reactions: rate limited")
        );
        assert!(s.reaction_of(&row).is_some());
        // Once the report scrolls out of the look-back, its reaction is dropped.
        s.clock = mst("2026-11-25T15:00:00Z");
        s.reload(&listed);
        assert!(s.reactions.is_empty());
    }

    #[test]
    fn scrolling_keeps_the_cursor_on_screen_and_the_first_heading_visible() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        s.show_all = true;
        s.reload(&none());
        assert_eq!(s.settle_scroll(8), 0);
        s.go_end();
        let scroll = s.settle_scroll(8);
        // The last release is followed by more than a page of notes (two
        // days with nothing stored), so it sits at the top of the page.
        assert_eq!(scroll, s.selected.min(s.rows.len() - 8));
        assert!(s.selected >= scroll && s.selected < scroll + 8);
        // With room for everything under it, the page ends at the last row.
        assert_eq!(s.settle_scroll(12), s.rows.len() - 12);
        s.step(-3);
        let up = s.settle_scroll(8);
        assert!(s.selected >= up && s.selected < up + 8);
        s.go_top();
        assert_eq!(
            s.settle_scroll(8),
            0,
            "back at the top the day heading shows"
        );
        assert_eq!(s.settle_scroll(500), 0, "everything fits");
    }

    #[test]
    fn missing_folders_are_the_empty_state_not_a_crash() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = CalendarState::new(
            Some(tmp.path().join("no-cal")),
            Some(tmp.path().join("no-earn")),
            Some(tmp.path().join("no-slate.json")),
        );
        s.clock = mst("2026-11-18T15:00:00Z");
        s.reload(&none());
        assert!(s.error.is_none(), "{:?}", s.error);
        assert!(s.missing_days && s.needs_fetch());
        assert!(s.rows.iter().all(|r| !r.selectable()));
        assert!(text(&s.rows)[0].starts_with("~ Your slate is empty."));
        s.step(1);
        s.go_end();
        assert_eq!(s.selected_symbol(), None);
        assert!(s.reactions_wanted().is_empty());
        // No data home at all: said plainly, and no fetch is attempted.
        let mut s = CalendarState::new(None, None, None);
        s.clock = mst("2026-11-18T15:00:00Z");
        s.reload(&none());
        assert_eq!(
            s.error.as_deref(),
            Some("couldn't find the wickd data folder")
        );
        assert!(!s.needs_fetch());
    }
}
