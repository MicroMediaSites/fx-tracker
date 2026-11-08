//! `wickd tui` state machine — pure (no I/O, no terminal), so every key path
//! is unit-testable. Key presses mutate [`App`] and return [`Action`]s that
//! the run loop turns into fetches / saves.

use std::collections::HashMap;

use chrono::{DateTime, Local};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use wickd_core::equities::{Asset, Bar, HistoryRange, MarketClock, Snapshot};

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
    Chart,
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
            view: DetailView::Chart,
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
        match self.mode {
            Mode::Watchlist => self.on_watchlist_key(key),
            Mode::Search => self.on_search_key(key),
            Mode::Detail => self.on_detail_key(key),
        }
    }

    fn on_watchlist_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let n = self.watchlist.len();
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return vec![Action::Quit],
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Down | KeyCode::Char('j') if n > 0 => {
                self.selected = (self.selected + 1).min(n - 1)
            }
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => self.selected = 0,
            KeyCode::Char('G') | KeyCode::End if n > 0 => self.selected = n - 1,
            KeyCode::Char('J') if n > 1 && self.selected + 1 < n => {
                self.watchlist.swap(self.selected, self.selected + 1);
                self.selected += 1;
                return vec![Action::SaveWatchlist];
            }
            KeyCode::Char('K') if self.selected > 0 && n > 1 => {
                self.watchlist.swap(self.selected, self.selected - 1);
                self.selected -= 1;
                return vec![Action::SaveWatchlist];
            }
            KeyCode::Char('/') | KeyCode::Char('a') => self.open_search(),
            KeyCode::Char('r') => return vec![Action::RefreshQuotes],
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                if let Some(sym) = self.selected_symbol().map(str::to_string) {
                    return self.open_detail(sym, Mode::Watchlist);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete if n > 0 => {
                let sym = self.watchlist.remove(self.selected);
                self.flash = Some(format!("removed {sym} — u to undo"));
                self.undo = Some((self.selected, sym));
                if self.selected >= self.watchlist.len() {
                    self.selected = self.watchlist.len().saturating_sub(1);
                }
                return vec![Action::SaveWatchlist];
            }
            KeyCode::Char('u') => {
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

    fn set_range(&mut self, range: HistoryRange) -> Vec<Action> {
        if let Some(d) = self.detail.as_mut() {
            d.range = range;
            d.scroll = 0;
            d.error = None;
        }
        self.request_history(range).into_iter().collect()
    }

    fn on_detail_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(d) = self.detail.as_ref() else {
            self.mode = Mode::Watchlist;
            return vec![];
        };
        let range_idx = HistoryRange::ALL
            .iter()
            .position(|r| *r == d.range)
            .unwrap_or(0);
        let rows = d.bars().map_or(0, Vec::len);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => {
                let back = d.back_to;
                self.detail = None;
                self.mode = back;
            }
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char(c @ '1'..='7') => {
                let i = (c as usize) - ('1' as usize);
                return self.set_range(HistoryRange::ALL[i]);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                let i = (range_idx + 1).min(HistoryRange::ALL.len() - 1);
                return self.set_range(HistoryRange::ALL[i]);
            }
            KeyCode::Left | KeyCode::Char('h') => {
                return self.set_range(HistoryRange::ALL[range_idx.saturating_sub(1)]);
            }
            KeyCode::Char('t') | KeyCode::Tab => {
                if let Some(d) = self.detail.as_mut() {
                    d.view = match d.view {
                        DetailView::Chart => DetailView::Table,
                        DetailView::Table => DetailView::Chart,
                    };
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(d) = self.detail.as_mut() {
                    d.scroll = (d.scroll + 1).min(rows.saturating_sub(1));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(d) = self.detail.as_mut() {
                    d.scroll = d.scroll.saturating_sub(1);
                }
            }
            KeyCode::PageDown => {
                if let Some(d) = self.detail.as_mut() {
                    d.scroll = (d.scroll + 20).min(rows.saturating_sub(1));
                }
            }
            KeyCode::PageUp => {
                if let Some(d) = self.detail.as_mut() {
                    d.scroll = d.scroll.saturating_sub(20);
                }
            }
            KeyCode::Char('a') | KeyCode::Char('w') => {
                let sym = d.symbol.clone();
                return self.toggle_watchlist(sym);
            }
            KeyCode::Char('/') => {
                self.detail = None;
                self.open_search();
            }
            KeyCode::Char('r') => {
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
        a.on_key(ch('g'));
        assert_eq!(a.selected, 0);
        assert_eq!(a.on_key(ch('q')), vec![Action::Quit]);
    }

    #[test]
    fn remove_undo_and_reorder() {
        let mut a = app();
        a.on_key(ch('j')); // QQQ
        assert_eq!(a.on_key(ch('d')), vec![Action::SaveWatchlist]);
        assert_eq!(a.watchlist, vec!["SPY", "AAPL"]);
        a.on_key(ch('u'));
        assert_eq!(a.watchlist, vec!["SPY", "QQQ", "AAPL"]);
        assert_eq!(a.selected, 1);
        a.on_key(ch('J'));
        assert_eq!(a.watchlist, vec!["SPY", "AAPL", "QQQ"]);
        assert_eq!(a.selected, 2);
        a.on_key(ch('K'));
        a.on_key(ch('K'));
        assert_eq!(a.watchlist, vec!["QQQ", "SPY", "AAPL"]);
        assert_eq!(a.selected, 0);
    }

    #[test]
    fn removing_last_row_moves_selection_up() {
        let mut a = app();
        a.on_key(ch('G'));
        a.on_key(ch('d'));
        assert_eq!(a.selected, 1);
        a.on_key(ch('d'));
        a.on_key(ch('d'));
        assert!(a.watchlist.is_empty());
        // Keys on an empty list are harmless.
        assert!(a.on_key(ch('d')).is_empty());
        assert!(a.on_key(key(KeyCode::Enter)).is_empty());
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
        // h at the first range stays put; l steps right.
        assert!(a.on_key(ch('h')).is_empty());
        assert_eq!(
            a.on_key(ch('l')),
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
        // `a` in detail toggles it onto the list.
        a.on_key(ch('a'));
        assert!(a.in_watchlist("NVDA"));
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
