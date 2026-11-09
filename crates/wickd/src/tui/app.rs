//! `wickd tui` state machine — pure (no I/O, no terminal), so every key path
//! is unit-testable. Key presses mutate [`App`] and return [`Action`]s that
//! the run loop turns into fetches / saves.

use std::collections::HashMap;

use chrono::{DateTime, Local, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use wickd_core::equities::alpaca::NewsItem;
use wickd_core::equities::{Asset, Bar, HistoryRange, MarketClock, Snapshot};

use super::chart::{self, Candle, ChartView};
use super::inplay::{self, Row, SortBy};
use super::keymap::{self, Cmd, Ctx, Pending, Step};
use super::search;
use super::store::normalize_symbol;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Watchlist,
    Search,
    Detail,
    InPlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailView {
    Candles,
    Line,
    Table,
}

/// A row on the watchlist screen: a list entry (list index, entry index) or
/// a watchlist symbol (watchlist index).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenRow {
    List(usize, usize),
    Watch(usize),
}

/// Side effects requested by a key press.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Re-fetch quotes (and sparklines) now.
    RefreshQuotes,
    /// Fetch history for a symbol/range.
    FetchHistory(String, HistoryRange),
    /// Persist the watchlist.
    SaveWatchlist,
    /// Remove a symbol from a named list (`~/.wickd/lists.json`).
    RemoveFromList { list: String, symbol: String },
    /// Rebuild the in-play list.
    FetchInPlay,
    /// Show an article in the reader window (opening it if needed); `label`
    /// is the badge shown on the page.
    ReaderShow {
        url: String,
        label: String,
    },
    /// Close the reader window.
    ReaderClose,
    /// Flash a short message on the reader window's badge.
    ReaderNotice(String),
    /// Fetch recent headlines for one symbol.
    FetchNews(String),
    Quit,
}

/// The "in play today" screen.
#[derive(Debug)]
pub struct InPlayState {
    /// All rows, sorted by `sort` (filtering happens on read).
    pub rows: Vec<Row>,
    pub selected: usize,
    pub sort: SortBy,
    /// Minimum price filter (`None` = show everything).
    pub min_price: Option<f64>,
    pub loading: bool,
    /// Sources that failed (e.g. a screener not on this plan).
    pub notes: Vec<String>,
    pub error: Option<String>,
    pub updated: Option<DateTime<Local>>,
    /// Headlines per symbol: the side pane *and* the NEWS column read this.
    pub news: HashMap<String, Result<Vec<NewsItem>, String>>,
    /// When each symbol's headlines were fetched (for the cache TTL).
    pub news_at: HashMap<String, DateTime<Utc>>,
    /// After a failed fetch, don't retry a symbol before this time.
    pub news_retry_after: HashMap<String, DateTime<Utc>>,
    pub news_pending: Vec<String>,
    /// `tab` moved focus into the headlines pane.
    pub focus_news: bool,
    /// Selected headline (index into the selected symbol's headlines).
    pub news_cursor: usize,
    /// The reader window is open (it follows the headline cursor).
    pub reader_open: bool,
    /// What the reader window is paging through: pinned when an article is
    /// opened, so list rebuilds and selection changes can't move it.
    pub reader_pin: Option<ReaderPin>,
}

/// Rows a J/K (shift) step moves in a list.
pub const LIST_PAGE: usize = 10;
/// Headlines a J/K step moves in the headlines pane.
pub const NEWS_PAGE: usize = 3;

/// Rows (from the top of the visible list) that get a NEWS age.
pub const NEWS_ROWS: usize = 60;
/// How long fetched headlines are trusted before a refetch.
pub const NEWS_TTL_MINUTES: i64 = 10;
/// Backoff before retrying a symbol whose headline fetch failed.
pub const NEWS_RETRY_SECONDS: i64 = 60;

impl Default for InPlayState {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            selected: 0,
            sort: SortBy::Change,
            min_price: Some(inplay::DEFAULT_MIN_PRICE),
            loading: false,
            notes: Vec::new(),
            error: None,
            updated: None,
            news: HashMap::new(),
            news_at: HashMap::new(),
            news_retry_after: HashMap::new(),
            news_pending: Vec::new(),
            focus_news: false,
            news_cursor: 0,
            reader_open: false,
            reader_pin: None,
        }
    }
}

impl InPlayState {
    pub fn visible(&self) -> Vec<&Row> {
        inplay::visible(&self.rows, self.min_price)
    }

    pub fn selected_symbol(&self) -> Option<String> {
        self.visible().get(self.selected).map(|r| r.symbol.clone())
    }

    /// Headlines loaded for the selected symbol.
    pub fn selected_headlines(&self) -> &[NewsItem] {
        match self.selected_symbol().and_then(|s| self.news.get(&s)) {
            Some(Ok(items)) => items,
            _ => &[],
        }
    }

    /// Pin the reader to the selected symbol's headlines at the cursor and
    /// return the `ReaderShow` for it (if that headline has a link).
    pub fn pin_cursor(&mut self) -> Option<Action> {
        let symbol = self.selected_symbol()?;
        let items = self.selected_headlines().to_vec();
        self.cursor_url()?;
        self.reader_pin = Some(ReaderPin {
            symbol,
            items,
            index: self.news_cursor,
        });
        self.reader_pin.as_ref().and_then(ReaderPin::show)
    }

    /// Step the pinned reader to the next/previous *linked* headline. Keeps
    /// the TUI's headline cursor in sync when it's on the same symbol.
    pub fn step_pin(&mut self, forward: bool) -> Result<Action, String> {
        let selected = self.selected_symbol();
        let Some(pin) = self.reader_pin.as_mut() else {
            return Err("no article open".into());
        };
        if pin.items.is_empty() {
            return Err("headlines unavailable".into());
        }
        let Some(i) = pin.next_linked(forward) else {
            return Err(if forward {
                "last linked headline".into()
            } else {
                "first linked headline".into()
            });
        };
        pin.index = i;
        if selected.as_deref() == Some(pin.symbol.as_str()) {
            self.news_cursor = i;
        }
        pin.show().ok_or_else(|| "this headline has no link".into())
    }

    /// Link of the headline under the news cursor.
    pub fn cursor_url(&self) -> Option<String> {
        self.selected_headlines()
            .get(self.news_cursor)
            .and_then(|n| n.url.clone())
            .filter(|u| !u.trim().is_empty())
    }
}

/// `g f`: filter the current list by ticker or company name; `enter` jumps
/// the real cursor to the picked line, `esc` restores it.
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub query: String,
    /// Index into the current matches.
    pub selected: usize,
    /// The list cursor before `g f`, restored on `esc`.
    pub saved: usize,
}

/// The reader's own copy of the headlines it pages through.
#[derive(Debug, Clone)]
pub struct ReaderPin {
    pub symbol: String,
    pub items: Vec<NewsItem>,
    pub index: usize,
}

impl ReaderPin {
    fn url(&self, i: usize) -> Option<String> {
        self.items
            .get(i)
            .and_then(|n| n.url.clone())
            .filter(|u| !u.trim().is_empty())
    }

    pub fn label(&self) -> String {
        format!(
            "{} · {} of {} · ctrl+shift+← prev · ctrl+shift+→ next",
            self.symbol,
            self.index + 1,
            self.items.len()
        )
    }

    pub fn show(&self) -> Option<Action> {
        self.url(self.index).map(|url| Action::ReaderShow {
            url,
            label: self.label(),
        })
    }

    /// Index of the next/previous headline with a link.
    fn next_linked(&self, forward: bool) -> Option<usize> {
        let n = self.items.len();
        let mut i = self.index;
        loop {
            if forward {
                if i + 1 >= n {
                    return None;
                }
                i += 1;
            } else {
                if i == 0 {
                    return None;
                }
                i -= 1;
            }
            if self.url(i).is_some() {
                return Some(i);
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct SearchState {
    pub query: String,
    /// Indices into [`App::assets`].
    pub results: Vec<usize>,
    pub selected: usize,
}

#[derive(Debug)]
pub struct DetailState {
    pub symbol: String,
    pub range: HistoryRange,
    pub view: DetailView,
    /// Chart style `tab` returns to from the table.
    pub last_chart: DetailView,
    /// Zoom / pan / cursor / session state for the chart.
    pub chart: ChartView,
    /// Loaded history per range (kept so flipping back is instant).
    pub history: HashMap<HistoryRange, Vec<Bar>>,
    /// Bars *before* the visible window, same timeframe (MA warm-up on daily
    /// ranges, Ichimoku warm-up on all), per range; never drawn as candles.
    pub warmup: HashMap<HistoryRange, Vec<Bar>>,
    /// Ranges with a request in flight.
    pub loading: Vec<HistoryRange>,
    pub error: Option<String>,
    /// Table scroll offset (rows from the newest bar).
    pub scroll: usize,
    /// Where Esc returns to.
    pub back_to: Mode,
}

impl DetailState {
    fn new(symbol: String, back_to: Mode) -> Self {
        Self {
            symbol,
            range: HistoryRange::Day1,
            view: DetailView::Candles,
            last_chart: DetailView::Candles,
            chart: ChartView::default(),
            history: HashMap::new(),
            warmup: HashMap::new(),
            loading: Vec::new(),
            error: None,
            scroll: 0,
            back_to,
        }
    }

    /// Whether the chart shows all bars rather than US regular hours: the
    /// `v e` toggle for stocks; always for forex pairs (they trade 24/5).
    pub fn show_extended(&self) -> bool {
        self.chart.extended || super::fx::is_fx(&self.symbol)
    }

    pub fn bars(&self) -> Option<&Vec<Bar>> {
        self.history.get(&self.range)
    }

    /// Candles the chart draws for the current range, zoom and session
    /// setting, plus whether extended hours are included.
    pub fn candles(&self) -> Option<(Vec<Candle>, bool)> {
        let bars = self.bars()?;
        let (bars, extended) = chart::session_bars(bars, self.range, self.show_extended());
        Some((
            chart::build(&bars, self.chart.merge_for(bars.len())),
            extended,
        ))
    }

    pub fn is_loading(&self) -> bool {
        self.loading.contains(&self.range)
    }
}

/// Which way each displayed price last moved, so it can show green after an
/// uptick and red after a downtick (and keep that colour until it changes
/// again). Keyed `"<symbol>:<field>"`, e.g. `SPY:last`, `EUR_USD:bid`.
#[derive(Debug, Default)]
pub struct TickDirs(HashMap<String, (f64, i8)>);

impl TickDirs {
    /// Record a price; a change from the previous value sets the direction.
    pub fn observe(&mut self, key: String, price: f64) {
        match self.0.get_mut(&key) {
            Some((prev, dir)) => {
                if price > *prev {
                    *dir = 1;
                } else if price < *prev {
                    *dir = -1;
                }
                *prev = price;
            }
            None => {
                self.0.insert(key, (price, 0));
            }
        }
    }

    /// +1 after an uptick, -1 after a downtick, 0 before the first change.
    pub fn dir(&self, key: &str) -> i8 {
        self.0.get(key).map_or(0, |(_, d)| *d)
    }
}

#[derive(Debug)]
pub struct App {
    pub watchlist: Vec<String>,
    /// Named lists (`wickd list`), shown as sections above the watchlist.
    /// Rows count lists first, then the watchlist: `selected` indexes that
    /// combined order (see [`App::row`]).
    pub lists: Vec<(String, Vec<crate::lists::Entry>)>,
    pub selected: usize,
    pub snapshots: HashMap<String, Snapshot>,
    /// Forex pair quotes (OANDA), keyed like `snapshots`.
    pub fx_quotes: HashMap<String, super::fx::FxQuote>,
    /// Up/down colouring for every price that updates.
    pub ticks: TickDirs,
    /// Today's closes per symbol, for the watchlist sparkline.
    pub sparks: HashMap<String, Vec<f64>>,
    pub assets: Vec<Asset>,
    /// symbol → index into `assets`.
    asset_index: HashMap<String, usize>,
    pub assets_error: Option<String>,
    pub clock: Option<MarketClock>,
    pub mode: Mode,
    pub show_help: bool,
    /// A Helix-style prefix waiting for its second key (`g`, `w`, `t`, `v`).
    pub pending: Option<Pending>,
    pub search: SearchState,
    pub detail: Option<DetailState>,
    /// Active `g f` filter on the watchlist / in-play list.
    pub find: Option<FindState>,
    pub inplay: InPlayState,
    pub quotes_error: Option<String>,
    /// Last OANDA (forex) failure, shown on the forex section.
    pub fx_error: Option<String>,
    /// Attached to the stream daemon: forex rows tick live.
    pub fx_live: bool,
    /// Quote refresh interval (`--refresh`), shown when not streaming.
    pub refresh_secs: u64,
    pub last_quotes: Option<DateTime<Local>>,
    /// Transient one-line message for the footer.
    pub flash: Option<String>,
    /// Last removed watchlist entry, for `u`ndo.
    undo: Option<(usize, String)>,
    pub feed_label: &'static str,
    /// Live Alpaca market-data budget (shared with the client).
    pub api_budget: Option<wickd_core::equities::alpaca::RateLimitCell>,
    /// History runs ~15 min behind (any plan without real-time SIP).
    pub history_delayed: bool,
}

impl App {
    pub fn new(watchlist: Vec<String>, feed_label: &'static str) -> Self {
        Self {
            watchlist,
            lists: Vec::new(),
            selected: 0,
            snapshots: HashMap::new(),
            fx_quotes: HashMap::new(),
            ticks: TickDirs::default(),
            sparks: HashMap::new(),
            assets: Vec::new(),
            asset_index: HashMap::new(),
            assets_error: None,
            clock: None,
            mode: Mode::Watchlist,
            show_help: false,
            pending: None,
            search: SearchState::default(),
            detail: None,
            find: None,
            inplay: InPlayState::default(),
            quotes_error: None,
            fx_error: None,
            fx_live: false,
            refresh_secs: 15,
            last_quotes: None,
            flash: None,
            undo: None,
            feed_label,
            history_delayed: true,
            api_budget: None,
        }
    }

    // ── data updates ────────────────────────────────────────────────────────

    /// Replace the stock search entries (forex entries are kept).
    pub fn set_assets(&mut self, assets: Vec<Asset>) {
        self.merge_assets(false, assets);
    }

    /// Replace the forex search entries (stock entries are kept).
    pub fn set_fx_assets(&mut self, assets: Vec<Asset>) {
        self.merge_assets(true, assets);
    }

    fn merge_assets(&mut self, fx: bool, incoming: Vec<Asset>) {
        let is_fx = |a: &Asset| a.exchange == super::fx::FX_EXCHANGE;
        let mut assets: Vec<Asset> = std::mem::take(&mut self.assets)
            .into_iter()
            .filter(|a| is_fx(a) != fx)
            .collect();
        assets.extend(incoming);
        self.asset_index = assets
            .iter()
            .enumerate()
            .map(|(i, a)| (a.symbol.clone(), i))
            .collect();
        self.assets = assets;
        self.assets_error = None;
        if self.mode == Mode::Search {
            self.run_search();
        }
    }

    pub fn asset(&self, symbol: &str) -> Option<&Asset> {
        self.asset_index.get(symbol).map(|&i| &self.assets[i])
    }

    /// Warm-up bars for a range arrived (with its history).
    pub fn set_warmup(&mut self, symbol: &str, range: HistoryRange, bars: Vec<Bar>) {
        if let Some(d) = self.detail.as_mut().filter(|d| d.symbol == symbol) {
            d.warmup.insert(range, bars);
        }
    }

    pub fn set_history(
        &mut self,
        symbol: &str,
        range: HistoryRange,
        result: Result<Vec<Bar>, String>,
    ) {
        let Some(d) = self.detail.as_mut().filter(|d| d.symbol == symbol) else {
            return; // stale reply for a symbol we've left
        };
        d.loading.retain(|r| *r != range);
        match result {
            Ok(bars) => {
                d.history.insert(range, bars);
                if d.range == range {
                    d.error = None;
                }
            }
            Err(e) => {
                if d.range == range {
                    d.error = Some(e);
                }
            }
        }
    }

    /// Symbols whose quotes the run loop should poll: the watchlist plus the
    /// symbol open in the detail view (which may not be on the list).
    pub fn quote_symbols(&self) -> Vec<String> {
        let mut syms = self.watchlist.clone();
        for (_, entries) in &self.lists {
            for e in entries {
                if !syms.contains(&e.symbol) {
                    syms.push(e.symbol.clone());
                }
            }
        }
        if let Some(d) = &self.detail {
            if !syms.contains(&d.symbol) {
                syms.push(d.symbol.clone());
            }
        }
        syms
    }

    pub fn selected_symbol(&self) -> Option<&str> {
        self.row_symbol(self.selected)
    }

    /// Rows taken by the list sections (they come before the watchlist).
    pub fn list_rows(&self) -> usize {
        self.lists.iter().map(|(_, e)| e.len()).sum()
    }

    /// Every row on the watchlist screen: list entries, then the watchlist.
    pub fn total_rows(&self) -> usize {
        self.list_rows() + self.watchlist.len()
    }

    /// What a row is.
    pub fn row(&self, r: usize) -> Option<ScreenRow> {
        let mut base = 0;
        for (li, (_, entries)) in self.lists.iter().enumerate() {
            if r < base + entries.len() {
                return Some(ScreenRow::List(li, r - base));
            }
            base += entries.len();
        }
        (r - base < self.watchlist.len()).then_some(ScreenRow::Watch(r - base))
    }

    pub fn row_symbol(&self, r: usize) -> Option<&str> {
        match self.row(r)? {
            ScreenRow::List(li, ei) => Some(self.lists[li].1[ei].symbol.as_str()),
            ScreenRow::Watch(i) => Some(self.watchlist[i].as_str()),
        }
    }

    /// The watchlist index under the cursor (None on a list row).
    fn watch_selected(&self) -> Option<usize> {
        match self.row(self.selected) {
            Some(ScreenRow::Watch(i)) => Some(i),
            _ => None,
        }
    }

    /// Replace the named lists (a `lists.json` reload), keeping the cursor on
    /// the same symbol in the same kind of section when it's still there.
    pub fn set_lists(&mut self, file: crate::lists::ListsFile) {
        let keep = self.row(self.selected).map(|r| {
            let sym = self.row_symbol(self.selected).unwrap_or_default().to_string();
            let list = match r {
                ScreenRow::List(li, _) => Some(self.lists[li].0.clone()),
                ScreenRow::Watch(_) => None,
            };
            (list, sym)
        });
        self.lists = file.lists.into_iter().collect();
        let found = keep.and_then(|(list, sym)| {
            (0..self.total_rows()).find(|&r| {
                self.row_symbol(r) == Some(sym.as_str())
                    && match (self.row(r), &list) {
                        (Some(ScreenRow::List(li, _)), Some(name)) => self.lists[li].0 == *name,
                        (Some(ScreenRow::Watch(_)), None) => true,
                        _ => false,
                    }
            })
        });
        self.selected = found.unwrap_or(self.selected).min(self.total_rows().saturating_sub(1));
    }

    /// A row's list note, if it's a list row.
    pub fn row_note(&self, r: usize) -> Option<&str> {
        match self.row(r)? {
            ScreenRow::List(li, ei) => self.lists[li].1[ei].note.as_deref(),
            ScreenRow::Watch(_) => None,
        }
    }

    /// Apply a stock snapshot refresh (and note each price's direction).
    pub fn set_snapshots(&mut self, map: HashMap<String, Snapshot>) {
        for (sym, s) in &map {
            if let Some(p) = s.last() {
                self.ticks.observe(format!("{sym}:last"), p);
            }
            if let Some(q) = s.latest_quote.as_ref().filter(|q| q.bid > 0.0 && q.ask > 0.0) {
                self.ticks.observe(format!("{sym}:bid"), q.bid);
                self.ticks.observe(format!("{sym}:ask"), q.ask);
            }
        }
        self.snapshots.extend(map);
    }

    /// Note a pair's displayed prices (bid, ask, mid) for up/down colouring.
    fn observe_fx(&mut self, sym: &str) {
        use rust_decimal::prelude::ToPrimitive;
        if let Some(q) = self.fx_quotes.get(sym) {
            let vals = [("bid", q.bid), ("ask", q.ask), ("mid", q.mid())];
            for (field, v) in vals {
                if let Some(v) = v.to_f64() {
                    self.ticks.observe(format!("{sym}:{field}"), v);
                }
            }
        }
    }

    /// Merge a REST quote refresh: the day stats always update, but bid/ask
    /// only when newer than what live ticks already showed.
    pub fn set_fx_quotes(&mut self, quotes: HashMap<String, super::fx::FxQuote>) {
        for (sym, q) in quotes {
            match self.fx_quotes.get_mut(&sym) {
                Some(cur) if cur.time > q.time => {
                    let (bid, ask, time) = (cur.bid, cur.ask, cur.time);
                    *cur = super::fx::FxQuote { bid, ask, time, ..q };
                }
                _ => {
                    self.fx_quotes.insert(sym.clone(), q);
                }
            }
            self.observe_fx(&sym);
        }
    }

    /// A live tick from the stream daemon: update the pair's row and, if its
    /// chart is open, the newest bar of every loaded range.
    pub fn apply_tick(&mut self, q: wickd_core::hub_client::HubQuote) {
        use rust_decimal::prelude::ToPrimitive;
        let mid = (q.bid + q.ask) / rust_decimal::Decimal::TWO;
        match self.fx_quotes.get_mut(&q.instrument) {
            Some(cur) if q.time < cur.time => return,
            Some(cur) => {
                cur.bid = q.bid;
                cur.ask = q.ask;
                cur.time = q.time;
                cur.high = cur.high.map(|h| h.max(mid));
                cur.low = cur.low.map(|l| l.min(mid));
            }
            None => {
                self.fx_quotes.insert(
                    q.instrument.clone(),
                    super::fx::FxQuote {
                        bid: q.bid,
                        ask: q.ask,
                        time: q.time,
                        open: None,
                        high: None,
                        low: None,
                        closes: Vec::new(),
                    },
                );
            }
        }
        self.observe_fx(&q.instrument);
        let Some(price) = mid.to_f64() else {
            return; // unrepresentable: skip rather than plot a bogus bar
        };
        if let Some(d) = self.detail.as_mut().filter(|d| d.symbol == q.instrument) {
            for (range, bars) in d.history.iter_mut() {
                super::fx::apply_tick(bars, price, q.time, super::fx::bar_seconds(*range));
            }
        }
    }

    /// Index of the first forex pair (the watchlist is stocks, then pairs);
    /// `watchlist.len()` when there are none.
    pub fn fx_start(&self) -> usize {
        self.watchlist
            .iter()
            .position(|s| super::fx::is_fx(s))
            .unwrap_or(self.watchlist.len())
    }

    /// Forex pairs to quote (watchlist + an open pair's detail).
    pub fn fx_symbols(&self) -> Vec<String> {
        self.quote_symbols()
            .into_iter()
            .filter(|s| super::fx::is_fx(s))
            .collect()
    }

    /// Where a symbol belongs when (re)inserted near `idx`: inside its own
    /// section.
    fn section_index(&self, sym: &str, idx: usize) -> usize {
        let start = self.fx_start();
        if super::fx::is_fx(sym) {
            idx.clamp(start, self.watchlist.len())
        } else {
            idx.min(start)
        }
    }

    pub fn in_watchlist(&self, symbol: &str) -> bool {
        self.watchlist.iter().any(|s| s == symbol)
    }

    // ── key handling ────────────────────────────────────────────────────────

    /// Keymap context for the current screen. `None` in search, whose text
    /// box takes raw keystrokes instead.
    pub fn ctx(&self) -> Option<Ctx> {
        match self.mode {
            Mode::Watchlist => Some(Ctx::Watchlist),
            Mode::Detail => Some(Ctx::Detail),
            Mode::InPlay => Some(Ctx::InPlay),
            Mode::Search => None,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Vec<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return vec![Action::Quit];
        }
        if self.show_help {
            // Any key closes help.
            self.show_help = false;
            return vec![];
        }
        self.flash = None;
        if self.find.is_some() {
            return self.on_find_key(key);
        }
        let Some(ctx) = self.ctx() else {
            return self.on_search_key(key);
        };
        let Some(tok) = keymap::token(&key) else {
            return vec![];
        };
        let (pending, step) = keymap::step(ctx, self.pending.as_ref(), tok);
        self.pending = pending;
        match step {
            Step::Act(cmd, n) => match ctx {
                Ctx::Watchlist => self.exec_watchlist(cmd, n),
                Ctx::Detail => self.exec_detail(cmd),
                Ctx::InPlay => self.exec_inplay(cmd),
            },
            Step::Pending | Step::Cancel | Step::None => vec![],
        }
    }

    fn exec_watchlist(&mut self, cmd: Cmd, n: Option<usize>) -> Vec<Action> {
        let len = self.total_rows();
        let lr = self.list_rows();
        // Watchlist edits act on the watchlist index under the cursor.
        let ws = self.watch_selected();
        match cmd {
            Cmd::Quit => return vec![Action::Quit],
            Cmd::Help => self.show_help = true,
            Cmd::Down if len > 0 => self.selected = (self.selected + 1).min(len - 1),
            Cmd::Up => self.selected = self.selected.saturating_sub(1),
            Cmd::PageDown if len > 0 => self.selected = (self.selected + LIST_PAGE).min(len - 1),
            Cmd::PageUp => self.selected = self.selected.saturating_sub(LIST_PAGE),
            Cmd::GoTop => self.selected = 0,
            Cmd::GoEnd if len > 0 => self.selected = len - 1,
            Cmd::GoRow => {
                if let Some(row) = n.filter(|r| *r >= 1) {
                    if len > 0 {
                        self.selected = (row - 1).min(len - 1);
                    }
                }
            }
            Cmd::Search => self.open_search(),
            Cmd::Find => self.open_find(),
            Cmd::GoInPlay => return self.open_inplay(),
            Cmd::GoLists => {
                if lr > 0 {
                    self.find = None;
                    self.selected = 0;
                } else {
                    self.flash = Some("no lists yet — `wickd list add <name> SYM…`".into());
                }
            }
            Cmd::GoStocks if self.fx_start() > 0 => {
                self.find = None;
                self.selected = lr;
            }
            Cmd::GoForex => {
                let start = self.fx_start();
                if start < self.watchlist.len() {
                    self.find = None;
                    self.selected = lr + start;
                } else {
                    self.flash = Some("no forex pairs — / to search, tab to add".into());
                }
            }
            Cmd::WatchAdd => {
                self.open_search();
                self.flash = Some("find a symbol, then tab to add it".into());
            }
            Cmd::Refresh => return vec![Action::RefreshQuotes],
            Cmd::Open => {
                if let Some(sym) = self.selected_symbol().map(str::to_string) {
                    return self.open_detail(sym, Mode::Watchlist);
                }
            }
            // Moves stay inside a section (stocks above, pairs below).
            Cmd::WatchMoveDown
                if ws.is_some_and(|w| {
                    w + 1 < self.watchlist.len()
                        && super::fx::is_fx(&self.watchlist[w])
                            == super::fx::is_fx(&self.watchlist[w + 1])
                }) =>
            {
                let w = ws.unwrap_or_default();
                self.watchlist.swap(w, w + 1);
                self.selected += 1;
                return vec![Action::SaveWatchlist];
            }
            Cmd::WatchMoveUp
                if ws.is_some_and(|w| {
                    w > 0
                        && super::fx::is_fx(&self.watchlist[w])
                            == super::fx::is_fx(&self.watchlist[w - 1])
                }) =>
            {
                let w = ws.unwrap_or_default();
                self.watchlist.swap(w, w - 1);
                self.selected -= 1;
                return vec![Action::SaveWatchlist];
            }
            Cmd::WatchRemove if len > 0 => {
                if let Some(ScreenRow::List(li, ei)) = self.row(self.selected) {
                    // Optimistic: drop it here; the file write follows.
                    let (list, entries) = &mut self.lists[li];
                    let symbol = entries.remove(ei).symbol;
                    let list = list.clone();
                    self.flash = Some(format!("removed {symbol} from {list}"));
                    self.selected = self.selected.min(self.total_rows().saturating_sub(1));
                    return vec![Action::RemoveFromList { list, symbol }];
                }
                let Some(w) = ws else { return vec![] };
                let sym = self.watchlist.remove(w);
                self.flash = Some(format!("removed {sym} — w u to undo"));
                self.undo = Some((w, sym));
                self.selected = self.selected.min(self.total_rows().saturating_sub(1));
                return vec![Action::SaveWatchlist];
            }
            Cmd::WatchUndo => {
                if let Some((idx, sym)) = self.undo.take() {
                    if !self.in_watchlist(&sym) {
                        let idx = self.section_index(&sym, idx);
                        self.watchlist.insert(idx, sym.clone());
                        self.selected = lr + idx;
                        self.flash = Some(format!("restored {sym}"));
                        return vec![Action::SaveWatchlist, Action::RefreshQuotes];
                    }
                }
            }
            _ => {}
        }
        vec![]
    }

    fn open_inplay(&mut self) -> Vec<Action> {
        self.mode = Mode::InPlay;
        let mut out = Vec::new();
        if !self.inplay.loading {
            self.inplay.loading = true;
            out.push(Action::FetchInPlay);
        }
        out.extend(self.request_news(Utc::now()));
        out
    }

    /// Ask for headlines the list is missing: the selected row first, then
    /// the top [`NEWS_ROWS`] visible rows, skipping anything in flight or
    /// fetched within [`NEWS_TTL_MINUTES`].
    pub fn request_news(&mut self, now: DateTime<Utc>) -> Vec<Action> {
        let ip = &mut self.inplay;
        let vis: Vec<String> = ip.visible().iter().map(|r| r.symbol.clone()).collect();
        let mut wanted: Vec<String> = ip.selected_symbol().into_iter().collect();
        for s in vis.into_iter().take(NEWS_ROWS) {
            if !wanted.contains(&s) {
                wanted.push(s);
            }
        }
        let ttl = chrono::Duration::minutes(NEWS_TTL_MINUTES);
        let mut out = Vec::new();
        for sym in wanted {
            let fresh = ip.news_at.get(&sym).is_some_and(|t| now - *t < ttl);
            let backing_off = ip.news_retry_after.get(&sym).is_some_and(|t| now < *t);
            if fresh || backing_off || ip.news_pending.contains(&sym) {
                continue;
            }
            ip.news_pending.push(sym.clone());
            out.push(Action::FetchNews(sym));
        }
        out
    }

    /// Copy cached headlines onto the rows' NEWS columns (rows are rebuilt
    /// from scratch on every refresh, so this has to be re-applied).
    fn apply_news_cache(&mut self) {
        let ip = &mut self.inplay;
        for row in ip.rows.iter_mut() {
            if let Some(Ok(items)) = ip.news.get(&row.symbol) {
                row.latest_news = items.iter().map(|n| n.created_at).max();
                row.news_count = items.len();
            }
        }
    }

    /// A rebuilt in-play list arrived.
    pub fn set_inplay(&mut self, result: Result<(Vec<Row>, Vec<String>), String>) -> Vec<Action> {
        self.inplay.loading = false;
        match result {
            Ok((mut rows, notes)) => {
                for r in &rows {
                    if let Some(p) = r.last {
                        self.ticks.observe(format!("{}:last", r.symbol), p);
                    }
                }
                let ip = &mut self.inplay;
                // Keep the cursor on the same symbol across refreshes.
                let keep = ip.selected_symbol();
                inplay::sort_rows(&mut rows, ip.sort);
                ip.rows = rows;
                ip.notes = notes;
                ip.error = None;
                ip.updated = Some(Local::now());
                let vis = ip.visible();
                ip.selected = keep
                    .and_then(|k| vis.iter().position(|r| r.symbol == k))
                    .unwrap_or(0)
                    .min(vis.len().saturating_sub(1));
            }
            Err(e) => self.inplay.error = Some(e),
        }
        self.apply_news_cache();
        self.request_news(Utc::now())
    }

    /// The reader window opened, failed, closed, or asked (ctrl+shift+→/←
    /// pressed in the window) for the next/previous article.
    pub fn set_reader(&mut self, event: super::reader::ReaderEvent) -> Vec<Action> {
        use super::reader::{NavDir, ReaderEvent};
        match event {
            ReaderEvent::Opened => self.inplay.reader_open = true,
            ReaderEvent::Error(e) => self.flash = Some(format!("reader: {e}")),
            ReaderEvent::Closed => self.inplay.reader_open = false,
            ReaderEvent::Nav(dir) => {
                // Feedback goes to the window's badge — that's where you are.
                return vec![match self.inplay.step_pin(dir == NavDir::Next) {
                    Ok(show) => show,
                    Err(msg) => Action::ReaderNotice(msg),
                }];
            }
        }
        vec![]
    }

    /// Headline-pane keys (`tab` focus): move the headline cursor, open the
    /// article, or hand focus back to the list. `None` = not handled here.
    fn exec_headlines(&mut self, cmd: Cmd) -> Option<Vec<Action>> {
        let n = self.inplay.selected_headlines().len();
        let before = self.inplay.news_cursor;
        match cmd {
            Cmd::FocusNews | Cmd::Back | Cmd::FocusLeft => {
                self.inplay.focus_news = false;
                return Some(vec![]);
            }
            Cmd::FocusRight => return Some(vec![]),
            Cmd::Down => self.inplay.news_cursor = (before + 1).min(n.saturating_sub(1)),
            Cmd::Up => self.inplay.news_cursor = before.saturating_sub(1),
            Cmd::PageDown => {
                self.inplay.news_cursor = (before + NEWS_PAGE).min(n.saturating_sub(1))
            }
            Cmd::PageUp => self.inplay.news_cursor = before.saturating_sub(NEWS_PAGE),
            Cmd::GoTop => self.inplay.news_cursor = 0,
            Cmd::GoEnd => self.inplay.news_cursor = n.saturating_sub(1),
            Cmd::Open => {
                return Some(match self.inplay.pin_cursor() {
                    Some(a) => vec![a],
                    None => {
                        self.flash = Some("this headline has no link".into());
                        vec![]
                    }
                });
            }
            _ => return None,
        }
        // Moving the cursor while the reader is open loads that article.
        // Moving the cursor in the TUI while the reader is open re-pins it
        // to what you picked.
        if self.inplay.reader_open && self.inplay.news_cursor != before {
            if let Some(a) = self.inplay.pin_cursor() {
                return Some(vec![a]);
            }
        }
        Some(vec![])
    }

    pub fn set_news(&mut self, symbol: String, result: Result<Vec<NewsItem>, String>) {
        self.inplay.news_pending.retain(|s| *s != symbol);
        // Successes are fresh for the TTL. A failure (e.g. a rate-limit hit)
        // is shown in the pane and retried after a short backoff — never on
        // every keypress, which would keep a 429 burst going.
        let now = Utc::now();
        if result.is_ok() {
            self.inplay.news_at.insert(symbol.clone(), now);
            self.inplay.news_retry_after.remove(&symbol);
        } else {
            self.inplay.news_at.remove(&symbol);
            self.inplay.news_retry_after.insert(
                symbol.clone(),
                now + chrono::Duration::seconds(NEWS_RETRY_SECONDS),
            );
        }
        // A failed refresh keeps the headlines we already had: wiping them
        // would leave the pane, the NEWS column and the reader with nothing.
        let had_good = matches!(self.inplay.news.get(&symbol), Some(Ok(_)));
        if result.is_ok() || !had_good {
            self.inplay.news.insert(symbol, result);
        }
        self.apply_news_cache();
    }

    fn exec_inplay(&mut self, cmd: Cmd) -> Vec<Action> {
        if self.inplay.focus_news {
            if let Some(actions) = self.exec_headlines(cmd) {
                return actions;
            }
        }
        if cmd == Cmd::CloseReader {
            return if self.inplay.reader_open {
                vec![Action::ReaderClose]
            } else {
                vec![]
            };
        }
        if cmd == Cmd::FocusLeft {
            return vec![];
        }
        if cmd == Cmd::FocusNews || cmd == Cmd::FocusRight {
            if self.inplay.selected_headlines().is_empty() {
                self.flash = Some("no headlines for this symbol yet".into());
            } else {
                self.inplay.focus_news = true;
                self.inplay.news_cursor = 0;
            }
            return vec![];
        }
        let before_sel = self.inplay.selected;
        let actions = self.exec_inplay_list(cmd);
        if self.inplay.selected != before_sel {
            self.inplay.news_cursor = 0;
        }
        actions
    }

    fn exec_inplay_list(&mut self, cmd: Cmd) -> Vec<Action> {
        let len = self.inplay.visible().len();
        let ip_sel = self.inplay.selected;
        let resort = |app: &mut App, by: SortBy| {
            app.inplay.sort = by;
            inplay::sort_rows(&mut app.inplay.rows, by);
            app.inplay.selected = 0;
        };
        match cmd {
            Cmd::Find => self.open_find(),
            Cmd::Back | Cmd::GoWatchlist => self.mode = Mode::Watchlist,
            Cmd::Help => self.show_help = true,
            Cmd::Search => self.open_search(),
            Cmd::Down if len > 0 => self.inplay.selected = (ip_sel + 1).min(len - 1),
            Cmd::Up => self.inplay.selected = ip_sel.saturating_sub(1),
            Cmd::PageDown if len > 0 => self.inplay.selected = (ip_sel + LIST_PAGE).min(len - 1),
            Cmd::PageUp => self.inplay.selected = ip_sel.saturating_sub(LIST_PAGE),
            Cmd::GoTop => self.inplay.selected = 0,
            Cmd::GoEnd if len > 0 => self.inplay.selected = len - 1,
            Cmd::Refresh => {
                // Rebuild the list; refetch only the selected symbol's
                // headlines (the rest keep their 10-minute cache) so `r`
                // can't burn the API budget.
                if let Some(sym) = self.inplay.selected_symbol() {
                    self.inplay.news_at.remove(&sym);
                    self.inplay.news_retry_after.remove(&sym);
                }
                if !self.inplay.loading {
                    self.inplay.loading = true;
                    return vec![Action::FetchInPlay];
                }
            }
            Cmd::Open => {
                if let Some(sym) = self.inplay.selected_symbol() {
                    return self.open_detail(sym, Mode::InPlay);
                }
            }
            Cmd::WatchAdd => {
                if let Some(sym) = self.inplay.selected_symbol() {
                    if self.in_watchlist(&sym) {
                        self.flash = Some(format!("{sym} is already on the watchlist"));
                    } else {
                        return self.toggle_watchlist(sym);
                    }
                }
            }
            Cmd::WatchRemove => {
                if let Some(sym) = self.inplay.selected_symbol() {
                    if self.in_watchlist(&sym) {
                        return self.toggle_watchlist(sym);
                    }
                    self.flash = Some(format!("{sym} isn't on the watchlist"));
                }
            }
            Cmd::SortChange => resort(self, SortBy::Change),
            Cmd::SortRelVolume => resort(self, SortBy::RelVolume),
            Cmd::SortVolume => resort(self, SortBy::Volume),
            Cmd::TogglePriceFilter => {
                self.inplay.min_price = match self.inplay.min_price {
                    Some(_) => None,
                    None => Some(inplay::DEFAULT_MIN_PRICE),
                };
                self.inplay.selected = 0;
            }
            _ => {}
        }
        self.request_news(Utc::now())
    }

    /// Symbols of the list `g f` filters on this screen, in display order.
    fn find_items(&self) -> Vec<String> {
        match self.mode {
            Mode::InPlay => self
                .inplay
                .visible()
                .iter()
                .map(|r| r.symbol.clone())
                .collect(),
            _ => (0..self.total_rows())
                .filter_map(|r| self.row_symbol(r).map(str::to_string))
                .collect(),
        }
    }

    /// Indices (into the current list) matching the find query by ticker or
    /// company name, case-insensitive; everything when the query is empty.
    pub fn find_matches(&self) -> Vec<usize> {
        let q = self
            .find
            .as_ref()
            .map(|f| f.query.trim().to_lowercase())
            .unwrap_or_default();
        self.find_items()
            .iter()
            .enumerate()
            .filter(|(_, sym)| {
                q.is_empty()
                    || sym.to_lowercase().contains(&q)
                    || self
                        .asset(sym)
                        .is_some_and(|a| a.name.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn open_find(&mut self) {
        let current = match self.mode {
            Mode::InPlay => self.inplay.selected,
            _ => self.selected,
        };
        self.inplay.focus_news = false;
        self.find = Some(FindState {
            query: String::new(),
            selected: current,
            saved: current,
        });
    }

    fn on_find_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let matches = self.find_matches();
        let n = matches.len();
        let Some(f) = self.find.as_mut() else {
            return vec![];
        };
        match key.code {
            KeyCode::Esc => {
                let saved = f.saved;
                self.find = None;
                match self.mode {
                    Mode::InPlay => self.inplay.selected = saved,
                    _ => self.selected = saved,
                }
                return vec![];
            }
            KeyCode::Enter => {
                let picked = matches.get(f.selected).copied();
                self.find = None;
                let Some(i) = picked else {
                    self.flash = Some("no matches".into());
                    return vec![];
                };
                return match self.mode {
                    Mode::InPlay => {
                        self.inplay.selected = i;
                        self.inplay.news_cursor = 0;
                        self.request_news(Utc::now())
                    }
                    _ => {
                        self.selected = i;
                        vec![]
                    }
                };
            }
            KeyCode::Down | KeyCode::Char('n') if key.code == KeyCode::Down || ctrl => {
                f.selected = (f.selected + 1).min(n.saturating_sub(1));
            }
            KeyCode::Up | KeyCode::Char('p') if key.code == KeyCode::Up || ctrl => {
                f.selected = f.selected.saturating_sub(1);
            }
            KeyCode::Backspace => {
                f.query.pop();
                f.selected = 0;
            }
            KeyCode::Char('u') if ctrl => {
                f.query.clear();
                f.selected = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                f.query.push(c);
                f.selected = 0;
            }
            _ => {}
        }
        vec![]
    }

    fn open_search(&mut self) {
        self.mode = Mode::Search;
        self.search = SearchState::default();
    }

    fn run_search(&mut self) {
        self.search.results = search::search(&self.assets, &self.search.query);
        self.search.selected = 0;
    }

    /// Symbol the Enter/Tab keys act on in search: the highlighted result, or
    /// the typed text itself when nothing matches (lets you open a symbol the
    /// asset list hasn't loaded yet).
    pub fn search_target(&self) -> Option<String> {
        if let Some(&i) = self.search.results.get(self.search.selected) {
            return Some(self.assets[i].symbol.clone());
        }
        let typed = normalize_symbol(&self.search.query);
        let looks_like_symbol = !typed.is_empty()
            && typed.len() <= 6
            && typed.chars().all(|c| c.is_ascii_alphanumeric() || c == '.');
        looks_like_symbol.then_some(typed)
    }

    fn on_search_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.mode = Mode::Watchlist,
            KeyCode::Down | KeyCode::Char('n') if key.code == KeyCode::Down || ctrl => {
                if !self.search.results.is_empty() {
                    self.search.selected =
                        (self.search.selected + 1).min(self.search.results.len() - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('p') if key.code == KeyCode::Up || ctrl => {
                self.search.selected = self.search.selected.saturating_sub(1)
            }
            KeyCode::Enter => {
                if let Some(sym) = self.search_target() {
                    return self.open_detail(sym, Mode::Search);
                }
            }
            KeyCode::Tab => {
                if let Some(sym) = self.search_target() {
                    return self.toggle_watchlist(sym);
                }
            }
            KeyCode::Backspace => {
                self.search.query.pop();
                self.run_search();
            }
            KeyCode::Char('u') if ctrl => {
                self.search.query.clear();
                self.run_search();
            }
            KeyCode::Char(c) if !ctrl => {
                self.search.query.push(c);
                self.run_search();
            }
            _ => {}
        }
        vec![]
    }

    pub(super) fn toggle_watchlist(&mut self, sym: String) -> Vec<Action> {
        if let Some(pos) = self.watchlist.iter().position(|s| *s == sym) {
            self.watchlist.remove(pos);
            self.selected = self.selected.min(self.total_rows().saturating_sub(1));
            self.flash = Some(format!("removed {sym} from watchlist"));
            vec![Action::SaveWatchlist]
        } else {
            // Stocks go at the end of the stocks section, pairs at the end.
            let idx = self.section_index(&sym, self.watchlist.len());
            self.watchlist.insert(idx, sym.clone());
            self.flash = Some(format!("added {sym} to watchlist"));
            vec![Action::SaveWatchlist, Action::RefreshQuotes]
        }
    }

    fn open_detail(&mut self, sym: String, back_to: Mode) -> Vec<Action> {
        let d = DetailState::new(sym.clone(), back_to);
        let range = d.range;
        self.detail = Some(d);
        self.mode = Mode::Detail;
        self.request_history(range)
            .into_iter()
            .chain([Action::RefreshQuotes])
            .collect()
    }

    /// Ask for the current range's history unless it's loaded or in flight.
    fn request_history(&mut self, range: HistoryRange) -> Option<Action> {
        let d = self.detail.as_mut()?;
        if d.history.contains_key(&range) || d.loading.contains(&range) {
            return None;
        }
        d.loading.push(range);
        Some(Action::FetchHistory(d.symbol.clone(), range))
    }

    /// Force a reload of the current range (drops the cached bars).
    pub fn reload_history(&mut self) -> Option<Action> {
        let d = self.detail.as_mut()?;
        let range = d.range;
        if d.loading.contains(&range) {
            return None;
        }
        d.history.remove(&range);
        self.request_history(range)
    }

    /// Background refresh of the current range: keeps the cached bars on
    /// screen while the request is in flight (the reply replaces them), so a
    /// live 1D chart doesn't flash "loading" every minute.
    pub fn refresh_history(&mut self) -> Option<Action> {
        let d = self.detail.as_mut()?;
        let range = d.range;
        if d.loading.contains(&range) {
            return None;
        }
        d.loading.push(range);
        Some(Action::FetchHistory(d.symbol.clone(), range))
    }

    fn set_range(&mut self, range: HistoryRange) -> Vec<Action> {
        if let Some(d) = self.detail.as_mut() {
            d.range = range;
            d.scroll = 0;
            d.error = None;
            d.chart.jump_end();
        }
        self.request_history(range).into_iter().collect()
    }

    /// Change zoom or the session filter, keeping the cursor on the same
    /// moment in time.
    fn reshape_chart(&mut self, cmd: Cmd) {
        let Some(d) = self.detail.as_mut() else {
            return;
        };
        let before = d.candles().map(|(c, _)| c).unwrap_or_default();
        let n_bars = d.bars().map_or(0, |b| {
            chart::session_bars(b, d.range, d.show_extended()).0.len()
        });
        match cmd {
            Cmd::ZoomIn => d.chart.zoom_in(n_bars),
            Cmd::ZoomOut => {
                if !d.chart.zoom_out(n_bars) {
                    self.flash = Some("fully zoomed out".into());
                    return;
                }
            }
            Cmd::ZoomReset => d.chart.zoom_fit(),
            Cmd::ToggleExtended => {
                if super::fx::is_fx(&d.symbol) {
                    self.flash = Some("forex trades 24/5 — every bar is shown".into());
                    return;
                }
                if !d.range.intraday() {
                    self.flash = Some("extended hours only apply to 1D, 5D and 1M".into());
                    return;
                }
                d.chart.extended = !d.chart.extended;
            }
            _ => return,
        }
        let after = d.candles().map(|(c, _)| c).unwrap_or_default();
        d.chart.reanchor(&before, &after);
    }

    fn exec_detail(&mut self, cmd: Cmd) -> Vec<Action> {
        let Some(d) = self.detail.as_ref() else {
            self.mode = Mode::Watchlist;
            return vec![];
        };
        let range_idx = HistoryRange::ALL
            .iter()
            .position(|r| *r == d.range)
            .unwrap_or(0);
        let rows = d.bars().map_or(0, Vec::len);
        let last_row = rows.saturating_sub(1);
        let sym = d.symbol.clone();
        let back = d.back_to;
        let is_chart = d.view != DetailView::Table;
        let n_candles = d.candles().map_or(0, |(c, _)| c.len());
        let scroll = |app: &mut App, f: &dyn Fn(usize) -> usize| {
            if let Some(d) = app.detail.as_mut() {
                d.scroll = f(d.scroll).min(last_row);
            }
        };
        let view = |app: &mut App, v: Option<DetailView>| {
            if let Some(d) = app.detail.as_mut() {
                // None = tab: table ↔ the last chart style.
                let next = v.unwrap_or(match d.view {
                    DetailView::Table => d.last_chart,
                    _ => DetailView::Table,
                });
                if next != DetailView::Table {
                    d.last_chart = next;
                }
                d.view = next;
            }
        };
        match cmd {
            Cmd::Back => {
                self.detail = None;
                self.mode = back;
            }
            Cmd::GoWatchlist => {
                self.detail = None;
                self.mode = Mode::Watchlist;
            }
            Cmd::GoInPlay => {
                self.detail = None;
                return self.open_inplay();
            }
            Cmd::Help => self.show_help = true,
            Cmd::Range(r) => return self.set_range(r),
            Cmd::NextRange => {
                let i = (range_idx + 1).min(HistoryRange::ALL.len() - 1);
                return self.set_range(HistoryRange::ALL[i]);
            }
            Cmd::PrevRange => {
                return self.set_range(HistoryRange::ALL[range_idx.saturating_sub(1)]);
            }
            Cmd::ToggleView => view(self, None),
            Cmd::ViewCandles => view(self, Some(DetailView::Candles)),
            Cmd::ViewLine => view(self, Some(DetailView::Line)),
            Cmd::ViewTable => view(self, Some(DetailView::Table)),
            Cmd::Down => scroll(self, &|s| s + 1),
            Cmd::Up => scroll(self, &|s| s.saturating_sub(1)),
            Cmd::PageDown => scroll(self, &|s| s + 20),
            Cmd::PageUp => scroll(self, &|s| s.saturating_sub(20)),
            Cmd::GoTop if is_chart => {
                if let Some(d) = self.detail.as_mut() {
                    d.chart.jump_start(n_candles);
                }
            }
            Cmd::GoEnd if is_chart => {
                if let Some(d) = self.detail.as_mut() {
                    d.chart.jump_end();
                }
            }
            Cmd::GoTop => scroll(self, &|_| 0),
            Cmd::GoEnd => scroll(self, &|_| usize::MAX),
            Cmd::CursorLeft | Cmd::CursorRight if is_chart => {
                let delta = if cmd == Cmd::CursorLeft { -1 } else { 1 };
                if let Some(d) = self.detail.as_mut() {
                    d.chart.move_cursor(n_candles, delta);
                }
            }
            Cmd::PanLeft | Cmd::PanRight if is_chart => {
                if let Some(d) = self.detail.as_mut() {
                    let page = (d.chart.visible.get() / 2).max(1) as isize;
                    let delta = if cmd == Cmd::PanLeft { -page } else { page };
                    d.chart.pan(n_candles, delta);
                }
            }
            Cmd::ToggleLevels => {
                if let Some(d) = self.detail.as_mut() {
                    d.chart.levels = !d.chart.levels;
                }
            }
            Cmd::ToggleIchimoku => {
                if let Some(d) = self.detail.as_mut() {
                    d.chart.ichimoku = !d.chart.ichimoku;
                }
            }
            Cmd::ToggleMa => {
                if let Some(d) = self.detail.as_mut() {
                    d.chart.ma = !d.chart.ma;
                    if d.chart.ma && !super::chart::ma_range(d.range) {
                        self.flash =
                            Some("50/200-day averages show on the 6M and 1Y charts".into());
                    }
                }
            }
            Cmd::ZoomIn | Cmd::ZoomOut | Cmd::ZoomReset | Cmd::ToggleExtended => {
                self.reshape_chart(cmd);
            }
            Cmd::WatchAdd => {
                if self.in_watchlist(&sym) {
                    self.flash = Some(format!("{sym} is already on the watchlist"));
                } else {
                    return self.toggle_watchlist(sym);
                }
            }
            Cmd::WatchRemove => {
                if self.in_watchlist(&sym) {
                    return self.toggle_watchlist(sym);
                }
                self.flash = Some(format!("{sym} isn't on the watchlist"));
            }
            Cmd::Search => {
                self.detail = None;
                self.open_search();
            }
            Cmd::Refresh => {
                return self
                    .reload_history()
                    .into_iter()
                    .chain([Action::RefreshQuotes])
                    .collect();
            }
            _ => {}
        }
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    fn ch(c: char) -> KeyEvent {
        key(KeyCode::Char(c))
    }

    /// Type a sequence of plain characters.
    fn typ(a: &mut App, keys: &str) -> Vec<Action> {
        keys.chars().flat_map(|c| a.on_key(ch(c))).collect()
    }

    fn app() -> App {
        let mut a = App::new(vec!["SPY".into(), "QQQ".into(), "AAPL".into()], "IEX");
        a.set_assets(vec![
            Asset {
                symbol: "NVDA".into(),
                name: "NVIDIA Corporation".into(),
                exchange: "NASDAQ".into(),
                tradable: true,
            },
            Asset {
                symbol: "SPY".into(),
                name: "SPDR S&P 500 ETF Trust".into(),
                exchange: "ARCA".into(),
                tradable: true,
            },
        ]);
        a
    }

    #[test]
    fn navigation_clamps_and_quit() {
        let mut a = app();
        a.on_key(ch('k'));
        assert_eq!(a.selected, 0);
        for _ in 0..5 {
            a.on_key(ch('j'));
        }
        assert_eq!(a.selected, 2);
        typ(&mut a, "gg");
        assert_eq!(a.selected, 0);
        typ(&mut a, "ge");
        assert_eq!(a.selected, 2);
        typ(&mut a, "g2g");
        assert_eq!(a.selected, 1);
        // Row past the end clamps.
        typ(&mut a, "g99g");
        assert_eq!(a.selected, 2);
        assert_eq!(a.on_key(ch('q')), vec![Action::Quit]);
    }

    #[test]
    fn pending_prefix_shows_and_clears() {
        let mut a = app();
        a.on_key(ch('w'));
        assert_eq!(a.pending.as_ref().map(|p| p.prefix), Some('w'));
        // q after a prefix cancels instead of quitting.
        assert!(a.on_key(ch('q')).is_empty());
        assert!(a.pending.is_none());
        a.on_key(ch('g'));
        a.on_key(key(KeyCode::Esc));
        assert!(a.pending.is_none());
    }

    #[test]
    fn remove_undo_and_reorder() {
        let mut a = app();
        a.on_key(ch('j')); // QQQ
        assert_eq!(typ(&mut a, "wd"), vec![Action::SaveWatchlist]);
        assert_eq!(a.watchlist, vec!["SPY", "AAPL"]);
        typ(&mut a, "wu");
        assert_eq!(a.watchlist, vec!["SPY", "QQQ", "AAPL"]);
        assert_eq!(a.selected, 1);
        typ(&mut a, "wj");
        assert_eq!(a.watchlist, vec!["SPY", "AAPL", "QQQ"]);
        assert_eq!(a.selected, 2);
        typ(&mut a, "wkwk");
        assert_eq!(a.watchlist, vec!["QQQ", "SPY", "AAPL"]);
        assert_eq!(a.selected, 0);
        // The old bare keys no longer edit the list.
        typ(&mut a, "dJK");
        assert_eq!(a.watchlist, vec!["QQQ", "SPY", "AAPL"]);
    }

    #[test]
    fn removing_last_row_moves_selection_up() {
        let mut a = app();
        typ(&mut a, "ge");
        typ(&mut a, "wd");
        assert_eq!(a.selected, 1);
        typ(&mut a, "wdwd");
        assert!(a.watchlist.is_empty());
        // Keys on an empty list are harmless.
        assert!(typ(&mut a, "wd").is_empty());
        assert!(typ(&mut a, "g1g").is_empty());
        assert!(a.on_key(key(KeyCode::Enter)).is_empty());
    }

    #[test]
    fn watchlist_add_opens_search() {
        let mut a = app();
        typ(&mut a, "wa");
        assert_eq!(a.mode, Mode::Search);
    }

    #[test]
    fn search_add_and_open_detail() {
        let mut a = app();
        a.on_key(ch('/'));
        assert_eq!(a.mode, Mode::Search);
        for c in "nvid".chars() {
            a.on_key(ch(c));
        }
        assert_eq!(a.search_target().as_deref(), Some("NVDA"));
        // Tab adds without leaving search.
        assert_eq!(
            a.on_key(key(KeyCode::Tab)),
            vec![Action::SaveWatchlist, Action::RefreshQuotes]
        );
        assert!(a.in_watchlist("NVDA"));
        assert_eq!(a.mode, Mode::Search);
        // Enter opens the detail view and asks for 1D history + a quote.
        let acts = a.on_key(key(KeyCode::Enter));
        assert_eq!(
            acts,
            vec![
                Action::FetchHistory("NVDA".into(), HistoryRange::Day1),
                Action::RefreshQuotes
            ]
        );
        assert_eq!(a.mode, Mode::Detail);
        // Esc returns to search, where we came from.
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.mode, Mode::Search);
    }

    #[test]
    fn search_falls_back_to_typed_symbol() {
        let mut a = app();
        a.on_key(ch('/'));
        for c in "brk.b".chars() {
            a.on_key(ch(c));
        }
        assert!(a.search.results.is_empty());
        assert_eq!(a.search_target().as_deref(), Some("BRK.B"));
        a.on_key(key(KeyCode::Backspace));
        assert_eq!(a.search.query, "brk.");
        // 'q' types into the query rather than quitting.
        a.on_key(ch('q'));
        assert_eq!(a.mode, Mode::Search);
    }

    #[test]
    fn detail_ranges_cache_and_reload() {
        let mut a = app();
        a.on_key(key(KeyCode::Enter)); // SPY detail, 1D requested
        a.set_history("SPY", HistoryRange::Day1, Ok(vec![]));
        assert!(!a.detail.as_ref().unwrap().is_loading());
        assert_eq!(
            a.on_key(ch('4')),
            vec![Action::FetchHistory("SPY".into(), HistoryRange::Month6)]
        );
        // In flight → pressing again doesn't duplicate the request.
        assert!(a.on_key(ch('4')).is_empty());
        // Back to 1D is cached → no fetch.
        assert!(a.on_key(ch('1')).is_empty());
        // [ at the first range stays put; ] steps right.
        assert!(a.on_key(ch('[')).is_empty());
        assert_eq!(
            a.on_key(ch(']')),
            vec![Action::FetchHistory("SPY".into(), HistoryRange::Day5)]
        );
        // r reloads the current range even if cached.
        a.set_history("SPY", HistoryRange::Day5, Ok(vec![]));
        assert_eq!(
            a.on_key(ch('r')),
            vec![
                Action::FetchHistory("SPY".into(), HistoryRange::Day5),
                Action::RefreshQuotes
            ]
        );
    }

    #[test]
    fn detail_prefixes_view_range_and_go() {
        let mut a = app();
        a.on_key(key(KeyCode::Enter)); // SPY
        assert_eq!(
            typ(&mut a, "ty"),
            vec![Action::FetchHistory("SPY".into(), HistoryRange::Year1)]
        );
        assert_eq!(a.detail.as_ref().unwrap().range, HistoryRange::Year1);
        typ(&mut a, "vt");
        assert_eq!(a.detail.as_ref().unwrap().view, DetailView::Table);
        typ(&mut a, "vt"); // explicit, not a toggle
        assert_eq!(a.detail.as_ref().unwrap().view, DetailView::Table);
        typ(&mut a, "vc");
        assert_eq!(a.detail.as_ref().unwrap().view, DetailView::Candles);
        a.on_key(key(KeyCode::Tab));
        assert_eq!(a.detail.as_ref().unwrap().view, DetailView::Table);
        // Tab returns to the last chart style, line included.
        typ(&mut a, "vl");
        assert_eq!(a.detail.as_ref().unwrap().view, DetailView::Line);
        a.on_key(key(KeyCode::Tab));
        a.on_key(key(KeyCode::Tab));
        assert_eq!(a.detail.as_ref().unwrap().view, DetailView::Line);
        typ(&mut a, "gw");
        assert_eq!(a.mode, Mode::Watchlist);
        assert!(a.detail.is_none());
    }

    fn session_bars(n: usize) -> Vec<Bar> {
        // Regular-hours 5-minute bars from 14:30Z (09:30 EST) on 2026-11-06.
        (0..n)
            .map(|i| {
                let m = 14 * 60 + 30 + i * 5;
                let f = i as f64;
                Bar {
                    time: format!("2026-11-06T{:02}:{:02}:00Z", m / 60, m % 60)
                        .parse()
                        .unwrap(),
                    open: 100.0 + f,
                    high: 101.0 + f,
                    low: 99.0 + f,
                    close: 100.5 + f,
                    volume: 10.0,
                    vwap: None,
                }
            })
            .collect()
    }

    fn lists_file(entries: &[(&str, &[&str])]) -> crate::lists::ListsFile {
        let mut f = crate::lists::ListsFile::default();
        for (name, syms) in entries {
            let syms: Vec<String> = syms.iter().map(|s| s.to_string()).collect();
            crate::lists::add(&mut f, name, &syms, Some("why"), Utc::now());
        }
        f
    }

    #[test]
    fn list_rows_come_first_and_keys_respect_them() {
        let mut a = App::new(vec!["SPY".into(), "EUR_USD".into()], "IEX");
        a.set_lists(lists_file(&[("tomorrow", &["NVDA", "GBP_USD"])]));
        assert_eq!((a.list_rows(), a.total_rows()), (2, 4));
        // Lists loading under the cursor don't move it off its symbol.
        assert_eq!(a.selected_symbol(), Some("SPY"));
        assert_eq!(a.row_note(0), Some("why"));
        // List symbols get quotes too (stocks and pairs).
        assert!(a.quote_symbols().contains(&"NVDA".to_string()));
        assert!(a.fx_symbols().contains(&"GBP_USD".to_string()));
        typ(&mut a, "gs");
        assert_eq!(a.selected_symbol(), Some("SPY"));
        typ(&mut a, "gx");
        assert_eq!(a.selected_symbol(), Some("EUR_USD"));
        typ(&mut a, "gl");
        assert_eq!(a.selected, 0);
        // j walks from the list into the watchlist.
        typ(&mut a, "jj");
        assert_eq!(a.selected_symbol(), Some("SPY"));
        // w d on a list row removes it from the list (not the watchlist).
        typ(&mut a, "gl");
        let acts = {
            a.on_key(key(KeyCode::Char('w')));
            a.on_key(key(KeyCode::Char('d')))
        };
        assert_eq!(
            acts,
            vec![Action::RemoveFromList {
                list: "tomorrow".into(),
                symbol: "NVDA".into()
            }]
        );
        assert_eq!(a.list_rows(), 1);
        assert_eq!(a.watchlist, ["SPY", "EUR_USD"]);
        // Moving a list row is a no-op; moving a watchlist row still works.
        typ(&mut a, "wj");
        assert_eq!(a.selected_symbol(), Some("GBP_USD"));
    }

    #[test]
    fn reloading_lists_keeps_the_cursor_on_its_symbol() {
        let mut a = App::new(vec!["SPY".into(), "QQQ".into()], "IEX");
        a.set_lists(lists_file(&[("tomorrow", &["NVDA"])]));
        typ(&mut a, "gs");
        typ(&mut a, "j");
        assert_eq!(a.selected_symbol(), Some("QQQ"));
        // An agent adds two symbols: the cursor stays on QQQ.
        a.set_lists(lists_file(&[("tomorrow", &["NVDA", "AMD", "TSLA"])]));
        assert_eq!(a.selected_symbol(), Some("QQQ"));
        // Find covers list rows too.
        typ(&mut a, "gf");
        typ(&mut a, "tsl");
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.selected_symbol(), Some("TSLA"));
    }

    #[test]
    fn watchlist_sections_jump_move_and_add_in_place() {
        let mut a = App::new(
            vec!["SPY".into(), "QQQ".into(), "EUR_USD".into(), "USD_JPY".into()],
            "IEX",
        );
        assert_eq!(a.fx_start(), 2);
        typ(&mut a, "gx");
        assert_eq!(a.selected, 2);
        // Moving the first pair up would cross into stocks: refused.
        typ(&mut a, "wk");
        assert_eq!((a.watchlist[2].as_str(), a.selected), ("EUR_USD", 2));
        typ(&mut a, "gs");
        assert_eq!(a.selected, 0);
        // A new stock lands at the end of stocks, a new pair at the very end.
        a.toggle_watchlist("AAPL".into());
        a.toggle_watchlist("GBP_USD".into());
        assert_eq!(
            a.watchlist,
            ["SPY", "QQQ", "AAPL", "EUR_USD", "USD_JPY", "GBP_USD"]
        );
        assert_eq!(a.fx_symbols(), ["EUR_USD", "USD_JPY", "GBP_USD"]);
    }

    #[test]
    fn live_ticks_update_rows_and_the_open_chart_and_rest_never_rolls_back() {
        use rust_decimal_macros::dec;
        let mut a = App::new(vec!["EUR_USD".into()], "IEX");
        a.on_key(key(KeyCode::Enter));
        let mut bars = session_bars(3);
        let last_time = bars.last().unwrap().time;
        for b in &mut bars {
            b.close = 1.1;
        }
        a.set_history("EUR_USD", HistoryRange::Day1, Ok(bars));
        let tick = |bid, ask, secs| wickd_core::hub_client::HubQuote {
            instrument: "EUR_USD".into(),
            bid,
            ask,
            time: last_time + chrono::Duration::seconds(secs),
        };
        a.apply_tick(tick(dec!(1.2000), dec!(1.2002), 30));
        let q = &a.fx_quotes["EUR_USD"];
        assert_eq!((q.bid, q.ask), (dec!(1.2000), dec!(1.2002)));
        let d = a.detail.as_ref().unwrap();
        assert!((d.bars().unwrap().last().unwrap().close - 1.2001).abs() < 1e-9);
        // An older REST quote keeps the live bid/ask but brings the day stats.
        let stale = super::super::fx::FxQuote {
            bid: dec!(1.1),
            ask: dec!(1.1),
            time: last_time,
            open: Some(dec!(1.15)),
            high: Some(dec!(1.21)),
            low: Some(dec!(1.1)),
            closes: vec![1.1],
        };
        a.set_fx_quotes(HashMap::from([("EUR_USD".to_string(), stale)]));
        let q = &a.fx_quotes["EUR_USD"];
        assert_eq!((q.bid, q.open), (dec!(1.2000), Some(dec!(1.15))));
    }

    #[test]
    fn prices_remember_their_last_move() {
        let mut t = TickDirs::default();
        t.observe("SPY:last".into(), 100.0);
        assert_eq!(t.dir("SPY:last"), 0);
        t.observe("SPY:last".into(), 100.5);
        assert_eq!(t.dir("SPY:last"), 1);
        // Unchanged keeps the last direction; a drop flips it.
        t.observe("SPY:last".into(), 100.5);
        assert_eq!(t.dir("SPY:last"), 1);
        t.observe("SPY:last".into(), 99.0);
        assert_eq!(t.dir("SPY:last"), -1);
        assert_eq!(t.dir("QQQ:last"), 0);
    }

    #[test]
    fn forex_search_entries_survive_a_stock_list_refresh() {
        let mut a = app();
        let pair = Asset {
            symbol: "EUR_USD".into(),
            name: "EUR/USD".into(),
            exchange: super::super::fx::FX_EXCHANGE.into(),
            tradable: true,
        };
        a.set_fx_assets(vec![pair.clone()]);
        a.set_assets(vec![]);
        assert_eq!(a.asset("EUR_USD"), Some(&pair));
    }

    #[test]
    fn forex_pairs_chart_every_bar_not_just_us_hours() {
        let mut a = App::new(vec!["EUR_USD".into()], "IEX");
        a.on_key(key(KeyCode::Enter));
        // Overnight bars (02:00Z) that a stock chart would hide.
        let mut bars = session_bars(10);
        for b in &mut bars {
            b.time -= chrono::Duration::hours(12);
        }
        a.set_history("EUR_USD", HistoryRange::Day1, Ok(bars));
        let d = a.detail.as_ref().unwrap();
        assert!(d.show_extended() && !d.chart.extended);
        assert_eq!(d.candles().unwrap().0.len(), 10);
    }

    #[test]
    fn chart_cursor_pan_zoom_and_session_keys() {
        let mut a = app();
        a.on_key(key(KeyCode::Enter)); // SPY
        a.set_history("SPY", HistoryRange::Day1, Ok(session_bars(60)));
        a.detail.as_ref().unwrap().chart.visible.set(20);
        // h steps the cursor back from the latest candle.
        typ(&mut a, "hh");
        assert_eq!(a.detail.as_ref().unwrap().chart.cursor, Some(57));
        // g e re-pins to latest; g g jumps to the oldest.
        typ(&mut a, "ge");
        assert_eq!(a.detail.as_ref().unwrap().chart.cursor, None);
        typ(&mut a, "gg");
        assert_eq!(a.detail.as_ref().unwrap().chart.cursor, Some(0));
        // L pans forward half a window.
        typ(&mut a, "L");
        assert_eq!(a.detail.as_ref().unwrap().chart.cursor, Some(10));
        // Zoom out from fit merges bars; the cursor stays on the same time.
        let t = a.detail.as_ref().unwrap().candles().unwrap().0[10].time;
        typ(&mut a, "-");
        let d = a.detail.as_ref().unwrap();
        assert_eq!(
            (d.chart.fit, d.chart.zoom_level()),
            (false, chart::Zoom::Merge(2))
        );
        let (c, _) = d.candles().unwrap();
        assert_eq!(c.len(), 30);
        assert!(c[d.chart.cursor.unwrap()].time <= t);
        // 0 returns to fit; + from fit (60 bars in 120 columns) → 4-column slots.
        typ(&mut a, "0");
        assert!(a.detail.as_ref().unwrap().chart.fit);
        typ(&mut a, "+");
        assert_eq!(
            a.detail.as_ref().unwrap().chart.zoom_level(),
            chart::Zoom::Slot(4)
        );
        // [ ] change range (and reset the cursor).
        assert_eq!(
            typ(&mut a, "]"),
            vec![Action::FetchHistory("SPY".into(), HistoryRange::Day5)]
        );
        assert_eq!(a.detail.as_ref().unwrap().chart.cursor, None);
        typ(&mut a, "[");
        assert_eq!(a.detail.as_ref().unwrap().range, HistoryRange::Day1);
        // v e toggles extended hours on intraday ranges only.
        typ(&mut a, "ve");
        assert!(a.detail.as_ref().unwrap().chart.extended);
        typ(&mut a, "5ve");
        assert!(a.flash.as_deref().unwrap_or("").contains("extended hours"));
    }

    #[test]
    fn v_m_toggles_moving_averages_and_warmup_is_kept_per_range() {
        let mut a = app();
        a.on_key(key(KeyCode::Enter)); // SPY, 1D
        assert!(!a.detail.as_ref().unwrap().chart.ma);
        // On 1D the toggle works but explains where the lines show.
        typ(&mut a, "vm");
        assert!(a.detail.as_ref().unwrap().chart.ma);
        assert!(a.flash.as_deref().unwrap_or("").contains("6M and 1Y"));
        // On 1Y: no note; warm-up for that range is stored for the symbol only.
        typ(&mut a, "5");
        typ(&mut a, "vm");
        typ(&mut a, "vm");
        assert!(a.flash.is_none());
        a.set_warmup("SPY", HistoryRange::Year1, session_bars(3));
        a.set_warmup("QQQ", HistoryRange::Year1, session_bars(9)); // stale symbol: ignored
        assert_eq!(
            a.detail
                .as_ref()
                .unwrap()
                .warmup
                .get(&HistoryRange::Year1)
                .map(Vec::len),
            Some(3)
        );
    }

    #[test]
    fn v_s_toggles_support_resistance() {
        let mut a = app();
        a.on_key(key(KeyCode::Enter));
        typ(&mut a, "vs");
        assert!(a.detail.as_ref().unwrap().chart.levels);
        typ(&mut a, "vs");
        assert!(!a.detail.as_ref().unwrap().chart.levels);
    }

    #[test]
    fn v_i_toggles_ichimoku() {
        let mut a = app();
        a.on_key(key(KeyCode::Enter));
        assert!(!a.detail.as_ref().unwrap().chart.ichimoku);
        typ(&mut a, "vi");
        assert!(a.detail.as_ref().unwrap().chart.ichimoku);
        typ(&mut a, "vi");
        assert!(!a.detail.as_ref().unwrap().chart.ichimoku);
    }

    #[test]
    fn table_view_ignores_chart_cursor_keys() {
        let mut a = app();
        a.on_key(key(KeyCode::Enter));
        a.set_history("SPY", HistoryRange::Day1, Ok(session_bars(30)));
        typ(&mut a, "vt");
        typ(&mut a, "hh");
        assert_eq!(a.detail.as_ref().unwrap().chart.cursor, None);
        typ(&mut a, "ge");
        assert_eq!(a.detail.as_ref().unwrap().scroll, 29);
    }

    fn ip_row(sym: &str, chg: f64, last: f64, rvol: f64) -> Row {
        Row {
            symbol: sym.into(),
            last: Some(last),
            chg_pct: Some(chg),
            from_open_pct: None,
            volume: Some(rvol * 1e6),
            avg_volume: Some(1e6),
            rvol: Some(rvol),
            tags: vec![],
            latest_news: None,
            news_count: 0,
        }
    }

    #[test]
    fn in_play_screen_keys_sort_filter_news_and_back() {
        let mut a = app();
        // g i opens the screen, asks for the list.
        assert_eq!(typ(&mut a, "gi"), vec![Action::FetchInPlay]);
        assert_eq!(a.mode, Mode::InPlay);
        // A second g i while loading doesn't double-fetch.
        a.mode = Mode::Watchlist;
        assert!(typ(&mut a, "gi").is_empty());
        // Rows arrive: sorted by biggest move; headlines requested for every
        // visible row (selected first). PENNY is under $5, so it's hidden.
        let acts = a.set_inplay(Ok((
            vec![
                ip_row("SMALL", 1.0, 50.0, 0.5),
                ip_row("BIG", -12.0, 30.0, 3.0),
                ip_row("PENNY", 40.0, 1.5, 9.0),
            ],
            vec!["movers unavailable".into()],
        )));
        assert_eq!(
            acts,
            vec![
                Action::FetchNews("BIG".into()),
                Action::FetchNews("SMALL".into())
            ]
        );
        assert_eq!(a.inplay.visible().len(), 2);
        // In flight → moving around doesn't re-request.
        assert!(typ(&mut a, "jk").is_empty());
        // v f drops the price filter (PENNY now visible → its news requested).
        assert_eq!(typ(&mut a, "vf"), vec![Action::FetchNews("PENNY".into())]);
        assert_eq!(a.inplay.visible().len(), 3);
        typ(&mut a, "vr");
        assert_eq!(a.inplay.selected_symbol().as_deref(), Some("PENNY"));
        // w a adds the selected name to the watchlist.
        assert_eq!(
            typ(&mut a, "wa"),
            vec![Action::SaveWatchlist, Action::RefreshQuotes]
        );
        assert!(a.in_watchlist("PENNY"));
        // enter opens the chart; esc comes back to in play.
        let acts = a.on_key(key(KeyCode::Enter));
        assert!(acts.contains(&Action::FetchHistory("PENNY".into(), HistoryRange::Day1)));
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.mode, Mode::InPlay);
        // esc from in play returns to the watchlist.
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.mode, Mode::Watchlist);
    }

    #[test]
    fn news_cache_fills_rows_and_survives_list_rebuilds() {
        let mut a = app();
        typ(&mut a, "gi");
        a.set_inplay(Ok((vec![ip_row("A", 9.0, 10.0, 1.0)], vec![])));
        assert_eq!(a.inplay.rows[0].latest_news, None);
        let t: chrono::DateTime<chrono::Utc> = "2026-11-06T12:00:00Z".parse().unwrap();
        let item = |id, t| NewsItem {
            id,
            headline: "h".into(),
            summary: String::new(),
            created_at: t,
            symbols: vec!["A".into()],
            source: "benzinga".into(),
            url: None,
        };
        a.set_news(
            "A".into(),
            Ok(vec![item(1, t), item(2, t - chrono::Duration::hours(5))]),
        );
        assert_eq!(
            (a.inplay.rows[0].latest_news, a.inplay.rows[0].news_count),
            (Some(t), 2)
        );
        // The list rebuilds from scratch (as every minute / on return) —
        // the cached headlines are re-applied and nothing is re-requested.
        let acts = a.set_inplay(Ok((vec![ip_row("A", 9.5, 10.0, 1.0)], vec![])));
        assert!(acts.is_empty(), "{acts:?}");
        assert_eq!(a.inplay.rows[0].latest_news, Some(t));
        // After the TTL, a rebuild asks again.
        let later = chrono::Utc::now() + chrono::Duration::minutes(NEWS_TTL_MINUTES + 1);
        assert_eq!(a.request_news(later), vec![Action::FetchNews("A".into())]);
        // A failed fetch backs off: moving the cursor doesn't re-request it,
        // but once the backoff passes it's retried.
        a.inplay.news_pending.clear();
        a.set_news("A".into(), Err("rate limited".into()));
        assert!(typ(&mut a, "jk").is_empty());
        assert!(a.request_news(chrono::Utc::now()).is_empty());
        let after = chrono::Utc::now() + chrono::Duration::seconds(NEWS_RETRY_SECONDS + 1);
        assert_eq!(a.request_news(after), vec![Action::FetchNews("A".into())]);
        // r rebuilds the list and refetches only the selected symbol's
        // headlines, keeping the cached ones on screen meanwhile.
        a.inplay.news_pending.clear();
        a.set_news("A".into(), Ok(vec![item(1, t)]));
        assert_eq!(typ(&mut a, "r"), vec![Action::FetchInPlay]);
        assert!(a.inplay.news.contains_key("A"));
        let acts = a.set_inplay(Ok((vec![ip_row("A", 9.5, 10.0, 1.0)], vec![])));
        assert_eq!(acts, vec![Action::FetchNews("A".into())]);
    }

    #[test]
    fn headline_focus_opens_the_reader_and_follows_the_cursor() {
        use crate::tui::reader::{NavDir, ReaderEvent};
        let mut a = app();
        typ(&mut a, "gi");
        a.set_inplay(Ok((
            vec![ip_row("A", 9.0, 10.0, 1.0), ip_row("B", 5.0, 10.0, 1.0)],
            vec![],
        )));
        let t: chrono::DateTime<chrono::Utc> = "2026-11-06T12:00:00Z".parse().unwrap();
        let item = |id: i64, url: Option<&str>| NewsItem {
            id,
            headline: format!("h{id}"),
            summary: String::new(),
            created_at: t,
            symbols: vec!["A".into()],
            source: "benzinga".into(),
            url: url.map(str::to_string),
        };
        a.set_news(
            "A".into(),
            Ok(vec![
                item(1, Some("https://x/1")),
                item(2, None),
                item(3, Some("https://x/3")),
            ]),
        );
        // tab moves focus into the headlines; enter opens the first article.
        a.on_key(key(KeyCode::Tab));
        assert!(a.inplay.focus_news);
        let show = |url: &str, i: usize| Action::ReaderShow {
            url: url.into(),
            label: format!("A · {i} of 3 · ctrl+shift+← prev · ctrl+shift+→ next"),
        };
        assert_eq!(a.on_key(key(KeyCode::Enter)), vec![show("https://x/1", 1)]);
        a.set_reader(ReaderEvent::Opened);
        // ctrl+shift+→ in the window: skips the link-less headline 2.
        assert_eq!(
            a.set_reader(ReaderEvent::Nav(NavDir::Next)),
            vec![show("https://x/3", 3)]
        );
        // At the end: a notice for the window's badge instead of silence.
        assert_eq!(
            a.set_reader(ReaderEvent::Nav(NavDir::Next)),
            vec![Action::ReaderNotice("last linked headline".into())]
        );
        // ctrl+shift+← back to 1.
        assert_eq!(
            a.set_reader(ReaderEvent::Nav(NavDir::Prev)),
            vec![show("https://x/1", 1)]
        );
        // j moves to a link-less headline: nothing to load.
        assert!(typ(&mut a, "j").is_empty());
        // j again loads article 3 in the open window.
        assert_eq!(typ(&mut a, "j"), vec![show("https://x/3", 3)]);
        // enter on a headline without a link explains instead of failing.
        typ(&mut a, "k");
        assert!(a.on_key(key(KeyCode::Enter)).is_empty());
        assert!(a.flash.as_deref().unwrap_or("").contains("no link"));
        // esc returns focus to the list; j then moves the list, resetting the cursor.
        a.on_key(key(KeyCode::Esc));
        assert!(!a.inplay.focus_news);
        assert_eq!(a.mode, Mode::InPlay);
        typ(&mut a, "j");
        assert_eq!(
            (a.inplay.selected_symbol().as_deref(), a.inplay.news_cursor),
            (Some("B"), 0)
        );
        // tab on a symbol with no headlines says so.
        a.on_key(key(KeyCode::Tab));
        assert!(!a.inplay.focus_news);
        // x closes the reader window.
        assert_eq!(typ(&mut a, "x"), vec![Action::ReaderClose]);
        // Reader closing is tracked.
        a.set_reader(ReaderEvent::Closed);
        assert!(!a.inplay.reader_open);
    }

    #[test]
    fn vim_navigation_everywhere() {
        // Watchlist: J/K page (clamped), j/k step.
        let mut a = App::new((0..25).map(|i| format!("S{i}")).collect(), "IEX");
        typ(&mut a, "J");
        assert_eq!(a.selected, LIST_PAGE);
        typ(&mut a, "JJ");
        assert_eq!(a.selected, 24);
        typ(&mut a, "K");
        assert_eq!(a.selected, 24 - LIST_PAGE);
        // In play: J pages the list; l moves right into headlines, h back.
        typ(&mut a, "gi");
        let rows: Vec<Row> = (0..15)
            .map(|i| ip_row(&format!("R{i:02}"), 30.0 - i as f64, 10.0, 1.0))
            .collect();
        a.set_inplay(Ok((rows, vec![])));
        typ(&mut a, "J");
        assert_eq!(a.inplay.selected, LIST_PAGE);
        let sym = a.inplay.selected_symbol().unwrap();
        let t: chrono::DateTime<chrono::Utc> = "2026-11-06T12:00:00Z".parse().unwrap();
        let items: Vec<NewsItem> = (0..8)
            .map(|id| NewsItem {
                id,
                headline: format!("h{id}"),
                summary: String::new(),
                created_at: t,
                symbols: vec![sym.clone()],
                source: "benzinga".into(),
                url: Some(format!("https://x/{id}")),
            })
            .collect();
        a.set_news(sym.clone(), Ok(items));
        typ(&mut a, "h"); // left edge: nothing
        assert!(!a.inplay.focus_news);
        typ(&mut a, "l");
        assert!(a.inplay.focus_news);
        typ(&mut a, "J");
        assert_eq!(a.inplay.news_cursor, NEWS_PAGE);
        typ(&mut a, "K");
        assert_eq!(a.inplay.news_cursor, 0);
        typ(&mut a, "h");
        assert!(!a.inplay.focus_news);
        // enter (not l) opens the chart from the list.
        let acts = a.on_key(key(KeyCode::Enter));
        assert!(acts.contains(&Action::FetchHistory(sym, HistoryRange::Day1)));
    }

    #[test]
    fn find_filters_the_list_and_enter_jumps_to_the_line() {
        // In play: "g f spac" narrows to SPCX (by company name), enter jumps.
        let mut a = app();
        a.set_assets(vec![
            Asset {
                symbol: "SPCX".into(),
                name: "Space Exploration Technologies Corp.".into(),
                exchange: "NASDAQ".into(),
                tradable: true,
            },
            Asset {
                symbol: "NVDA".into(),
                name: "NVIDIA Corporation".into(),
                exchange: "NASDAQ".into(),
                tradable: true,
            },
        ]);
        typ(&mut a, "gi");
        a.set_inplay(Ok((
            vec![
                ip_row("NVDA", 9.0, 10.0, 1.0),
                ip_row("AAPL", 7.0, 10.0, 1.0),
                ip_row("SPCX", 5.0, 10.0, 1.0),
            ],
            vec![],
        )));
        assert_eq!(a.inplay.selected, 0);
        typ(&mut a, "gf");
        assert!(a.find.is_some());
        typ(&mut a, "spac");
        assert_eq!(a.find_matches(), vec![2]);
        // While filtering, letters (even q/j) type rather than act.
        assert_eq!(a.mode, Mode::InPlay);
        let acts = a.on_key(key(KeyCode::Enter));
        assert!(a.find.is_none());
        assert_eq!(a.inplay.selected_symbol().as_deref(), Some("SPCX"));
        assert!(acts.contains(&Action::FetchNews("SPCX".into())) || acts.is_empty());
        // Ticker match, arrows pick among several, esc restores the cursor.
        typ(&mut a, "gf");
        typ(&mut a, "a"); // NVDA (name "NVIDIA Corporation") , AAPL, SPCX ("Space ...") all contain "a"
        assert_eq!(a.find_matches().len(), 3);
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.find.as_ref().unwrap().selected, 1);
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.inplay.selected_symbol().as_deref(), Some("SPCX"));
        // No matches + enter: cancels with a note.
        typ(&mut a, "gf");
        typ(&mut a, "zzzz");
        assert!(a.find_matches().is_empty());
        a.on_key(key(KeyCode::Enter));
        assert!(a.find.is_none());
        assert!(a.flash.as_deref().unwrap_or("").contains("no matches"));
        // Watchlist works the same.
        a.on_key(key(KeyCode::Esc)); // back to watchlist
        assert_eq!(a.mode, Mode::Watchlist);
        typ(&mut a, "gf");
        typ(&mut a, "aap");
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.selected_symbol(), Some("AAPL"));
    }

    #[test]
    fn reader_pin_survives_refresh_failures_and_list_rebuilds() {
        use crate::tui::reader::{NavDir, ReaderEvent};
        let mut a = app();
        typ(&mut a, "gi");
        a.set_inplay(Ok((
            vec![ip_row("A", 9.0, 10.0, 1.0), ip_row("B", 5.0, 10.0, 1.0)],
            vec![],
        )));
        let t: chrono::DateTime<chrono::Utc> = "2026-11-06T12:00:00Z".parse().unwrap();
        let item = |id: i64| NewsItem {
            id,
            headline: format!("h{id}"),
            summary: String::new(),
            created_at: t,
            symbols: vec!["A".into()],
            source: "benzinga".into(),
            url: Some(format!("https://x/{id}")),
        };
        a.set_news("A".into(), Ok(vec![item(1), item(2), item(3)]));
        a.on_key(key(KeyCode::Tab));
        a.on_key(key(KeyCode::Enter)); // pins A, article 1
        a.set_reader(ReaderEvent::Opened);
        // A refresh of A's headlines fails (rate limit): the good ones stay.
        a.set_news("A".into(), Err("Alpaca rate limit hit".into()));
        assert_eq!(a.inplay.selected_headlines().len(), 3);
        // The list rebuilds and B now sorts first: the reader stays on A.
        a.set_inplay(Ok((
            vec![ip_row("B", 20.0, 10.0, 1.0), ip_row("A", 9.0, 10.0, 1.0)],
            vec![],
        )));
        let acts = a.set_reader(ReaderEvent::Nav(NavDir::Next));
        assert_eq!(
            acts,
            vec![Action::ReaderShow {
                url: "https://x/2".into(),
                label: "A · 2 of 3 · ctrl+shift+← prev · ctrl+shift+→ next".into(),
            }]
        );
        // With nothing pinned, nav explains itself on the badge.
        a.inplay.reader_pin = None;
        assert_eq!(
            a.set_reader(ReaderEvent::Nav(NavDir::Next)),
            vec![Action::ReaderNotice("no article open".into())]
        );
    }

    #[test]
    fn in_play_refresh_keeps_the_selected_symbol() {
        let mut a = app();
        typ(&mut a, "gi");
        a.set_inplay(Ok((
            vec![ip_row("A", 9.0, 10.0, 1.0), ip_row("B", 5.0, 10.0, 1.0)],
            vec![],
        )));
        typ(&mut a, "j"); // B
        a.set_inplay(Ok((
            vec![ip_row("B", 20.0, 10.0, 1.0), ip_row("A", 9.0, 10.0, 1.0)],
            vec![],
        )));
        assert_eq!(a.inplay.selected_symbol().as_deref(), Some("B"));
        // Errors keep the old rows.
        a.set_inplay(Err("boom".into()));
        assert_eq!(
            (a.inplay.rows.len(), a.inplay.error.as_deref()),
            (2, Some("boom"))
        );
    }

    #[test]
    fn background_refresh_keeps_bars_on_screen() {
        let mut a = app();
        a.on_key(key(KeyCode::Enter)); // SPY, 1D requested
        let one = Bar {
            time: "2026-11-06T15:00:00Z".parse().unwrap(),
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
            vwap: None,
        };
        a.set_history("SPY", HistoryRange::Day1, Ok(vec![one.clone()]));
        assert_eq!(
            a.refresh_history(),
            Some(Action::FetchHistory("SPY".into(), HistoryRange::Day1))
        );
        let d = a.detail.as_ref().unwrap();
        assert!(d.is_loading());
        assert_eq!(d.bars().unwrap().len(), 1, "cached bars stay visible");
        // A second tick while in flight doesn't stack requests.
        assert!(a.refresh_history().is_none());
        a.set_history("SPY", HistoryRange::Day1, Ok(vec![one.clone(), one]));
        assert_eq!(a.detail.as_ref().unwrap().bars().unwrap().len(), 2);
    }

    #[test]
    fn stale_history_replies_are_ignored_and_errors_shown() {
        let mut a = app();
        a.on_key(key(KeyCode::Enter)); // SPY
        a.set_history("QQQ", HistoryRange::Day1, Ok(vec![]));
        assert!(a.detail.as_ref().unwrap().bars().is_none());
        a.set_history("SPY", HistoryRange::Day1, Err("boom".into()));
        assert_eq!(a.detail.as_ref().unwrap().error.as_deref(), Some("boom"));
    }

    #[test]
    fn detail_symbol_is_polled_even_off_watchlist() {
        let mut a = app();
        a.on_key(ch('/'));
        a.on_key(ch('n'));
        a.on_key(key(KeyCode::Enter)); // NVDA, not on the list
        assert!(a.quote_symbols().contains(&"NVDA".to_string()));
        // `w a` in detail adds it; again is a no-op; `w d` removes.
        assert_eq!(
            typ(&mut a, "wa"),
            vec![Action::SaveWatchlist, Action::RefreshQuotes]
        );
        assert!(a.in_watchlist("NVDA"));
        assert!(typ(&mut a, "wa").is_empty());
        typ(&mut a, "wd");
        assert!(!a.in_watchlist("NVDA"));
    }

    #[test]
    fn help_swallows_next_key_and_ctrl_c_always_quits() {
        let mut a = app();
        a.on_key(ch('?'));
        assert!(a.show_help);
        assert!(a.on_key(ch('q')).is_empty());
        assert!(!a.show_help);
        a.on_key(ch('/'));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(a.on_key(ctrl_c), vec![Action::Quit]);
    }
}
