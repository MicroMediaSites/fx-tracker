//! The calendar screen's state (`g c`): what is coming that can move the
//! market, for today and the six days after it.
//!
//! Two stores feed it, both read from disk and never blocked on:
//!
//! - **economic events** from `~/.wickd/calendar/` (kept fresh by the
//!   `com.openthink.wickd-calendar` job; this screen only reads it), shown
//!   in the machine's local time;
//! - **earnings** from `~/.wickd/earnings/` ([`wickd_core::earnings_store`]),
//!   which this screen does refresh: opening it asks for a background fetch
//!   when a shown weekday has no file or the files are more than six hours
//!   old ([`CalendarState::needs_fetch`]), and `r` forces one.
//!
//! Each day is one list of events in time order: a row says whether it is
//! an economic release or an earnings report, and nothing separates the two.
//! [`build`] turns the two stores into display rows and is a pure function
//! of its inputs (the clock and the time zone are passed in), so everything
//! the screen shows is unit-tested without a terminal or a network.

use std::cell::Cell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use chrono::{
    DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, Utc, Weekday,
};
use rust_decimal::Decimal;

use wickd_core::backtest::surprise::impact_rank;
use wickd_core::calendar_store::{self, CalendarEvent};
use wickd_core::earnings_store::{self, clean_text, DayFile, DayOutcome, Session};

use super::app::Mode;

/// Days on the screen: today and the six after it.
pub const DAYS: i64 = 7;

/// Earnings files older than this are fetched again when the screen opens.
const STALE_HOURS: i64 = 6;

/// How long after an automatic fetch before another may start, so a feed
/// that is down isn't asked again on every visit.
const AUTO_RETRY: std::time::Duration = std::time::Duration::from_secs(600);

/// Companies worth at least this are shown by default ($10 billion).
pub fn large_cap() -> Decimal {
    Decimal::from(10_000_000_000u64)
}

/// The only currency this screen shows: it is a US stock calendar.
const CURRENCY: &str = "USD";

/// One economic release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconRow {
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
    pub name: String,
    pub session: Session,
    pub eps_forecast: Option<Decimal>,
    pub last_year_eps: Option<Decimal>,
    pub market_cap: Option<Decimal>,
    /// On the watchlist or a `wickd list`.
    pub listed: bool,
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

/// `Today · Wed Nov 18`, `Tomorrow · Thu Nov 19`, then `Fri Nov 20`.
pub fn day_label(day: NaiveDate, today: NaiveDate) -> String {
    let date = day.format("%a %b %-d");
    match (day - today).num_days() {
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
    /// Symbols on the watchlist or a list.
    pub listed: &'a HashSet<String>,
    /// Show every impact and every company.
    pub show_all: bool,
}

/// The screen's rows: a heading per day, then everything on that day as one
/// list in time order, economic releases and earnings together. Each row
/// says which it is; there are no captions between them.
///
/// Order within a day: reports due before the open, then releases by time
/// up to the 4:00 PM Eastern close, then reports due after the close, then
/// any later releases, then reports with no time given. Companies in the
/// same session run largest first.
///
/// By default only US releases of medium impact and up are shown, and only
/// companies worth [`large_cap`] or more, plus every listed symbol whatever
/// its size. A weekend day with nothing on it is left out.
pub fn build(inp: &Inputs) -> Vec<Row> {
    let now = inp.clock.now();
    let today = inp.clock.today();
    // Each economic row with its instant and local time. How far the store
    // reaches (any currency) tells "not published yet" from "nothing
    // scheduled".
    let econ_all: Vec<(DateTime<Utc>, NaiveDateTime, &CalendarEvent)> = inp
        .econ
        .iter()
        .filter_map(|e| {
            let at = DateTime::<Utc>::from_timestamp(e.time_unix()?, 0)?;
            Some((at, inp.clock.local(at), e))
        })
        .collect();
    let econ_through = econ_all.iter().map(|(_, t, _)| t.date()).max();

    let mut rows = Vec::new();
    for i in 0..DAYS {
        let day = today + Duration::days(i);
        let econ: Vec<&(DateTime<Utc>, NaiveDateTime, &CalendarEvent)> = econ_all
            .iter()
            .filter(|(_, t, e)| {
                t.date() == day
                    && e.currency.eq_ignore_ascii_case(CURRENCY)
                    && (inp.show_all
                        || impact_rank(&e.impact) >= 2
                        || e.impact.eq_ignore_ascii_case("holiday"))
            })
            .collect();
        let file = inp.earnings.iter().find(|d| d.date == day);
        let reporting = file.map_or(0, |f| f.rows.len());
        if is_weekend(day) && econ.is_empty() && reporting == 0 {
            continue;
        }
        rows.push(Row::Day(day_label(day, today)));

        // (slot in the day, instant, size (largest first), name) → row.
        type Key = (u8, i64, std::cmp::Reverse<Option<Decimal>>, String);
        let mut items: Vec<(Key, Row)> = Vec::new();
        let close = eastern_instant(day, 16, 0);
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
                    std::cmp::Reverse(None),
                    e.event.clone(),
                ),
                Row::Econ(EconRow {
                    time: clock_text(*local),
                    past: *at < now,
                    impact: e.impact.to_lowercase(),
                    event: clean_text(&e.event, 80),
                    forecast: clean_text(&e.forecast, 12),
                    previous: clean_text(&e.previous, 12),
                }),
            ));
        }
        let shown: Vec<&earnings_store::EarningsRow> = file
            .map(|f| {
                f.rows
                    .iter()
                    .filter(|r| {
                        inp.show_all
                            || inp.listed.contains(&r.symbol)
                            || r.market_cap.is_some_and(|c| c >= large_cap())
                    })
                    .collect()
            })
            .unwrap_or_default();
        for r in &shown {
            let slot = match r.session {
                Session::PreMarket => 0,
                Session::AfterHours => 2,
                Session::Unknown => 4,
            };
            items.push((
                (slot, 0, std::cmp::Reverse(r.market_cap), r.symbol.clone()),
                Row::Earn(EarnRow {
                    symbol: r.symbol.clone(),
                    name: r.name.clone(),
                    session: r.session,
                    eps_forecast: r.eps_forecast,
                    last_year_eps: r.last_year_eps,
                    market_cap: r.market_cap,
                    listed: inp.listed.contains(&r.symbol),
                    past: session_passed(day, r.session, now),
                }),
            ));
        }
        items.sort_by(|a, b| a.0.cmp(&b.0));
        rows.extend(items.into_iter().map(|(_, row)| row));

        // What isn't there, said plainly under the day's events.
        if econ.is_empty() {
            rows.push(Row::Note(match econ_through {
                Some(through) if day <= through => "No US economic releases.".into(),
                _ => "Economic events for this day aren't published yet (the feed covers the current week).".into(),
            }));
        }
        let plural = |n: usize| if n == 1 { "company" } else { "companies" };
        match file {
            None if is_weekend(day) => {}
            None => rows.push(Row::Note(
                "No earnings stored for this day yet. Press r to fetch.".into(),
            )),
            Some(f) if f.rows.is_empty() => rows.push(Row::Note("No company reports.".into())),
            Some(f) => {
                let hidden = f.rows.len() - shown.len();
                if shown.is_empty() {
                    rows.push(Row::Note(format!(
                        "No large company reports. {hidden} smaller {} · a shows everything",
                        plural(hidden)
                    )));
                } else if hidden > 0 {
                    rows.push(Row::Note(format!(
                        "+ {hidden} smaller {} · a shows everything",
                        plural(hidden)
                    )));
                }
            }
        }
        rows.push(Row::Blank);
    }
    while rows.last() == Some(&Row::Blank) {
        rows.pop();
    }
    rows
}

#[derive(Debug)]
pub struct CalendarState {
    /// `~/.wickd/calendar`; `None` when the data home couldn't be resolved.
    pub econ_dir: Option<PathBuf>,
    /// `~/.wickd/earnings`.
    pub earn_dir: Option<PathBuf>,
    pub clock: Clock,
    pub rows: Vec<Row>,
    /// The row under the cursor (index into `rows`; always selectable when
    /// any row is).
    pub selected: usize,
    /// First row shown, as last settled by the renderer.
    pub scroll: Cell<usize>,
    /// Body rows on screen, as last measured by the renderer.
    pub page: Cell<usize>,
    /// Every impact and every company, instead of the important ones.
    pub show_all: bool,
    /// An earnings fetch is in flight.
    pub loading: bool,
    /// Why a store couldn't be read.
    pub error: Option<String>,
    /// Why the last earnings fetch fell short (stored data is still shown).
    pub fetch_error: Option<String>,
    /// The oldest fetch time among the shown earnings days.
    pub earnings_as_of: Option<DateTime<Utc>>,
    /// When the economic store was last written.
    pub econ_as_of: Option<SystemTime>,
    /// A shown weekday has no earnings file.
    pub missing_days: bool,
    /// Where `esc` returns to.
    pub back_to: Mode,
    /// When the last automatic fetch was started.
    last_auto: Option<Instant>,
}

impl CalendarState {
    pub fn new(econ_dir: Option<PathBuf>, earn_dir: Option<PathBuf>) -> Self {
        Self {
            econ_dir,
            earn_dir,
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
            back_to: Mode::Watchlist,
            last_auto: None,
        }
    }

    /// The real stores under the wickd data home.
    pub fn at_data_home() -> Self {
        Self::new(
            wickd_core::events::calendar_dir().ok(),
            earnings_store::earnings_dir().ok(),
        )
    }

    /// The weekdays on screen, which are the days an earnings fetch asks for.
    pub fn fetch_dates(&self) -> Vec<NaiveDate> {
        earnings_store::weekdays_from(self.clock.today(), (DAYS - 1) as u32)
    }

    /// Read both stores and rebuild the rows, keeping the cursor on the same
    /// row when it is still there.
    pub fn reload(&mut self, listed: &HashSet<String>) {
        let keep = self
            .rows
            .get(self.selected)
            .filter(|r| r.selectable())
            .cloned();
        let today = self.clock.today();
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
            .and_then(|d| newest_mtime(d, today));

        let earnings = match &self.earn_dir {
            Some(dir) => earnings_store::read_days(dir, today, last).unwrap_or_else(|e| {
                self.error = Some(e);
                Vec::new()
            }),
            None => Vec::new(),
        };
        let wanted = self.fetch_dates();
        self.missing_days = wanted
            .iter()
            .any(|d| !earnings.iter().any(|f| f.date == *d));
        self.earnings_as_of = earnings.iter().map(|f| f.fetched_at).min();
        if self.econ_dir.is_none() || self.earn_dir.is_none() {
            self.error
                .get_or_insert_with(|| "couldn't find the wickd data folder".into());
        }

        self.rows = build(&Inputs {
            clock: self.clock,
            econ: &econ,
            earnings: &earnings,
            listed,
            show_all: self.show_all,
        });
        self.selected = keep
            .and_then(|k| self.rows.iter().position(|r| *r == k))
            .or_else(|| {
                self.nearest_selectable(self.selected.min(self.rows.len().saturating_sub(1)))
            })
            .unwrap_or(0);
    }

    /// Put the cursor on the next thing still to come (the first row that
    /// hasn't happened yet), or on the first row when everything has.
    pub fn go_next_upcoming(&mut self) {
        self.selected = (0..self.rows.len())
            .find(|i| self.rows[*i].selectable() && !self.rows[*i].past())
            .or_else(|| self.nearest_selectable(0))
            .unwrap_or(0);
    }

    /// Whether opening the screen should start a background fetch: a shown
    /// weekday has no file, or the oldest file is more than six hours old.
    /// Never while one is running, and not again within ten minutes of the
    /// last automatic one.
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

    /// The ticker under the cursor, when it's an earnings row.
    pub fn selected_symbol(&self) -> Option<&str> {
        match self.rows.get(self.selected) {
            Some(Row::Earn(e)) => Some(&e.symbol),
            _ => None,
        }
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
            // Moving up: bring the row's day heading (and the captions
            // under it) back with it when they fit on the page.
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

/// When the economic store's file for `day`'s month was last written.
fn newest_mtime(dir: &Path, day: NaiveDate) -> Option<SystemTime> {
    std::fs::metadata(dir.join(format!("{}.csv", day.format("%Y-%m"))))
        .and_then(|m| m.modified())
        .ok()
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
            // The store reaches Friday the 20th and no further.
            ev(
                "2026-11-20",
                "15:00",
                "USD",
                "Revised UoM Consumer Sentiment",
                "medium",
                "51.7",
            ),
        ]
    }

    fn earnings() -> Vec<DayFile> {
        vec![
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
                    "TINY",
                    Session::PreMarket,
                    Some(dec!(5000000)),
                )],
            ),
            day("2026-11-20", vec![]),
        ]
    }

    fn built(show_all: bool, listed: &[&str]) -> Vec<Row> {
        let listed: HashSet<String> = listed.iter().map(|s| s.to_string()).collect();
        build(&Inputs {
            // Tuesday evening in Denver is already Wednesday in UTC.
            clock: mst("2026-11-18T15:00:00Z"),
            econ: &econ(),
            earnings: &earnings(),
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
                    if e.listed { "*" } else { "" },
                    e.symbol,
                    if e.past { " (past)" } else { "" }
                ),
                Row::Note(n) => format!("~ {n}"),
                Row::Blank => String::new(),
            })
            .collect()
    }

    #[test]
    fn a_day_is_one_list_in_time_order_with_the_default_filter() {
        // 15:00 UTC Wednesday = 8:00 AM in Denver, after the 9:30 Eastern open.
        let got = text(&built(false, &["MINE"]));
        assert_eq!(
            got,
            [
                "# Today · Wed Nov 18",
                // Due before the open, largest first; already out.
                "pre-open TJX (past)",
                "pre-open TGT (past)",
                // The low-impact 6:30 release and the EUR one are filtered out.
                "12:00 PM high FOMC Meeting Minutes",
                // Due after the 4:00 PM Eastern close (2:00 PM here).
                "post-close PANW",
                // Tiny, but on a list: shown and marked.
                "post-close *MINE",
                // Stored under the 19th in UTC; shown on the 18th, local
                // time, after the close.
                "6:00 PM medium FOMC Member Speaks",
                "~ + 2 smaller companies · a shows everything",
                "",
                "# Tomorrow · Thu Nov 19",
                "6:30 AM medium Unemployment Claims",
                "~ No large company reports. 1 smaller company · a shows everything",
                "",
                "# Fri Nov 20",
                "8:00 AM medium Revised UoM Consumer Sentiment",
                "~ No company reports.",
                "",
                // The weekend is skipped: nothing on it.
                "# Mon Nov 23",
                "~ Economic events for this day aren't published yet (the feed covers the current week).",
                "~ No earnings stored for this day yet. Press r to fetch.",
                "",
                "# Tue Nov 24",
                "~ Economic events for this day aren't published yet (the feed covers the current week).",
                "~ No earnings stored for this day yet. Press r to fetch.",
            ]
        );
        // No caption rows: every row is a day, an event, a note or a gap.
        assert!(built(false, &[]).iter().all(|r| matches!(
            r,
            Row::Day(_) | Row::Econ(_) | Row::Earn(_) | Row::Note(_) | Row::Blank
        )));
    }

    #[test]
    fn everything_shows_low_impact_and_small_companies_and_marks_past_releases() {
        let got = text(&built(true, &[]));
        assert_eq!(
            &got[..11],
            [
                "# Today · Wed Nov 18",
                "pre-open TJX (past)",
                "pre-open TGT (past)",
                "pre-open SMOL (past)",
                "6:30 AM low Housing Starts (past)",
                "12:00 PM high FOMC Meeting Minutes",
                "post-close PANW",
                "post-close MINE",
                "6:00 PM medium FOMC Member Speaks",
                // No time given: last in the day.
                "no-time NOTIME",
                "",
            ]
        );
        // Still US only.
        assert!(!got.iter().any(|l| l.contains("Final CPI")));
        assert!(
            !got.iter().any(|l| l.starts_with("~ +")),
            "nothing is hidden"
        );
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
    fn in_the_evening_the_cursor_starts_on_the_next_thing_to_come() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        // 6:30 PM Mountain on the 18th: the minutes, the 6:00 PM speaker,
        // and all of today's timed reports are behind us.
        s.clock = mst("2026-11-19T01:30:00Z");
        s.show_all = true;
        s.reload(&HashSet::new());
        let got = text(&s.rows);
        for past in [
            "pre-open TJX (past)",
            "6:30 AM low Housing Starts (past)",
            "post-close PANW (past)",
            "6:00 PM medium FOMC Member Speaks (past)",
        ] {
            assert!(got.contains(&past.to_string()), "{past} in {got:?}");
        }
        // No time given: not counted as out until the day ends.
        assert!(got.contains(&"no-time NOTIME".to_string()));
        s.go_next_upcoming();
        assert_eq!(s.selected_symbol(), Some("NOTIME"));
        // Before the open nothing has happened yet: the first row.
        s.clock = mst("2026-11-18T12:00:00Z");
        s.reload(&HashSet::new());
        s.go_next_upcoming();
        assert_eq!(text(&s.rows[s.selected..=s.selected]), ["pre-open TJX"]);
        assert_eq!(s.selected, 1);
        // An empty screen is fine.
        let mut e = CalendarState::new(None, None);
        e.go_next_upcoming();
        assert_eq!(e.selected, 0);
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

    #[test]
    fn a_day_with_no_releases_inside_the_feed_says_so() {
        let listed = HashSet::new();
        let rows = build(&Inputs {
            clock: mst("2026-11-18T15:00:00Z"),
            // Only Friday has a release, so Wed and Thu are inside the feed
            // and empty.
            econ: &[ev("2026-11-20", "15:00", "USD", "Sentiment", "medium", "")],
            earnings: &[],
            listed: &listed,
            show_all: false,
        });
        let got = text(&rows);
        assert_eq!(got[1], "~ No US economic releases.");
        assert_eq!(
            got[2],
            "~ No earnings stored for this day yet. Press r to fetch."
        );
    }

    #[test]
    fn labels_and_values_read_like_a_person_wrote_them() {
        let today = d("2026-11-18");
        assert_eq!(day_label(today, today), "Today · Wed Nov 18");
        assert_eq!(day_label(d("2026-11-19"), today), "Tomorrow · Thu Nov 19");
        assert_eq!(day_label(d("2026-12-01"), today), "Tue Dec 1");
        assert_eq!(fmt_cap(dec!(3762974100000)), "$3.76T");
        assert_eq!(fmt_cap(dec!(72832852000)), "$72.8B");
        assert_eq!(fmt_cap(dec!(258701689000)), "$259B");
        assert_eq!(fmt_cap(dec!(475001472)), "$475M");
        assert_eq!(fmt_cap(dec!(950000)), "$950000");
    }

    /// A state reading two temp stores, at 8:00 AM MST on Wed Nov 18.
    fn state(tmp: &Path) -> CalendarState {
        let econ_dir = tmp.join("calendar");
        let earn_dir = tmp.join("earnings");
        std::fs::create_dir_all(&econ_dir).unwrap();
        calendar_store::merge_into_store(&econ_dir, econ()).unwrap();
        for f in earnings() {
            earnings_store::write_day(&earn_dir, &f).unwrap();
        }
        let mut s = CalendarState::new(Some(econ_dir), Some(earn_dir));
        s.clock = mst("2026-11-18T15:00:00Z");
        s
    }

    #[test]
    fn the_state_reads_both_stores_and_the_cursor_skips_headings() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        s.reload(&HashSet::new());
        assert!(s.error.is_none(), "{:?}", s.error);
        assert!(s.econ_as_of.is_some());
        assert_eq!(
            s.earnings_as_of,
            Some("2026-11-18T00:30:00Z".parse().unwrap())
        );
        // The cursor starts on the first event, not on the day heading.
        assert_eq!(s.selected, 1);
        assert_eq!(s.selected_symbol(), Some("TJX"));
        s.step(2); // TGT, then the Fed minutes
        assert_eq!(
            text(&s.rows[s.selected..=s.selected]),
            ["12:00 PM high FOMC Meeting Minutes"]
        );
        assert_eq!(s.selected_symbol(), None, "a release has no ticker");
        s.step(1);
        assert_eq!(s.selected_symbol(), Some("PANW"));
        s.step(-10);
        assert_eq!(s.selected, 1, "stops at the first row");
        // Down from the day's last event skips the note, the gap and the
        // next heading.
        s.step(4);
        assert_eq!(
            text(&s.rows[s.selected..=s.selected]),
            ["6:00 PM medium FOMC Member Speaks"]
        );
        s.step(1);
        assert_eq!(
            text(&s.rows[s.selected..=s.selected]),
            ["6:30 AM medium Unemployment Claims"]
        );
        s.go_end();
        assert_eq!(
            text(&s.rows[s.selected..=s.selected]),
            ["8:00 AM medium Revised UoM Consumer Sentiment"]
        );
        s.step(5);
        assert!(
            matches!(s.rows[s.selected], Row::Econ(_)),
            "stops at the last row"
        );
        s.go_top();
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn toggling_everything_keeps_the_cursor_on_the_same_company() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        s.reload(&HashSet::new());
        s.step(3); // TJX → TGT → minutes → PANW
        assert_eq!(s.selected_symbol(), Some("PANW"));
        s.show_all = true;
        s.reload(&HashSet::new());
        assert_eq!(s.selected_symbol(), Some("PANW"));
        s.show_all = false;
        s.reload(&HashSet::new());
        assert_eq!(s.selected_symbol(), Some("PANW"));
    }

    #[test]
    fn a_fetch_is_wanted_for_missing_or_old_days_but_not_twice_in_a_row() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        s.reload(&HashSet::new());
        // Mon and Tue of next week have no file.
        assert!(s.missing_days && s.needs_fetch());
        assert_eq!(
            s.fetch_dates()
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>(),
            [
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
        s.fetch_finished(Ok(vec![]), &HashSet::new());
        assert!(!s.loading && !s.missing_days && s.fetch_error.is_none());
        s.last_auto = None;
        assert!(!s.needs_fetch(), "all days stored and under six hours old");
        // Noon the same day: seven hours after the fetch.
        s.clock = mst("2026-11-18T19:00:00Z");
        assert!(s.needs_fetch(), "more than six hours old");
        s.fetch_started(true);
        s.fetch_finished(Err("offline".into()), &HashSet::new());
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
        s.fetch_finished(Ok(vec![ok, bad]), &HashSet::new());
        assert_eq!(
            s.fetch_error.as_deref(),
            Some("couldn't fetch earnings for 1 of 2 days (HTTP 503); showing what is stored")
        );
        assert!(!s.rows.is_empty(), "the stored days are still shown");
    }

    #[test]
    fn scrolling_keeps_the_cursor_on_screen_and_the_first_heading_visible() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = state(tmp.path());
        s.show_all = true;
        s.reload(&HashSet::new());
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
        );
        s.clock = mst("2026-11-18T15:00:00Z");
        s.reload(&HashSet::new());
        assert!(s.error.is_none());
        assert!(s.missing_days && s.needs_fetch());
        assert!(s.rows.iter().all(|r| !r.selectable()));
        s.step(1);
        s.go_end();
        assert_eq!(s.selected_symbol(), None);
        // No data home at all: said plainly, and no fetch is attempted.
        let mut s = CalendarState::new(None, None);
        s.clock = mst("2026-11-18T15:00:00Z");
        s.reload(&HashSet::new());
        assert_eq!(
            s.error.as_deref(),
            Some("couldn't find the wickd data folder")
        );
        assert!(!s.needs_fetch());
    }
}
