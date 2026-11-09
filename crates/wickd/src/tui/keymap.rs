//! `wickd tui` keys, Helix-style (the same pattern as prview and pablo).
//!
//! Navigation is vim-shaped everywhere: **j/k** move up/down a list, **h/l**
//! move left/right (chart cursor; list ↔ headlines pane), and the shifted
//! letters take bigger steps (**J/K** a page, **H/L** half a chart screen).
//!
//! Bare keys do the everyday things: move, open, back, search, quit. A
//! letter *prefix* opens a category, and the next key picks the action:
//!
//!   g  go to       g g top · g e end · g w watchlist · g i in play · g <n> g row n
//!   w  watchlist   w a add · w d remove · w u undo · w j / w k move
//!   t  time range  t d 1D · t w 5D · t m 1M · t 6 6M · t y 1Y · t 5 5Y · t a MAX
//!   v  view        v c candles · v l line · v t table · v e extended hours
//!
//! While a prefix is pending the key panel lists its second keys. `esc`
//! cancels it, and so does any key that isn't one of its second keys (which
//! then does nothing else). `g <digits>` closes on `g` or `enter`.
//!
//! [`BINDINGS`] is the single source of truth: [`step`] resolves keys from
//! it, and the key panel, help screen and footer are all drawn from it, so
//! they can't drift. Search's text box is not keymapped (typing goes into the
//! query).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use wickd_core::equities::HistoryRange;

/// Which screen a binding applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ctx {
    Watchlist,
    Detail,
    InPlay,
}

/// What a key does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Up,
    Down,
    PageUp,
    PageDown,
    Open,
    Back,
    Quit,
    Search,
    Refresh,
    Help,
    ToggleView,
    PrevRange,
    NextRange,
    CursorLeft,
    CursorRight,
    PanLeft,
    PanRight,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleExtended,
    ToggleMa,
    ToggleIchimoku,
    ToggleLevels,
    Range(HistoryRange),
    GoTop,
    GoEnd,
    /// `g <n> g`: 1-based row (the number arrives with the step).
    GoRow,
    GoWatchlist,
    GoInPlay,
    GoStocks,
    GoForex,
    Find,
    FocusNews,
    FocusRight,
    FocusLeft,
    CloseReader,
    SortChange,
    SortRelVolume,
    SortVolume,
    TogglePriceFilter,
    WatchAdd,
    WatchRemove,
    WatchUndo,
    WatchMoveDown,
    WatchMoveUp,
    ViewCandles,
    ViewLine,
    ViewTable,
}

/// A key as the keymap sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum K {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
    Tab,
    Backspace,
    PageUp,
    PageDown,
    /// Display-only: the `<n>` of `g <n> g`.
    Number,
}

impl K {
    pub fn show(self) -> String {
        match self {
            K::Char(c) => c.to_string(),
            K::Up => "↑".into(),
            K::Down => "↓".into(),
            K::Left => "←".into(),
            K::Right => "→".into(),
            K::Enter => "enter".into(),
            K::Esc => "esc".into(),
            K::Tab => "tab".into(),
            K::Backspace => "bksp".into(),
            K::PageUp => "pgup".into(),
            K::PageDown => "pgdn".into(),
            K::Number => "<n>".into(),
        }
    }
}

/// Map a crossterm key to a keymap key (None for keys the TUI ignores).
pub fn token(key: &KeyEvent) -> Option<K> {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    Some(match key.code {
        KeyCode::Char(c) => K::Char(c),
        KeyCode::Up => K::Up,
        KeyCode::Down => K::Down,
        KeyCode::Left => K::Left,
        KeyCode::Right => K::Right,
        KeyCode::Enter => K::Enter,
        KeyCode::Esc => K::Esc,
        KeyCode::Tab => K::Tab,
        KeyCode::Backspace => K::Backspace,
        KeyCode::PageUp => K::PageUp,
        KeyCode::PageDown => K::PageDown,
        _ => return None,
    })
}

pub struct Binding {
    pub prefix: Option<char>,
    pub keys: &'static [K],
    pub cmd: Cmd,
    pub label: &'static str,
    pub ctx: &'static [Ctx],
}

/// Prefix letters and their category names, in display order.
pub const PREFIXES: [(char, &str); 4] = [
    ('g', "go to"),
    ('w', "watchlist"),
    ('t', "time range"),
    ('v', "view"),
];

const W: &[Ctx] = &[Ctx::Watchlist];
const D: &[Ctx] = &[Ctx::Detail];
const WD: &[Ctx] = &[Ctx::Watchlist, Ctx::Detail];
const I: &[Ctx] = &[Ctx::InPlay];
const DI: &[Ctx] = &[Ctx::Detail, Ctx::InPlay];
const WDI: &[Ctx] = &[Ctx::Watchlist, Ctx::Detail, Ctx::InPlay];
const WI: &[Ctx] = &[Ctx::Watchlist, Ctx::InPlay];

const fn bare(keys: &'static [K], cmd: Cmd, label: &'static str, ctx: &'static [Ctx]) -> Binding {
    Binding {
        prefix: None,
        keys,
        cmd,
        label,
        ctx,
    }
}

const fn pre(
    p: char,
    keys: &'static [K],
    cmd: Cmd,
    label: &'static str,
    ctx: &'static [Ctx],
) -> Binding {
    Binding {
        prefix: Some(p),
        keys,
        cmd,
        label,
        ctx,
    }
}

/// Every binding. Order is display order within a group.
pub static BINDINGS: &[Binding] = &[
    // ── bare ──
    bare(&[K::Down, K::Char('j')], Cmd::Down, "down", WDI),
    bare(&[K::Up, K::Char('k')], Cmd::Up, "up", WDI),
    bare(
        &[K::Char('J'), K::PageDown],
        Cmd::PageDown,
        "down a page",
        WDI,
    ),
    bare(&[K::Char('K'), K::PageUp], Cmd::PageUp, "up a page", WDI),
    bare(
        &[K::Enter, K::Right, K::Char('l')],
        Cmd::Open,
        "open symbol",
        W,
    ),
    bare(&[K::Enter], Cmd::Open, "open (chart / article)", I),
    bare(&[K::Right, K::Char('l')], Cmd::FocusRight, "→ headlines", I),
    bare(&[K::Left, K::Char('h')], Cmd::FocusLeft, "← list", I),
    bare(&[K::Left, K::Char('h')], Cmd::CursorLeft, "cursor ←", D),
    bare(&[K::Right, K::Char('l')], Cmd::CursorRight, "cursor →", D),
    bare(&[K::Char('H')], Cmd::PanLeft, "pan back", D),
    bare(&[K::Char('L')], Cmd::PanRight, "pan forward", D),
    bare(&[K::Char('+'), K::Char('=')], Cmd::ZoomIn, "zoom in", D),
    bare(&[K::Char('-')], Cmd::ZoomOut, "zoom out", D),
    bare(&[K::Char('0')], Cmd::ZoomReset, "reset zoom", D),
    bare(&[K::Char('[')], Cmd::PrevRange, "previous range", D),
    bare(&[K::Char(']')], Cmd::NextRange, "next range", D),
    bare(&[K::Char('1')], Cmd::Range(HistoryRange::Day1), "1D", D),
    bare(&[K::Char('2')], Cmd::Range(HistoryRange::Day5), "5D", D),
    bare(&[K::Char('3')], Cmd::Range(HistoryRange::Month1), "1M", D),
    bare(&[K::Char('4')], Cmd::Range(HistoryRange::Month6), "6M", D),
    bare(&[K::Char('5')], Cmd::Range(HistoryRange::Year1), "1Y", D),
    bare(&[K::Char('6')], Cmd::Range(HistoryRange::Year5), "5Y", D),
    bare(&[K::Char('7')], Cmd::Range(HistoryRange::Max), "MAX", D),
    bare(&[K::Tab], Cmd::ToggleView, "chart ↔ table", D),
    bare(&[K::Tab], Cmd::FocusNews, "list ↔ headlines", I),
    bare(&[K::Char('x')], Cmd::CloseReader, "close reader window", I),
    bare(&[K::Char('/')], Cmd::Search, "search", WDI),
    bare(&[K::Char('r')], Cmd::Refresh, "refresh", WDI),
    bare(&[K::Esc, K::Char('q'), K::Backspace], Cmd::Back, "back", DI),
    bare(&[K::Char('?')], Cmd::Help, "help", WDI),
    bare(&[K::Char('q')], Cmd::Quit, "quit", W),
    // ── g: go to ──
    pre('g', &[K::Char('g')], Cmd::GoTop, "top", WDI),
    pre('g', &[K::Char('e')], Cmd::GoEnd, "end", WDI),
    pre('g', &[K::Number], Cmd::GoRow, "row n (then g)", W),
    pre('g', &[K::Char('w')], Cmd::GoWatchlist, "watchlist", DI),
    pre('g', &[K::Char('i')], Cmd::GoInPlay, "in play today", WD),
    pre('g', &[K::Char('s')], Cmd::GoStocks, "stocks section", W),
    pre('g', &[K::Char('x')], Cmd::GoForex, "forex section", W),
    pre(
        'g',
        &[K::Char('f')],
        Cmd::Find,
        "find in list (filter, enter jumps)",
        WI,
    ),
    // ── w: watchlist ──
    pre('w', &[K::Char('a')], Cmd::WatchAdd, "add this symbol", D),
    pre('w', &[K::Char('a')], Cmd::WatchAdd, "add (search)", W),
    pre('w', &[K::Char('a')], Cmd::WatchAdd, "add to watchlist", I),
    pre('w', &[K::Char('d')], Cmd::WatchRemove, "remove", WDI),
    pre('w', &[K::Char('u')], Cmd::WatchUndo, "undo remove", W),
    pre('w', &[K::Char('j')], Cmd::WatchMoveDown, "move down", W),
    pre('w', &[K::Char('k')], Cmd::WatchMoveUp, "move up", W),
    // ── t: time range ──
    pre(
        't',
        &[K::Char('d')],
        Cmd::Range(HistoryRange::Day1),
        "1 day",
        D,
    ),
    pre(
        't',
        &[K::Char('w')],
        Cmd::Range(HistoryRange::Day5),
        "5 days",
        D,
    ),
    pre(
        't',
        &[K::Char('m')],
        Cmd::Range(HistoryRange::Month1),
        "1 month",
        D,
    ),
    pre(
        't',
        &[K::Char('6')],
        Cmd::Range(HistoryRange::Month6),
        "6 months",
        D,
    ),
    pre(
        't',
        &[K::Char('y')],
        Cmd::Range(HistoryRange::Year1),
        "1 year",
        D,
    ),
    pre(
        't',
        &[K::Char('5')],
        Cmd::Range(HistoryRange::Year5),
        "5 years",
        D,
    ),
    pre(
        't',
        &[K::Char('a')],
        Cmd::Range(HistoryRange::Max),
        "all (max)",
        D,
    ),
    // ── v: view ──
    pre('v', &[K::Char('c')], Cmd::ViewCandles, "candles", D),
    pre('v', &[K::Char('l')], Cmd::ViewLine, "line", D),
    pre(
        'v',
        &[K::Char('e')],
        Cmd::ToggleExtended,
        "extended hours",
        D,
    ),
    pre(
        'v',
        &[K::Char('m')],
        Cmd::ToggleMa,
        "50/200-day averages",
        D,
    ),
    pre(
        'v',
        &[K::Char('i')],
        Cmd::ToggleIchimoku,
        "Ichimoku cloud",
        D,
    ),
    pre(
        'v',
        &[K::Char('s')],
        Cmd::ToggleLevels,
        "support / resistance",
        D,
    ),
    pre(
        'v',
        &[K::Char('c')],
        Cmd::SortChange,
        "sort: biggest move",
        I,
    ),
    pre(
        'v',
        &[K::Char('r')],
        Cmd::SortRelVolume,
        "sort: relative volume",
        I,
    ),
    pre(
        'v',
        &[K::Char('a')],
        Cmd::SortVolume,
        "sort: most active",
        I,
    ),
    pre(
        'v',
        &[K::Char('f')],
        Cmd::TogglePriceFilter,
        "price ≥ $5 filter",
        I,
    ),
    pre('v', &[K::Char('t')], Cmd::ViewTable, "price table", D),
];

/// A prefix waiting for its second key; `digits` while typing `g <n>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub prefix: char,
    pub digits: Option<String>,
}

/// What a key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Run a command (with the typed number for `g <n> g`).
    Act(Cmd, Option<usize>),
    /// A prefix is now (still) pending.
    Pending,
    /// A pending prefix was cancelled; nothing else happens.
    Cancel,
    /// Key unbound here.
    None,
}

fn in_ctx(b: &Binding, ctx: Ctx) -> bool {
    b.ctx.contains(&ctx)
}

/// Prefixes that have at least one binding in `ctx`.
pub fn prefixes_in(ctx: Ctx) -> Vec<(char, &'static str)> {
    PREFIXES
        .iter()
        .copied()
        .filter(|(p, _)| {
            BINDINGS
                .iter()
                .any(|b| b.prefix == Some(*p) && in_ctx(b, ctx))
        })
        .collect()
}

/// Resolve one key. Returns the new pending state and what happened.
pub fn step(ctx: Ctx, pending: Option<&Pending>, key: K) -> (Option<Pending>, Step) {
    let idle = |s: Step| (None, s);

    // Typing a row number after `g`.
    if let Some(Pending {
        prefix,
        digits: Some(d),
    }) = pending
    {
        return match key {
            K::Char(c) if c.is_ascii_digit() && d.len() < 6 => (
                Some(Pending {
                    prefix: *prefix,
                    digits: Some(format!("{d}{c}")),
                }),
                Step::Pending,
            ),
            K::Backspace => {
                let mut d = d.clone();
                d.pop();
                (
                    Some(Pending {
                        prefix: *prefix,
                        digits: Some(d),
                    }),
                    Step::Pending,
                )
            }
            K::Enter | K::Char('g') if !d.is_empty() => idle(Step::Act(Cmd::GoRow, d.parse().ok())),
            _ => idle(Step::Cancel),
        };
    }

    if let Some(p) = pending {
        if key == K::Esc {
            return idle(Step::Cancel);
        }
        let rows = || {
            BINDINGS
                .iter()
                .filter(|b| b.prefix == Some(p.prefix) && in_ctx(b, ctx))
        };
        if let K::Char(c) = key {
            if c.is_ascii_digit() && rows().any(|b| b.cmd == Cmd::GoRow) {
                return (
                    Some(Pending {
                        prefix: p.prefix,
                        digits: Some(c.to_string()),
                    }),
                    Step::Pending,
                );
            }
        }
        return match rows().find(|b| b.keys.contains(&key)) {
            Some(b) => idle(Step::Act(b.cmd, None)),
            None => idle(Step::Cancel),
        };
    }

    if let Some(b) = BINDINGS
        .iter()
        .find(|b| b.prefix.is_none() && in_ctx(b, ctx) && b.keys.contains(&key))
    {
        return idle(Step::Act(b.cmd, None));
    }
    if let K::Char(c) = key {
        if prefixes_in(ctx).iter().any(|(p, _)| *p == c) {
            return (
                Some(Pending {
                    prefix: c,
                    digits: None,
                }),
                Step::Pending,
            );
        }
    }
    idle(Step::None)
}

/// One line of a listing: the keys (`↓ j`) and what they do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub keys: String,
    pub label: &'static str,
}

fn keys_text(b: &Binding) -> String {
    b.keys
        .iter()
        .map(|k| k.show())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Second keys of `prefix` in `ctx` (the key panel while a prefix is pending).
pub fn prefix_entries(ctx: Ctx, prefix: char) -> Vec<Entry> {
    BINDINGS
        .iter()
        .filter(|b| b.prefix == Some(prefix) && in_ctx(b, ctx))
        .map(|b| Entry {
            keys: keys_text(b),
            label: b.label,
        })
        .collect()
}

/// Bare keys in `ctx`, with the 1–7 range shortcuts folded into one entry.
pub fn bare_entries(ctx: Ctx) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    let mut ranges = false;
    for b in BINDINGS
        .iter()
        .filter(|b| b.prefix.is_none() && in_ctx(b, ctx))
    {
        if matches!(b.cmd, Cmd::Range(_)) {
            if !ranges {
                ranges = true;
                out.push(Entry {
                    keys: "1-7".into(),
                    label: "range 1D…MAX",
                });
            }
            continue;
        }
        out.push(Entry {
            keys: keys_text(b),
            label: b.label,
        });
    }
    out
}

/// Footer text while a chord is being typed: `g`, `g 12`.
pub fn pending_text(p: &Pending) -> String {
    match &p.digits {
        Some(d) => format!("{} {d}", p.prefix),
        None => p.prefix.to_string(),
    }
}

/// Category name for a prefix letter.
pub fn prefix_name(p: char) -> &'static str {
    PREFIXES
        .iter()
        .find(|(c, _)| *c == p)
        .map(|(_, n)| *n)
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ctx: Ctx, keys: &[K]) -> Vec<Step> {
        let mut pending = None;
        keys.iter()
            .map(|&k| {
                let (p, s) = step(ctx, pending.as_ref(), k);
                pending = p;
                s
            })
            .collect()
    }

    fn c(ch: char) -> K {
        K::Char(ch)
    }

    #[test]
    fn bare_keys_resolve_per_screen() {
        assert_eq!(
            run(Ctx::Watchlist, &[c('q')]),
            vec![Step::Act(Cmd::Quit, None)]
        );
        assert_eq!(
            run(Ctx::Detail, &[c('q')]),
            vec![Step::Act(Cmd::Back, None)]
        );
        assert_eq!(
            run(Ctx::Watchlist, &[K::Enter]),
            vec![Step::Act(Cmd::Open, None)]
        );
        assert_eq!(
            run(Ctx::Detail, &[c('4')]),
            vec![Step::Act(Cmd::Range(HistoryRange::Month6), None)]
        );
        // Unbound bare key.
        assert_eq!(run(Ctx::Watchlist, &[c('z')]), vec![Step::None]);
        // Esc with nothing pending on the watchlist does nothing (no accidental quit).
        assert_eq!(run(Ctx::Watchlist, &[K::Esc]), vec![Step::None]);
    }

    #[test]
    fn prefix_then_second_key() {
        assert_eq!(
            run(Ctx::Watchlist, &[c('g'), c('e')]),
            vec![Step::Pending, Step::Act(Cmd::GoEnd, None)]
        );
        assert_eq!(
            run(Ctx::Detail, &[c('t'), c('y')]),
            vec![
                Step::Pending,
                Step::Act(Cmd::Range(HistoryRange::Year1), None)
            ]
        );
        assert_eq!(
            run(Ctx::Detail, &[c('v'), c('t')]),
            vec![Step::Pending, Step::Act(Cmd::ViewTable, None)]
        );
        assert_eq!(
            run(Ctx::Watchlist, &[c('w'), c('j')]),
            vec![Step::Pending, Step::Act(Cmd::WatchMoveDown, None)]
        );
    }

    #[test]
    fn esc_or_unknown_second_key_cancels_without_acting() {
        assert_eq!(
            run(Ctx::Watchlist, &[c('g'), K::Esc]),
            vec![Step::Pending, Step::Cancel]
        );
        // 'q' after a prefix cancels; it does NOT quit.
        assert_eq!(
            run(Ctx::Watchlist, &[c('w'), c('q')]),
            vec![Step::Pending, Step::Cancel]
        );
        // And the next key is bare again.
        assert_eq!(
            run(Ctx::Watchlist, &[c('w'), c('x'), c('q')]),
            vec![Step::Pending, Step::Cancel, Step::Act(Cmd::Quit, None)]
        );
    }

    #[test]
    fn go_row_number() {
        assert_eq!(
            run(Ctx::Watchlist, &[c('g'), c('1'), c('2'), c('g')]),
            vec![
                Step::Pending,
                Step::Pending,
                Step::Pending,
                Step::Act(Cmd::GoRow, Some(12))
            ]
        );
        assert_eq!(
            run(Ctx::Watchlist, &[c('g'), c('3'), K::Enter]),
            vec![Step::Pending, Step::Pending, Step::Act(Cmd::GoRow, Some(3))]
        );
        // Backspace edits; deleting every digit then 'g' cancels rather than acting.
        assert_eq!(
            run(Ctx::Watchlist, &[c('g'), c('3'), K::Backspace, c('g')]),
            vec![Step::Pending, Step::Pending, Step::Pending, Step::Cancel]
        );
        // Row numbers are watchlist-only; on the detail screen 'g 3' just cancels.
        assert_eq!(
            run(Ctx::Detail, &[c('g'), c('3')]),
            vec![Step::Pending, Step::Cancel]
        );
    }

    #[test]
    fn prefixes_only_open_where_they_have_bindings() {
        // t and v are detail-only.
        assert_eq!(run(Ctx::Watchlist, &[c('t')]), vec![Step::None]);
        assert_eq!(run(Ctx::Watchlist, &[c('v')]), vec![Step::None]);
        assert_eq!(
            prefixes_in(Ctx::Watchlist)
                .iter()
                .map(|p| p.0)
                .collect::<String>(),
            "gw"
        );
        assert_eq!(
            prefixes_in(Ctx::Detail)
                .iter()
                .map(|p| p.0)
                .collect::<String>(),
            "gwtv"
        );
        assert_eq!(
            prefixes_in(Ctx::InPlay)
                .iter()
                .map(|p| p.0)
                .collect::<String>(),
            "gwv"
        );
    }

    #[test]
    fn no_key_is_bound_twice_in_one_place() {
        for ctx in [Ctx::Watchlist, Ctx::Detail, Ctx::InPlay] {
            let mut seen: Vec<(Option<char>, K)> = Vec::new();
            for b in BINDINGS.iter().filter(|b| in_ctx(b, ctx)) {
                for k in b.keys {
                    assert!(
                        !seen.contains(&(b.prefix, *k)),
                        "{ctx:?}: {:?} {:?} bound twice",
                        b.prefix,
                        k
                    );
                    seen.push((b.prefix, *k));
                }
            }
            // A bare key must not shadow a prefix letter.
            for (p, _) in prefixes_in(ctx) {
                assert!(
                    !seen.contains(&(None, K::Char(p))),
                    "{ctx:?}: bare '{p}' shadows its prefix"
                );
            }
        }
    }

    #[test]
    fn listings_come_from_the_table() {
        let g = prefix_entries(Ctx::Watchlist, 'g');
        assert!(g.iter().any(|e| e.keys == "<n>"));
        let bare = bare_entries(Ctx::Detail);
        assert_eq!(bare.iter().filter(|e| e.keys == "1-7").count(), 1);
        assert_eq!(
            pending_text(&Pending {
                prefix: 'g',
                digits: Some("12".into())
            }),
            "g 12"
        );
        assert_eq!(prefix_name('t'), "time range");
    }
}
