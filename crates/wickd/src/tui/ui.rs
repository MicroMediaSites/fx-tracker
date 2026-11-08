//! Rendering for `wickd tui`. Pure functions of [`App`]; all times are shown
//! in the machine's local zone (Mountain for Matt).

use chrono::{DateTime, Local, Utc};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Tabs};
use ratatui::Frame;

use wickd_core::equities::{Bar, HistoryRange, Snapshot};

use super::app::{App, DetailState, DetailView, Mode};
use super::chart::{ChartWidget, Palette, Style2, Zoom};
use super::inplay::{self, Tag};
use super::keymap::{self, Ctx, Entry};

// Same teal/red as the wickd desktop app (TradingView/OANDA palette).
const UP: Color = Color::Rgb(8, 153, 129);
const DOWN: Color = Color::Rgb(242, 54, 69);
const GRID: Color = Color::Rgb(48, 54, 66);
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
    if c.abs() >= 0.01 || c == 0.0 {
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
        Mode::InPlay => render_inplay(frame, chunks[1], app),
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
    if let Some(f) = &app.find {
        let n = app.find_matches().len();
        let line = Line::from(vec![
            Span::styled(
                "find ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(f.query.clone()),
            Span::styled("▏", Style::default().fg(ACCENT)),
            Span::styled(
                format!(
                    "  {n} match{} · ↑↓ pick · enter jump · esc cancel",
                    if n == 1 { "" } else { "es" }
                ),
                Style::default().fg(DIM),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
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
    // While `g f` is active only matching rows show.
    let shown: Vec<&String> = match &app.find {
        Some(_) => app
            .find_matches()
            .into_iter()
            .map(|i| &app.watchlist[i])
            .collect(),
        None => app.watchlist.iter().collect(),
    };
    let rows = shown.into_iter().map(|sym| {
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
    let sel = app.find.as_ref().map_or(app.selected, |f| f.selected);
    let mut state = TableState::default().with_selected(Some(sel));
    frame.render_stateful_widget(table, area, &mut state);
}

// ── search ──────────────────────────────────────────────────────────────────

// ── in play ─────────────────────────────────────────────────────────────────

fn fmt_pct(p: Option<f64>) -> (String, Color) {
    match p {
        Some(v) => (format!("{v:+.1}%"), change_color(Some(v))),
        None => ("—".into(), DIM),
    }
}

fn render_inplay(frame: &mut Frame, area: Rect, app: &App) {
    let ip = &app.inplay;
    let now = Utc::now();
    let wide = area.width >= 120;
    let cols = Layout::default()
        .direction(if wide {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints(if wide {
            [Constraint::Percentage(62), Constraint::Percentage(38)]
        } else {
            [Constraint::Percentage(60), Constraint::Percentage(40)]
        })
        .split(area);

    // Status line: session, sort, filter, freshness, degraded sources.
    let phase = inplay::session_phase(app.clock.as_ref(), now);
    let mut status = vec![
        Span::styled(format!(" {phase}"), Style::default().fg(ACCENT)),
        Span::styled(
            format!(" · sorted by {}", ip.sort.label()),
            Style::default().fg(DIM),
        ),
        Span::styled(
            match ip.min_price {
                Some(m) => format!(" · price ≥ ${m:.0}"),
                None => " · all prices".into(),
            },
            Style::default().fg(DIM),
        ),
    ];
    match (&ip.updated, ip.loading) {
        (_, true) => status.push(Span::styled(" · refreshing…", Style::default().fg(DIM))),
        (Some(t), false) => status.push(Span::styled(
            format!(" · updated {}", t.format("%-I:%M %p")),
            Style::default().fg(DIM),
        )),
        _ => {}
    }
    let mut header = vec![Line::from(status)];
    if phase == "pre-market" {
        header.push(Line::from(Span::styled(
            " chg% is the gap vs yesterday's close; top movers still show yesterday until the open",
            Style::default().fg(DIM),
        )));
    }
    for n in ip.notes.iter().chain(ip.error.iter()) {
        header.push(Line::from(Span::styled(
            format!(" {n}"),
            Style::default().fg(Color::Yellow),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GRID))
        .title(" in play today ");
    let inner = block.inner(cols[0]);
    frame.render_widget(block, cols[0]);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header.len() as u16), Constraint::Min(1)])
        .split(inner);
    frame.render_widget(Paragraph::new(header), parts[0]);

    let rows_vis: Vec<&inplay::Row> = match &app.find {
        Some(_) => {
            let all = ip.visible();
            app.find_matches().into_iter().map(|i| all[i]).collect()
        }
        None => ip.visible(),
    };
    if rows_vis.is_empty() {
        let msg = if ip.loading {
            "scanning movers, most active and your watchlist…"
        } else {
            "nothing to show yet — r to refresh"
        };
        frame.render_widget(
            Paragraph::new(msg).style(Style::default().fg(DIM)),
            parts[1],
        );
    } else {
        let head = header_row(
            &["SYMBOL", "NAME"],
            &["LAST", "CHG%", "FROM OPEN", "VOL", "RVOL", "NEWS"],
            &["", ""],
        );
        let rows = rows_vis.iter().map(|r| {
            let name = app
                .asset(&r.symbol)
                .map(|a| short_name(&a.name))
                .unwrap_or_default();
            let (chg, chg_c) = fmt_pct(r.chg_pct);
            let (fo, fo_c) = fmt_pct(r.from_open_pct);
            let rvol = r
                .rvol
                .map(|v| format!("{v:.1}×"))
                .unwrap_or_else(|| "—".into());
            let rvol_style = match r.rvol {
                Some(v) if v >= 2.0 => Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                Some(_) => Style::default(),
                None => Style::default().fg(DIM),
            };
            let news = r
                .latest_news
                .map(|t| inplay::age(t, now))
                .unwrap_or_default();
            let tags: String = r.tags.iter().map(|t| t.glyph()).collect();
            let right = |s: String| Cell::from(Line::from(s).alignment(Alignment::Right));
            let tag_color = if r.tags.contains(&Tag::Gainer) {
                UP
            } else if r.tags.contains(&Tag::Loser) {
                DOWN
            } else {
                DIM
            };
            Row::new(vec![
                Cell::from(r.symbol.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
                Cell::from(name),
                right(r.last.map(fmt_price).unwrap_or_else(|| "—".into())),
                right(chg).style(Style::default().fg(chg_c)),
                right(fo).style(Style::default().fg(fo_c)),
                right(r.volume.map(fmt_volume).unwrap_or_else(|| "—".into())),
                right(rvol).style(rvol_style),
                right(news).style(Style::default().fg(ACCENT)),
                Cell::from(""),
                Cell::from(tags).style(Style::default().fg(tag_color)),
            ])
        });
        let widths = [
            Constraint::Length(7),
            Constraint::Min(10),
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Length(9),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Length(0),
            Constraint::Length(4),
        ];
        let table = Table::new(rows, widths)
            .header(head)
            .column_spacing(1)
            .row_highlight_style(
                Style::default()
                    .bg(Color::Rgb(44, 52, 66))
                    .add_modifier(Modifier::BOLD),
            );
        let sel = app.find.as_ref().map_or(ip.selected, |f| f.selected);
        let mut state = TableState::default().with_selected(Some(sel));
        frame.render_stateful_widget(table, parts[1], &mut state);
    }

    // Headlines for the selected symbol.
    let sel = ip.selected_symbol();
    let mut title = match &sel {
        Some(s) => format!(" {s} · headlines "),
        None => " headlines ".into(),
    };
    if ip.focus_news {
        title.push_str("· j/k pick · enter open · tab back ");
    } else {
        title.push_str("· tab to pick ");
    }
    if ip.reader_open {
        title.push_str("· reader open: ctrl+shift+←/→ in the window ");
    }
    let nblock = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GRID))
        .title(title);
    let ninner = nblock.inner(cols[1]);
    frame.render_widget(nblock, cols[1]);
    let width = ninner.width.saturating_sub(1) as usize;
    let mut lines: Vec<Line> = Vec::new();
    let mut picked_line = 0usize;
    match sel.as_ref().and_then(|s| ip.news.get(s)) {
        None if sel.is_some() => lines.push(Line::from(Span::styled(
            "loading…",
            Style::default().fg(DIM),
        ))),
        None => {}
        Some(Err(e)) => lines.push(Line::from(Span::styled(
            format!("headlines unavailable: {e}"),
            Style::default().fg(Color::Yellow),
        ))),
        Some(Ok(items)) if items.is_empty() => lines.push(Line::from(Span::styled(
            "no headlines in the last few days — a move without news is often just the market",
            Style::default().fg(DIM),
        ))),
        Some(Ok(items)) => {
            for (i, n) in items.iter().enumerate() {
                let picked = ip.focus_news && i == ip.news_cursor;
                if picked {
                    picked_line = lines.len();
                }
                let hl = if picked {
                    Style::default().bg(Color::Rgb(44, 52, 66))
                } else {
                    Style::default()
                };
                let when = local(n.created_at);
                let stamp = if wickd_core::equities::eastern_date(n.created_at)
                    == wickd_core::equities::eastern_date(now)
                {
                    when.format("%-I:%M %p").to_string()
                } else {
                    when.format("%a %b %-d").to_string()
                };
                let link = if n.url.as_deref().is_some_and(|u| !u.trim().is_empty()) {
                    "  ↗"
                } else {
                    "  (no link)"
                };
                lines.push(
                    Line::from(vec![
                        Span::styled(if picked { "▶ " } else { "" }, Style::default().fg(ACCENT)),
                        Span::styled(stamp, Style::default().fg(ACCENT)),
                        Span::styled(
                            format!("  {}", inplay::age(n.created_at, now)),
                            Style::default().fg(DIM),
                        ),
                        Span::styled(link, Style::default().fg(DIM)),
                    ])
                    .style(hl),
                );
                for l in wrap(&n.headline, width) {
                    lines.push(
                        Line::from(Span::styled(
                            l,
                            Style::default().add_modifier(Modifier::BOLD),
                        ))
                        .style(hl),
                    );
                }
                if !n.summary.trim().is_empty() {
                    for l in wrap(n.summary.trim(), width).into_iter().take(3) {
                        lines.push(Line::from(Span::styled(l, Style::default().fg(DIM))));
                    }
                }
                lines.push(Line::from(""));
            }
        }
    }
    // Keep the picked headline on screen.
    let room = ninner.height.saturating_sub(4) as usize;
    let scroll = picked_line.saturating_sub(room) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), ninner);
}

/// Greedy word wrap to `width` columns.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let w = word.chars().count();
        let c = cur.chars().count();
        if c > 0 && c + 1 + w > width {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

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
            DetailView::Candles => {
                render_chart_view(frame, body, d, snap, Style2::Candles, app.history_delayed)
            }
            DetailView::Line => {
                render_chart_view(frame, body, d, snap, Style2::Line, app.history_delayed)
            }
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

/// Readout timestamp for a candle starting at `t`.
fn readout_time(range: HistoryRange, t: DateTime<Utc>) -> String {
    let d = wickd_core::equities::eastern_date(t);
    match range {
        HistoryRange::Day1 | HistoryRange::Day5 | HistoryRange::Month1 => {
            local(t).format("%a %b %-d %-I:%M %p").to_string()
        }
        HistoryRange::Month6 | HistoryRange::Year1 => d.format("%a %b %-d %Y").to_string(),
        HistoryRange::Year5 => d.format("wk of %b %-d %Y").to_string(),
        HistoryRange::Max => d.format("%b %Y").to_string(),
    }
}

/// Time-axis label when `t` starts a new period vs `prev` (hour on 1D, day
/// on 5D/1M, month on 6M/1Y, quarter on 5Y, year on MAX). `true` = strong
/// boundary, drawn as a separator.
fn axis_boundary(
    range: HistoryRange,
    prev: Option<DateTime<Utc>>,
    t: DateTime<Utc>,
) -> Option<(String, bool)> {
    use chrono::{Datelike, Timelike};
    let ed = wickd_core::equities::eastern_date;
    match range {
        HistoryRange::Day1 => {
            let (lt, lp) = (local(t), prev.map(local));
            (lp.is_none_or(|p| p.hour() != lt.hour()))
                .then(|| (lt.format("%-I %p").to_string(), false))
        }
        HistoryRange::Day5 | HistoryRange::Month1 => {
            if prev.is_none_or(|p| ed(p) != ed(t)) {
                Some((ed(t).format("%b %-d").to_string(), true))
            } else if prev.is_some_and(|p| local(p).hour() != local(t).hour()) {
                Some((local(t).format("%-I %p").to_string(), false))
            } else {
                None
            }
        }
        HistoryRange::Month6 | HistoryRange::Year1 => {
            let (d, p) = (ed(t), prev.map(ed));
            if p.is_none_or(|p| p.year() != d.year()) {
                Some((d.format("%Y").to_string(), true))
            } else if p.is_some_and(|p| p.month() != d.month()) {
                Some((d.format("%b").to_string(), false))
            } else {
                None
            }
        }
        HistoryRange::Year5 => {
            let (d, p) = (ed(t), prev.map(ed));
            let q = |d: chrono::NaiveDate| (d.year(), (d.month() - 1) / 3);
            if p.is_none_or(|p| p.year() != d.year()) {
                Some((d.format("%Y").to_string(), true))
            } else if p.is_some_and(|p| q(p) != q(d)) {
                Some((d.format("%b").to_string(), false))
            } else {
                None
            }
        }
        HistoryRange::Max => {
            let (d, p) = (ed(t), prev.map(ed));
            (p.is_none_or(|p| p.year() != d.year())).then(|| (d.format("%Y").to_string(), false))
        }
    }
}

fn render_chart_view(
    frame: &mut Frame,
    area: Rect,
    d: &DetailState,
    snap: Option<&Snapshot>,
    style: Style2,
    delayed: bool,
) {
    let range = d.range;
    let reference = (range == HistoryRange::Day1)
        .then(|| snap.and_then(Snapshot::prev_close))
        .flatten();
    let Some((candles, extended)) = d.candles() else {
        return;
    };
    let zoom = d.chart.zoom_level();
    let mut title = format!(
        " {} · {}",
        if style == Style2::Candles {
            "candles"
        } else {
            "line"
        },
        range.timeframe()
    );
    let merged = candles.iter().map(|c| c.bars).max().unwrap_or(1);
    if d.chart.fit {
        if merged > 1 {
            title.push_str(&format!(" × {merged}"));
        }
        title.push_str(" · fit");
    } else if let Zoom::Merge(k) = zoom {
        title.push_str(&format!(" × {k}"));
    }
    if range.intraday() {
        title.push_str(if extended {
            " · extended hours"
        } else {
            " · regular hours"
        });
    }
    if let Some(p) = reference {
        title.push_str(&format!(" · prev close {}", fmt_price(p)));
    }
    if delayed {
        title.push_str(" · 15m delayed");
    }
    title.push_str(" · −/+ zoom  h/l cursor  H/L pan ");
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GRID))
        .title(Span::styled(title, Style::default().fg(DIM)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let time_label = move |t: DateTime<Utc>| readout_time(range, t);
    let boundary = move |p: Option<DateTime<Utc>>, t: DateTime<Utc>| axis_boundary(range, p, t);
    frame.render_widget(
        ChartWidget {
            candles: &candles,
            view: &d.chart,
            style,
            reference,
            last_price: snap.and_then(Snapshot::last),
            delayed,
            fmt_price,
            fmt_volume,
            time_label: &time_label,
            boundary: &boundary,
            palette: Palette {
                up: UP,
                down: DOWN,
                dim: DIM,
                grid: GRID,
                text: Color::Reset,
            },
        },
        inner,
    );
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
    left.extend(help_section("in play", Ctx::InPlay));
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
            chart.contains("regular hours · prev close 182.00")
                && chart.contains("· fit")
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
        let help = draw(&app, 120, 50);
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
        assert!(
            c.contains("candles · 5Min") && c.contains("vol") && c.contains(" O "),
            "{c}"
        );
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{c}\n");
        }
        // Many bars on a narrow screen are merged, and the title says so.
        press(&mut app, KeyCode::Char('6'));
        app.set_history("SPY", HistoryRange::Year5, Ok(walk(260, 400.0)));
        // Fit shows the whole range, merging to fit (260 weeks in ~80 columns).
        // (The first frame measures the plot width; fit settles on the next.)
        draw(&app, 90, 20);
        let fit = draw(&app, 90, 20);
        assert!(fit.contains("candles · 1Week × 4 · fit"), "{fit}");
        // Zooming out from fit merges further.
        press(&mut app, KeyCode::Char('-'));
        let y5 = draw(&app, 90, 20);
        assert!(y5.contains("candles · 1Week × 8"), "{y5}");
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{y5}\n");
        }
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('l'));
        assert!(draw(&app, 90, 20).contains("line · 1Week"));
    }

    // ── visual snapshots (opt-in) ────────────────────────────────────────────

    /// Deterministic pseudo-random walk of 5-minute regular-session bars over
    /// `days` trading days ending 2026-11-06, with a U-shaped volume profile.
    fn session_walk(days: usize, start: f64, seed: u64) -> Vec<Bar> {
        let mut rng = seed;
        let mut next = move || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((rng >> 33) as f64 / (1u64 << 31) as f64) - 0.5
        };
        let mut out = Vec::new();
        let mut price = start;
        for day in 0..days {
            let date = chrono::NaiveDate::from_ymd_opt(2026, 11, 6).unwrap()
                - chrono::Duration::days(((days - 1 - day) * 7 / 5) as i64);
            let open_utc = date.and_hms_opt(14, 30, 0).unwrap().and_utc();
            for i in 0..78 {
                let o = price + next() * 0.4;
                let c = o + next() * 1.4 + (i as f64 / 78.0 - 0.5) * 0.05;
                let hi = o.max(c) + next().abs() * 0.8;
                let lo = o.min(c) - next().abs() * 0.8;
                price = c;
                let u = (i as f64 / 77.0 - 0.5).abs() * 2.0; // 1 at open/close, 0 midday
                out.push(Bar {
                    time: open_utc + chrono::Duration::minutes(5 * i as i64),
                    open: o,
                    high: hi,
                    low: lo,
                    close: c,
                    volume: 20_000.0 * (1.0 + 4.0 * u * u + next().abs() * 2.0)
                        + if i == 77 { 400_000.0 } else { 0.0 },
                    vwap: None,
                });
            }
        }
        out
    }

    fn daily_walk(n: usize, start: f64, seed: u64) -> Vec<Bar> {
        let mut rng = seed;
        let mut next = move || {
            rng = rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((rng >> 33) as f64 / (1u64 << 31) as f64) - 0.5
        };
        let mut price = start;
        let end = chrono::NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        (0..n)
            .map(|i| {
                let o = price + next() * 1.5;
                let c = o + next() * 6.0 + 0.15;
                price = c;
                let d = end - chrono::Duration::days(((n - 1 - i) * 7 / 5) as i64);
                Bar {
                    time: d.and_hms_opt(5, 0, 0).unwrap().and_utc(),
                    open: o,
                    high: o.max(c) + next().abs() * 3.0,
                    low: o.min(c) - next().abs() * 3.0,
                    close: c,
                    volume: 4e6 * (1.0 + next().abs() * 3.0),
                    vwap: None,
                }
            })
            .collect()
    }

    fn css_color(c: Color, fallback: &str) -> String {
        match c {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            Color::Black => "#0b0f14".into(),
            Color::Red => "#ef4444".into(),
            Color::Green => "#22c55e".into(),
            Color::Yellow => "#eab308".into(),
            Color::Cyan => "#22d3ee".into(),
            Color::DarkGray => "#6b7280".into(),
            Color::Gray => "#9ca3af".into(),
            Color::White => "#f3f4f6".into(),
            _ => fallback.into(),
        }
    }

    /// Render a buffer as a coloured HTML page (dark terminal look).
    fn to_html(buf: &ratatui::buffer::Buffer, title: &str) -> String {
        const FG: &str = "#d1d5db";
        const BG: &str = "#151a22";
        let mut h = format!(
            "<!doctype html><meta charset=utf-8><title>{title}</title><style>\
             body{{margin:0;background:{BG}}}pre{{margin:0;padding:12px;font:13px/1.18 Menlo,monospace;color:{FG}}}\
             span{{white-space:pre;display:inline-block;width:1ch;overflow:hidden}}</style><pre>"
        );
        let a = buf.area;
        for y in 0..a.height {
            for x in 0..a.width {
                let cell = &buf[(x, y)];
                let mut fg = css_color(cell.fg, FG);
                let mut bg = css_color(cell.bg, "transparent");
                if cell.modifier.contains(Modifier::REVERSED) {
                    std::mem::swap(&mut fg, &mut bg);
                    if fg == "transparent" {
                        fg = BG.into();
                    }
                    if bg == "transparent" {
                        bg = FG.into();
                    }
                }
                let mut style = format!("color:{fg};background:{bg};");
                if cell.modifier.contains(Modifier::BOLD) {
                    style.push_str("font-weight:bold;");
                }
                if cell.modifier.contains(Modifier::DIM) {
                    style.push_str("opacity:.55;");
                }
                let sym = cell
                    .symbol()
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                h.push_str(&format!("<span style=\"{style}\">{sym}</span>"));
            }
            h.push('\n');
        }
        h.push_str("</pre>");
        h
    }

    /// `WICKD_TUI_SNAPSHOT_DIR=/some/dir cargo test -p wickd snapshot_charts -- --ignored`
    /// writes HTML renders of the chart views for visual review.
    #[test]
    #[ignore]
    fn snapshot_charts() {
        let dir = std::env::var("WICKD_TUI_SNAPSHOT_DIR").expect("set WICKD_TUI_SNAPSHOT_DIR");
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (158u16, 50u16);
        let shoot = |app: &App, name: &str| {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| render(f, app)).unwrap(); // measure
            term.draw(|f| render(f, app)).unwrap();
            let html = to_html(term.backend().buffer(), name);
            std::fs::write(format!("{dir}/{name}.html"), html).unwrap();
        };
        let mut app = sample_app();
        press(&mut app, KeyCode::Enter); // SPY
        let day = session_walk(1, 684.0, 7);
        app.snapshots
            .get_mut("SPY")
            .unwrap()
            .prev_daily_bar
            .as_mut()
            .unwrap()
            .close = 684.0;
        app.snapshots
            .get_mut("SPY")
            .unwrap()
            .latest_trade
            .as_mut()
            .unwrap()
            .price = day.last().unwrap().close;
        app.set_history("SPY", HistoryRange::Day1, Ok(day));
        shoot(&app, "1-1d-candles-default");

        for _ in 0..2 {
            press(&mut app, KeyCode::Char('+'));
        }
        for _ in 0..6 {
            press(&mut app, KeyCode::Char('h'));
        }
        shoot(&app, "2-1d-zoomed-in-cursor");

        press(&mut app, KeyCode::Char('0'));
        press(&mut app, KeyCode::Char('2'));
        app.set_history("SPY", HistoryRange::Day5, Ok(session_walk(5, 670.0, 11)));
        shoot(&app, "3-5d-day-separators");

        press(&mut app, KeyCode::Char('5'));
        app.set_history("SPY", HistoryRange::Year1, Ok(daily_walk(252, 560.0, 3)));
        shoot(&app, "4-1y-daily");

        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('l'));
        shoot(&app, "5-1y-line");

        press(&mut app, KeyCode::Esc);
        sample_inplay(&mut app);
        press(&mut app, KeyCode::Char('j'));
        shoot(&app, "6-in-play");
    }

    #[test]
    fn wrap_breaks_on_words() {
        assert_eq!(
            wrap("one two three four", 9),
            vec!["one two", "three", "four"]
        );
        assert!(wrap("", 10).is_empty());
    }

    fn sample_inplay(app: &mut App) {
        use crate::tui::inplay::Row as IpRow;
        let now = Utc::now();
        let row = |s: &str,
                   chg: f64,
                   fo: Option<f64>,
                   rvol: Option<f64>,
                   tags: Vec<Tag>,
                   news: Option<i64>| IpRow {
            symbol: s.into(),
            last: Some(40.0 + chg),
            chg_pct: Some(chg),
            from_open_pct: fo,
            volume: rvol.map(|r| r * 2e6),
            avg_volume: Some(2e6),
            rvol,
            tags,
            latest_news: news.map(|m| now - chrono::Duration::minutes(m)),
            news_count: news.map_or(0, |_| 1),
        };
        press(app, KeyCode::Char('g'));
        press(app, KeyCode::Char('i'));
        app.set_inplay(Ok((
            vec![
                row(
                    "NVDA",
                    6.4,
                    Some(2.1),
                    Some(3.4),
                    vec![Tag::Gainer, Tag::Active, Tag::Watch],
                    Some(35),
                ),
                row(
                    "SPY",
                    -0.4,
                    Some(-0.2),
                    Some(1.1),
                    vec![Tag::Active, Tag::Watch],
                    None,
                ),
                row(
                    "WDAY",
                    -11.2,
                    Some(-3.0),
                    Some(5.8),
                    vec![Tag::Loser],
                    Some(600),
                ),
                row("ABCD", 22.0, None, None, vec![Tag::Gainer], Some(90)),
            ],
            vec!["most active unavailable: not allowed on this Alpaca plan (forbidden)".into()],
        )));
        app.set_news(
            "WDAY".into(),
            Ok(vec![wickd_core::equities::alpaca::NewsItem {
                id: 1,
                headline: "Workday shares slide after the company cuts its full-year subscription revenue outlook".into(),
                summary: "The HR software maker lowered guidance citing slower deal closings in the quarter.".into(),
                created_at: now - chrono::Duration::hours(10),
                symbols: vec!["WDAY".into()],
                source: "benzinga".into(),
                url: Some("https://www.benzinga.com/news/1".into()),
            }]),
        );
    }

    #[test]
    fn find_shows_only_matches_and_a_prompt() {
        let mut app = sample_app();
        sample_inplay(&mut app);
        for c in ['g', 'f', 'w', 'd'] {
            press(&mut app, KeyCode::Char(c));
        }
        let t = draw(&app, 150, 20);
        assert!(t.contains("find wd") && t.contains("1 match"), "{t}");
        assert!(t.contains("WDAY") && !t.contains("NVDA "), "{t}");
    }

    #[test]
    fn in_play_screen_renders_rows_notes_and_headlines() {
        let mut app = sample_app();
        sample_inplay(&mut app);
        // Default sort: biggest move → ABCD (+22) then WDAY (−11.2).
        press(&mut app, KeyCode::Char('j')); // WDAY
        let t = draw(&app, 150, 24);
        assert!(
            t.contains("in play today") && t.contains("sorted by biggest move"),
            "{t}"
        );
        assert!(
            t.contains("WDAY") && t.contains("-11.2%") && t.contains("5.8×"),
            "{t}"
        );
        assert!(t.contains("most active unavailable"), "{t}");
        assert!(
            t.contains("WDAY · headlines") && t.contains("Workday shares slide"),
            "{t}"
        );
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{t}\n");
        }
        // Narrow terminals stack the headlines below and don't panic.
        draw(&app, 80, 30);
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
        assert_eq!(fmt_change(-0.6445), "-0.64");
        assert_eq!(fmt_change(-0.0012), "-0.0012");
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
