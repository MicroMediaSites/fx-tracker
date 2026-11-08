//! `wickd tui` state machine — pure (no I/O, no terminal), so every key path
//! is unit-testable. Key presses mutate [`App`] and return [`Action`]s that
//! the run loop turns into fetches / saves.

use std::collections::HashMap;

use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use wickd_core::equities::{Asset, Bar, HistoryRange, MarketClock, Snapshot};

use super::chart::{self, Candle, ChartView};
use super::keymap::{self, Cmd, Ctx, Pending, Step};
use super::search;
use super::store::normalize_symbol;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Watchlist,
    Search,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailView {
    Candles,
    Line,
    Table,
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
    Quit,
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
            loading: Vec::new(),
            error: None,
            scroll: 0,
            back_to,
        }
    }

    pub fn bars(&self) -> Option<&Vec<Bar>> {
        self.history.get(&self.range)
    }

    /// Candles the chart draws for the current range, zoom and session
    /// setting, plus whether extended hours are included.
    pub fn candles(&self) -> Option<(Vec<Candle>, bool)> {
        let bars = self.bars()?;
        let (bars, extended) = chart::session_bars(bars, self.range, self.chart.extended);
        Some((
            chart::build(&bars, self.chart.merge_for(bars.len())),
            extended,
        ))
    }

    pub fn is_loading(&self) -> bool {
        self.loading.contains(&self.range)
    }
}

#[derive(Debug)]
pub struct App {
    pub watchlist: Vec<String>,
    pub selected: usize,
    pub snapshots: HashMap<String, Snapshot>,
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
    pub quotes_error: Option<String>,
    pub last_quotes: Option<DateTime<Local>>,
    /// Transient one-line message for the footer.
    pub flash: Option<String>,
    /// Last removed watchlist entry, for `u`ndo.
    undo: Option<(usize, String)>,
    pub feed_label: &'static str,
}

impl App {
    pub fn new(watchlist: Vec<String>, feed_label: &'static str) -> Self {
        Self {
            watchlist,
            selected: 0,
            snapshots: HashMap::new(),
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
            quotes_error: None,
            last_quotes: None,
            flash: None,
            undo: None,
            feed_label,
        }
    }

    // ── data updates ────────────────────────────────────────────────────────

    pub fn set_assets(&mut self, assets: Vec<Asset>) {
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
        if let Some(d) = &self.detail {
            if !syms.contains(&d.symbol) {
                syms.push(d.symbol.clone());
            }
        }
        syms
    }

    pub fn selected_symbol(&self) -> Option<&str> {
        self.watchlist.get(self.selected).map(String::as_str)
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
            },
            Step::Pending | Step::Cancel | Step::None => vec![],
        }
    }

    fn exec_watchlist(&mut self, cmd: Cmd, n: Option<usize>) -> Vec<Action> {
        let len = self.watchlist.len();
        match cmd {
            Cmd::Quit => return vec![Action::Quit],
            Cmd::Help => self.show_help = true,
            Cmd::Down if len > 0 => self.selected = (self.selected + 1).min(len - 1),
            Cmd::Up => self.selected = self.selected.saturating_sub(1),
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
            Cmd::WatchMoveDown if len > 1 && self.selected + 1 < len => {
                self.watchlist.swap(self.selected, self.selected + 1);
                self.selected += 1;
                return vec![Action::SaveWatchlist];
            }
            Cmd::WatchMoveUp if self.selected > 0 && len > 1 => {
                self.watchlist.swap(self.selected, self.selected - 1);
                self.selected -= 1;
                return vec![Action::SaveWatchlist];
            }
            Cmd::WatchRemove if len > 0 => {
                let sym = self.watchlist.remove(self.selected);
                self.flash = Some(format!("removed {sym} — w u to undo"));
                self.undo = Some((self.selected, sym));
                if self.selected >= self.watchlist.len() {
                    self.selected = self.watchlist.len().saturating_sub(1);
                }
                return vec![Action::SaveWatchlist];
            }
            Cmd::WatchUndo => {
                if let Some((idx, sym)) = self.undo.take() {
                    if !self.in_watchlist(&sym) {
                        let idx = idx.min(self.watchlist.len());
                        self.watchlist.insert(idx, sym.clone());
                        self.selected = idx;
                        self.flash = Some(format!("restored {sym}"));
                        return vec![Action::SaveWatchlist, Action::RefreshQuotes];
                    }
                }
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

    fn toggle_watchlist(&mut self, sym: String) -> Vec<Action> {
        if let Some(pos) = self.watchlist.iter().position(|s| *s == sym) {
            self.watchlist.remove(pos);
            if self.selected >= self.watchlist.len() {
                self.selected = self.watchlist.len().saturating_sub(1);
            }
            self.flash = Some(format!("removed {sym} from watchlist"));
            vec![Action::SaveWatchlist]
        } else {
            self.watchlist.push(sym.clone());
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
            chart::session_bars(b, d.range, d.chart.extended).0.len()
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
