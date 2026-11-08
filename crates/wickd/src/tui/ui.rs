//! Rendering for `wickd tui`. Pure functions of [`App`]; all times are shown
//! in the machine's local zone (Mountain for Matt).

use chrono::{DateTime, Local, Utc};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Block, Borders, Cell, Chart, Clear, Dataset, GraphType, Paragraph, Row, Table,
    TableState, Tabs,
};
use ratatui::Frame;

use wickd_core::equities::{Bar, HistoryRange, Snapshot};

use super::app::{App, DetailState, DetailView, Mode};
use super::candles::{self, CandleChart};
use super::keymap::{self, Ctx, Entry};

const UP: Color = Color::Green;
const DOWN: Color = Color::Red;
const DIM: Color = Color::DarkGray;
const ACCENT: Color = Color::Cyan;

// ── formatting helpers (unit-tested below) ──────────────────────────────────

pub fn fmt_price(p: f64) -> String {
    if p.abs() >= 1.0 {
        format!("{p:.2}")
    } else {
        format!("{p:.4}")
    }
}

pub fn fmt_change(c: f64) -> String {
    if c.abs() >= 1.0 || c == 0.0 {
        format!("{c:+.2}")
    } else {
        format!("{c:+.4}")
    }
}

pub fn fmt_volume(v: f64) -> String {
    let a = v.abs();
    if a >= 1e9 {
        format!("{:.2}B", v / 1e9)
    } else if a >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if a >= 1e3 {
        format!("{:.1}K", v / 1e3)
    } else {
        format!("{v:.0}")
    }
}

/// Unicode block sparkline of `values`, resampled to at most `width` cells
/// (each cell shows the last value in its bucket, like a close-price line).
pub fn sparkline(values: &[f64], width: usize) -> String {
    const TICKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if values.is_empty() || width == 0 {
        return String::new();
    }
    let n = values.len();
    let cells = width.min(n);
    let sampled: Vec<f64> = (0..cells)
        .map(|i| values[((i + 1) * n / cells).saturating_sub(1)])
        .collect();
    let (lo, hi) = sampled
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &v| {
            (lo.min(v), hi.max(v))
        });
    let span = hi - lo;
    sampled
        .iter()
        .map(|&v| {
            if span <= f64::EPSILON {
                TICKS[3]
            } else {
                TICKS[(((v - lo) / span) * 7.0).round() as usize]
            }
        })
        .collect()
}

fn change_color(c: Option<f64>) -> Color {
    match c {
        Some(c) if c > 0.0 => UP,
        Some(c) if c < 0.0 => DOWN,
        _ => Color::Reset,
    }
}

fn local(t: DateTime<Utc>) -> DateTime<Local> {
    t.with_timezone(&Local)
}

/// Alpaca's names are verbose ("Apple Inc. Common Stock"); drop the
/// boilerplate share-class suffixes for display.
pub fn short_name(name: &str) -> String {
    const SUFFIXES: [&str; 6] = [
        " Common Stock",
        " Class A Common Stock",
        " Class A Ordinary Shares",
        " Ordinary Shares",
        " American Depositary Shares",
        " Common Shares",
    ];
    let mut n = name.trim().to_string();
    for s in SUFFIXES {
        if let Some(stripped) = n.strip_suffix(s) {
            n = stripped.to_string();
            break;
        }
    }
    n
}

// ── top level ───────────────────────────────────────────────────────────────

pub fn render(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(frame, chunks[0], app);
    match app.mode {
        Mode::Watchlist => render_watchlist(frame, chunks[1], app),
        Mode::Search => render_search(frame, chunks[1], app),
        Mode::Detail => match &app.detail {
            Some(d) => render_detail(frame, chunks[1], app, d),
            None => render_watchlist(frame, chunks[1], app),
        },
    }
    render_footer(frame, chunks[2], app);
    if let (Some(p), Some(ctx)) = (&app.pending, app.ctx()) {
        render_key_panel(frame, chunks[1], ctx, p);
    }
    if app.show_help {
        render_help(frame, frame.area());
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let mut left = vec![Span::styled(
        "wickd ",
        Style::default().add_modifier(Modifier::BOLD),
    )];
    match &app.clock {
        Some(c) if c.is_open => {
            left.push(Span::styled("● market open", Style::default().fg(UP)));
            left.push(Span::styled(
                format!("  closes {}", local(c.next_close).format("%-I:%M %p")),
                Style::default().fg(DIM),
            ));
        }
        Some(c) => {
            left.push(Span::styled("○ market closed", Style::default().fg(DIM)));
            left.push(Span::styled(
                format!("  opens {}", local(c.next_open).format("%a %-I:%M %p")),
                Style::default().fg(DIM),
            ));
        }
        None => left.push(Span::styled("market …", Style::default().fg(DIM))),
    }
    let right = match app.last_quotes {
        Some(t) => format!("quotes {} · {}", app.feed_label, t.format("%-I:%M:%S %p")),
        None => format!("quotes {} · loading…", app.feed_label),
    };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(right.chars().count() as u16),
        ])
        .split(area);
    frame.render_widget(Paragraph::new(Line::from(left)), cols[0]);
    frame.render_widget(
        Paragraph::new(Span::styled(right, Style::default().fg(DIM))).alignment(Alignment::Right),
        cols[1],
    );
}

/// The footer's hint line for a screen, generated from the keymap: the
/// prefix categories first (they hold most commands), then the bare keys
/// that aren't plain movement.
fn footer_hints(ctx: Ctx) -> String {
    let mut parts: Vec<String> = keymap::prefixes_in(ctx)
        .into_iter()
        .map(|(p, name)| format!("{p} {name}…"))
        .collect();
    for e in keymap::bare_entries(ctx) {
        let first = e.keys.split(' ').next().unwrap_or("");
        if matches!(first, "↓" | "↑" | "←" | "→" | "pgdn" | "pgup") {
            continue;
        }
        parts.push(format!("{first} {}", e.label));
    }
    parts.join(" · ")
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    // A chord in progress wins over everything else.
    if let Some(p) = &app.pending {
        let hint = match &p.digits {
            Some(_) => "  type a row number, then g or enter · esc cancels".to_string(),
            None => format!("  {}… · esc cancels", keymap::prefix_name(p.prefix)),
        };
        let line = Line::from(vec![
            Span::styled(
                keymap::pending_text(p),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(hint, Style::default().fg(DIM)),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let hints = match app.ctx() {
        Some(ctx) => footer_hints(ctx),
        None => "type to search · ↑↓ select · enter open · tab add/remove · esc back".to_string(),
    };
    let msg = app
        .flash
        .clone()
        .or_else(|| app.quotes_error.as_ref().map(|e| format!("quotes: {e}")))
        .or_else(|| app.assets_error.as_ref().map(|e| format!("symbols: {e}")));
    let line = match msg {
        Some(m) => Line::from(Span::styled(m, Style::default().fg(Color::Yellow))),
        None => Line::from(Span::styled(hints, Style::default().fg(DIM))),
    };
    frame.render_widget(Paragraph::new(line), area);
}

/// While a prefix is pending: its second keys, bottom-right of the body.
fn render_key_panel(frame: &mut Frame, area: Rect, ctx: Ctx, p: &keymap::Pending) {
    let entries = keymap::prefix_entries(ctx, p.prefix);
    if entries.is_empty() {
        return;
    }
    let key_w = entries
        .iter()
        .map(|e| e.keys.chars().count())
        .max()
        .unwrap_or(1);
    let lines: Vec<Line> = entries
        .iter()
        .map(|e| {
            Line::from(vec![
                Span::styled(
                    format!(" {:<key_w$}  ", e.keys),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(e.label),
            ])
        })
        .collect();
    let title = format!(" {} · {} ", p.prefix, keymap::prefix_name(p.prefix));
    let inner_w = lines
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0)
        .max(title.chars().count());
    let w = (inner_w as u16 + 3).min(area.width);
    let h = (lines.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(w + 2),
        y: area.y + area.height.saturating_sub(h + 1),
        width: w,
        height: h,
    };
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT))
                .title(title),
        ),
        rect,
    );
}

/// Header row: left-aligned labels, then right-aligned (numeric) labels,
/// then left-aligned trailing labels — matching how the cells align.
fn header_row(left: &[&str], right: &[&str], trailing: &[&str]) -> Row<'static> {
    let cell = |s: &str, a: Alignment| Cell::from(Line::from(s.to_string()).alignment(a));
    let cells: Vec<Cell> = left
        .iter()
        .map(|s| cell(s, Alignment::Left))
        .chain(right.iter().map(|s| cell(s, Alignment::Right)))
        .chain(trailing.iter().map(|s| cell(s, Alignment::Left)))
        .collect();
    Row::new(cells).style(Style::default().fg(DIM).add_modifier(Modifier::BOLD))
}

// ── watchlist ───────────────────────────────────────────────────────────────

fn quote_cells(snap: Option<&Snapshot>) -> (String, String, String, Color) {
    let Some(s) = snap else {
        return ("…".into(), String::new(), String::new(), DIM);
    };
    let last = s.last().map(fmt_price).unwrap_or_else(|| "—".into());
    let chg = s.change().map(fmt_change).unwrap_or_default();
    let pct = s
        .change_pct()
        .map(|p| format!("{p:+.2}%"))
        .unwrap_or_default();
    (last, chg, pct, change_color(s.change()))
}

fn render_watchlist(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" watchlist · {} ", app.watchlist.len()));
    if app.watchlist.is_empty() {
        frame.render_widget(
            Paragraph::new("No symbols yet — press / to search and tab to add.")
                .style(Style::default().fg(DIM))
                .block(block),
            area,
        );
        return;
    }
    let spark_w: usize = 24;
    let header = header_row(
        &["SYMBOL", "NAME"],
        &["LAST", "CHG", "CHG%", "DAY RANGE", "VOLUME"],
        &["TODAY"],
    );
    let rows = app.watchlist.iter().map(|sym| {
        let snap = app.snapshots.get(sym);
        let (last, chg, pct, color) = quote_cells(snap);
        let name = app
            .asset(sym)
            .map(|a| short_name(&a.name))
            .unwrap_or_default();
        let range = snap
            .and_then(|s| s.daily_bar.as_ref())
            .map(|b| format!("{} – {}", fmt_price(b.low), fmt_price(b.high)))
            .unwrap_or_default();
        let vol = snap
            .and_then(|s| s.daily_bar.as_ref())
            .map(|b| fmt_volume(b.volume))
            .unwrap_or_default();
        let spark = app
            .sparks
            .get(sym)
            .map(|v| sparkline(v, spark_w))
            .unwrap_or_default();
        Row::new(vec![
            Cell::from(sym.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(name),
            Cell::from(Line::from(last).alignment(Alignment::Right)),
            Cell::from(Line::from(chg).alignment(Alignment::Right))
                .style(Style::default().fg(color)),
            Cell::from(Line::from(pct).alignment(Alignment::Right))
                .style(Style::default().fg(color)),
            Cell::from(Line::from(range).alignment(Alignment::Right)),
            Cell::from(Line::from(vol).alignment(Alignment::Right)),
            Cell::from(spark).style(Style::default().fg(color)),
        ])
    });
    let widths = [
        Constraint::Length(7),
        Constraint::Min(12),
        Constraint::Length(10),
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Length(19),
        Constraint::Length(8),
        Constraint::Length(spark_w as u16),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(2)
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = TableState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

// ── search ──────────────────────────────────────────────────────────────────

fn render_search(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);
    let input = Paragraph::new(Line::from(vec![
        Span::styled("› ", Style::default().fg(ACCENT)),
        Span::raw(app.search.query.clone()),
        Span::styled("▏", Style::default().fg(ACCENT)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" search symbols "),
    );
    frame.render_widget(input, chunks[0]);

    let block = Block::default().borders(Borders::ALL);
    if app.assets.is_empty() {
        let msg = match &app.assets_error {
            Some(e) => format!("symbol list unavailable ({e}) — enter still opens a typed symbol"),
            None => "loading symbol list…".to_string(),
        };
        frame.render_widget(
            Paragraph::new(msg)
                .style(Style::default().fg(DIM))
                .block(block),
            chunks[1],
        );
        return;
    }
    if app.search.results.is_empty() {
        let msg = if app.search.query.trim().is_empty() {
            "type a ticker or company name".to_string()
        } else {
            match app.search_target() {
                Some(t) => format!("no matches — enter opens {t} anyway"),
                None => "no matches".to_string(),
            }
        };
        frame.render_widget(
            Paragraph::new(msg)
                .style(Style::default().fg(DIM))
                .block(block),
            chunks[1],
        );
        return;
    }
    let rows = app.search.results.iter().map(|&i| {
        let a = &app.assets[i];
        let mark = if app.in_watchlist(&a.symbol) {
            "★"
        } else {
            " "
        };
        Row::new(vec![
            Cell::from(mark).style(Style::default().fg(ACCENT)),
            Cell::from(a.symbol.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(short_name(&a.name)),
            Cell::from(a.exchange.clone()).style(Style::default().fg(DIM)),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(8),
            Constraint::Min(20),
            Constraint::Length(8),
        ],
    )
    .block(block.title(format!(" {} match(es) ", app.search.results.len())))
    .column_spacing(2)
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = TableState::default().with_selected(Some(app.search.selected));
    frame.render_stateful_widget(table, chunks[1], &mut state);
}

// ── detail ──────────────────────────────────────────────────────────────────

fn render_detail(frame: &mut Frame, area: Rect, app: &App, d: &DetailState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // title + stats
            Constraint::Length(1), // range tabs + summary
            Constraint::Min(5),    // chart / table
        ])
        .split(area);

    // Title + quote stats.
    let snap = app.snapshots.get(&d.symbol);
    let asset = app.asset(&d.symbol);
    let mut title = vec![Span::styled(
        format!(" {} ", d.symbol),
        Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
    )];
    if let Some(a) = asset {
        title.push(Span::raw(format!("  {}", short_name(&a.name))));
        title.push(Span::styled(
            format!(" · {}", a.exchange),
            Style::default().fg(DIM),
        ));
    }
    if app.in_watchlist(&d.symbol) {
        title.push(Span::styled("  ★ watchlist", Style::default().fg(ACCENT)));
    }
    let (last, chg, pct, color) = quote_cells(snap);
    let price_line = Line::from(vec![
        Span::styled(
            format!(" {last} "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{chg} ({pct})"), Style::default().fg(color)),
        Span::styled(
            snap.and_then(|s| s.latest_trade.as_ref())
                .map(|t| {
                    format!(
                        "   last trade {}",
                        local(t.time).format("%b %-d %-I:%M:%S %p")
                    )
                })
                .unwrap_or_default(),
            Style::default().fg(DIM),
        ),
    ]);
    let stats_line = Line::from(stat_spans(snap));
    frame.render_widget(
        Paragraph::new(vec![Line::from(title), price_line, stats_line]).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(DIM)),
        ),
        chunks[0],
    );

    // Range tabs + range summary.
    let selected = HistoryRange::ALL
        .iter()
        .position(|r| *r == d.range)
        .unwrap_or(0);
    let tab_titles: Vec<String> = HistoryRange::ALL
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{} {}", i + 1, r.label()))
        .collect();
    let tabs_w: u16 = tab_titles
        .iter()
        .map(|t| t.chars().count() as u16 + 3)
        .sum();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(tabs_w), Constraint::Min(10)])
        .split(chunks[1]);
    frame.render_widget(
        Tabs::new(tab_titles)
            .select(selected)
            .style(Style::default().fg(DIM))
            .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        cols[0],
    );
    frame.render_widget(
        Paragraph::new(range_summary(d)).alignment(Alignment::Right),
        cols[1],
    );

    // Body.
    let body_block = Block::default().borders(Borders::ALL);
    let body = chunks[2];
    if let Some(err) = &d.error {
        frame.render_widget(
            Paragraph::new(format!("could not load history: {err}"))
                .style(Style::default().fg(Color::Yellow))
                .block(body_block),
            body,
        );
        return;
    }
    match d.bars() {
        None => {
            frame.render_widget(
                Paragraph::new("loading history…")
                    .style(Style::default().fg(DIM))
                    .block(body_block),
                body,
            );
        }
        Some(bars) if bars.is_empty() => {
            frame.render_widget(
                Paragraph::new("no bars for this range")
                    .style(Style::default().fg(DIM))
                    .block(body_block),
                body,
            );
        }
        Some(bars) => match d.view {
            DetailView::Candles => render_candles(frame, body, d.range, bars, snap),
            DetailView::Line => render_chart(frame, body, d.range, bars, snap),
            DetailView::Table => render_bar_table(frame, body, d.range, bars, d.scroll),
        },
    }
}

fn stat_spans(snap: Option<&Snapshot>) -> Vec<Span<'static>> {
    let Some(s) = snap else {
        return vec![Span::styled(" loading quote…", Style::default().fg(DIM))];
    };
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut push = |label: &str, value: String| {
        out.push(Span::styled(format!(" {label} "), Style::default().fg(DIM)));
        out.push(Span::raw(format!("{value} ")));
    };
    if let Some(b) = &s.daily_bar {
        push("open", fmt_price(b.open));
        push("high", fmt_price(b.high));
        push("low", fmt_price(b.low));
        push("vol", fmt_volume(b.volume));
        if let Some(vw) = b.vwap {
            push("vwap", fmt_price(vw));
        }
    }
    if let Some(p) = s.prev_close() {
        push("prev", fmt_price(p));
    }
    if let Some(q) = &s.latest_quote {
        if q.bid > 0.0 && q.ask > 0.0 {
            push(
                "bid/ask",
                format!("{} × {}", fmt_price(q.bid), fmt_price(q.ask)),
            );
        }
    }
    out
}

fn range_summary(d: &DetailState) -> Line<'static> {
    let Some(bars) = d.bars().filter(|b| !b.is_empty()) else {
        return Line::from(Span::styled(
            if d.is_loading() { "loading… " } else { "" },
            Style::default().fg(DIM),
        ));
    };
    let first = bars.first().unwrap().open;
    let last = bars.last().unwrap().close;
    let hi = bars
        .iter()
        .map(|b| b.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let lo = bars.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    let pct = if first != 0.0 {
        (last - first) / first * 100.0
    } else {
        0.0
    };
    Line::from(vec![
        Span::styled(format!("{} ", d.range.label()), Style::default().fg(DIM)),
        Span::styled(
            format!("{pct:+.2}%"),
            Style::default().fg(change_color(Some(pct))),
        ),
        Span::styled(
            format!(
                "  hi {}  lo {}  {} bars ",
                fmt_price(hi),
                fmt_price(lo),
                bars.len()
            ),
            Style::default().fg(DIM),
        ),
    ])
}

fn bar_time_label(range: HistoryRange, t: DateTime<Utc>) -> String {
    let l = local(t);
    match range {
        HistoryRange::Day1 => l.format("%-I:%M %p").to_string(),
        HistoryRange::Day5 | HistoryRange::Month1 => l.format("%b %-d %-I:%M%P").to_string(),
        // Daily+ bars are stamped at midnight ET; label by the exchange date.
        _ => wickd_core::equities::eastern_date(t)
            .format("%b %-d %Y")
            .to_string(),
    }
}

fn render_candles(
    frame: &mut Frame,
    area: Rect,
    range: HistoryRange,
    bars: &[Bar],
    snap: Option<&Snapshot>,
) {
    let reference = (range == HistoryRange::Day1)
        .then(|| snap.and_then(Snapshot::prev_close))
        .flatten();
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    let k = candles::bars_per_candle(bars, reference, inner.width, fmt_price);
    let mut title = format!(" candles · {}", range.timeframe());
    if k > 1 {
        title.push_str(&format!(" × {k}"));
    }
    if let Some(p) = reference {
        title.push_str(&format!(" · prev close {}", fmt_price(p)));
    }
    title.push(' ');
    frame.render_widget(
        block.title(Span::styled(title, Style::default().fg(DIM))),
        area,
    );
    let label = |i: usize| {
        bars.get(i)
            .map(|b| bar_time_label(range, b.time))
            .unwrap_or_default()
    };
    frame.render_widget(
        CandleChart {
            bars,
            reference,
            fmt_price,
            time_label: &label,
            up: UP,
            down: DOWN,
            dim: DIM,
        },
        inner,
    );
}

fn render_chart(
    frame: &mut Frame,
    area: Rect,
    range: HistoryRange,
    bars: &[Bar],
    snap: Option<&Snapshot>,
) {
    // X is the bar index, not time, so overnight/weekend gaps don't flatten
    // the line into long horizontal runs.
    let points: Vec<(f64, f64)> = bars
        .iter()
        .enumerate()
        .map(|(i, b)| (i as f64, b.close))
        .collect();
    let mut lo = bars.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    let mut hi = bars
        .iter()
        .map(|b| b.high)
        .fold(f64::NEG_INFINITY, f64::max);

    // On 1D, draw the prior close as a reference line (like most tickers).
    let prev_close = (range == HistoryRange::Day1)
        .then(|| snap.and_then(Snapshot::prev_close))
        .flatten();
    if let Some(p) = prev_close {
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let pad = ((hi - lo) * 0.05).max(hi.abs() * 0.001);
    let (y0, y1) = (lo - pad, hi + pad);
    let x1 = (points.len().saturating_sub(1)).max(1) as f64;

    let up = bars.last().map(|b| b.close).unwrap_or(0.0)
        >= prev_close.unwrap_or_else(|| bars.first().map(|b| b.open).unwrap_or(0.0));
    let line_color = if up { UP } else { DOWN };

    let ref_points: Vec<(f64, f64)> = prev_close
        .map(|p| vec![(0.0, p), (x1, p)])
        .unwrap_or_default();

    let mut datasets = vec![];
    if !ref_points.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(DIM))
                .data(&ref_points),
        );
    }
    datasets.push(
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(line_color))
            .data(&points),
    );

    let n = bars.len();
    let x_labels: Vec<Span> = [0, n / 2, n - 1]
        .iter()
        .map(|&i| {
            Span::styled(
                bar_time_label(range, bars[i].time),
                Style::default().fg(DIM),
            )
        })
        .collect();
    let y_labels: Vec<Span> = [y0, (y0 + y1) / 2.0, y1]
        .iter()
        .map(|&v| Span::styled(fmt_price(v), Style::default().fg(DIM)))
        .collect();

    let title = match prev_close {
        Some(p) => format!(" line · prev close {} ", fmt_price(p)),
        None => " line · close ".to_string(),
    };
    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title, Style::default().fg(DIM))),
        )
        .x_axis(Axis::default().bounds([0.0, x1]).labels(x_labels))
        .y_axis(Axis::default().bounds([y0, y1]).labels(y_labels));
    frame.render_widget(chart, area);
}

fn render_bar_table(
    frame: &mut Frame,
    area: Rect,
    range: HistoryRange,
    bars: &[Bar],
    scroll: usize,
) {
    let header = header_row(
        &["TIME"],
        &["OPEN", "HIGH", "LOW", "CLOSE", "CHG%", "VOLUME"],
        &[],
    );
    // Newest first; change is vs the previous (older) bar's close.
    let rows: Vec<Row> = bars
        .iter()
        .enumerate()
        .rev()
        .map(|(i, b)| {
            let pct = (i > 0 && bars[i - 1].close != 0.0)
                .then(|| (b.close - bars[i - 1].close) / bars[i - 1].close * 100.0);
            let right = |s: String| Cell::from(Line::from(s).alignment(Alignment::Right));
            Row::new(vec![
                Cell::from(bar_time_label(range, b.time)),
                right(fmt_price(b.open)),
                right(fmt_price(b.high)),
                right(fmt_price(b.low)),
                right(fmt_price(b.close)),
                right(pct.map(|p| format!("{p:+.2}%")).unwrap_or_default())
                    .style(Style::default().fg(change_color(pct))),
                right(fmt_volume(b.volume)),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
            Constraint::Length(9),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(Span::styled(
        format!(" {} bars · newest first ", range.timeframe()),
        Style::default().fg(DIM),
    )))
    .column_spacing(2)
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = TableState::default().with_selected(Some(scroll));
    frame.render_stateful_widget(table, area, &mut state);
}

// ── help ────────────────────────────────────────────────────────────────────

fn help_section(title: &str, ctx: Ctx) -> Vec<Line<'static>> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let key = Style::default().fg(ACCENT);
    let row = |k: String, label: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {k:<14}"), key),
            Span::raw(label),
        ])
    };
    let mut out = vec![Line::from(Span::styled(title.to_string(), bold))];
    for Entry { keys, label } in keymap::bare_entries(ctx) {
        out.push(row(keys, label));
    }
    for (p, name) in keymap::prefixes_in(ctx) {
        out.push(Line::from(Span::styled(
            format!("  {p} · {name}"),
            Style::default().fg(DIM),
        )));
        for Entry { keys, label } in keymap::prefix_entries(ctx, p) {
            out.push(row(format!("{p} {keys}"), label));
        }
    }
    out
}

fn render_help(frame: &mut Frame, area: Rect) {
    let mut left = help_section("watchlist", Ctx::Watchlist);
    left.push(Line::from(""));
    left.push(Line::from(Span::styled(
        "search",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for (k, l) in [
        ("type", "filter by ticker or name"),
        ("↑ ↓", "select"),
        ("enter", "open symbol"),
        ("tab", "add / remove from watchlist"),
        ("esc", "back"),
    ] {
        left.push(Line::from(vec![
            Span::styled(format!("  {k:<14}"), Style::default().fg(ACCENT)),
            Span::raw(l),
        ]));
    }
    let mut right = help_section("symbol", Ctx::Detail);
    right.push(Line::from(""));
    right.push(Line::from(Span::styled(
        "history is all-exchange data, 15 min behind on the free plan",
        Style::default().fg(DIM),
    )));
    right.push(Line::from(Span::styled(
        "any key closes",
        Style::default().fg(DIM),
    )));

    let w = 96.min(area.width.saturating_sub(2));
    let h = (left.len().max(right.len()) as u16 + 2).min(area.height.saturating_sub(2));
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, rect);
    let block = Block::default().borders(Borders::ALL).title(" keys ");
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(inner);
    frame.render_widget(Paragraph::new(left), cols[0]);
    frame.render_widget(Paragraph::new(right), cols[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use wickd_core::equities::{Asset, MarketClock, Quote, Trade};

    fn bar_at(t: &str, c: f64) -> Bar {
        Bar {
            time: t.parse().unwrap(),
            open: c - 0.5,
            high: c + 1.0,
            low: c - 1.0,
            close: c,
            volume: 1.2e6,
            vwap: Some(c),
        }
    }

    fn sample_app() -> App {
        let mut app = App::new(vec!["SPY".into(), "NVDA".into(), "XYZ".into()], "IEX");
        app.set_assets(vec![
            Asset {
                symbol: "NVDA".into(),
                name: "NVIDIA Corporation Common Stock".into(),
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
        let snap = |last: f64, prev: f64| Snapshot {
            latest_trade: Some(Trade {
                time: "2026-11-06T20:59:58Z".parse().unwrap(),
                price: last,
                size: 100.0,
            }),
            latest_quote: Some(Quote {
                time: "2026-11-06T20:59:59Z".parse().unwrap(),
                bid: last - 0.01,
                bid_size: 2.0,
                ask: last + 0.01,
                ask_size: 3.0,
            }),
            minute_bar: None,
            daily_bar: Some(bar_at("2026-11-06T05:00:00Z", last)),
            prev_daily_bar: Some(bar_at("2026-11-05T05:00:00Z", prev)),
        };
        app.snapshots.insert("SPY".into(), snap(681.25, 684.10));
        app.snapshots.insert("NVDA".into(), snap(188.40, 182.00));
        app.sparks.insert(
            "SPY".into(),
            (0..78).map(|i| 684.0 - f64::from(i) * 0.04).collect(),
        );
        app.sparks.insert(
            "NVDA".into(),
            (0..78)
                .map(|i| 182.0 + (f64::from(i) * 0.3).sin() + f64::from(i) * 0.08)
                .collect(),
        );
        app.clock = Some(MarketClock {
            timestamp: "2026-11-08T17:00:00Z".parse().unwrap(),
            is_open: false,
            next_open: "2026-11-09T14:30:00Z".parse().unwrap(),
            next_close: "2026-11-09T21:00:00Z".parse().unwrap(),
        });
        app
    }

    fn draw(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render(f, app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn press(app: &mut App, code: KeyCode) -> Vec<crate::tui::app::Action> {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn renders_every_screen() {
        let mut app = sample_app();
        let wl = draw(&app, 130, 14);
        assert!(
            wl.contains("NVIDIA Corporation") && !wl.contains("Common Stock"),
            "{wl}"
        );
        assert!(wl.contains("+6.40") && wl.contains("+3.52%"), "{wl}");
        assert!(wl.contains("market closed"), "{wl}");
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{wl}\n");
        }

        press(&mut app, KeyCode::Char('/'));
        for c in "nvi".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let se = draw(&app, 100, 10);
        assert!(se.contains("NVDA") && se.contains("1 match"), "{se}");
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{se}\n");
        }

        press(&mut app, KeyCode::Enter);
        let loading = draw(&app, 120, 24);
        assert!(loading.contains("loading history"), "{loading}");
        let bars: Vec<Bar> = (0..78)
            .map(|i| {
                bar_at(
                    &format!(
                        "2026-11-06T{:02}:{:02}:00Z",
                        14 + (30 + i * 5) / 60,
                        (30 + i * 5) % 60
                    ),
                    182.0 + f64::from(i) * 0.08,
                )
            })
            .collect();
        app.set_history("NVDA", HistoryRange::Day1, Ok(bars));
        let chart = draw(&app, 120, 24);
        assert!(
            chart.contains("candles · 5Min · prev close 182.00")
                && chart.contains("1 1D")
                && chart.contains("vol"),
            "{chart}"
        );
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{chart}\n");
        }

        press(&mut app, KeyCode::Tab);
        let table = draw(&app, 120, 14);
        assert!(
            table.contains("newest first") && table.contains("188.16"),
            "{table}"
        );
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{table}\n");
        }

        press(&mut app, KeyCode::Char('?'));
        let help = draw(&app, 120, 30);
        assert!(
            help.contains("chart ↔ table") && help.contains("t · time range"),
            "{help}"
        );
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{help}\n");
        }
    }

    #[test]
    fn key_panel_lists_second_keys_while_pending() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Char('w'));
        let wl = draw(&app, 100, 16);
        assert!(
            wl.contains("w · watchlist") && wl.contains("move down"),
            "{wl}"
        );
        assert!(wl.contains("watchlist… · esc cancels"), "{wl}");
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{wl}\n");
        }
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Enter); // open SPY
        press(&mut app, KeyCode::Char('t'));
        let d = draw(&app, 100, 20);
        assert!(
            d.contains("t · time range") && d.contains("6 months"),
            "{d}"
        );
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{d}\n");
        }
        // g + digits shows the number being typed.
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('w')); // back to watchlist
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('1'));
        let g = draw(&app, 100, 12);
        assert!(g.contains("g 1") && g.contains("then g or enter"), "{g}");
    }

    /// A wiggly random-walk session: mixed up/down candles of varying size
    /// and volume, so the candle view is exercised like real data.
    fn walk(n: usize, start: f64) -> Vec<Bar> {
        let mut price = start;
        (0..n)
            .map(|i| {
                let f = i as f64;
                let open = price;
                let close = open + (f * 0.7).sin() * 0.6 + (f * 0.13).cos() * 0.25;
                price = close;
                Bar {
                    time: chrono::Utc::now() - chrono::Duration::minutes(5 * (n - i) as i64),
                    open,
                    high: open.max(close) + 0.15 + (f * 1.3).sin().abs() * 0.3,
                    low: open.min(close) - 0.15 - (f * 0.9).cos().abs() * 0.3,
                    close,
                    volume: 1e5 * (1.0 + (f * 0.37).sin().abs() * 4.0),
                    vwap: None,
                }
            })
            .collect()
    }

    #[test]
    fn candles_are_default_and_line_and_table_switch() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Enter); // SPY
        app.set_history("SPY", HistoryRange::Day1, Ok(walk(78, 684.0)));
        let c = draw(&app, 120, 26);
        assert!(c.contains("candles · 5Min") && c.contains("vol"), "{c}");
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{c}\n");
        }
        // Many bars on a narrow screen are merged, and the title says so.
        press(&mut app, KeyCode::Char('6'));
        app.set_history("SPY", HistoryRange::Year5, Ok(walk(260, 400.0)));
        let y5 = draw(&app, 90, 20);
        assert!(y5.contains("candles · 1Week × 4"), "{y5}");
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{y5}\n");
        }
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('l'));
        assert!(draw(&app, 90, 20).contains("line · close"));
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        let mut app = sample_app();
        draw(&app, 20, 6);
        press(&mut app, KeyCode::Enter);
        app.set_history(
            "SPY",
            HistoryRange::Day1,
            Ok(vec![bar_at("2026-11-06T15:00:00Z", 1.0)]),
        );
        draw(&app, 20, 6);
        press(&mut app, KeyCode::Char('?'));
        draw(&app, 10, 4);
    }

    #[test]
    fn price_and_change_formatting() {
        assert_eq!(fmt_price(271.456), "271.46");
        assert_eq!(fmt_price(0.12345), "0.1235");
        assert_eq!(fmt_change(3.5), "+3.50");
        assert_eq!(fmt_change(-0.0123), "-0.0123");
        assert_eq!(fmt_change(0.0), "+0.00");
    }

    #[test]
    fn volume_formatting() {
        assert_eq!(fmt_volume(950.0), "950");
        assert_eq!(fmt_volume(12_345.0), "12.3K");
        assert_eq!(fmt_volume(45_600_000.0), "45.6M");
        assert_eq!(fmt_volume(1_230_000_000.0), "1.23B");
    }

    #[test]
    fn sparkline_scales_and_resamples() {
        assert_eq!(sparkline(&[1.0, 2.0, 3.0], 10), "▁▅█");
        assert_eq!(sparkline(&[5.0, 5.0], 4), "▄▄");
        assert_eq!(sparkline(&[], 4), "");
        let long: Vec<f64> = (0..100).map(f64::from).collect();
        let s = sparkline(&long, 10);
        assert_eq!(s.chars().count(), 10);
        assert!(s.ends_with('█') && s.starts_with('▁'));
    }

    #[test]
    fn names_are_shortened() {
        assert_eq!(short_name("Apple Inc. Common Stock"), "Apple Inc.");
        assert_eq!(
            short_name("SPDR S&P 500 ETF Trust"),
            "SPDR S&P 500 ETF Trust"
        );
    }
}
