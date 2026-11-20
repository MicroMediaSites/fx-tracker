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
use super::chart::{self, ChartWidget, Palette, Style2, Zoom};
use super::fx;
use rust_decimal::prelude::ToPrimitive;
use super::inplay::{self, Tag};
use super::keymap::{self, Ctx, Entry};

// Same teal/red as the wickd desktop app (TradingView/OANDA palette).
const UP: Color = Color::Rgb(8, 153, 129);
const DOWN: Color = Color::Rgb(242, 54, 69);
const GRID: Color = Color::Rgb(48, 54, 66);
/// Selected-row highlight: a subtle background, never inverted colours (an
/// inverted row turns coloured cells — sparklines, change % — into blocks).
const SELECTED_BG: Color = Color::Rgb(44, 52, 66);

fn selected_row() -> Style {
    Style::default()
        .bg(SELECTED_BG)
        .add_modifier(Modifier::BOLD)
}
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

/// Style for a price by its last move: green after an uptick, red after a
/// downtick, default before the first change.
fn tick_style(app: &App, key: &str) -> Style {
    match app.ticks.dir(key) {
        1 => Style::default().fg(UP),
        -1 => Style::default().fg(DOWN),
        _ => Style::default(),
    }
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
        Mode::Journal => render_journal(frame, chunks[1], app),
        Mode::Calendar => render_calendar(frame, chunks[1], app),
        Mode::Detail => match &app.detail {
            Some(d) => render_detail(frame, chunks[1], app, d),
            None => render_watchlist(frame, chunks[1], app),
        },
    }
    render_footer(frame, chunks[2], app);
    if let (Some(p), Some(ctx)) = (&app.pending, app.ctx()) {
        render_key_panel(frame, chunks[1], ctx, p);
    }
    if let Some(card) = &app.info {
        render_info(frame, frame.area(), card);
    }
    if app.show_help {
        render_help(frame, frame.area(), app.mode);
    }
    if app.agent.open {
        let sees = super::context::screen_label(app);
        let left = app.mode == Mode::Detail;
        super::agent::render(frame, chunks[1], &app.agent, &sees, left);
    }
}

/// The journal (`g j`): one markdown entry, laid out for the page width and
/// scrolled by line.
fn render_journal(frame: &mut Frame, area: Rect, app: &App) {
    use ratatui::widgets::Padding;
    let j = &app.journal;
    let dim = Style::default().fg(DIM);
    let mut title = vec![Span::styled(
        " Journal ",
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if let Some(e) = j.current() {
        title.push(Span::styled("· ", dim));
        title.push(Span::styled(
            format!("{} ", e.name),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        title.push(Span::styled(
            format!("· {} of {} ", j.idx + 1, j.entries.len()),
            dim,
        ));
    }
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .padding(Padding::horizontal(2))
        .title(Line::from(title));
    if let Some(t) = j.mtime {
        let t: DateTime<Local> = t.into();
        block = block.title(
            Line::from(Span::styled(
                format!(" saved {} ", t.format("%a %-I:%M %p")),
                dim,
            ))
            .alignment(Alignment::Right),
        );
    }
    let inner = block.inner(area);
    j.page.set(usize::from(inner.height));

    if j.current().is_none() || j.error.is_some() {
        frame.render_widget(block, area);
        let lines = match &j.error {
            Some(e) => vec![Line::from(Span::styled(
                format!("couldn't read the journal: {e}"),
                Style::default().fg(Color::Yellow),
            ))],
            None => vec![
                Line::from("No journal entries yet."),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Looking for .md files in {}", j.dir.display()),
                    dim,
                )),
                Line::from(Span::styled(
                    "Set WICKD_JOURNAL_DIR to read a different folder.",
                    dim,
                )),
            ],
        };
        let body = Rect {
            y: inner.y + 1.min(inner.height),
            height: inner.height.saturating_sub(1),
            ..inner
        };
        frame.render_widget(
            Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }),
            body,
        );
        return;
    }

    let (lines, total, scroll) = j.with_rendered(inner.width, |r| {
        let total = r.lines.len();
        let scroll = j
            .scroll
            .min(total.saturating_sub(usize::from(inner.height)));
        let shown: Vec<Line> = r
            .lines
            .iter()
            .skip(scroll)
            .take(usize::from(inner.height))
            .cloned()
            .collect();
        (shown, total, scroll)
    });
    // Where the page is in the entry, bottom right.
    let end = (scroll + usize::from(inner.height)).min(total);
    let place = if total <= usize::from(inner.height) {
        " all ".to_string()
    } else if end == total {
        " end ".to_string()
    } else if scroll == 0 {
        " top ".to_string()
    } else {
        format!(" {}% ", end * 100 / total.max(1))
    };
    block = block.title_bottom(Line::from(Span::styled(place, dim)).alignment(Alignment::Right));
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// `text` cut to `width` columns (ending in `…` when cut) and padded to
/// exactly `width`.
fn fit(text: &str, width: usize) -> String {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if text.width() <= width {
        return format!("{text}{}", " ".repeat(width - text.width()));
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    if width > 0 {
        out.push('…');
        used += 1;
    }
    format!("{out}{}", " ".repeat(width.saturating_sub(used)))
}

/// `text` right-aligned in `width` columns (cut from the right if longer).
fn fit_right(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if text.width() >= width {
        return fit(text, width);
    }
    format!("{}{text}", " ".repeat(width - text.width()))
}

/// Column widths of the calendar screen for a body `w` columns wide. Every
/// row, economic or earnings, uses the same columns. On a narrow terminal
/// PREVIOUS goes first, then SIZE/IMPACT, then REACTION, so the event keeps
/// its room.
struct CalCols {
    event: usize,
    reaction: bool,
    weight: bool,
    previous: bool,
}

const CAL_TIME: usize = 10;
const CAL_TYPE: usize = 8;
const CAL_REACTION: usize = 9;
const CAL_WEIGHT: usize = 11;
const CAL_NUM: usize = 10;
const CAL_SYMBOL: usize = 6;
/// The least the EVENT column is given before a secondary column is dropped.
const CAL_EVENT_MIN: usize = 24;

impl CalCols {
    fn new(w: usize) -> Self {
        // "  TIME  TYPE  EVENT  REACTION  SIZE/IMPACT  EXPECTED  PREVIOUS"
        let fixed = 2 + CAL_TIME + 2 + CAL_TYPE + 2 + (2 + CAL_NUM);
        let (r, wt, p) = (2 + CAL_REACTION, 2 + CAL_WEIGHT, 2 + CAL_NUM);
        let reaction = w >= fixed + r + CAL_EVENT_MIN;
        let weight = reaction && w >= fixed + r + wt + CAL_EVENT_MIN;
        let previous = weight && w >= fixed + r + wt + p + CAL_EVENT_MIN;
        let used = fixed
            + if reaction { r } else { 0 }
            + if weight { wt } else { 0 }
            + if previous { p } else { 0 };
        Self {
            event: w.saturating_sub(used),
            reaction,
            weight,
            previous,
        }
    }
}

/// `EPS 2.05`, or a dash when the feed gave none.
fn fmt_eps(v: Option<rust_decimal::Decimal>) -> String {
    v.map_or("–".to_string(), |d| format!("EPS {:.2}", d))
}

/// The column captions, pinned above the scrolling rows.
fn calendar_header(w: usize) -> Line<'static> {
    let cols = CalCols::new(w);
    let mut s = format!(
        "  {}  {}  {}",
        fit_right("TIME", CAL_TIME),
        fit("TYPE", CAL_TYPE),
        fit("EVENT", cols.event)
    );
    if cols.reaction {
        s.push_str(&fit_right("REACTION", 2 + CAL_REACTION));
    }
    if cols.weight {
        s.push_str(&fit_right("SIZE/IMPACT", 2 + CAL_WEIGHT));
    }
    s.push_str(&fit_right("EXPECTED", 2 + CAL_NUM));
    if cols.previous {
        s.push_str(&fit_right("PREVIOUS", 2 + CAL_NUM));
    }
    Line::from(Span::styled(
        fit(&s, w),
        Style::default().fg(DIM).add_modifier(Modifier::BOLD),
    ))
}

/// One calendar row as a line exactly `w` columns wide. Economic releases
/// and earnings share the same columns; TYPE says which one a row is.
/// `reaction` is how the row's stock moved on its first day after the
/// report, when known.
fn calendar_line(
    row: &super::calendar::Row,
    reaction: Option<&wickd_core::earnings_reaction::Reaction>,
    w: usize,
    selected: bool,
) -> Line<'static> {
    use super::calendar::Row as R;
    use unicode_width::UnicodeWidthStr;
    let dim = Style::default().fg(DIM);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let plain = Style::default();
    let cols = CalCols::new(w);
    // TIME and TYPE, the same for both kinds of row.
    let lead = |time: &str, kind: &str, faded: bool| -> Vec<Span<'static>> {
        vec![
            Span::styled(
                format!("  {}  ", fit_right(time, CAL_TIME)),
                if faded { dim } else { plain },
            ),
            Span::styled(format!("{}  ", fit(kind, CAL_TYPE)), dim),
        ]
    };
    let spans: Vec<Span<'static>> = match row {
        R::Blank => vec![Span::raw(" ".repeat(w))],
        R::Day(label) => vec![Span::styled(fit(label, w), bold)],
        R::Note(text) => vec![Span::styled(fit(&format!("  {text}"), w), dim)],
        R::Econ(e) => {
            let (weight_style, event_style) = match (e.past, e.impact.as_str()) {
                (true, _) => (dim, dim),
                (_, "high") => (
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                    bold,
                ),
                (_, "low") => (dim, dim),
                _ => (plain, plain),
            };
            let num = if e.past { dim } else { plain };
            let mut v = lead(&e.time, "economic", e.past);
            // Two columns in from the edge, so titles line up with tickers
            // (which keep that space for the slate / list marker).
            v.push(Span::styled(
                fit(&format!("  {}", e.event), cols.event),
                event_style,
            ));
            if cols.reaction {
                v.push(Span::raw(" ".repeat(2 + CAL_REACTION)));
            }
            if cols.weight {
                v.push(Span::styled(
                    fit_right(&e.impact, 2 + CAL_WEIGHT),
                    weight_style,
                ));
            }
            v.push(Span::styled(fit_right(&e.forecast, 2 + CAL_NUM), num));
            if cols.previous {
                v.push(Span::styled(fit_right(&e.previous, 2 + CAL_NUM), dim));
            }
            v
        }
        R::Earn(e) => {
            // A report that is out is dimmed like a past release, unless it
            // is one being watched: then its reaction is the point.
            let watched = e.slate || e.listed;
            let faded = e.past && !watched;
            let text = if faded { dim } else { plain };
            let mut v = lead(super::calendar::session_text(e.session), "earnings", faded);
            v.push(Span::styled(
                match (e.slate, e.listed) {
                    (true, _) => "◆ ",
                    (false, true) => "★ ",
                    _ => "  ",
                },
                Style::default().fg(ACCENT),
            ));
            v.push(Span::styled(
                format!("{} ", fit(&e.symbol, CAL_SYMBOL)),
                match (faded, watched) {
                    (true, _) => dim,
                    (false, true) => bold,
                    (false, false) => plain,
                },
            ));
            // The company, then the slate's one-line why in what room is left.
            let room = cols.event.saturating_sub(2 + CAL_SYMBOL + 1);
            let why = e.why.as_deref().filter(|_| e.slate);
            match why {
                Some(why) if e.name.width() + 4 < room => {
                    let lead_text = if e.name.is_empty() {
                        String::new()
                    } else {
                        format!("{} · ", e.name)
                    };
                    let rest = room - lead_text.width();
                    v.push(Span::styled(lead_text, text));
                    v.push(Span::styled(fit(why, rest), dim));
                }
                _ => v.push(Span::styled(fit(&e.name, room), text)),
            }
            if cols.reaction {
                v.push(match reaction {
                    Some(r) => Span::styled(
                        fit_right(
                            &format!(
                                "{:+.1}%{}",
                                r.change_pct,
                                if r.complete { "" } else { "…" }
                            ),
                            2 + CAL_REACTION,
                        ),
                        Style::default()
                            .fg(change_color(Some(r.change_pct)))
                            .add_modifier(Modifier::BOLD),
                    ),
                    None => Span::raw(" ".repeat(2 + CAL_REACTION)),
                });
            }
            if cols.weight {
                v.push(Span::styled(
                    fit_right(
                        &e.market_cap
                            .map_or("–".to_string(), super::calendar::fmt_cap),
                        2 + CAL_WEIGHT,
                    ),
                    text,
                ));
            }
            v.push(Span::styled(
                fit_right(&fmt_eps(e.eps_forecast), 2 + CAL_NUM),
                text,
            ));
            if cols.previous {
                v.push(Span::styled(
                    fit_right(&fmt_eps(e.last_year_eps), 2 + CAL_NUM),
                    dim,
                ));
            }
            v
        }
    };
    let line = Line::from(spans);
    if selected {
        line.patch_style(selected_row())
    } else {
        line
    }
}

/// A reaction as a sentence: what the stock did on its first trading day
/// after the report.
fn reaction_sentence(r: &wickd_core::earnings_reaction::Reaction) -> String {
    let day = r.day.format("%a %b %-d");
    let mut s = if r.complete {
        format!(
            "Reaction on {day}: opened {:+.1}%, closed {:+.1}%",
            r.gap_pct, r.change_pct
        )
    } else {
        format!(
            "Reaction so far ({day}): opened {:+.1}%, now {:+.1}%",
            r.gap_pct, r.change_pct
        )
    };
    if let Some(p) = r.close_in_range {
        s.push_str(&format!(", {p:.0}% of the way up the day's range"));
    }
    if let Some(v) = r.rel_volume {
        s.push_str(&format!(
            ", on {v:.1}× its usual volume{}",
            if r.complete { "" } else { " so far" }
        ));
    }
    s.push('.');
    s
}

/// What a company reported, as a sentence: EPS against the consensus and
/// the size of the beat or miss.
fn reported_sentence(r: &wickd_core::earnings_results::ResultRow) -> String {
    use rust_decimal::Decimal;
    let Some(eps) = r.eps else {
        return "Reported: no EPS figure in the feed for this report.".into();
    };
    let mut s = format!("Reported: EPS {eps:.2}");
    if let Some(c) = r.consensus {
        s.push_str(&format!(" against {c:.2} expected"));
    }
    match r.surprise_pct {
        Some(p) if p > Decimal::ZERO => s.push_str(&format!(", a beat of {p:.1}%")),
        Some(p) if p < Decimal::ZERO => s.push_str(&format!(", a miss of {:.1}%", -p)),
        Some(_) => s.push_str(", in line"),
        None => {}
    }
    s.push('.');
    s
}

/// What the box under the list says for the company under the cursor, in
/// the order things happen: the expectation as written before the report,
/// the result (the reported figures, then the note written afterwards),
/// and the stock's reaction. `None` when there is nothing to say.
fn calendar_box(app: &App) -> Option<(String, Vec<String>)> {
    let c = &app.calendar;
    let e = c.selected_earn()?;
    let watched = e.slate || e.listed;
    let expect = e.expect.as_deref().filter(|_| e.slate);
    let note = e.result.as_deref().filter(|_| e.slate);
    let reaction = c.reaction_of(e);
    // Once a watched company's report is out there is always a result to
    // speak of, even if it is only "the figures aren't in yet".
    let out = watched && e.past;
    if expect.is_none() && note.is_none() && reaction.is_none() && !out {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut section = |name: &'static str, text: Vec<String>, lines: &mut Vec<String>| {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(text);
        parts.push(name);
    };
    if let Some(t) = expect {
        section("expectation", t.lines().map(str::to_string).collect(), &mut lines);
    }
    if out || note.is_some() {
        let mut text = Vec::new();
        if out {
            // The headlines first: they are out within minutes of a report.
            // The reported-figures table follows when it has caught up (it
            // runs a day or so behind), and "nothing yet" is only said when
            // there is neither.
            let heads = c.headlines_of(e);
            for h in heads {
                // ↗: `g a` opens it in the reader window.
                let link = if h.has_link() {
                    "  ↗ g a"
                } else {
                    ""
                };
                text.push(format!(
                    "{}  {}{link}",
                    c.clock.local(h.time).format("%a %-I:%M %p"),
                    h.headline
                ));
                // The release's own opening figures, in the company's words.
                text.extend(h.lede.iter().cloned());
            }
            match c.reported_of(e) {
                Some(r) => text.push(reported_sentence(r)),
                None if !heads.is_empty() => {}
                None if c.headlines_loading || c.results_loading => {
                    text.push("Fetching the results…".into())
                }
                None => text.push(
                    "No results in yet: no headline from the news feed and no reported figures. Press r to update."
                        .into(),
                ),
            }
        }
        if let Some(n) = note {
            text.extend(n.lines().map(str::to_string));
        }
        section("result", text, &mut lines);
    }
    if let Some(r) = reaction {
        section("reaction", vec![reaction_sentence(r)], &mut lines);
    }
    Some((format!(" {} · {} ", e.symbol, parts.join(" · ")), lines))
}

/// The calendar (`g c`): the earnings watch. A heading per day and one row
/// per event, in local time, with the selected company's expectation and
/// reaction in a box underneath.
fn render_calendar(frame: &mut Frame, area: Rect, app: &App) {
    use ratatui::widgets::Padding;
    let c = &app.calendar;
    let dim = Style::default().fg(DIM);

    // The box takes what its text needs, up to half the screen: an
    // expectation, headlines, a note and a reaction together run to a dozen
    // lines, and the reaction at the end is the part that mustn't be cut.
    let boxed = calendar_box(app);
    let box_h: u16 = boxed.as_ref().map_or(0, |(_, lines)| {
        let inner = usize::from(area.width.saturating_sub(4)).max(1);
        let need: usize = lines
            .iter()
            .map(|l| l.chars().count().max(1).div_ceil(inner))
            .sum();
        (need as u16 + 2).min((area.height / 2).max(3))
    });
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(box_h)])
        .split(area);
    let area = parts[0];
    if let Some((title, lines)) = boxed {
        let text: Vec<Line> = lines
            .into_iter()
            .map(|l| {
                if l.trim().is_empty() {
                    Line::default()
                } else {
                    Line::from(format!(" {l}"))
                }
            })
            .collect();
        frame.render_widget(
            Paragraph::new(text)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(ACCENT))
                        .title(Span::styled(
                            title,
                            Style::default().add_modifier(Modifier::BOLD),
                        )),
                ),
            parts[1],
        );
    }

    let mut title = vec![
        Span::styled(" Calendar ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            if c.show_all {
                "· everything that reports "
            } else {
                "· your earnings watch "
            },
            dim,
        ),
    ];
    if !c.show_all {
        title.push(Span::styled("◆ slate  ★ list ", dim));
    }
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .padding(Padding::horizontal(2))
        .title(Line::from(title));

    // Top right: how fresh each store is, or that a fetch is running.
    let stamp = |t: DateTime<Utc>| c.clock.local(t).format("%a %-I:%M %p").to_string();
    let status = if c.updating.is_some() {
        Some("updating everything…".to_string())
    } else if c.loading {
        Some("fetching earnings…".to_string())
    } else {
        let parts: Vec<String> = [
            c.earnings_as_of.map(|t| format!("earnings as of {}", stamp(t))),
            c.econ_as_of
                .map(|t| format!("economic as of {}", stamp(DateTime::<Utc>::from(t)))),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    };
    if let Some(s) = status {
        block = block
            .title(Line::from(Span::styled(format!(" {s} "), dim)).alignment(Alignment::Right));
    }
    // Bottom left: what went wrong fetching, earnings first.
    if let Some(e) = c
        .fetch_error
        .as_ref()
        .or(c.reactions_error.as_ref())
        .or(c.headlines_error.as_ref())
        .or(c.results_error.as_ref())
    {
        let room = usize::from(area.width).saturating_sub(14);
        block = block.title_bottom(Line::from(Span::styled(
            format!(" {} ", fit(e, room).trim_end()),
            Style::default().fg(Color::Yellow),
        )));
    }

    let inner = block.inner(area);
    let w = usize::from(inner.width);

    if c.error.is_some() || c.rows.is_empty() {
        frame.render_widget(block, area);
        let lines = match &c.error {
            Some(e) => vec![Line::from(Span::styled(
                format!("couldn't read the calendar: {e}"),
                Style::default().fg(Color::Yellow),
            ))],
            None => vec![Line::from("Nothing stored yet. Press r to fetch.")],
        };
        let body = Rect {
            y: inner.y + 1.min(inner.height),
            height: inner.height.saturating_sub(1),
            ..inner
        };
        frame.render_widget(
            Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }),
            body,
        );
        return;
    }

    // The captions stay put; the rows scroll under them.
    let head = Rect {
        height: 1.min(inner.height),
        ..inner
    };
    let body = Rect {
        y: inner.y + head.height,
        height: inner.height.saturating_sub(head.height),
        ..inner
    };
    let page = usize::from(body.height);
    let scroll = c.settle_scroll(page);
    let total = c.rows.len();
    let lines: Vec<Line> = c
        .rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(page)
        .map(|(i, r)| {
            let reaction = match r {
                super::calendar::Row::Earn(e) => c.reaction_of(e),
                _ => None,
            };
            calendar_line(r, reaction, w, i == c.selected && r.selectable())
        })
        .collect();
    // Where the page is in the week, bottom right.
    let end = (scroll + page).min(total);
    let place = if total <= page {
        " all ".to_string()
    } else if end == total {
        " end ".to_string()
    } else if scroll == 0 {
        " top ".to_string()
    } else {
        format!(" {}% ", end * 100 / total.max(1))
    };
    block = block.title_bottom(Line::from(Span::styled(place, dim)).alignment(Alignment::Right));
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(calendar_header(w)), head);
    frame.render_widget(Paragraph::new(lines), body);
}

/// SEC's `MMDD` fiscal year end as `Jan 31` (unchanged if it isn't MMDD).
fn month_day(mmdd: &str) -> String {
    const M: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    match (mmdd.get(..2).and_then(|m| m.parse::<usize>().ok()), mmdd.get(2..4).and_then(|d| d.parse::<u32>().ok())) {
        (Some(m @ 1..=12), Some(d)) if mmdd.len() == 4 => format!("{} {d}", M[m - 1]),
        _ => mmdd.to_string(),
    }
}

/// The `i` card, centred over the current screen.
fn render_info(frame: &mut Frame, area: Rect, card: &crate::tui::app::InfoCard) {
    use crate::tui::app::InfoState;
    let w = area.width.saturating_sub(8).clamp(40, 110);
    let max_h = area.height.saturating_sub(4).clamp(10, 40);
    let dim = Style::default().fg(DIM);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let heading = |t: &str| Line::from(Span::styled(t.to_string(), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)));
    let (title, lines): (String, Vec<Line>) = match &card.state {
        InfoState::Loading => (
            format!(" {} ", card.symbol),
            vec![Line::from(Span::styled("loading…", dim))],
        ),
        InfoState::Failed(e) => (
            format!(" {} ", card.symbol),
            vec![Line::from(Span::styled(
                format!("couldn't load info: {e}"),
                Style::default().fg(Color::Yellow),
            ))],
        ),
        InfoState::Ready(i) => {
            let kind = match i.kind {
                crate::info::SymbolKind::Stock => "stock",
                crate::info::SymbolKind::Forex => "forex pair",
            };
            let mut l: Vec<Line> = Vec::new();
            match &i.summary {
                Some(sum) => l.push(Line::from(sum.clone())),
                None => l.push(Line::from(Span::styled("No description found.", dim))),
            }
            if let Some(src) = &i.source {
                l.push(Line::from(vec![
                    Span::styled("source  ", dim),
                    Span::raw(format!("Wikipedia — {}  ", src.title)),
                    Span::styled("(o opens it)", dim),
                ]));
            }
            if !i.lists.is_empty() {
                l.push(Line::from(""));
                l.push(heading("On your lists"));
                for ln in &i.lists {
                    l.push(Line::from(Span::styled(format!("  {}", ln.list), bold)));
                    if let Some(n) = &ln.note {
                        l.push(Line::from(format!("    {n}")));
                    }
                    if let Some(p) = &ln.plan {
                        for line in p.lines() {
                            l.push(Line::from(Span::styled(format!("    {line}"), dim)));
                        }
                    }
                }
            }
            for c in &i.currencies {
                l.push(Line::from(""));
                l.push(heading(&format!(
                    "{} — {}",
                    c.code,
                    c.name.clone().unwrap_or_default()
                )));
                if let Some(sum) = &c.summary {
                    l.push(Line::from(sum.clone()));
                }
            }
            let mut facts: Vec<(String, String)> =
                i.facts.iter().map(|f| (f.label.clone(), f.value.clone())).collect();
            if let Some(sec) = &i.sec {
                for (label, v) in [
                    ("Industry", &sec.industry),
                    ("Headquarters", &sec.headquarters),
                    ("Incorporated", &sec.incorporated),
                ] {
                    if let Some(v) = v {
                        facts.push((label.to_string(), v.clone()));
                    }
                }
                if let Some(fy) = &sec.fiscal_year_end {
                    facts.push(("Fiscal year end".into(), month_day(fy)));
                }
            }
            if !facts.is_empty() {
                l.push(Line::from(""));
                l.push(heading("Facts"));
                for (label, v) in facts {
                    l.push(Line::from(vec![
                        Span::styled(format!("  {label:<18}"), dim),
                        Span::raw(v),
                    ]));
                }
            }
            if !i.notes.is_empty() {
                l.push(Line::from(""));
                for n in &i.notes {
                    l.push(Line::from(Span::styled(n.clone(), dim)));
                }
            }
            (format!(" {} · {} · {kind} ", i.symbol, i.name), l)
        }
    };
    // Fit the box to the wrapped content (scrolls when it doesn't fit).
    let inner_w = usize::from(w.saturating_sub(2)).max(1);
    let wrapped: usize = lines
        .iter()
        .map(|l| l.width().max(1).div_ceil(inner_w))
        .sum();
    let h = (wrapped as u16 + 2).clamp(6, max_h);
    let rect = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 3,
        w.min(area.width),
        h.min(area.height),
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .scroll((card.scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT))
                    .title(Span::styled(title, bold))
                    .title_bottom(Span::styled(" esc close · j/k scroll · o open source ", dim)),
            ),
        rect,
    );
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
    let budget = app
        .api_budget
        .as_ref()
        .and_then(|c| c.lock().ok().and_then(|g| *g));
    let quotes = match app.last_quotes {
        Some(t) => format!("quotes {} · {}", app.feed_label, t.format("%-I:%M:%S %p")),
        None => format!("quotes {} · loading…", app.feed_label),
    };
    let api = budget.map(|b| format!("api {}/{}  ", b.remaining, b.limit));
    let api_low = budget.is_some_and(|b| b.limit > 0 && b.remaining * 5 < b.limit);
    let right = format!("{}{quotes}", api.clone().unwrap_or_default());
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(right.chars().count() as u16),
        ])
        .split(area);
    frame.render_widget(Paragraph::new(Line::from(left)), cols[0]);
    let mut spans = Vec::new();
    if let Some(a) = api {
        let st = if api_low {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(DIM)
        };
        spans.push(Span::styled(a, st));
    }
    spans.push(Span::styled(quotes, Style::default().fg(DIM)));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
        cols[1],
    );
}

/// The smallest price-line step for a symbol: a cent, or a tenth of a pip
/// for a forex pair (OANDA's quote precision).
pub(crate) fn price_tick(symbol: &str) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    if super::fx::is_fx(symbol) {
        super::fx::pip_size(symbol).to_f64().unwrap_or(0.0001) / 10.0
    } else {
        0.01
    }
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
    if let Some(q) = &app.price_input {
        let line = Line::from(vec![
            Span::styled(
                "price ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(q.clone()),
            Span::styled("▏", Style::default().fg(ACCENT)),
            Span::styled(
                "  enter puts the line there · empty removes it · esc cancels",
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

/// Width of the EARNINGS column (`today post-close`).
const REPORTS_W: u16 = 16;

/// A report today stands out; one on the next weekday is quieter.
fn reports_style(today: bool) -> Style {
    if today {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Gray)
    }
}

/// The EARNINGS cell for a symbol: when it reports, if that is today or the
/// next weekday; empty otherwise.
fn reports_cell(app: &App, sym: &str) -> Cell<'static> {
    let today = app.calendar.clock.today();
    match app.reports_soon.get(sym) {
        Some(soon) => Cell::from(soon.short(today)).style(reports_style(soon.date == today)),
        None => Cell::from(""),
    }
}

/// Whether a table `width` columns wide shows the EARNINGS column: only
/// when one of its symbols reports soon, and only when there is room for it
/// without taking space from the columns already there (`needed` is the
/// table's width without it).
fn shows_reports<'a>(
    app: &App,
    width: u16,
    needed: u16,
    mut symbols: impl Iterator<Item = &'a str>,
) -> bool {
    width >= needed + REPORTS_W + 2 && symbols.any(|s| app.reports_soon.contains_key(s))
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
    // Screen rows on show (only matches while `g f` is active) and the
    // selected one, in the combined order: list entries, then the watchlist.
    let (shown, sel): (Vec<usize>, Option<usize>) = match &app.find {
        Some(f) => {
            let m = app.find_matches();
            let sel = m.get(f.selected).copied();
            (m, sel)
        }
        None => ((0..app.total_rows()).collect(), Some(app.selected)),
    };
    let lr = app.list_rows();
    // Lists first: the open one as a table, every other list as a one-line
    // header (`g l` opens the next). The watchlist keeps a readable share:
    // its natural height, up to half the screen.
    let area = if app.lists.is_empty() {
        area
    } else {
        let rows: Vec<usize> = shown.iter().copied().filter(|&r| r < lr).collect();
        let plan = sel
            .and_then(|s| app.row_entry(s))
            .filter(|e| plan_lines(e, area.width) > 0);
        // The plan box grows with the plan, up to a third of the screen.
        let plan_h: u16 =
            plan.map_or(0, |e| (plan_lines(e, area.width) + 2).min((area.height / 3).max(4)));
        let collapsed = app.lists.len() as u16 - 1;
        let start = app.fx_start();
        let watch_natural = (start.min(1) * (start + 3)
            + (app.watchlist.len() - start).min(1) * (app.watchlist.len() - start + 3))
            as u16;
        let watch_need = watch_natural.min(area.height / 2);
        let list_natural = rows.len().max(1) as u16 + 3;
        let list_h = list_natural
            .min(area.height.saturating_sub(collapsed + plan_h + watch_need))
            .max(5);
        // Headers before the open list, the open list, headers after.
        let mut constraints = Vec::new();
        for i in 0..app.lists.len() {
            constraints.push(Constraint::Length(if i == app.expanded { list_h } else { 1 }));
        }
        constraints.push(Constraint::Length(plan_h));
        constraints.push(Constraint::Min(0));
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);
        for (i, (name, entries)) in app.lists.iter().enumerate() {
            if i == app.expanded {
                let title = format!(" {name} · {} · list · g l next ", entries.len());
                let pos = sel.and_then(|s| rows.iter().position(|&r| r == s));
                render_list_table(frame, parts[i], app, &rows, pos, title);
            } else {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(" ▸ ", Style::default().fg(ACCENT)),
                        Span::styled(name.clone(), Style::default().fg(Color::Gray)),
                        Span::styled(format!(" · {}", entries.len()), Style::default().fg(DIM)),
                    ])),
                    parts[i],
                );
            }
        }
        if let Some(e) = plan {
            render_plan(frame, parts[app.lists.len()], e);
        }
        parts[app.lists.len() + 1]
    };
    // The rest works in watchlist indices.
    let shown: Vec<usize> = shown.into_iter().filter(|&r| r >= lr).map(|r| r - lr).collect();
    let sel = sel.and_then(|s| s.checked_sub(lr));
    if app.watchlist.is_empty() {
        let block = Block::default().borders(Borders::ALL).title(" watchlist · 0 ");
        frame.render_widget(
            Paragraph::new("No symbols yet — press / to search and tab to add.")
                .style(Style::default().fg(DIM))
                .block(block),
            area,
        );
        return;
    }
    let start = app.fx_start();
    let (stocks, pairs): (Vec<usize>, Vec<usize>) = shown.into_iter().partition(|&i| i < start);
    let has_stocks = start > 0;
    let has_pairs = start < app.watchlist.len();
    // Row position of the selection within one section's table.
    let pos = |rows: &[usize]| sel.and_then(|s| rows.iter().position(|&i| i == s));

    if !has_pairs {
        let title = format!(" watchlist · {} ", app.watchlist.len());
        render_stock_table(frame, area, app, &stocks, pos(&stocks), title);
        return;
    }
    let feed = if app.fx_live {
        "live (stream)".to_string()
    } else {
        format!("refresh {}s", app.refresh_secs)
    };
    let mut fx_title = format!(
        " forex · {} · OANDA practice · {feed} ",
        app.watchlist.len() - start
    );
    if let Some(e) = &app.fx_error {
        let e: String = e.chars().take(60).collect();
        fx_title = format!("{fx_title}· {e} ");
    }
    if !has_stocks {
        render_fx_table(frame, area, app, &pairs, pos(&pairs), fx_title);
        return;
    }
    // Both sections at their natural height, stacked; if that doesn't fit,
    // pairs keep up to half the screen and the stocks table scrolls.
    let stock_h = stocks.len() as u16 + 3;
    let fx_h = (pairs.len() as u16 + 3).max(4);
    let constraints = if stock_h + fx_h <= area.height {
        [
            Constraint::Length(stock_h),
            Constraint::Length(fx_h),
            Constraint::Min(0),
        ]
    } else {
        let fx_h = fx_h.min((area.height / 2).max(4));
        [Constraint::Min(4), Constraint::Length(fx_h), Constraint::Length(0)]
    };
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    let stock_title = format!(" stocks · {} ", start);
    render_stock_table(frame, parts[0], app, &stocks, pos(&stocks), stock_title);
    render_fx_table(frame, parts[1], app, &pairs, pos(&pairs), fx_title);
}

/// Lines the plan box needs for an entry's free-form plan at `width`
/// (wrapped, line breaks kept). 0 = no plan, no box.
fn plan_lines(e: &crate::lists::Entry, width: u16) -> u16 {
    let inner = usize::from(width.saturating_sub(4)).max(1);
    e.plan.as_deref().map_or(0, |p| {
        p.lines()
            .map(|l| l.chars().count().max(1).div_ceil(inner))
            .sum::<usize>() as u16
    })
}

/// The selected list entry's plan, shown exactly as written (free form).
fn render_plan(frame: &mut Frame, area: Rect, e: &crate::lists::Entry) {
    let lines: Vec<Line> = e
        .plan
        .as_deref()
        .unwrap_or_default()
        .lines()
        .map(|l| {
            if l.trim().is_empty() {
                Line::default() // a blank line stays one row
            } else {
                Line::from(format!(" {l}"))
            }
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(ACCENT))
                    .title(Span::styled(
                        format!(" {} · plan ", e.symbol),
                        Style::default().add_modifier(Modifier::BOLD),
                    )),
            ),
        area,
    );
}

/// One named list (`wickd list`): symbol, name, note, price and change for
/// stocks and pairs alike (pairs at OANDA precision, mid price).
fn render_list_table(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    rows: &[usize],
    sel: Option<usize>,
    title: String,
) {
    // SYMBOL 8 + NAME 28 + NOTE's minimum 20 + LAST 10 + CHG% 8, four gaps
    // of 2 and the border.
    let reports = shows_reports(
        app,
        area.width,
        8 + 28 + 20 + 10 + 8 + 4 * 2 + 2,
        rows.iter().filter_map(|&r| app.row_symbol(r)),
    );
    let header = if reports {
        header_row(&["SYMBOL", "NAME", "EARNINGS", "NOTE"], &["LAST", "CHG%"], &[])
    } else {
        header_row(&["SYMBOL", "NAME", "NOTE"], &["LAST", "CHG%"], &[])
    };
    let body = rows.iter().filter_map(|&r| {
        let sym = app.row_symbol(r)?.to_string();
        let note = app.row_note(r).unwrap_or_default().to_string();
        let name = app
            .asset(&sym)
            .map(|a| short_name(&a.name))
            .unwrap_or_default();
        let (last, pct, color, key) = if fx::is_fx(&sym) {
            match app.fx_quotes.get(&sym) {
                Some(q) => (
                    fx_price(&sym, q.mid()),
                    q.change_pct()
                        .map(|p| format!("{}%", signed_1dp(p, 2)))
                        .unwrap_or_default(),
                    change_color(q.change().and_then(|c| c.to_f64())),
                    format!("{sym}:mid"),
                ),
                None => ("…".into(), String::new(), DIM, String::new()),
            }
        } else {
            let (last, _, pct, color) = quote_cells(app.snapshots.get(&sym));
            (last, pct, color, format!("{sym}:last"))
        };
        let mut cells = vec![
            Cell::from(sym.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(name),
        ];
        if reports {
            cells.push(reports_cell(app, &sym));
        }
        cells.extend([
            Cell::from(note).style(Style::default().fg(Color::Gray)),
            Cell::from(Line::from(last).alignment(Alignment::Right)).style(tick_style(app, &key)),
            Cell::from(Line::from(pct).alignment(Alignment::Right))
                .style(Style::default().fg(color)),
        ]);
        Some(Row::new(cells))
    });
    let mut widths = vec![Constraint::Length(8), Constraint::Length(28)];
    if reports {
        widths.push(Constraint::Length(REPORTS_W));
    }
    widths.extend([
        Constraint::Min(20),
        Constraint::Length(10),
        Constraint::Length(8),
    ]);
    let table = Table::new(body, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(ACCENT))
                .title(title),
        )
        .column_spacing(2)
        .row_highlight_style(selected_row());
    let mut state = TableState::default().with_selected(sel);
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_stock_table(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    rows: &[usize],
    sel: Option<usize>,
    title: String,
) {
    let spark_w: usize = 24;
    // SYMBOL 7 + NAME's minimum 12 + LAST 10 + CHG 9 + CHG% 8 + DAY RANGE 19
    // + VOLUME 8 + the sparkline, seven gaps of 2 and the border.
    let reports = shows_reports(
        app,
        area.width,
        7 + 12 + 10 + 9 + 8 + 19 + 8 + spark_w as u16 + 7 * 2 + 2,
        rows.iter().map(|&i| app.watchlist[i].as_str()),
    );
    let header = if reports {
        header_row(
            &["SYMBOL", "NAME", "EARNINGS"],
            &["LAST", "CHG", "CHG%", "DAY RANGE", "VOLUME"],
            &["TODAY"],
        )
    } else {
        header_row(
            &["SYMBOL", "NAME"],
            &["LAST", "CHG", "CHG%", "DAY RANGE", "VOLUME"],
            &["TODAY"],
        )
    };
    let rows = rows.iter().map(|&i| {
        let sym = &app.watchlist[i];
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
        let mut cells = vec![
            Cell::from(sym.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from(name),
        ];
        if reports {
            cells.push(reports_cell(app, sym));
        }
        cells.extend([
            Cell::from(Line::from(last).alignment(Alignment::Right))
                .style(tick_style(app, &format!("{sym}:last"))),
            Cell::from(Line::from(chg).alignment(Alignment::Right))
                .style(Style::default().fg(color)),
            Cell::from(Line::from(pct).alignment(Alignment::Right))
                .style(Style::default().fg(color)),
            Cell::from(Line::from(range).alignment(Alignment::Right)),
            Cell::from(Line::from(vol).alignment(Alignment::Right)),
            Cell::from(spark).style(Style::default().fg(color)),
        ]);
        Row::new(cells)
    });
    let mut widths = vec![Constraint::Length(7), Constraint::Min(12)];
    if reports {
        widths.push(Constraint::Length(REPORTS_W));
    }
    widths.extend([
        Constraint::Length(10),
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Length(19),
        Constraint::Length(8),
        Constraint::Length(spark_w as u16),
    ]);
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .column_spacing(2)
        .row_highlight_style(selected_row());
    let mut state = TableState::default().with_selected(sel);
    frame.render_stateful_widget(table, area, &mut state);
}

fn fmt_fx5(p: f64) -> String {
    format!("{p:.5}")
}

fn fmt_fx3(p: f64) -> String {
    format!("{p:.3}")
}

/// Chart/table price formatter for a symbol: OANDA precision for pairs
/// (5 decimals, 3 for JPY quotes), the stock formatter otherwise.
fn price_fmt(sym: &str) -> fn(f64) -> String {
    if !fx::is_fx(sym) {
        fmt_price
    } else if fx::decimals(sym) == 3 {
        fmt_fx3
    } else {
        fmt_fx5
    }
}

/// A pair's price at OANDA's precision (5 decimals, 3 for JPY quotes).
fn fx_price(sym: &str, p: rust_decimal::Decimal) -> String {
    let dp = fx::decimals(sym);
    format!("{:.*}", dp as usize, p.round_dp(dp))
}

/// A signed decimal at one place (`+12.3`), for pips and percent cells.
fn signed_1dp(d: rust_decimal::Decimal, dp: u32) -> String {
    let r = d.round_dp(dp);
    let sign = if r.is_sign_negative() && !r.is_zero() { "-" } else { "+" };
    format!("{sign}{:.*}", dp as usize, r.abs())
}

fn render_fx_table(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    rows: &[usize],
    sel: Option<usize>,
    title: String,
) {
    let spark_w: usize = 24;
    let header = header_row(
        &["PAIR"],
        &["BID", "ASK", "SPREAD", "CHG PIPS", "CHG%", "DAY RANGE"],
        &["TODAY"],
    );
    let rows = rows.iter().map(|&i| {
        let sym = &app.watchlist[i];
        let q = app.fx_quotes.get(sym);
        let chg = q.and_then(|q| q.change());
        let color = change_color(chg.and_then(|c| c.to_f64()));
        let cell = |s: String| Cell::from(Line::from(s).alignment(Alignment::Right));
        let (bid, ask, spread) = match q {
            Some(q) => (
                fx_price(sym, q.bid),
                fx_price(sym, q.ask),
                format!("{:.1}", fx::pips(sym, q.ask - q.bid).round_dp(1)),
            ),
            None => ("…".into(), String::new(), String::new()),
        };
        let pips = chg
            .map(|c| signed_1dp(fx::pips(sym, c), 1))
            .unwrap_or_default();
        let pct = q
            .and_then(|q| q.change_pct())
            .map(|p| format!("{}%", signed_1dp(p, 2)))
            .unwrap_or_default();
        let range = q
            .and_then(|q| Some(format!("{} – {}", fx_price(sym, q.low?), fx_price(sym, q.high?))))
            .unwrap_or_default();
        let spark = q.map(|q| sparkline(&q.closes, spark_w)).unwrap_or_default();
        Row::new(vec![
            Cell::from(sym.clone()).style(Style::default().add_modifier(Modifier::BOLD)),
            cell(bid).style(tick_style(app, &format!("{sym}:bid"))),
            cell(ask).style(tick_style(app, &format!("{sym}:ask"))),
            cell(spread).style(Style::default().fg(DIM)),
            cell(pips).style(Style::default().fg(color)),
            cell(pct).style(Style::default().fg(color)),
            cell(range),
            Cell::from(spark).style(Style::default().fg(color)),
        ])
    });
    let widths = [
        Constraint::Min(8),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(19),
        Constraint::Length(spark_w as u16),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title))
        .column_spacing(2)
        .row_highlight_style(selected_row());
    let mut state = TableState::default().with_selected(sel);
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
                right(r.last.map(fmt_price).unwrap_or_else(|| "—".into()))
                    .style(tick_style(app, &format!("{}:last", r.symbol))),
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
            .row_highlight_style(selected_row());
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
                    Style::default().bg(SELECTED_BG)
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
    .row_highlight_style(selected_row());
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
    if let Some(soon) = app.reports_soon.get(&d.symbol) {
        let today = app.calendar.clock.today();
        title.push(Span::styled(
            format!("  {}", soon.long(today)),
            reports_style(soon.date == today),
        ));
    }
    let fxq = app.fx_quotes.get(&d.symbol);
    let (price_line, stats_line) = if fx::is_fx(&d.symbol) {
        fx_header_lines(app, &d.symbol, fxq)
    } else {
        stock_header_lines(app, &d.symbol, snap)
    };
    frame.render_widget(
        Paragraph::new(vec![Line::from(title), price_line, stats_line]).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(DIM)),
        ),
        chunks[0],
    );
    render_detail_body(frame, chunks[1], chunks[2], app, d, snap);
}

fn stock_header_lines(
    app: &App,
    sym: &str,
    snap: Option<&Snapshot>,
) -> (Line<'static>, Line<'static>) {
    let (last, chg, pct, color) = quote_cells(snap);
    let price_line = Line::from(vec![
        Span::styled(
            format!(" {last} "),
            tick_style(app, &format!("{sym}:last")).add_modifier(Modifier::BOLD),
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
    (price_line, Line::from(stat_spans(app, sym, snap)))
}

/// Forex header: mid price, change in pips and % since the trading day's
/// open (17:00 New York), then open/high/low, bid × ask and spread.
fn fx_header_lines(
    app: &App,
    sym: &str,
    q: Option<&fx::FxQuote>,
) -> (Line<'static>, Line<'static>) {
    let live = app.fx_live;
    let Some(q) = q else {
        return (
            Line::from(Span::styled(" loading quote…", Style::default().fg(DIM))),
            Line::default(),
        );
    };
    let chg = q.change();
    let color = change_color(chg.and_then(|c| c.to_f64()));
    let pips = chg.map(|c| format!("{} pips", signed_1dp(fx::pips(sym, c), 1)));
    let pct = q.change_pct().map(|p| format!("{}%", signed_1dp(p, 2)));
    let change = match (pips, pct) {
        (Some(a), Some(b)) => format!("{a} ({b})"),
        _ => String::new(),
    };
    let when = if live {
        "   live".to_string()
    } else {
        format!("   last tick {}", local(q.time).format("%b %-d %-I:%M:%S %p"))
    };
    let price_line = Line::from(vec![
        Span::styled(
            format!(" {} ", fx_price(sym, q.mid())),
            tick_style(app, &format!("{sym}:mid")).add_modifier(Modifier::BOLD),
        ),
        Span::styled(change, Style::default().fg(color)),
        Span::styled(when, Style::default().fg(DIM)),
    ]);
    let mut stats: Vec<Span<'static>> = Vec::new();
    let mut push = |label: &str, value: String| {
        stats.push(Span::styled(format!(" {label} "), Style::default().fg(DIM)));
        stats.push(Span::raw(format!("{value} ")));
    };
    for (label, v) in [("open", q.open), ("high", q.high), ("low", q.low)] {
        if let Some(v) = v {
            push(label, fx_price(sym, v));
        }
    }
    stats.extend(bid_ask_spans(
        app,
        sym,
        fx_price(sym, q.bid),
        fx_price(sym, q.ask),
    ));
    let mut push = |label: &str, value: String| {
        stats.push(Span::styled(format!(" {label} "), Style::default().fg(DIM)));
        stats.push(Span::raw(format!("{value} ")));
    };
    push(
        "spread",
        format!("{:.1} pips", fx::pips(sym, q.ask - q.bid).round_dp(1)),
    );
    (price_line, Line::from(stats))
}

fn render_detail_body(
    frame: &mut Frame,
    tabs_area: Rect,
    body: Rect,
    app: &App,
    d: &DetailState,
    snap: Option<&Snapshot>,
) {
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
        .split(tabs_area);
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
            DetailView::Candles => render_chart_view(frame, body, app, d, snap, Style2::Candles),
            DetailView::Line => render_chart_view(frame, body, app, d, snap, Style2::Line),
            DetailView::Table => {
                render_bar_table(frame, body, d.range, bars, d.scroll, price_fmt(&d.symbol))
            }
        },
    }
}

/// ` bid/ask 1.2 × 1.3 ` with each side coloured by its last move.
fn bid_ask_spans(app: &App, sym: &str, bid: String, ask: String) -> Vec<Span<'static>> {
    vec![
        Span::styled(" bid/ask ", Style::default().fg(DIM)),
        Span::styled(bid, tick_style(app, &format!("{sym}:bid"))),
        Span::raw(" × "),
        Span::styled(ask, tick_style(app, &format!("{sym}:ask"))),
        Span::raw(" "),
    ]
}

fn stat_spans(app: &App, sym: &str, snap: Option<&Snapshot>) -> Vec<Span<'static>> {
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
            out.extend(bid_ask_spans(app, sym, fmt_price(q.bid), fmt_price(q.ask)));
        }
    }
    out
}

fn range_summary(d: &DetailState) -> Line<'static> {
    let fmt_price = price_fmt(&d.symbol);
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

/// The chart's reference line and last-price line: on 1D the previous close
/// for stocks, the trading day's open (17:00 New York) for pairs; the last
/// price from the snapshot (stocks) or the live OANDA mid (pairs). Shared
/// with the chart context the agent gets (`tui::context::open_chart`).
pub fn chart_price_refs(
    app: &App,
    d: &DetailState,
    snap: Option<&Snapshot>,
) -> (Option<f64>, Option<f64>) {
    let is_fx = fx::is_fx(&d.symbol);
    let fxq = app.fx_quotes.get(&d.symbol);
    let reference = if d.range != HistoryRange::Day1 {
        None
    } else if is_fx {
        fxq.and_then(|q| q.open).and_then(|o| o.to_f64())
    } else {
        snap.and_then(Snapshot::prev_close)
    };
    let last_price = if is_fx {
        fxq.and_then(|q| q.mid().to_f64())
    } else {
        snap.and_then(Snapshot::last)
    };
    (reference, last_price)
}

fn render_chart_view(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    d: &DetailState,
    snap: Option<&Snapshot>,
    style: Style2,
) {
    let range = d.range;
    let is_fx = fx::is_fx(&d.symbol);
    let fmt_price = price_fmt(&d.symbol);
    // Pairs stream live from OANDA: never "delayed".
    let delayed = app.history_delayed && !is_fx;
    let (reference, last_price) = chart_price_refs(app, d, snap);
    let Some((candles, extended)) = d.candles() else {
        return;
    };
    // 50/200-day averages (daily ranges; warm-up bars make the 200-day span
    // the whole chart). Daily ranges are never session-filtered, so candle
    // bar indices line up with the visible bars.
    let warm = d.warmup.get(&range).map_or(&[][..], Vec::as_slice);
    let mut overlays = match (d.chart.ma && chart::ma_range(range), d.bars()) {
        (true, Some(vis)) => chart::moving_averages(warm, vis, &candles),
        _ => Vec::new(),
    };
    // Ichimoku on the same bars the candles were built from, with the
    // warm-up history before them (filtered to the same session hours) so the
    // cloud spans the whole chart on every range.
    let (cloud, future) = match (d.chart.ichimoku, d.bars()) {
        (true, Some(bars)) => {
            let (warm, vis, _) = chart::indicator_bars(warm, bars, range, d.show_extended());
            let (ov, cloud, fut) = chart::ichimoku_overlays(&warm, &vis, &candles);
            overlays.extend(ov);
            (Some(cloud), fut)
        }
        _ => (None, 0),
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
    if is_fx {
        title.push_str(" · 24/5 · tick volume");
    } else if range.intraday() {
        title.push_str(if extended {
            " · extended hours"
        } else {
            " · regular hours"
        });
    }
    if let Some(p) = reference {
        let label = if is_fx { "day open" } else { "prev close" };
        title.push_str(&format!(" · {label} {}", fmt_price(p)));
    }
    if delayed {
        title.push_str(" · 15m delayed");
    }
    if d.chart.ichimoku {
        title.push_str(" · Ichimoku 9/26/52");
    }
    if d.chart.levels {
        title.push_str(" · S/R");
    }
    if d.chart.ma {
        title.push_str(if chart::ma_range(range) {
            " · MA 50/200"
        } else {
            " · MA: 6M/1Y only"
        });
    }
    title.push_str(" · −/+ zoom  h/l cursor  H/L pan  j/k price line  p type a price ");
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
            price_tick: price_tick(&d.symbol),
            reference,
            last_price,
            delayed,
            overlays: &overlays,
            cloud: cloud.as_ref(),
            future,
            levels: d.chart.levels,
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
    fmt_price: fn(f64) -> String,
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
    .row_highlight_style(selected_row());
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

fn render_help(frame: &mut Frame, area: Rect, mode: Mode) {
    // The journal and the calendar have their own short cards (the full
    // one is already a screenful, and none of it applies there).
    let short = match mode {
        Mode::Journal => Some(("journal", Ctx::Journal)),
        Mode::Calendar => Some(("calendar", Ctx::Calendar)),
        _ => None,
    };
    if let Some((name, ctx)) = short {
        let mut lines = help_section(name, ctx);
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "any key closes",
            Style::default().fg(DIM),
        )));
        let w = 46.min(area.width.saturating_sub(2));
        let h = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
        let rect = Rect {
            x: area.x + (area.width.saturating_sub(w)) / 2,
            y: area.y + (area.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        };
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" keys ")),
            rect,
        );
        return;
    }
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
                if cell.modifier.contains(Modifier::ITALIC) {
                    style.push_str("font-style:italic;");
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

    /// Watchlist with both sections: sample stocks + LIVE forex quotes from
    /// OANDA practice.
    /// `WICKD_TUI_SNAPSHOT_DIR=/some/dir cargo test -p wickd snapshot_watchlist_sections -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn snapshot_watchlist_sections() {
        let dir = std::env::var("WICKD_TUI_SNAPSHOT_DIR").expect("set WICKD_TUI_SNAPSHOT_DIR");
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = sample_app();
        for p in fx::DEFAULT_PAIRS {
            app.toggle_watchlist(p.to_string());
        }
        let client = super::super::oanda_client().expect("OANDA practice credentials");
        app.set_fx_quotes(fx::quotes(&client, &app.fx_symbols(), Utc::now()).await.unwrap());
        // One synthetic move each way so the up/down colouring shows: stocks
        // via a snapshot refresh, pairs via a live tick (bid and ask).
        let mut snaps = app.snapshots.clone();
        for (sym, d) in [("SPY", 0.25), ("NVDA", -0.15)] {
            let s = snaps.get_mut(sym).unwrap();
            s.latest_trade.as_mut().unwrap().price += d;
            let q = s.latest_quote.as_mut().unwrap();
            q.bid += d;
            q.ask += d;
        }
        app.set_snapshots(app.snapshots.clone());
        app.set_snapshots(snaps);
        for (sym, d) in [("EUR_USD", "0.00002"), ("USD_JPY", "-0.004"), ("GBP_USD", "-0.00001")] {
            let q = app.fx_quotes[sym].clone();
            let d: rust_decimal::Decimal = d.parse().unwrap();
            app.apply_tick(wickd_core::hub_client::HubQuote {
                instrument: sym.into(),
                bid: q.bid + d,
                ask: q.ask + d,
                time: q.time + chrono::Duration::seconds(1),
            });
        }
        app.fx_live = true;
        app.flash = None;
        // A list an agent built for tomorrow (with notes).
        let mut lists = crate::lists::ListsFile::default();
        for (sym, why, call, constraints, signal) in [
            (
                "NVDA",
                "Earnings Wed. Chips crashed last week; first test of the 185 shelf",
                "Holds 185 into earnings",
                "1% risk; stop below Fri low; half size (VIX ~24)",
                "Act if it reclaims 185 after 8:00 on above-average volume",
            ),
            ("SPY", "Open drive above 682 = trend day?", "", "Watch only before CPI", ""),
            ("EUR_USD", "ECB speakers all week", "Fades rallies into 1.1200", "", "Short a rejection at 1.1200"),
        ] {
            let text: Vec<String> = [("Expect", call), ("Go", signal), ("Limits", constraints)]
                .iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(l, v)| format!("{l}: {v}"))
                .collect();
            let plan = crate::lists::Fields {
                note: Some(why.to_string()),
                plan: (!text.is_empty()).then(|| text.join("\n")),
            };
            crate::lists::add(&mut lists, "mon-11-9", &[sym.to_string()], &plan, Utc::now());
        }
        app.set_lists(lists);
        app.selected = 0;
        let (w, h) = (158u16, 36u16);
        let shoot = |app: &App, name: &str| {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| render(f, app)).unwrap();
            std::fs::write(
                format!("{dir}/{name}.html"),
                to_html(term.backend().buffer(), name),
            )
            .unwrap();
        };
        shoot(&app, "1-watchlist-stocks-and-forex");
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('x'));
        shoot(&app, "2-g-x-jumps-to-forex");
        press(&mut app, KeyCode::Char('/'));
        for c in "eur".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        app.set_fx_assets(fx::instruments(&client).await.unwrap());
        shoot(&app, "3-search-finds-pairs");
    }

    /// A pair's chart screen with LIVE OANDA data (1D, 1Y + Ichimoku, USD_JPY).
    /// `WICKD_TUI_SNAPSHOT_DIR=/some/dir cargo test -p wickd snapshot_fx_detail -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn snapshot_fx_detail() {
        let dir = std::env::var("WICKD_TUI_SNAPSHOT_DIR").expect("set WICKD_TUI_SNAPSHOT_DIR");
        std::fs::create_dir_all(&dir).unwrap();
        let client = super::super::oanda_client().expect("OANDA practice credentials");
        let now = Utc::now();
        let (w, h) = (158u16, 44u16);
        let shoot = |app: &App, name: &str| {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| render(f, app)).unwrap();
            term.draw(|f| render(f, app)).unwrap();
            std::fs::write(
                format!("{dir}/{name}.html"),
                to_html(term.backend().buffer(), name),
            )
            .unwrap();
        };
        for (sym, n) in [("EUR_USD", "1"), ("USD_JPY", "3")] {
            let mut app = App::new(vec![sym.to_string()], "IEX");
            app.set_fx_assets(fx::instruments(&client).await.unwrap());
            app.fx_quotes = fx::quotes(&client, &[sym.to_string()], now).await.unwrap();
            app.fx_live = true;
            press(&mut app, KeyCode::Enter);
            for r in [HistoryRange::Day1, HistoryRange::Year1] {
                let (warm, vis) = fx::history(&client, sym, r, now).await.unwrap();
                app.set_warmup(sym, r, warm);
                app.set_history(sym, r, Ok(vis));
            }
            app.flash = None;
            let tag = sym.to_lowercase();
            shoot(&app, &format!("{n}-{tag}-1d"));
            if sym == "EUR_USD" {
                press(&mut app, KeyCode::Char('5'));
                press(&mut app, KeyCode::Char('v'));
                press(&mut app, KeyCode::Char('i'));
                app.flash = None;
                shoot(&app, "2-eur_usd-1y-ichimoku");
            }
        }
    }

    /// Info cards with LIVE data (`wickd info` sources): QQQ, NVDA (on a
    /// list), EUR_USD.
    /// `WICKD_TUI_SNAPSHOT_DIR=/some/dir cargo test -p wickd snapshot_info_cards -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn snapshot_info_cards() {
        let dir = std::env::var("WICKD_TUI_SNAPSHOT_DIR").expect("set WICKD_TUI_SNAPSHOT_DIR");
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = sample_app();
        let (w, h) = (158u16, 44u16);
        for (n, sym) in [("1", "QQQ"), ("2", "NVDA"), ("3", "EUR_USD")] {
            let mut info = crate::info::fetch(sym, false).await.unwrap();
            if sym == "NVDA" {
                info.lists = vec![crate::info::ListNote {
                    list: "tomorrow".into(),
                    note: Some("earnings Wed; watch the 185 break".into()),
                    plan: Some("Pullback first, then higher.\nBuy a reclaim of 185 after 8:00; stop below 180.".into()),
                }];
            }
            app.info = Some(crate::tui::app::InfoCard {
                symbol: sym.into(),
                state: crate::tui::app::InfoState::Ready(Box::new(info)),
                scroll: 0,
            });
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| render(f, &app)).unwrap();
            let name = format!("{n}-info-{}", sym.to_lowercase());
            std::fs::write(
                format!("{dir}/{name}.html"),
                to_html(term.backend().buffer(), &name),
            )
            .unwrap();
        }
    }

    /// Matt's "train wreck" case: five day lists × 18 symbols, 9 stocks and 5
    /// pairs, on a ~158×46 terminal. Offline (sample quotes only).
    /// `WICKD_TUI_SNAPSHOT_DIR=/some/dir cargo test -p wickd snapshot_day_lists -- --ignored`
    #[test]
    #[ignore]
    fn snapshot_day_lists() {
        let dir = std::env::var("WICKD_TUI_SNAPSHOT_DIR").expect("set WICKD_TUI_SNAPSHOT_DIR");
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = sample_app();
        // Real company names from the local symbol cache, when there is one.
        #[derive(serde::Deserialize)]
        struct Cache {
            assets: Vec<Asset>,
        }
        let path = wickd_core::paths::wickd_data_home()
            .unwrap()
            .join("cache/alpaca-assets.json");
        if let Some(c) = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Cache>(&b).ok())
        {
            app.set_assets(c.assets);
        }
        for s in ["AAPL", "MSFT", "AMZN", "META", "GOOGL", "WDAY"] {
            app.toggle_watchlist(s.to_string());
        }
        for p in fx::DEFAULT_PAIRS {
            app.toggle_watchlist(p.to_string());
        }
        let syms = [
            "NET", "ABNB", "FROG", "IREN", "IONQ", "AXON", "DKNG", "TEAM", "AMD", "LRCX", "CEG",
            "NVDA", "TSLA", "PLTR", "SHOP", "SNOW", "COIN", "UBER",
        ];
        let made: DateTime<Utc> = "2026-11-09T06:00:00Z".parse().unwrap();
        let mut f = crate::lists::ListsFile::default();
        for day in ["fri-11-13", "mon-11-9", "thu-11-12", "tue-11-10", "wed-11-11"] {
            for (i, s) in syms.iter().enumerate() {
                let plan = crate::lists::Fields {
                    note: Some(format!("earnings reaction, {}% Fri", 5 + i)),
                    plan: (i % 3 != 2).then(|| {
                        "Strong report, but the market is jumpy (VIX ~24).\n\nExpect it to hold Friday's close and drift higher.\nGo: above Friday's close after 8:00 on heavy volume. Stop below Friday's low, half size."
                            .into()
                    }),
                };
                crate::lists::add(&mut f, day, &[s.to_string()], &plan, made);
            }
        }
        app.set_lists(f);
        app.flash = None;
        let (w, h) = (158u16, 46u16);
        let shoot = |app: &App, name: &str| {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| render(f, app)).unwrap();
            std::fs::write(
                format!("{dir}/{name}.html"),
                to_html(term.backend().buffer(), name),
            )
            .unwrap();
        };
        // Open mon-11-9 (Matt's "today"), cursor on its first row.
        app.expanded = 0;
        app.selected = 0;
        shoot(&app, "1-mon-open-plan");
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('l'));
        app.flash = None;
        shoot(&app, "2-g-l-opens-tue");
        // A row with no plan: no plan box.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        shoot(&app, "3-row-without-plan");
    }

    fn journal_app(dir: &std::path::Path) -> App {
        let mut app = App::new(vec!["AAPL".into()], "iex");
        app.journal = crate::tui::journal::JournalState::new(dir.to_path_buf());
        app
    }

    fn screen(app: &App, w: u16, h: u16) -> Vec<String> {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render(f, app)).unwrap();
        let buf = term.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect()
    }

    #[test]
    fn journal_screen_shows_the_newest_entry_and_scrolls() {
        let tmp = tempfile::tempdir().unwrap();
        let body: String = (1..=60).map(|i| format!("para {i}\n\n")).collect();
        std::fs::write(tmp.path().join("2026-11-12.md"), "# Thursday").unwrap();
        std::fs::write(
            tmp.path().join("2026-11-13.md"),
            format!("# Friday\n\n{body}"),
        )
        .unwrap();
        let mut app = journal_app(tmp.path());
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.mode, Mode::Journal);
        let rows = screen(&app, 80, 20);
        assert!(
            rows[1].contains("Journal · 2026-11-13 · 2 of 2"),
            "{}",
            rows[1]
        );
        assert!(rows[2].contains("Friday") && rows[5].contains("para 1"));
        assert!(rows[18].contains(" top "), "{}", rows[18]);
        // J pages down; g e goes to the end; g g back to the top.
        press(&mut app, KeyCode::Char('J'));
        let rows = screen(&app, 80, 20);
        assert!(!rows.iter().any(|r| r.contains("Friday")));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('e'));
        let rows = screen(&app, 80, 20);
        assert!(rows[17].contains("para 60") && rows[18].contains(" end "));
        // h = the older entry, from its top.
        press(&mut app, KeyCode::Char('h'));
        let rows = screen(&app, 80, 20);
        assert!(rows[1].contains("2026-11-12 · 1 of 2") && rows[2].contains("Thursday"));
        assert!(rows[18].contains(" all "));
        // ? lists this screen's keys, all of them on one card.
        press(&mut app, KeyCode::Char('?'));
        let help = screen(&app, 80, 30).join("\n");
        for k in [
            "older entry",
            "newer entry",
            "next heading",
            "reload from disk",
            "g w",
            "back",
        ] {
            assert!(help.contains(k), "help is missing {k}:\n{help}");
        }
        press(&mut app, KeyCode::Char('?'));
        // esc returns to where g j was pressed.
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Watchlist);
    }

    #[test]
    fn journal_empty_state_says_where_it_looks() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("missing");
        let mut app = journal_app(&dir);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('j'));
        let all = screen(&app, 120, 12).join("\n");
        assert!(all.contains("No journal entries yet."));
        assert!(all.contains("missing"), "{all}");
        assert!(all.contains("WICKD_JOURNAL_DIR"));
    }

    /// An app whose calendar reads temp stores and a temp slate, at 8:00 AM
    /// Mountain on Wed Nov 18, 2026. Slate: NVDA (reported Tuesday after the
    /// close) and TGT (this morning). Watchlist: PANW.
    fn calendar_app(tmp: &std::path::Path) -> App {
        use rust_decimal_macros::dec;
        use wickd_core::calendar_store::{merge_into_store, CalendarEvent};
        use wickd_core::earnings_store::{write_day, DayFile, EarningsRow, Session};
        let ev = |date: &str, time: &str, event: &str, impact: &str, forecast: &str| CalendarEvent {
            date: date.into(),
            time: time.into(),
            currency: "USD".into(),
            event: event.into(),
            impact: impact.into(),
            actual: String::new(),
            forecast: forecast.into(),
            previous: "1.40M".into(),
        };
        let econ_dir = tmp.join("calendar");
        std::fs::create_dir_all(&econ_dir).unwrap();
        merge_into_store(
            &econ_dir,
            vec![
                ev("2026-11-18", "13:30", "Housing Starts", "low", "1.25M"),
                ev("2026-11-18", "19:00", "FOMC Meeting Minutes", "high", ""),
                ev("2026-11-19", "13:30", "Unemployment Claims", "medium", "198K"),
            ],
        )
        .unwrap();
        let row = |date: &str, symbol: &str, name: &str, session, cap, eps| EarningsRow {
            date: date.parse().unwrap(),
            symbol: symbol.into(),
            name: name.into(),
            session,
            eps_forecast: Some(eps),
            estimates: Some(9),
            market_cap: Some(cap),
            last_year_eps: Some(dec!(1.78)),
            fiscal_quarter: "Oct/2026".into(),
        };
        let earn_dir = tmp.join("earnings");
        let file = |date: &str, rows: Vec<EarningsRow>| DayFile {
            date: date.parse().unwrap(),
            fetched_at: "2026-11-18T00:30:00Z".parse().unwrap(),
            source: "nasdaq".into(),
            rows,
        };
        write_day(
            &earn_dir,
            &file(
                "2026-11-17",
                vec![row(
                    "2026-11-17",
                    "NVDA",
                    "NVIDIA Corporation",
                    Session::AfterHours,
                    dec!(3762974100000),
                    dec!(2.49),
                )],
            ),
        )
        .unwrap();
        write_day(
            &earn_dir,
            &file(
                "2026-11-18",
                vec![
                    row(
                        "2026-11-18",
                        "TJX",
                        "TJX Companies, Inc. (The)",
                        Session::PreMarket,
                        dec!(144195600000),
                        dec!(1.33),
                    ),
                    row(
                        "2026-11-18",
                        "TGT",
                        "Target Corporation",
                        Session::PreMarket,
                        dec!(72832852000),
                        dec!(2.05),
                    ),
                    row(
                        "2026-11-18",
                        "HUYA",
                        "HUYA Inc.",
                        Session::PreMarket,
                        dec!(475001472),
                        dec!(0.02),
                    ),
                    row(
                        "2026-11-18",
                        "PANW",
                        "Palo Alto Networks, Inc.",
                        Session::AfterHours,
                        dec!(258701689000),
                        dec!(0.52),
                    ),
                ],
            ),
        )
        .unwrap();
        let pick = |symbol: &str, date: &str, session, why: &str, expect: Option<&str>| crate::slate::Entry {
            symbol: symbol.into(),
            date: date.parse().unwrap(),
            session,
            why: Some(why.into()),
            expect: expect.map(str::to_string),
            result: None,
            added: "2026-11-15T03:00:00Z".parse().unwrap(),
        };
        let slate_path = tmp.join("slate.json");
        let slate = crate::slate::SlateFile {
            version: 1,
            entries: vec![
                pick(
                    "NVDA",
                    "2026-11-17",
                    Session::AfterHours,
                    "sets the tone for AI",
                    Some("Beat and raise: +5%.\nA cut to the outlook: -10%."),
                ),
                pick("TGT", "2026-11-18", Session::PreMarket, "consumer read", None),
            ],
        };
        std::fs::write(&slate_path, serde_json::to_vec_pretty(&slate).unwrap()).unwrap();
        let mut app = App::new(vec!["PANW".into()], "iex");
        app.calendar =
            crate::tui::calendar::CalendarState::new(Some(econ_dir), Some(earn_dir), Some(slate_path));
        app.calendar.clock = crate::tui::calendar::Clock::mst("2026-11-18T15:00:00Z");
        app
    }

    /// `g c`, with the opening fetches answered (no reactions yet).
    fn open_calendar(app: &mut App) {
        press(app, KeyCode::Char('g'));
        press(app, KeyCode::Char('c'));
        app.set_earnings_fetch(Ok(vec![]));
        app.set_reactions(Ok(vec![]));
        app.set_headlines(Ok((0, 0, None)));
        app.set_results(Ok(vec![]));
        app.flash = None;
    }

    /// Store a report's results headlines, the way a headlines fetch would.
    fn store_headlines(app: &App, symbol: &str, date: &str, items: &[(&str, &str)]) {
        use wickd_core::earnings_headlines::{headlines_dir, store_fetch};
        use wickd_core::equities::alpaca::NewsItem;
        let items: Vec<NewsItem> = items
            .iter()
            .enumerate()
            .map(|(i, (time, headline))| NewsItem {
                id: i as i64,
                headline: headline.to_string(),
                summary: String::new(),
                created_at: time.parse().unwrap(),
                symbols: vec![symbol.to_string()],
                source: "benzinga".into(),
                url: None,
            })
            .collect();
        store_fetch(
            &headlines_dir(app.calendar.earn_dir.as_ref().unwrap()),
            symbol,
            date.parse().unwrap(),
            &items,
            Utc::now(),
        )
        .unwrap();
    }

    /// Store a report's press release (its title and its text), the way a
    /// headlines fetch that read the body would.
    fn store_release(app: &App, symbol: &str, date: &str, title: (&str, &str), html: &str) {
        use wickd_core::earnings_headlines::{headlines_dir, lede, store_fetch_with};
        use wickd_core::equities::alpaca::NewsItem;
        let item = NewsItem {
            id: 7,
            headline: title.1.to_string(),
            summary: String::new(),
            created_at: title.0.parse().unwrap(),
            symbols: vec![symbol.to_string()],
            source: "fiscal.ai".into(),
            url: None,
        };
        let mut ledes = std::collections::HashMap::new();
        ledes.insert(7, lede(html, ""));
        store_fetch_with(
            &headlines_dir(app.calendar.earn_dir.as_ref().unwrap()),
            symbol,
            date.parse().unwrap(),
            &[item],
            &ledes,
            Utc::now(),
        )
        .unwrap();
    }

    /// Store what a company reported, the way a results fetch would.
    fn store_reported(app: &App, symbol: &str, date: &str, eps: &str, consensus: &str, surprise: &str) {
        use wickd_core::earnings_results::{results_dir, write_results, ResultRow, ResultsFile};
        write_results(
            &results_dir(app.calendar.earn_dir.as_ref().unwrap()),
            &ResultsFile {
                symbol: symbol.into(),
                fetched_at: Utc::now(),
                source: "nasdaq".into(),
                rows: vec![
                    ResultRow {
                        fiscal_quarter: "Oct 2026".into(),
                        reported: date.parse().unwrap(),
                        eps: Some(eps.parse().unwrap()),
                        consensus: Some(consensus.parse().unwrap()),
                        surprise_pct: Some(surprise.parse().unwrap()),
                    },
                    // Last quarter's row is always there too.
                    ResultRow {
                        fiscal_quarter: "Jul 2026".into(),
                        reported: "2026-08-26".parse().unwrap(),
                        eps: Some("9.99".parse().unwrap()),
                        consensus: Some("9.00".parse().unwrap()),
                        surprise_pct: Some("11".parse().unwrap()),
                    },
                ],
            },
        )
        .unwrap();
    }

    fn reacted(day: &str, gap: f64, change: f64, complete: bool) -> wickd_core::earnings_reaction::Reaction {
        wickd_core::earnings_reaction::Reaction {
            day: day.parse().unwrap(),
            complete,
            prior_close: 100.0,
            open: 100.0 + gap,
            close: 100.0 + change,
            gap_pct: gap,
            change_pct: change,
            close_in_range: Some(80.0),
            rel_volume: Some(2.5),
        }
    }

    /// NVDA's finished reaction and TGT's so far.
    fn give_reactions(app: &mut App) {
        use wickd_core::earnings_store::Session;
        app.set_reactions(Ok(vec![
            (
                ("NVDA".into(), "2026-11-17".parse().unwrap(), Session::AfterHours),
                reacted("2026-11-18", 2.1, 4.2, true),
            ),
            (
                ("TGT".into(), "2026-11-18".parse().unwrap(), Session::PreMarket),
                reacted("2026-11-18", -1.5, -3.1, false),
            ),
        ]));
    }

    #[test]
    fn calendar_watch_shows_the_slate_the_lists_and_the_big_releases() {
        use crate::tui::app::Action;
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        press(&mut app, KeyCode::Char('g'));
        // Days are missing from the store, and two watched reports are out:
        // opening asks for the days, their reactions, their results headlines
        // and the reported figures.
        let actions = press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.mode, Mode::Calendar);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::FetchEarnings(d), Action::FetchReactions(r), Action::FetchHeadlines(h), Action::FetchResults(f)]
                    if d.len() == 8 && r.len() == 2 && h.len() == 2 && f == &["NVDA", "TGT"]
            ),
            "{actions:?}"
        );
        let rows = screen(&app, 130, 30);
        assert!(rows[1].contains("Calendar · your earnings watch"), "{}", rows[1]);
        assert!(rows[1].contains("fetching earnings…"), "{}", rows[1]);
        app.set_earnings_fetch(Ok(vec![]));
        let rows = screen(&app, 130, 30);
        // 00:30 UTC on the 18th is Tuesday 5:30 PM in Denver.
        assert!(rows[1].contains("earnings as of Tue 5:30 PM"), "{}", rows[1]);
        let all = rows.join("\n");
        let line = |needle: &str| {
            rows.iter()
                .find(|r| r.contains(needle))
                .unwrap_or_else(|| panic!("no line with {needle:?} in\n{all}"))
                .clone()
        };
        // One caption row for the whole screen.
        for caption in ["TIME", "TYPE", "EVENT", "REACTION", "SIZE/IMPACT", "EXPECTED", "PREVIOUS"] {
            assert!(rows[2].contains(caption), "{caption}: {}", rows[2]);
        }
        assert_eq!(all.matches("EXPECTED").count(), 1);
        assert!(!all.contains("Earnings ·"), "no captions between events: {all}");
        // Yesterday stays for the reaction, then today.
        assert!(rows[3].contains("Yesterday · Tue Nov 17"), "{}", rows[3]);
        let nvda = line("NVIDIA Corporation");
        for part in ["post-close", "earnings", "◆ NVDA", "· sets the tone for AI", "$3.76T", "EPS 2.49"] {
            assert!(nvda.contains(part), "{part}: {nvda}");
        }
        assert!(all.contains("Today · Wed Nov 18"));
        let tgt = line("Target Corporation");
        assert!(tgt.contains("◆ TGT") && tgt.contains("· consumer read") && tgt.contains("pre-open"), "{tgt}");
        // The one high-impact release; the low and medium ones aren't on the watch.
        let minutes = line("FOMC Meeting Minutes");
        assert!(
            minutes.contains("12:00 PM") && minutes.contains("economic") && minutes.contains("high"),
            "{minutes}"
        );
        assert!(!all.contains("Housing Starts") && !all.contains("Unemployment Claims"));
        // On the watchlist: shown and marked differently from the slate.
        assert!(line("Palo Alto").contains("★ PANW"), "{}", line("Palo Alto"));
        // Bigger than anything shown, and not picked: not here.
        assert!(!all.contains("TJX Companies") && !all.contains("HUYA"));
        assert!(all.contains("2 other companies report · e shows everything"));
        // Event titles and tickers start in the same column.
        let column = |line: &str, needle: &str| line[..line.find(needle).unwrap()].chars().count();
        assert_eq!(
            column(&minutes, "FOMC Meeting"),
            column(&tgt, "TGT    Target"),
            "{minutes}\n{tgt}"
        );
        // Every drawn line is the same width: columns line up.
        assert!(rows[1..29].iter().all(|r| r.chars().count() == 130));

        // The reactions land: a figure on each watched row that has one.
        give_reactions(&mut app);
        let rows = screen(&app, 130, 30);
        let find = |needle: &str| rows.iter().find(|r| r.contains(needle)).unwrap().clone();
        assert!(find("NVIDIA Corporation").contains("+4.2%"), "{}", find("NVIDIA Corporation"));
        // Today's session is still open: marked as so far.
        assert!(find("Target Corporation").contains("-3.1%…"), "{}", find("Target Corporation"));
        assert!(!find("Palo Alto").contains('%'), "it hasn't reported yet");
    }

    #[test]
    fn calendar_box_reads_expectation_then_result_then_reaction() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        open_calendar(&mut app);
        // The cursor opens on the Fed minutes: nothing to say about a release.
        let all = screen(&app, 130, 30).join("\n");
        assert!(!all.contains("· expectation") && !all.contains("· result"), "{all}");
        // NVDA reported last night. Its expectation as written, and the
        // result said plainly even though the figures aren't in yet.
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.calendar.selected_symbol(), Some("NVDA"));
        let all = screen(&app, 130, 30).join("\n");
        assert!(all.contains(" NVDA · expectation · result "), "{all}");
        assert!(all.contains("Beat and raise: +5%.") && all.contains("A cut to the outlook: -10%."));
        assert!(
            all.contains("No results in yet: no headline from the news feed and no reported figures. Press r to update."),
            "{all}"
        );
        assert!(!all.contains("Reaction"));
        // While a fetch for them is out, it says that instead.
        app.calendar.headlines_loading = true;
        assert!(screen(&app, 130, 30).join("\n").contains("Fetching the results…"));
        // The headlines arrive, the same evening: shown as published with
        // their local time (21:21 UTC = 2:21 PM in Denver), the results line
        // first, and the "nothing yet" line is gone. Chatter isn't shown.
        store_headlines(
            &app,
            "NVDA",
            "2026-11-17",
            &[
                ("2026-11-17T21:05:00Z", "Why NVIDIA Shares Are Rising After Hours"),
                ("2026-11-17T21:22:00Z", "NVIDIA Sees Q4 Sales $130.00B +/- 3% vs $125.30B Est"),
                (
                    "2026-11-17T21:21:00Z",
                    "NVIDIA Q3 Adj. EPS $2.68 Beats $2.47 Estimate, Sales $115.80B Beat $109.15B Estimate",
                ),
            ],
        );
        app.set_headlines(Ok((1, 0, None)));
        let rows = screen(&app, 130, 30);
        let all = rows.join("\n");
        let row_of = |needle: &str| {
            rows.iter()
                .position(|r| r.contains(needle))
                .unwrap_or_else(|| panic!("no line with {needle:?} in\n{all}"))
        };
        assert!(
            rows[row_of("NVIDIA Q3 Adj. EPS $2.68 Beats")].contains("Tue 2:21 PM"),
            "{all}"
        );
        assert!(row_of("NVIDIA Q3 Adj. EPS $2.68 Beats") < row_of("Tue 2:22 PM  NVIDIA Sees Q4 Sales"));
        assert!(!all.contains("No results in yet") && !all.contains("Why NVIDIA"), "{all}");
        // The slower reported-figures table catches up later: its sentence
        // goes under the headlines, matched to this report (not last
        // quarter's row).
        store_reported(&app, "NVDA", "2026-11-17", "2.68", "2.49", "7.63");
        app.set_results(Ok(vec![]));
        let rows = screen(&app, 130, 32);
        let all = rows.join("\n");
        assert!(
            all.contains("Reported: EPS 2.68 against 2.49 expected, a beat of 7.6%."),
            "{all}"
        );
        assert!(!all.contains("9.99"));
        let row_of = |needle: &str| rows.iter().position(|r| r.contains(needle)).unwrap();
        assert!(row_of("NVIDIA Sees Q4 Sales") < row_of("Reported: EPS 2.68"));
        // The note written after the report sits under the figures.
        let path = app.calendar.slate_path.clone().unwrap();
        crate::slate::update_at(&path, |f| {
            crate::slate::set(
                f,
                "NVDA",
                &crate::slate::Change {
                    result: Some("Guided next quarter to 130B.\nMargins guided down.".into()),
                    ..Default::default()
                },
            )
        })
        .unwrap();
        app.reload_calendar();
        // With its reaction in, all three, in the order they happened.
        give_reactions(&mut app);
        let rows = screen(&app, 130, 34);
        let all = rows.join("\n");
        assert!(all.contains(" NVDA · expectation · result · reaction "), "{all}");
        let at = |needle: &str| {
            rows.iter()
                .position(|r| r.contains(needle))
                .unwrap_or_else(|| panic!("no line with {needle:?} in\n{all}"))
        };
        assert!(at("Beat and raise: +5%.") < at("Reported: EPS 2.68"));
        assert!(at("Reported: EPS 2.68") < at("Guided next quarter to 130B."));
        assert!(at("Guided next quarter to 130B.") < at("Margins guided down."));
        assert!(at("Margins guided down.") < at("Reaction on Wed Nov 18"));
        assert!(
            all.contains(
                "Reaction on Wed Nov 18: opened +2.1%, closed +4.2%, 80% of the way up the day's range, on 2.5× its usual volume."
            ),
            "{all}"
        );
        // The list is still there above the box, captions and all.
        assert!(all.contains("TIME") && all.contains("Today · Wed Nov 18"));
        // TGT reported this morning: no expectation written, a miss, and its
        // unfinished day is "so far".
        store_reported(&app, "TGT", "2026-11-18", "1.90", "2.05", "-7.32");
        app.set_results(Ok(vec![]));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.calendar.selected_symbol(), Some("TGT"));
        let all = screen(&app, 130, 30).join("\n");
        assert!(all.contains(" TGT · result · reaction "), "{all}");
        assert!(all.contains("Reported: EPS 1.90 against 2.05 expected, a miss of 7.3%."), "{all}");
        assert!(
            all.contains(
                "Reaction so far (Wed Nov 18): opened -1.5%, now -3.1%, 80% of the way up the day's range, on 2.5× its usual volume so far."
            ),
            "{all}"
        );
        // PANW reports tonight: on the watchlist, nothing to say yet.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.calendar.selected_symbol(), Some("PANW"));
        let all = screen(&app, 130, 30).join("\n");
        assert!(!all.contains("· result") && !all.contains("Reported:"), "{all}");
        // A failed reaction fetch says so along the bottom and keeps the figures.
        app.set_reactions(Err("rate limited".into()));
        let all = screen(&app, 130, 30).join("\n");
        assert!(all.contains("couldn't fetch reactions: rate limited"), "{all}");
        assert!(all.contains("-3.1%…"));
    }

    #[test]
    fn reported_figures_are_fetched_for_watched_reports_that_are_out_and_not_too_often() {
        use crate::tui::app::Action;
        use wickd_core::earnings_results::ResultOutcome;
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        press(&mut app, KeyCode::Char('g'));
        let actions = press(&mut app, KeyCode::Char('c'));
        // NVDA and TGT have reported; PANW (tonight) hasn't, and TJX isn't watched.
        assert!(
            actions.iter().any(|a| matches!(a, Action::FetchResults(s) if s == &["NVDA", "TGT"])),
            "{actions:?}"
        );
        assert!(app.calendar.results_loading);
        // While that request is out, nothing more is asked.
        assert_eq!(app.results_action(true), None);
        // It comes back with nothing new (the feed lags): the box says so,
        // and the timer doesn't ask again straight away.
        app.set_results(Ok(vec![]));
        assert!(!app.calendar.results_loading && app.calendar.results_error.is_none());
        assert_eq!(app.results_action(false), None, "asked moments ago");
        // r asks again regardless.
        app.set_earnings_fetch(Ok(vec![]));
        let actions = press(&mut app, KeyCode::Char('r'));
        assert!(
            actions.iter().any(|a| matches!(a, Action::FetchResults(s) if s == &["NVDA", "TGT"])),
            "{actions:?}"
        );
        // NVDA's figures land; TGT's fetch fails. Only TGT is still wanted,
        // and the failure is said along the bottom.
        store_reported(&app, "NVDA", "2026-11-17", "2.68", "2.49", "7.63");
        let outcome = |symbol: &str, error: Option<&str>| ResultOutcome {
            symbol: symbol.into(),
            rows: None,
            newest: None,
            written: error.is_none(),
            kept_stored: false,
            error: error.map(str::to_string),
        };
        app.set_results(Ok(vec![outcome("NVDA", None), outcome("TGT", Some("HTTP 503"))]));
        assert_eq!(app.calendar.results_wanted(true), ["TGT"]);
        app.set_earnings_fetch(Ok(vec![]));
        let all = screen(&app, 130, 30).join("\n");
        assert!(
            all.contains("couldn't fetch reported figures for 1 of 2 companies (HTTP 503)"),
            "{all}"
        );
        // Figures stored only for last quarter don't count as this report's.
        store_reported(&app, "TGT", "2026-08-19", "2.10", "2.00", "5");
        app.set_results(Ok(vec![]));
        assert_eq!(app.calendar.results_wanted(true), ["TGT"]);
        // Removing the watch removes the ask: nothing is fetched for
        // companies nobody picked.
        app.calendar.slate_remove("TGT").unwrap();
        app.reload_calendar();
        assert!(app.calendar.results_wanted(true).is_empty());
    }

    #[test]
    fn calendar_s_puts_a_company_on_the_slate_and_takes_it_off() {
        use crate::tui::app::Action;
        use wickd_core::earnings_store::Session;
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        open_calendar(&mut app);
        // On the Fed minutes: not a company.
        assert!(press(&mut app, KeyCode::Char('s')).is_empty());
        assert_eq!(app.flash.as_deref(), Some("only a company's row goes on the slate"));
        // PANW is on the watchlist, not the slate: s adds it for this report.
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.calendar.selected_symbol(), Some("PANW"));
        let actions = press(&mut app, KeyCode::Char('s'));
        let [Action::SlateAdd { symbol, date, session }] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(
            (symbol.as_str(), date.to_string().as_str(), *session),
            ("PANW", "2026-11-18", Session::AfterHours)
        );
        // What the event loop does with it.
        let r = app.calendar.slate_add(symbol, *date, *session);
        app.slate_written(r.map(|()| "PANW is on the slate".to_string()));
        assert_eq!(app.flash.as_deref(), Some("PANW is on the slate"));
        assert_eq!(app.calendar.selected_symbol(), Some("PANW"), "cursor kept");
        assert!(screen(&app, 130, 30).join("\n").contains("◆ PANW"));
        let path = app.calendar.slate_path.clone().unwrap();
        let stored = crate::slate::load_at(&path).unwrap();
        assert_eq!(stored.entries.len(), 3);
        assert_eq!(stored.entries[0].why.as_deref(), Some("sets the tone for AI"), "the others are untouched");
        // No notes on it: one s takes it straight off.
        let actions = press(&mut app, KeyCode::Char('s'));
        assert!(matches!(actions.as_slice(), [Action::SlateRemove(s)] if s == "PANW"), "{actions:?}");
        let r = app.calendar.slate_remove("PANW");
        app.slate_written(r.map(|()| "PANW is off the slate".to_string()));
        assert!(screen(&app, 130, 30).join("\n").contains("★ PANW"), "back to a list row");

        // NVDA has notes: the first s only warns, and any other key cancels.
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('g'));
        assert!(press(&mut app, KeyCode::Char('s')).is_empty());
        assert_eq!(
            app.flash.as_deref(),
            Some("NVDA has notes on the slate; press s again to remove it and them")
        );
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('k'));
        assert!(press(&mut app, KeyCode::Char('s')).is_empty(), "the warning starts over");
        let actions = press(&mut app, KeyCode::Char('s'));
        assert!(matches!(actions.as_slice(), [Action::SlateRemove(s)] if s == "NVDA"), "{actions:?}");
        let r = app.calendar.slate_remove("NVDA");
        app.slate_written(r.map(|()| "NVDA is off the slate".to_string()));
        assert_eq!(crate::slate::load_at(&path).unwrap().entries.len(), 1);
        // Off the slate and on no list, yesterday's report leaves the watch.
        assert!(!screen(&app, 130, 30).join("\n").contains("NVIDIA"));
    }

    #[test]
    fn calendar_everything_browses_the_full_feed_to_add_from() {
        use crate::tui::app::Action;
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        open_calendar(&mut app);
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.flash.as_deref(), Some("showing everything that reports"));
        app.flash = None;
        let rows = screen(&app, 130, 34);
        let all = rows.join("\n");
        assert!(rows[1].contains("Calendar · everything that reports"), "{}", rows[1]);
        assert!(all.contains("TJX Companies") && all.contains("HUYA Inc.") && all.contains("Housing Starts"));
        assert!(all.contains("Tomorrow · Thu Nov 19") && all.contains("Unemployment Claims"));
        assert!(all.contains("No earnings stored for this day yet. Press r to fetch."));
        // The day reads in time order: morning reports, the noon release,
        // then the evening report.
        let at = |needle: &str| rows.iter().position(|r| r.contains(needle)).unwrap();
        assert!(at("TJX Companies") < at("FOMC Meeting Minutes"));
        assert!(at("FOMC Meeting Minutes") < at("Palo Alto"));
        // A company nobody picked that has already reported is dimmed text,
        // but can still be added: s on TJX.
        while app.calendar.selected_symbol() != Some("TJX") {
            press(&mut app, KeyCode::Char('k'));
        }
        let actions = press(&mut app, KeyCode::Char('s'));
        assert!(
            matches!(actions.as_slice(), [Action::SlateAdd { symbol, .. }] if symbol == "TJX"),
            "{actions:?}"
        );
        let r = app.calendar.slate_add("TJX", "2026-11-18".parse().unwrap(), wickd_core::earnings_store::Session::PreMarket);
        let more = app.slate_written(r.map(|()| "TJX is on the slate".to_string()));
        // It reported this morning, so its reaction, its headlines and what
        // it reported are asked for at once.
        assert!(
            matches!(
                more.as_slice(),
                [Action::FetchReactions(r), Action::FetchHeadlines(h), Action::FetchResults(f)]
                    if r.iter().any(|a| a.0 == "TJX") && h.len() == 1 && h[0].0 == "TJX" && f == &["TJX"]
            ),
            "{more:?}"
        );
        // Back on the watch, it is there.
        press(&mut app, KeyCode::Char('e'));
        let all = screen(&app, 130, 30).join("\n");
        assert!(all.contains("◆ TJX") && !all.contains("HUYA"), "{all}");
    }

    #[test]
    fn calendar_rows_open_charts_and_esc_comes_back() {
        use crate::tui::app::Action;
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        open_calendar(&mut app);
        // The cursor starts on the next thing to come, the Fed minutes: not
        // a company.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Calendar);
        assert_eq!(
            app.flash.as_deref(),
            Some("only a company's row opens a chart")
        );
        press(&mut app, KeyCode::Char('j')); // PANW, after the close
        let actions = press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Detail);
        assert_eq!(app.detail.as_ref().map(|d| d.symbol.as_str()), Some("PANW"));
        assert!(actions
            .iter()
            .any(|a| matches!(a, Action::FetchHistory(s, _) if s == "PANW")));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Calendar);
        assert_eq!(app.calendar.selected_symbol(), Some("PANW"), "cursor kept");
        // r asks for a fetch; a second r while it runs doesn't.
        let actions = press(&mut app, KeyCode::Char('r'));
        assert!(matches!(actions.first(), Some(Action::FetchEarnings(_))), "{actions:?}");
        assert!(press(&mut app, KeyCode::Char('r')).is_empty());
        assert_eq!(app.flash.as_deref(), Some("already updating"));
        // A failed fetch keeps the stored rows and says so along the bottom.
        app.set_earnings_fetch(Err("offline".into()));
        let rows = screen(&app, 130, 30);
        assert!(
            rows[28].contains("couldn't fetch earnings: offline"),
            "{}",
            rows[28]
        );
        assert!(rows.join("\n").contains("Target Corporation"));
        // esc returns to where the calendar was opened from.
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Watchlist);
    }

    #[test]
    fn calendar_r_updates_everything_and_says_what_came_in() {
        use crate::tui::app::Action;
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        open_calendar(&mut app);
        // The evening: one key asks for all of it, whatever was asked when.
        let actions = press(&mut app, KeyCode::Char('r'));
        assert!(
            matches!(
                actions.as_slice(),
                [Action::FetchEarnings(_), Action::FetchReactions(r), Action::FetchHeadlines(h), Action::FetchResults(f)]
                    if r.len() == 2 && h.len() == 2 && f.len() == 2
            ),
            "{actions:?}"
        );
        assert_eq!(app.flash.as_deref(), Some("updating everything on your watch…"));
        assert!(screen(&app, 130, 30)[1].contains("updating everything…"));
        // A second r while it runs doesn't start another.
        assert!(press(&mut app, KeyCode::Char('r')).is_empty());
        assert_eq!(app.flash.as_deref(), Some("already updating"));
        // The pieces come back one by one; nothing is announced until the
        // last of them is in.
        app.set_earnings_fetch(Ok(vec![]));
        store_headlines(
            &app,
            "NVDA",
            "2026-11-17",
            &[("2026-11-17T21:21:00Z", "NVIDIA Q3 Adj. EPS $2.68 Beats $2.47 Estimate")],
        );
        app.set_headlines(Ok((2, 0, None)));
        app.set_results(Ok(vec![]));
        assert!(app.calendar.updating.is_some(), "the reactions are still out");
        give_reactions(&mut app);
        assert!(app.calendar.updating.is_none());
        // NVDA got headlines and a reaction, TGT a reaction: both are new,
        // in screen order.
        assert_eq!(app.flash.as_deref(), Some("Updated. New for NVDA, TGT."));
        assert!(!screen(&app, 130, 30)[1].contains("updating"));
        // Straight away again: everything is asked for again, and nothing new comes back.
        app.flash = None;
        let actions = press(&mut app, KeyCode::Char('r'));
        assert!(matches!(actions.first(), Some(Action::FetchEarnings(_))), "{actions:?}");
        // TGT still has no headline, and NVDA has a title but not the
        // release's figures: both are asked about again, despite having been
        // asked a moment ago.
        assert!(
            actions.iter().any(|a| matches!(a, Action::FetchHeadlines(h) if h.len() == 2)),
            "{actions:?}"
        );
        app.set_earnings_fetch(Ok(vec![]));
        app.set_headlines(Ok((2, 0, None)));
        app.set_results(Ok(vec![]));
        give_reactions(&mut app);
        assert_eq!(app.flash.as_deref(), Some("Updated. Nothing new came in."));
        // The next update brings the release's own figures for NVDA: that
        // counts as something new, it shows under the title, and NVDA isn't
        // asked about again.
        press(&mut app, KeyCode::Char('r'));
        app.set_earnings_fetch(Ok(vec![]));
        store_release(
            &app,
            "NVDA",
            "2026-11-17",
            ("2026-11-17T21:21:00Z", "NVIDIA Reports Financial Results for Third Quarter Fiscal 2027"),
            "<p>NVIDIA today reported revenue for the third quarter of $115.8 billion, up 103% from a year ago.</p>",
        );
        app.set_headlines(Ok((2, 0, None)));
        app.set_results(Ok(vec![]));
        give_reactions(&mut app);
        assert_eq!(app.flash.as_deref(), Some("Updated. New for NVDA."));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('g'));
        let rows = screen(&app, 130, 34);
        let all = rows.join("\n");
        let at = |needle: &str| {
            rows.iter()
                .position(|r| r.contains(needle))
                .unwrap_or_else(|| panic!("no line with {needle:?} in\n{all}"))
        };
        assert_eq!(
            at("NVIDIA today reported revenue for the third quarter of $115.8 billion") ,
            at("Tue 2:21 PM  NVIDIA Reports Financial Results") + 1,
            "the release's figures sit right under its title"
        );
        assert!(!all.contains("<p>"));
        assert_eq!(
            app.calendar.headlines_wanted(true).iter().map(|a| a.0.as_str()).collect::<Vec<_>>(),
            ["TGT"]
        );
        // A piece that fails is said along the bottom; the update still ends.
        press(&mut app, KeyCode::Char('r'));
        app.set_earnings_fetch(Ok(vec![]));
        app.set_headlines(Ok((1, 1, Some("HTTP 429".into()))));
        app.set_results(Ok(vec![]));
        give_reactions(&mut app);
        assert!(app.calendar.updating.is_none());
        let all = screen(&app, 130, 30).join("\n");
        assert!(all.contains("couldn't fetch headlines for 1 of 1 reports (HTTP 429)"), "{all}");
    }

    #[test]
    fn calendar_and_journal_are_never_each_others_way_back() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        app.journal = crate::tui::journal::JournalState::new(tmp.path().join("journal"));
        for keys in [['g', 'c'], ['g', 'j'], ['g', 'c'], ['g', 'j'], ['g', 'c']] {
            for k in keys {
                press(&mut app, KeyCode::Char(k));
            }
        }
        assert_eq!(app.mode, Mode::Calendar);
        // However many times the two were swapped, esc leaves in two steps
        // at most.
        press(&mut app, KeyCode::Esc);
        if app.mode != Mode::Watchlist {
            press(&mut app, KeyCode::Esc);
        }
        assert_eq!(app.mode, Mode::Watchlist);
    }

    #[test]
    fn calendar_narrow_terminal_drops_the_secondary_columns_and_help_lists_its_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        open_calendar(&mut app);
        give_reactions(&mut app);
        // 100 wide: PREVIOUS is the first column to go.
        let all = screen(&app, 100, 24).join("\n");
        assert!(all.contains("REACTION") && all.contains("SIZE/IMPACT") && all.contains("EXPECTED"), "{all}");
        assert!(!all.contains("PREVIOUS") && !all.contains("EPS 1.78"), "{all}");
        // 82 wide: SIZE/IMPACT goes too; the reaction stays.
        let all = screen(&app, 82, 24).join("\n");
        assert!(all.contains("REACTION") && all.contains("+4.2%") && all.contains("EPS 2.49"), "{all}");
        assert!(!all.contains("SIZE/IMPACT") && !all.contains("$3.76T"), "{all}");
        // 64 wide: time, type, event and expected.
        let rows = screen(&app, 64, 24);
        let all = rows.join("\n");
        assert!(all.contains("EXPECTED") && all.contains("EPS 2.49"), "{all}");
        assert!(!all.contains("REACTION") && !all.contains("+4.2%"), "{all}");
        assert!(all.contains("post-close") && all.contains("earnings") && all.contains("economic"));
        // A long name is cut with an ellipsis, not wrapped.
        assert!(all.contains('…'), "{all}");
        assert!(rows.iter().all(|r| r.chars().count() == 64));
        press(&mut app, KeyCode::Char('?'));
        let help = screen(&app, 100, 30).join("\n");
        assert!(help.contains("calendar"), "{help}");
        for key in [
            "update everything",
            "your watch ↔ everything",
            "slate: add / remove",
            "open the company's chart",
        ] {
            assert!(help.contains(key), "{key}: {help}");
        }
    }

    #[test]
    fn calendar_top_shows_the_day_heading_after_scrolling_and_toggling() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        open_calendar(&mut app);
        // A short screen, so the week doesn't fit and the page scrolls.
        let (w, h) = (130u16, 10u16);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('g'));
        // NVDA is selected, so its box takes the bottom; the list keeps its
        // captions and first heading.
        let rows = screen(&app, w, h);
        assert!(rows[2].contains("TIME") && rows[3].contains("Yesterday · Tue Nov 17"), "{}", rows.join("\n"));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('e'));
        let rows = screen(&app, w, h);
        assert!(!rows.join("\n").contains("Yesterday"), "scrolled to the end");
        // The captions stay put while the rows scroll.
        assert!(rows[2].contains("TIME") && rows[2].contains("EXPECTED"), "{}", rows[2]);
        // Back to the top, then a different row set: the heading is back.
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('e'));
        let rows = screen(&app, w, h);
        assert!(rows[3].contains("Yesterday · Tue Nov 17"), "{}", rows.join("\n"));
    }

    #[test]
    fn calendar_empty_and_unreadable_stores_say_so_plainly() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = App::new(vec!["AAPL".into()], "iex");
        app.calendar = crate::tui::calendar::CalendarState::new(
            Some(tmp.path().join("no-calendar")),
            Some(tmp.path().join("no-earnings")),
            Some(tmp.path().join("no-slate.json")),
        );
        app.calendar.clock = crate::tui::calendar::Clock::mst("2026-11-18T15:00:00Z");
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('c'));
        app.set_earnings_fetch(Err("offline".into()));
        let all = screen(&app, 130, 30).join("\n");
        assert!(all.contains("Your slate is empty. Press a to browse who reports and s to add a company"), "{all}");
        assert!(all.contains("Nothing you're watching."));
        assert!(all.contains("Earnings aren't stored for 5 of the coming weekdays"), "{all}");
        // A day file that isn't one: the error names it.
        let earn = tmp.path().join("no-earnings");
        std::fs::create_dir_all(&earn).unwrap();
        std::fs::write(earn.join("2026-11-18.json"), b"not json").unwrap();
        press(&mut app, KeyCode::Char('e'));
        let all = screen(&app, 130, 30).join("\n");
        assert!(all.contains("couldn't read the calendar"), "{all}");
        assert!(all.contains("2026-11-18.json"), "{all}");
    }

    /// `WICKD_TUI_SNAPSHOT_DIR=/some/dir cargo test -p wickd snapshot_calendar -- --ignored`
    /// writes HTML renders of the calendar screen from the real stores,
    /// slate, watchlist and lists under the wickd data home (`WICKD_HOME`).
    /// Reactions are worked out from the daily bars in the JSON file named
    /// by `WICKD_SNAPSHOT_BARS` (`{"SYM": [bars]}`), when given: the test
    /// itself fetches nothing.
    #[test]
    #[ignore]
    fn snapshot_calendar() {
        let dir = std::env::var("WICKD_TUI_SNAPSHOT_DIR").expect("set WICKD_TUI_SNAPSHOT_DIR");
        std::fs::create_dir_all(&dir).unwrap();
        let shoot = |app: &App, name: &str, w: u16, h: u16| {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| render(f, app)).unwrap();
            std::fs::write(
                format!("{dir}/{name}.html"),
                to_html(term.backend().buffer(), name),
            )
            .unwrap();
        };
        let mut app = App::new(
            crate::tui::store::load_watchlist().unwrap_or_default(),
            "iex",
        );
        if let Ok(f) = crate::lists::lists_path().and_then(|p| crate::lists::load_at(&p)) {
            app.set_lists(f);
        }
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('c'));
        app.flash = None;
        let (w, h) = (160u16, 45u16);
        // Stored data shows at once while a fetch runs (forced here: a
        // fresh store wouldn't start one).
        app.calendar.loading = true;
        shoot(&app, "1-opening-fetch-in-flight", w, h);
        app.set_earnings_fetch(Ok(vec![]));
        app.calendar.reactions_loading = false;
        // The headlines and reported figures are whatever the stores hold.
        app.set_headlines(Ok((0, 0, None)));
        app.set_results(Ok(vec![]));
        shoot(&app, "2-watch-before-reactions", w, h);
        // Real reactions from real bars, when a bars file is given.
        if let Ok(path) = std::env::var("WICKD_SNAPSHOT_BARS") {
            let bars: std::collections::HashMap<String, Vec<wickd_core::equities::Bar>> =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let now = app.calendar.clock.now();
            let got = app
                .calendar
                .reactions_wanted()
                .into_iter()
                .filter_map(|ask| {
                    let r = wickd_core::earnings_reaction::reaction(bars.get(&ask.0)?, ask.1, ask.2, now)?;
                    Some((ask, r))
                })
                .collect();
            app.set_reactions(Ok(got));
        }
        shoot(&app, "3-watch-with-reactions", w, h);
        // Each slate company in turn: its expectation and reaction box.
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('g'));
        let mut n = 0;
        for _ in 0..60 {
            if app.calendar.selected_earn().is_some_and(|e| e.slate) && n < 3 {
                n += 1;
                shoot(&app, &format!("4-{n}-slate-company-box"), w, h);
            }
            let before = app.calendar.selected;
            press(&mut app, KeyCode::Char('j'));
            if app.calendar.selected == before {
                break;
            }
        }
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('e'));
        app.flash = None;
        shoot(&app, "5-everything", w, h);
        press(&mut app, KeyCode::Char('e'));
        app.calendar.go_next_upcoming();
        app.flash = None;
        shoot(&app, "6-narrow-100", 100, 40);
        shoot(&app, "6-narrow-80", 80, 40);
        app.set_earnings_fetch(Err("error sending request".into()));
        shoot(&app, "7-fetch-failed", w, h);
        press(&mut app, KeyCode::Char('?'));
        shoot(&app, "8-help", w, h);
    }

    /// `sample_app` (watchlist SPY, NVDA, XYZ) with a day list, at 8:00 AM
    /// Mountain on Wed Nov 18, 2026: NVDA reports today after the close and
    /// TGT tomorrow before the open.
    fn reports_app() -> App {
        use crate::tui::calendar::{Clock, Soon};
        use wickd_core::earnings_store::Session;
        let mut app = sample_app();
        let mut lists = crate::lists::ListsFile::default();
        for (sym, note) in [
            ("NVDA", "No trade (day one after its report)"),
            ("TGT", "No trade, watch only; reports about 4:30 a.m."),
            ("SPY", "No trade; above 739.76 = buyers in control"),
        ] {
            crate::lists::add(
                &mut lists,
                "wed-11-18",
                &[sym.to_string()],
                &crate::lists::Fields::note(note),
                Utc::now(),
            );
        }
        app.set_lists(lists);
        app.selected = 0;
        app.calendar.clock = Clock::mst("2026-11-18T15:00:00Z");
        app.reports_soon.insert(
            "NVDA".into(),
            Soon {
                date: "2026-11-18".parse().unwrap(),
                session: Session::AfterHours,
            },
        );
        app.reports_soon.insert(
            "TGT".into(),
            Soon {
                date: "2026-11-19".parse().unwrap(),
                session: Session::PreMarket,
            },
        );
        app
    }

    #[test]
    fn rows_say_when_a_stock_reports_today_or_next() {
        let app = reports_app();
        let rows = screen(&app, 140, 30);
        let all = rows.join("\n");
        let line = |needle: &str| {
            rows.iter()
                .find(|r| r.contains(needle))
                .unwrap_or_else(|| panic!("no line with {needle:?} in\n{all}"))
                .clone()
        };
        // Both tables are wide enough: the list and the watchlist.
        assert_eq!(all.matches("EARNINGS").count(), 2, "{all}");
        let nvda = line("No trade (day one");
        assert!(nvda.contains("NVDA") && nvda.contains("today post-close"), "{nvda}");
        let tgt = line("reports about 4:30");
        assert!(tgt.contains("Thu pre-open"), "{tgt}");
        // SPY doesn't report: an empty cell, and its note is where the others' are.
        let spy = line("buyers in control");
        assert!(!spy.contains("today") && !spy.contains("Thu"), "{spy}");
        assert_eq!(
            spy.find("No trade; above"),
            nvda.find("No trade (day one"),
            "notes share a column"
        );
        // The watchlist's own NVDA row carries it too.
        let watch_nvda = rows
            .iter()
            .filter(|r| r.contains("NVDA") && r.contains("today post-close"))
            .count();
        assert_eq!(watch_nvda, 2, "{all}");
    }

    #[test]
    fn the_earnings_column_gives_way_on_a_narrow_terminal_and_hides_when_empty() {
        let mut app = reports_app();
        // 120 wide: room in the list (needs 102), not in the watchlist (127).
        let all = screen(&app, 120, 30).join("\n");
        assert_eq!(all.matches("EARNINGS").count(), 1, "{all}");
        assert!(all.contains("today post-close"));
        // 100 wide: neither, and the notes are untouched.
        let all = screen(&app, 100, 30).join("\n");
        assert!(!all.contains("EARNINGS") && !all.contains("post-close"), "{all}");
        assert!(all.contains("No trade (day one after its report)"), "{all}");
        // Exactly at the list's threshold the column is there; one short, not.
        assert!(screen(&app, 102, 30).join("\n").contains("EARNINGS"));
        assert!(!screen(&app, 101, 30).join("\n").contains("EARNINGS"));
        // Nobody reports soon: no column at any width.
        app.reports_soon.clear();
        let all = screen(&app, 160, 30).join("\n");
        assert!(!all.contains("EARNINGS"), "{all}");
    }

    #[test]
    fn the_chart_header_says_when_the_stock_reports() {
        let mut app = reports_app();
        press(&mut app, KeyCode::Enter); // the list's first row: NVDA
        assert_eq!(app.mode, Mode::Detail);
        let rows = screen(&app, 140, 30);
        assert!(
            rows[1].contains("NVDA") && rows[1].contains("reports today after the close"),
            "{}",
            rows[1]
        );
        // A stock that doesn't report soon says nothing.
        app.reports_soon.clear();
        assert!(!screen(&app, 140, 30).join("\n").contains("reports"));
    }

    #[test]
    fn report_markers_come_from_the_store_and_follow_a_fetch() {
        use wickd_core::earnings_store::{write_day, DayFile, EarningsRow, Session};
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        // calendar_app stores Wed Nov 18 with PANW after the close.
        app.refresh_reports_soon();
        let today = app.calendar.clock.today();
        assert_eq!(app.reports_soon["PANW"].short(today), "today post-close");
        assert_eq!(app.reports_soon["TGT"].short(today), "today pre-open");
        assert_eq!(app.reports_soon.len(), 4);
        // A fetch lands with tomorrow's file: the markers pick it up.
        write_day(
            app.calendar.earn_dir.as_ref().unwrap(),
            &DayFile {
                date: "2026-11-19".parse().unwrap(),
                fetched_at: Utc::now(),
                source: "nasdaq".into(),
                rows: vec![EarningsRow {
                    date: "2026-11-19".parse().unwrap(),
                    symbol: "WMT".into(),
                    name: "Walmart Inc.".into(),
                    session: Session::PreMarket,
                    eps_forecast: None,
                    estimates: None,
                    market_cap: None,
                    last_year_eps: None,
                    fiscal_quarter: String::new(),
                }],
            },
        )
        .unwrap();
        app.set_earnings_fetch(Ok(vec![]));
        assert_eq!(app.reports_soon["WMT"].short(today), "Thu pre-open");
        // No data folder: no markers, no panic.
        app.calendar.earn_dir = None;
        app.refresh_reports_soon();
        assert!(app.reports_soon.is_empty());
    }

    /// `WICKD_TUI_SNAPSHOT_DIR=/some/dir cargo test -p wickd snapshot_reports_soon -- --ignored`
    /// writes HTML renders of the EARNINGS column and the chart header from
    /// the real lists, watchlist and earnings store under the wickd data
    /// home (`WICKD_HOME`). Company names come from the earnings store; no
    /// quotes are fetched, so prices show as `…`.
    #[test]
    #[ignore]
    fn snapshot_reports_soon() {
        let dir = std::env::var("WICKD_TUI_SNAPSHOT_DIR").expect("set WICKD_TUI_SNAPSHOT_DIR");
        std::fs::create_dir_all(&dir).unwrap();
        let shoot = |app: &App, name: &str, w: u16, h: u16| {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| render(f, app)).unwrap();
            std::fs::write(
                format!("{dir}/{name}.html"),
                to_html(term.backend().buffer(), name),
            )
            .unwrap();
        };
        let mut app = App::new(
            crate::tui::store::load_watchlist().unwrap_or_default(),
            "iex",
        );
        if let Ok(f) = crate::lists::lists_path().and_then(|p| crate::lists::load_at(&p)) {
            app.set_lists(f);
        }
        app.refresh_reports_soon();
        // Names for the rows that report, from the store itself.
        let today = app.calendar.clock.today();
        let names: Vec<Asset> = app
            .calendar
            .earn_dir
            .as_ref()
            .and_then(|d| {
                wickd_core::earnings_store::read_days(
                    d,
                    today,
                    crate::tui::calendar::next_weekday(today),
                )
                .ok()
            })
            .unwrap_or_default()
            .into_iter()
            .flat_map(|d| d.rows)
            .map(|r| Asset {
                symbol: r.symbol,
                name: r.name,
                exchange: "NASDAQ".into(),
                tradable: true,
            })
            .collect();
        app.set_assets(names);
        // Open the newest day list.
        app.expanded = app.lists.len().saturating_sub(1);
        app.selected = 0;
        app.flash = None;
        shoot(&app, "1-list-and-watchlist-wide", 160, 45);
        shoot(&app, "2-list-only-at-120", 120, 45);
        shoot(&app, "3-narrow-no-column", 100, 45);
        // A chart of the first stock on the list that reports soon.
        let sym = app
            .lists
            .get(app.expanded)
            .and_then(|(_, e)| e.iter().find(|e| app.reports_soon.contains_key(&e.symbol)))
            .map(|e| e.symbol.clone());
        if let Some(sym) = sym {
            if let Some(i) = (0..app.total_rows()).find(|i| app.row_symbol(*i) == Some(sym.as_str())) {
                app.selected = i;
                press(&mut app, KeyCode::Enter);
                shoot(&app, "4-chart-header", 160, 20);
            }
        }
    }

    /// `WICKD_TUI_SNAPSHOT_DIR=/some/dir cargo test -p wickd snapshot_journal -- --ignored`
    /// writes HTML renders of the journal screen from the real journal
    /// folder (`WICKD_JOURNAL_DIR`, else `~/trading-journal`).
    #[test]
    #[ignore]
    fn snapshot_journal() {
        let dir = std::env::var("WICKD_TUI_SNAPSHOT_DIR").expect("set WICKD_TUI_SNAPSHOT_DIR");
        std::fs::create_dir_all(&dir).unwrap();
        let shoot = |app: &App, name: &str, w: u16, h: u16| {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| render(f, app)).unwrap();
            std::fs::write(
                format!("{dir}/{name}.html"),
                to_html(term.backend().buffer(), name),
            )
            .unwrap();
        };
        // Scroll so the first line containing `needle` is `above` rows down.
        let find = |app: &mut App, w: u16, needle: &str, above: usize| {
            let at = app.journal.with_rendered(w - 6, |r| {
                r.lines.iter().position(|l| {
                    l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                        .contains(needle)
                })
            });
            app.journal
                .scroll_to(at.expect(needle).saturating_sub(above));
        };
        let mut app = journal_app(&crate::tui::journal::default_dir());
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('j'));
        app.flash = None;
        let (w, h) = (160u16, 45u16);
        shoot(&app, "1-newest-entry-top", w, h);
        find(&mut app, w, "The plan", 1);
        shoot(&app, "2-the-plan-tables", w, h);
        // An older entry with a block quote and a numbered list, if any.
        while app.journal.step_entry(-1) {
            let has = |n: &str| app.journal.text.contains(n);
            if has("\n> ") && has("\n1. ") {
                app.flash = None;
                let quote = app.journal.text.find("\n> ").unwrap_or(0);
                let needle: String = app.journal.text[quote + 3..]
                    .chars()
                    .filter(|c| *c != '*')
                    .take(12)
                    .collect();
                find(&mut app, w, &needle, 3);
                shoot(&app, "3-block-quote", w, h);
                find(&mut app, w, "1. ", 6);
                shoot(&app, "3b-numbered-list", w, h);
                break;
            }
        }
        // A narrow terminal: tables wrap inside their columns.
        let mut app = journal_app(&crate::tui::journal::default_dir());
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('j'));
        app.flash = None;
        let (w, h) = (100u16, 40u16);
        find(&mut app, w, "The plan", 1);
        shoot(&app, "4-narrow-table-wraps", w, h);
        press(&mut app, KeyCode::Char('?'));
        shoot(&app, "5-help", 100, 40);
    }

    /// `WICKD_TUI_SNAPSHOT_DIR=/some/dir cargo test -p wickd snapshot_price_line -- --ignored`
    /// writes HTML renders of the chart's price line for visual review.
    #[test]
    #[ignore]
    fn snapshot_price_line() {
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
        press(&mut app, KeyCode::Char('5')); // 1Y daily
        app.set_history("SPY", HistoryRange::Year1, Ok(daily_walk(250, 560.0, 5)));
        shoot(&app, "0-before-no-line");
        for _ in 0..12 {
            press(&mut app, KeyCode::Char('h'));
        }
        press(&mut app, KeyCode::Char('k'));
        shoot(&app, "1-k-starts-the-line-at-the-cursor-close");
        press(&mut app, KeyCode::Char('K'));
        press(&mut app, KeyCode::Char('K'));
        press(&mut app, KeyCode::Char('k'));
        shoot(&app, "2-after-K-K-k");
        press(&mut app, KeyCode::Char('p'));
        for c in "612.4".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        shoot(&app, "3-p-typing-a-price");
        press(&mut app, KeyCode::Enter);
        shoot(&app, "4-line-at-the-typed-price");
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('l'));
        shoot(&app, "5-line-chart");
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

        // 50/200-day averages with ~210 sessions of warm-up before the window.
        let all = daily_walk(462, 470.0, 3);
        let (warm, vis) = all.split_at(210);
        app.set_history("SPY", HistoryRange::Year1, Ok(vis.to_vec()));
        app.set_warmup("SPY", HistoryRange::Year1, warm.to_vec());
        let last = vis.last().unwrap().close;
        let spy = app.snapshots.get_mut("SPY").unwrap();
        spy.latest_trade.as_mut().unwrap().price = last;
        let saved_prev = spy.prev_daily_bar.as_ref().unwrap().close;
        spy.prev_daily_bar.as_mut().unwrap().close = vis[vis.len() - 2].close;
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('m'));
        shoot(&app, "4b-1y-moving-averages");
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('m'));
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('i'));
        shoot(&app, "4c-1y-ichimoku");
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('i'));
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('s'));
        shoot(&app, "4d-1y-support-resistance");
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('i'));
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('i'));
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('m'));
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('m'));
        app.snapshots
            .get_mut("SPY")
            .unwrap()
            .prev_daily_bar
            .as_mut()
            .unwrap()
            .close = saved_prev;

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
    fn selected_row_is_a_background_not_inverted_and_budget_shows() {
        let mut app = sample_app();
        app.api_budget = Some(std::sync::Arc::new(std::sync::Mutex::new(Some(
            wickd_core::equities::alpaca::RateLimit {
                limit: 200,
                remaining: 31,
            },
        ))));
        let mut term = Terminal::new(TestBackend::new(130, 12)).unwrap();
        term.draw(|f| render(f, &app)).unwrap();
        let buf = term.backend().buffer().clone();
        // Row 3 (header line + border + column header) is the selected SPY row.
        let row: Vec<_> = (0..130u16).map(|x| buf[(x, 3)].clone()).collect();
        assert!(row.iter().any(|c| c.symbol() == "S"), "selected row is SPY");
        assert!(row.iter().all(|c| !c.modifier.contains(Modifier::REVERSED)));
        assert!(row.iter().filter(|c| c.bg == SELECTED_BG).count() > 50);
        // Sparkline cells keep their colour on the highlighted row.
        assert!(row
            .iter()
            .any(|c| "▁▂▃▄▅▆▇█".contains(c.symbol()) && c.fg == DOWN));
        // The API budget shows in the header.
        let header: String = (0..130u16)
            .map(|x| buf[(x, 0)].symbol().to_string())
            .collect();
        assert!(header.contains("api 31/200"), "{header}");
        // Low budget (<20%) is highlighted.
        let low =
            (0..130u16).find(|&x| buf[(x, 0)].symbol() == "a" && buf[(x + 1, 0)].symbol() == "p");
        assert_eq!(buf[(low.unwrap(), 0)].fg, Color::Yellow);
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
    fn fiscal_year_end_reads_as_a_date() {
        assert_eq!(month_day("0131"), "Jan 31");
        assert_eq!(month_day("1231"), "Dec 31");
        assert_eq!(month_day("13x"), "13x");
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

    // ── the agent panel (AGT-2549) ──────────────────────────────────────────

    /// `a`, a typed key, then `esc`: the panel opens over the screen, keys go
    /// to its input, and closing leaves the screen exactly as it was.
    fn agent_round_trip(app: &mut App, sees: &str) {
        let mode = app.mode;
        let before = screen(app, 140, 30);
        assert!(press(app, KeyCode::Char('a')).is_empty());
        assert!(app.agent.open, "{mode:?}");
        let open = screen(app, 140, 30).join("\n");
        assert!(open.contains(&format!("agent · sees {sees}")), "{mode:?}: {open}");
        assert!(open.contains("ask about this screen…"), "{open}");
        // `q`, `j` and `?` type; they don't go back, move or open help.
        for c in ['q', 'j', '?'] {
            assert!(press(app, KeyCode::Char(c)).is_empty());
        }
        assert_eq!(app.agent.input.text, "qj?");
        assert!(!app.show_help);
        assert!(screen(app, 140, 30).join("\n").contains("› qj?"));
        for _ in 0..3 {
            press(app, KeyCode::Backspace);
        }
        press(app, KeyCode::Esc);
        assert!(!app.agent.open);
        assert_eq!(app.mode, mode);
        assert_eq!(screen(app, 140, 30), before, "{mode:?}");
    }

    #[test]
    fn agent_panel_opens_over_every_screen_and_esc_leaves_it() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        std::fs::create_dir_all(tmp.path().join("journal")).unwrap();
        std::fs::write(tmp.path().join("journal/2026-11-18.md"), "# Wednesday\n\nplan").unwrap();
        app.journal = crate::tui::journal::JournalState::new(tmp.path().join("journal"));
        assert_eq!(app.mode, Mode::Watchlist);
        agent_round_trip(&mut app, "the watchlist");
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('i'));
        agent_round_trip(&mut app, "in play today");
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('j'));
        agent_round_trip(&mut app, "journal 2026-11-18");
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('w'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.mode, Mode::Detail);
        let sym = app.detail.as_ref().unwrap().symbol.clone();
        agent_round_trip(&mut app, &format!("{sym} 5m chart"));
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('c'));
        open_calendar(&mut app);
        agent_round_trip(&mut app, "the calendar (your watch)");
        // The watch ↔ everything toggle is on `e` now.
        press(&mut app, KeyCode::Char('e'));
        assert!(app.calendar.show_all);
        app.flash = None;
        agent_round_trip(&mut app, "the calendar (everything)");
    }

    #[test]
    fn agent_panel_streams_an_answer_over_the_chart_and_keeps_it() {
        use crate::tui::app::Action;
        let mut app = sample_app();
        press(&mut app, KeyCode::Enter); // SPY
        app.set_history("SPY", HistoryRange::Day1, Ok(walk(78, 684.0)));
        draw(&app, 150, 30);
        press(&mut app, KeyCode::Char('a'));
        for c in "is SPY above the cloud?".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let acts = press(&mut app, KeyCode::Enter);
        let [Action::AgentAsk { question, context: Some(ctx) }] = acts.as_slice() else {
            panic!("{acts:?}");
        };
        assert_eq!(question, "is SPY above the cloud?");
        let ctx: serde_json::Value = serde_json::from_str(ctx).unwrap();
        assert_eq!(ctx["screen"], "chart");
        assert_eq!(ctx["symbol"], "SPY");
        assert_eq!(ctx["timeframe"], "5m");
        assert_eq!(ctx["bars"].as_array().unwrap().len(), 60);
        assert!(ctx["cursor"]["close"].is_number() && ctx["price_lines"].is_object());
        assert!(ctx["bars"][59]["ichimoku"].is_object());

        app.agent.apply(&serde_json::json!({"type":"status","status":"requesting"}));
        let s = draw(&app, 150, 30);
        assert!(s.contains("› is SPY above the cloud?") && s.contains("thinking…"), "{s}");
        app.agent.apply(&serde_json::json!({"type":"text","delta":"Yes, **just**: price sits"}));
        let s = draw(&app, 150, 30);
        assert!(s.contains("Yes, just: price sits") && !s.contains("thinking…"), "{s}");
        // The chart is still there, left of the panel.
        assert!(s.contains("SPY"), "{s}");
        app.agent.apply(&serde_json::json!({"type":"text","delta":" above both spans."}));
        app.agent.apply(&serde_json::json!({
            "type": "done",
            "answer": "Yes, **just**: price sits above both spans.",
            "session_id": "s",
            "turns": 1,
        }));
        if std::env::var("WICKD_TUI_DUMP").is_ok() {
            println!("{}\n", draw(&app, 150, 30));
        }
        press(&mut app, KeyCode::Esc);
        assert!(!draw(&app, 150, 30).contains("above both spans"));
        press(&mut app, KeyCode::Char('a'));
        let s = draw(&app, 150, 30);
        assert!(s.contains("price sits above both spans."), "reopens with the conversation: {s}");
        // Narrow terminals get the full width.
        let s = draw(&app, 90, 20);
        assert!(s.lines().nth(1).unwrap().starts_with("┌ agent"), "{s}");
    }

    #[test]
    fn agent_context_carries_list_and_calendar_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        let mut lists = crate::lists::ListsFile::default();
        let fields = crate::lists::Fields {
            note: Some("buy above 185".into()),
            plan: Some("+3% by Fri".into()),
        };
        crate::lists::add(&mut lists, "thu-11-19", &["NVDA".to_string()], &fields, Utc::now());
        app.set_lists(lists);
        let ctx = crate::tui::context::screen(&app);
        assert_eq!(ctx["screen"], "watchlist");
        assert_eq!(ctx["list"]["name"], "thu-11-19");
        let row = &ctx["list"]["rows"][0];
        assert_eq!(row["symbol"], "NVDA");
        assert_eq!(row["note"], "buy above 185");
        assert_eq!(row["plan"], "+3% by Fri");
        assert!(ctx["watchlist"].is_array());
        assert_eq!(crate::tui::context::screen_label(&app), "the thu-11-19 list and watchlist");

        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('c'));
        open_calendar(&mut app);
        let ctx = crate::tui::context::screen(&app);
        assert_eq!(ctx["screen"], "calendar");
        assert_eq!(ctx["showing"], "watch");
        let rows = ctx["rows"].as_array().unwrap();
        let earn = rows.iter().find(|r| r["type"] == "earnings").expect("an earnings row");
        for k in ["symbol", "session", "day"] {
            assert!(earn[k].is_string(), "{k}: {earn}");
        }
        assert!(rows.iter().any(|r| r["type"] == "economic" && r["event"].is_string()), "{rows:?}");
        assert_eq!(ctx["selected"].as_str(), app.calendar.selected_symbol());

        // In play and search say what they are.
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('i'));
        assert_eq!(crate::tui::context::screen(&app)["screen"], "in_play");
    }

    #[test]
    fn help_and_footer_list_the_agent_key_and_the_calendar_toggle_on_e() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = calendar_app(tmp.path());
        let foot = screen(&app, 220, 20).join("\n");
        assert!(foot.contains("a ask the agent"), "{foot}");
        press(&mut app, KeyCode::Char('?'));
        let help = screen(&app, 120, 50).join("\n");
        assert!(help.contains("ask the agent"), "{help}");
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('c'));
        open_calendar(&mut app);
        let foot = screen(&app, 220, 30);
        let foot = foot.last().unwrap();
        assert!(foot.contains("e your watch ↔ everything"), "{foot}");
        assert!(foot.contains("a ask the agent"), "{foot}");
        press(&mut app, KeyCode::Char('?'));
        let help = screen(&app, 120, 34);
        let line = |k: &str, label: &str| {
            let row = format!("  {k:<14}{label}");
            help.iter().any(|l| l.contains(&row))
        };
        assert!(line("e", "your watch ↔ everything"), "{}", help.join("\n"));
        assert!(line("a", "ask the agent"), "{}", help.join("\n"));
        // Wide enough for the whole key bar, at the real width.
        press(&mut app, KeyCode::Esc);
        let foot = screen(&app, 160, 30);
        assert!(foot.last().unwrap().contains("a ask the agent"), "{}", foot.last().unwrap());
    }

    /// The agent panel's screenshots (AGT-2549):
    /// `WICKD_TUI_SNAPSHOT_DIR=/some/dir cargo test -p wickd snapshot_agent_panel -- --ignored`
    /// writes HTML renders of the panel over the EUR_USD 1h chart (live
    /// OANDA bars), over the calendar (the real stores under the wickd data
    /// home, read only), its first-open state, and the calendar's keys.
    /// With `WICKD_SNAPSHOT_ASK=1` the chart's first answer is a real `wickd
    /// agent ask` through the panel's own code path (the debug wickd binary,
    /// with `WICKD_HOME` pointed at a scratch folder so the real session is
    /// untouched); otherwise the conversation is a written stand-in.
    #[tokio::test]
    #[ignore]
    async fn snapshot_agent_panel() {
        use serde_json::json;
        let dir = std::env::var("WICKD_TUI_SNAPSHOT_DIR").expect("set WICKD_TUI_SNAPSHOT_DIR");
        std::fs::create_dir_all(&dir).unwrap();
        let shoot = |app: &App, name: &str, w: u16, h: u16| {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| render(f, app)).unwrap();
            term.draw(|f| render(f, app)).unwrap();
            std::fs::write(
                format!("{dir}/{name}.html"),
                to_html(term.backend().buffer(), name),
            )
            .unwrap();
        };
        let (w, h) = (160u16, 45u16);
        let events = |app: &mut App, evs: &[serde_json::Value]| {
            for e in evs {
                app.agent.apply(e);
            }
        };
        let ask = |app: &mut App, q: &str| -> Option<String> {
            for c in q.chars() {
                press(app, KeyCode::Char(c));
            }
            match press(app, KeyCode::Enter).pop() {
                Some(crate::tui::app::Action::AgentAsk { context, .. }) => context,
                other => panic!("{other:?}"),
            }
        };

        // 1. EUR_USD on the 1h chart (the 1M range), live bars.
        let client = super::super::oanda_client().expect("OANDA practice credentials");
        let now = Utc::now();
        let sym = "EUR_USD";
        let mut app = App::new(vec![sym.to_string()], "IEX");
        app.set_fx_assets(fx::instruments(&client).await.unwrap());
        app.fx_quotes = fx::quotes(&client, &[sym.to_string()], now).await.unwrap();
        app.fx_live = true;
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('3'));
        let (warm, vis) = fx::history(&client, sym, HistoryRange::Month1, now).await.unwrap();
        app.set_warmup(sym, HistoryRange::Month1, warm);
        app.set_history(sym, HistoryRange::Month1, Ok(vis));
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('i'));
        app.flash = None;
        shoot(&app, "0-eur_usd-1h-chart-before", w, h);
        press(&mut app, KeyCode::Char('a'));
        shoot(&app, "3-first-open-empty-over-chart", w, h);
        let q1 = "Where is price against the cloud on this chart, and what would change your read?";
        let ctx = ask(&mut app, q1);
        if std::env::var("WICKD_SNAPSHOT_ASK").is_ok() {
            let scratch = tempfile::tempdir().unwrap();
            std::env::set_var("WICKD_HOME", scratch.path());
            let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/wickd");
            let got = std::sync::Mutex::new(Vec::new());
            let r = crate::tui::agent::run_ask(&exe, q1, ctx, |ev| got.lock().unwrap().push(ev))
                .await;
            let got = got.into_inner().unwrap();
            let lines: String = got.iter().map(|e| format!("{e}\n")).collect();
            std::fs::write(format!("{dir}/ask-events.jsonl"), lines).unwrap();
            // Replay up to the middle of the answer for a mid-stream shot.
            let texts = got.iter().filter(|e| e["type"] == "text").count();
            let mut seen = 0;
            for e in &got {
                if e["type"] == "text" {
                    seen += 1;
                    if seen == texts / 2 {
                        shoot(&app, "1a-chart-answer-streaming", w, h);
                    }
                }
                app.agent.apply(e);
            }
            app.agent.finished(r);
        } else {
            events(&mut app, &[
                json!({"type":"status","status":"requesting"}),
                json!({"type":"tool","name":"Bash","input":{"command":"wickd playbook search ichimoku"}}),
            ]);
            shoot(&app, "1a-chart-thinking-with-lookup", w, h);
            events(&mut app, &[
                json!({"type":"text","delta":"Price is **above the cloud** on the 1h, but only just: the last few candles closed a few pips over span A, with the Kijun flat beneath them.\n\n"}),
                json!({"type":"text","delta":"- Tenkan has crossed back over the Kijun, a mild bullish turn.\n- The cloud ahead is thin and twists within a day, so support there is weak.\n"}),
            ]);
            shoot(&app, "1b-chart-answer-streaming", w, h);
            events(&mut app, &[
                json!({"type":"text","delta":"\nWhat would change my read: a close back inside the cloud, or Chikou dropping under price from 26 bars ago."}),
                json!({"type":"done","answer":"","session_id":"s","turns":1}),
            ]);
        }
        shoot(&app, "1-panel-over-eur_usd-1h-chart", w, h);

        // 2. The calendar, real stores (read only).
        let mut app = App::new(crate::tui::store::load_watchlist().unwrap_or_default(), "iex");
        if let Ok(f) = crate::lists::lists_path().and_then(|p| crate::lists::load_at(&p)) {
            app.set_lists(f);
        }
        press(&mut app, KeyCode::Char('g'));
        press(&mut app, KeyCode::Char('c'));
        app.set_earnings_fetch(Ok(vec![]));
        app.set_headlines(Ok((0, 0, None)));
        app.set_results(Ok(vec![]));
        app.calendar.reactions_loading = false;
        app.flash = None;
        // 4. The calendar's keys: the toggle on `e`, the agent on `a`.
        shoot(&app, "4a-calendar-key-bar", w, h);
        press(&mut app, KeyCode::Char('?'));
        shoot(&app, "4b-calendar-help-e-toggle-a-agent", w, h);
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('a'));
        let _ = ask(&mut app, "Which of these reports matters most for my watchlist this week?");
        events(&mut app, &[
            json!({"type":"tool","name":"Bash","input":{"command":"wickd earnings slate ls"}}),
            json!({"type":"text","delta":"Of what's on screen, the slate names carry the most for you: their expectations are written down, so the reaction is something you can check against."}),
            json!({"type":"done","answer":"","session_id":"s","turns":2}),
        ]);
        shoot(&app, "2-panel-over-calendar", w, h);
    }
}
