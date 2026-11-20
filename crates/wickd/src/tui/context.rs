//! Chart context: what a chart shows, as numbers (wickd-learn D-21).
//!
//! [`build`] turns a chart's history (warm-up + visible bars) into the bars
//! and every indicator value the chart draws at each bar — Ichimoku today,
//! plus the 50/200-day averages on daily timeframes. It runs on the same
//! session-filtered bars ([`chart::indicator_bars`]) and the same indicator
//! math ([`chart::ichimoku_series`], [`chart::moving_averages`]) the TUI chart
//! renders from, so `wickd chart context` and the chart never disagree.
//!
//! [`open_chart`] returns the same structure for the chart the TUI has open,
//! plus its cursor bar and price lines, and [`screen`] wraps it (or a list's
//! or the calendar's rows) for the agent panel to attach to every message.
//!
//! Values only: nothing here judges a setup or checks a rule (D-20).

use chrono::{DateTime, Utc};
use rust_decimal::prelude::ToPrimitive;
use serde::Serialize;
use serde_json::{json, Value};

use wickd_core::equities::{Bar, HistoryRange};

use super::app::{App, Mode};
use super::chart::{self, LevelKind, ICHI_KIJUN, ICHI_SENKOU_B, ICHI_SHIFT, ICHI_TENKAN};
use super::fx;

/// A chart timeframe (bar size) and the chart range that shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeframe {
    /// Short label: `5m`, `15m`, `1h`, `1d`, `1w`, `1mo`.
    pub label: &'static str,
    pub range: HistoryRange,
}

/// The timeframes the chart offers, one per bar size. Daily bars use the 1Y
/// range (the longer of the two daily ranges, so the 200-day average is
/// defined across the window).
pub const TIMEFRAMES: [Timeframe; 6] = [
    Timeframe {
        label: "5m",
        range: HistoryRange::Day1,
    },
    Timeframe {
        label: "15m",
        range: HistoryRange::Day5,
    },
    Timeframe {
        label: "1h",
        range: HistoryRange::Month1,
    },
    Timeframe {
        label: "1d",
        range: HistoryRange::Year1,
    },
    Timeframe {
        label: "1w",
        range: HistoryRange::Year5,
    },
    Timeframe {
        label: "1mo",
        range: HistoryRange::Max,
    },
];

impl Timeframe {
    /// Parse `5m`, `15m`, `1h`, `1d`, `1w`, `1mo` (case-insensitive; long
    /// forms like `1hour`, `1day`, `1week`, `1month` work too).
    pub fn parse(s: &str) -> Option<Timeframe> {
        let label = match s.trim().to_ascii_lowercase().as_str() {
            "5m" | "5min" => "5m",
            "15m" | "15min" => "15m",
            "1h" | "60m" | "1hour" => "1h",
            "1d" | "d" | "1day" => "1d",
            "1w" | "w" | "1week" => "1w",
            "1mo" | "1month" => "1mo",
            _ => return None,
        };
        TIMEFRAMES.iter().copied().find(|t| t.label == label)
    }

    /// The timeframe a chart range draws (6M and 1Y are both `1d`).
    pub fn of_range(range: HistoryRange) -> Timeframe {
        TIMEFRAMES
            .iter()
            .copied()
            .find(|t| t.range.timeframe() == range.timeframe())
            .map(|t| Timeframe { range, ..t })
            .unwrap_or(Timeframe { label: "?", range })
    }

    pub fn labels() -> String {
        TIMEFRAMES
            .iter()
            .map(|t| t.label)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Ichimoku at one bar. "Plotted" is what the chart draws over that bar;
/// "computed" is what the bar itself produces (drawn [`ICHI_SHIFT`] bars
/// later).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct IchimokuAt {
    /// Tenkan-sen: (9-bar high + low) / 2, ending at this bar.
    pub tenkan: Option<f64>,
    /// Kijun-sen: (26-bar high + low) / 2, ending at this bar.
    pub kijun: Option<f64>,
    /// Senkou span A drawn at this bar (computed 26 bars earlier).
    pub span_a_plotted: Option<f64>,
    /// Senkou span B drawn at this bar (computed 26 bars earlier).
    pub span_b_plotted: Option<f64>,
    /// Senkou span A computed at this bar: (tenkan + kijun) / 2.
    pub span_a_computed: Option<f64>,
    /// Senkou span B computed at this bar: (52-bar high + low) / 2.
    pub span_b_computed: Option<f64>,
    /// Chikou drawn at this bar: the close 26 bars later (null for the
    /// newest 26 bars).
    pub chikou: Option<f64>,
}

/// One bar and the indicator values the chart draws at it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BarContext {
    pub time: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub ichimoku: IchimokuAt,
    /// 50-day simple average of closes (daily timeframes only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ma50: Option<f64>,
    /// 200-day simple average of closes (daily timeframes only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ma200: Option<f64>,
}

/// The cloud the chart projects to the right of the newest bar.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CloudAhead {
    /// Bars after the newest bar (1..=26).
    pub bars_ahead: usize,
    pub span_a: Option<f64>,
    pub span_b: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct IchimokuSettings {
    pub tenkan: usize,
    pub kijun: usize,
    pub senkou_b: usize,
    pub shift: usize,
}

/// Where the bars come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A stock (Alpaca).
    Stock,
    /// A forex pair like `EUR_USD` (OANDA practice).
    Pair,
}

/// Which hours the bars cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Session {
    /// Intraday stock bars, 09:30-16:00 ET only.
    Regular,
    /// Intraday stock bars including pre/post-market.
    Extended,
    /// Forex pairs: every bar, Sunday evening to Friday evening.
    #[serde(rename = "24/5")]
    TwentyFourFive,
    /// Daily and longer bars.
    All,
}

/// What a chart shows, as numbers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChartContext {
    pub symbol: String,
    /// `stock` (Alpaca) or `pair` (OANDA).
    pub kind: Kind,
    /// Bar size: `5m`, `15m`, `1h`, `1d`, `1w`, `1mo`.
    pub timeframe: &'static str,
    /// The chart range that shows this timeframe (`1D`, `5D`, `1M`, …).
    pub range: &'static str,
    /// `regular` / `extended` hours (intraday stocks), `24/5` (pairs) or
    /// `all` (daily and longer bars).
    pub session: Session,
    /// Indicators with values in `bars` (the ones the chart can draw here).
    pub indicators: Vec<&'static str>,
    pub ichimoku_settings: IchimokuSettings,
    /// Bars the chart has for this range (before `bars` is cut to N).
    pub bars_available: usize,
    /// The newest N bars, oldest first.
    pub bars: Vec<BarContext>,
    /// The 26 projected cloud slots after the newest bar.
    pub cloud_ahead: Vec<CloudAhead>,
}

/// Per-bar values for every visible bar, plus the projected cloud.
fn all_bars(
    range: HistoryRange,
    warm: &[Bar],
    vis: &[Bar],
) -> (Vec<BarContext>, Vec<CloudAhead>, bool) {
    let all: Vec<Bar> = warm.iter().chain(vis).cloned().collect();
    let s = chart::ichimoku_series(&all);
    let off = warm.len();
    let at = |v: &[Option<f64>], i: usize| v.get(i).copied().flatten();
    // The same overlay code the chart draws the averages with, one candle
    // per bar.
    let ma = chart::ma_range(range);
    let mas = if ma {
        chart::moving_averages(warm, vis, &chart::build(vis, 1))
    } else {
        Vec::new()
    };
    let ma_at = |label: &str, i: usize| {
        mas.iter()
            .find(|o| o.label == label)
            .and_then(|o| o.values.get(i).copied().flatten())
    };
    let bars = vis
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let k = off + i;
            BarContext {
                time: b.time,
                open: b.open,
                high: b.high,
                low: b.low,
                close: b.close,
                volume: b.volume,
                ichimoku: IchimokuAt {
                    tenkan: at(&s.tenkan, k),
                    kijun: at(&s.kijun, k),
                    span_a_plotted: at(&s.span_a, k),
                    span_b_plotted: at(&s.span_b, k),
                    span_a_computed: at(&s.span_a, k + ICHI_SHIFT),
                    span_b_computed: at(&s.span_b, k + ICHI_SHIFT),
                    chikou: at(&s.chikou, k),
                },
                ma50: ma_at("MA50", i),
                ma200: ma_at("MA200", i),
            }
        })
        .collect();
    let ahead = if vis.is_empty() {
        Vec::new()
    } else {
        let last = all.len() - 1;
        (1..=ICHI_SHIFT)
            .map(|j| CloudAhead {
                bars_ahead: j,
                span_a: at(&s.span_a, last + j),
                span_b: at(&s.span_b, last + j),
            })
            .collect()
    };
    (bars, ahead, ma)
}

/// The chart context for `symbol` on `range` from its history (`warmup`
/// bars before the window, `bars` in it — as the chart loads them), with the
/// newest `n` bars. `extended` includes pre/post-market bars on intraday
/// stock ranges (pairs always show every bar).
pub fn build(
    symbol: &str,
    range: HistoryRange,
    warmup: &[Bar],
    bars: &[Bar],
    extended: bool,
    n: usize,
) -> ChartContext {
    let mut ctx = build_all(symbol, range, warmup, bars, extended);
    ctx.bars.drain(..ctx.bars.len().saturating_sub(n));
    ctx
}

/// [`build`] with every visible bar.
fn build_all(
    symbol: &str,
    range: HistoryRange,
    warmup: &[Bar],
    bars: &[Bar],
    extended: bool,
) -> ChartContext {
    let is_fx = fx::is_fx(symbol);
    let (warm, vis, ext) = chart::indicator_bars(warmup, bars, range, extended || is_fx);
    let (all, cloud_ahead, ma) = all_bars(range, &warm, &vis);
    let mut indicators = vec!["ichimoku"];
    if ma {
        indicators.extend(["ma50", "ma200"]);
    }
    ChartContext {
        symbol: symbol.to_string(),
        kind: if is_fx { Kind::Pair } else { Kind::Stock },
        timeframe: Timeframe::of_range(range).label,
        range: range.label(),
        session: if is_fx {
            Session::TwentyFourFive
        } else if !range.intraday() {
            Session::All
        } else if ext {
            Session::Extended
        } else {
            Session::Regular
        },
        indicators,
        ichimoku_settings: IchimokuSettings {
            tenkan: ICHI_TENKAN,
            kijun: ICHI_KIJUN,
            senkou_b: ICHI_SENKOU_B,
            shift: ICHI_SHIFT,
        },
        bars_available: all.len(),
        bars: all,
        cloud_ahead,
    }
}

// ── the chart the TUI has open ──────────────────────────────────────────────

/// The candle under the chart's cursor (merged candles cover several bars).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CursorContext {
    /// Candle index on the chart (0 = oldest).
    pub index: usize,
    /// True when the cursor rests on the newest candle (it follows live
    /// updates).
    pub latest: bool,
    /// Start time of the candle.
    pub time: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub change_pct: Option<f64>,
    /// Bars merged into this candle.
    pub bars_merged: usize,
    /// Time of the candle's last bar, where `ichimoku` is read.
    pub bar_time: DateTime<Utc>,
    pub ichimoku: IchimokuAt,
}

/// Which reference line a chart draws on 1D.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Reference {
    /// Stocks: the previous session's close.
    #[serde(rename = "prev close")]
    PrevClose,
    /// Pairs: the trading day's open (17:00 New York).
    #[serde(rename = "day open")]
    DayOpen,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReferenceLine {
    /// `prev close` (stocks) or `day open` (pairs), on 1D only.
    pub label: Reference,
    pub price: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LevelLine {
    /// `support` or `resistance`.
    pub kind: LevelKind,
    pub price: f64,
    /// Swing points that formed it.
    pub touches: usize,
}

/// The horizontal lines on the chart.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PriceLines {
    /// The price line set with `j`/`k` or `p`.
    pub price_line: Option<f64>,
    pub reference: Option<ReferenceLine>,
    pub last_price: Option<f64>,
    /// Support/resistance levels, when the chart shows them.
    pub levels: Vec<LevelLine>,
}

/// What's toggled on the open chart.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewState {
    pub ichimoku_shown: bool,
    pub ma_shown: bool,
    pub levels_shown: bool,
    pub fit: bool,
    /// Bars per candle as drawn.
    pub bars_per_candle: usize,
}

/// [`ChartContext`] for the chart the TUI has open, with its cursor and
/// price lines.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenChartContext {
    #[serde(flatten)]
    pub chart: ChartContext,
    pub cursor: Option<CursorContext>,
    pub price_lines: PriceLines,
    pub view: ViewState,
}

/// The context of the chart open in the TUI (`None` when no chart is open or
/// its history hasn't loaded), with the newest `n` bars.
pub fn open_chart(app: &App, n: usize) -> Option<OpenChartContext> {
    let d = app.detail.as_ref()?;
    let bars = d.bars()?;
    let warm = d.warmup.get(&d.range).map_or(&[][..], Vec::as_slice);
    // The same builder as the CLI, once, over every visible bar (so the
    // cursor's bar can be read even when it's older than the newest N).
    let mut chart_ctx = build_all(&d.symbol, d.range, warm, bars, d.show_extended());
    let (candles, _) = d.candles()?;
    let cursor = (!candles.is_empty()).then(|| {
        let i = d.chart.cursor_idx(candles.len());
        let c = &candles[i];
        let bar = chart_ctx.bars.get(c.end);
        CursorContext {
            index: i,
            latest: i + 1 == candles.len(),
            time: c.time,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
            change_pct: c.change_pct(),
            bars_merged: c.bars,
            bar_time: bar.map_or(c.time, |b| b.time),
            ichimoku: bar.map_or_else(IchimokuAt::default, |b| b.ichimoku),
        }
    });
    let n_all = chart_ctx.bars.len();
    chart_ctx.bars.drain(..n_all.saturating_sub(n));
    let (reference, last_price) = super::ui::chart_price_refs(app, d, app.snapshots.get(&d.symbol));
    let levels = if d.chart.levels {
        let (s, e) = d.chart.window(candles.len(), d.chart.visible.get());
        chart::swing_levels(&candles[s..e], 3)
            .into_iter()
            .map(|l| LevelLine {
                kind: l.kind,
                price: l.price,
                touches: l.touches,
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(OpenChartContext {
        chart: chart_ctx,
        cursor,
        price_lines: PriceLines {
            price_line: d.chart.price,
            reference: reference.map(|price| ReferenceLine {
                label: if fx::is_fx(&d.symbol) {
                    Reference::DayOpen
                } else {
                    Reference::PrevClose
                },
                price,
            }),
            last_price,
            levels,
        },
        view: ViewState {
            ichimoku_shown: d.chart.ichimoku,
            ma_shown: d.chart.ma,
            levels_shown: d.chart.levels,
            fit: d.chart.fit,
            bars_per_candle: candles.iter().map(|c| c.bars).max().unwrap_or(1),
        },
    })
}

// ── the whole screen, for the agent panel ──────────────────────────────────

/// Bars of the open chart attached to an agent message (the same default as
/// `wickd chart context --bars`).
pub const SCREEN_CHART_BARS: usize = 60;

/// Calendar rows attached at most (the watch is a few dozen; "everything"
/// can run to hundreds).
const SCREEN_CALENDAR_ROWS: usize = 150;

/// Journal text attached at most, in characters.
const SCREEN_JOURNAL_CHARS: usize = 8_000;

/// What's on the current screen, attached to every message the agent panel
/// sends (D-21): the open chart's [`OpenChartContext`], a list's or the
/// calendar's rows, the open journal entry, or at least the screen's name.
/// Values only, like the rest of this module.
pub fn screen(app: &App) -> Value {
    match app.mode {
        Mode::Detail => match open_chart(app, SCREEN_CHART_BARS) {
            Some(c) => tagged("chart", serde_json::to_value(c).unwrap_or(Value::Null)),
            None => {
                let d = app.detail.as_ref();
                json!({
                    "screen": "chart",
                    "symbol": d.map(|d| d.symbol.clone()),
                    "range": d.map(|d| d.range.label()),
                    "loading": true,
                })
            }
        },
        Mode::Watchlist => watchlist_screen(app),
        Mode::InPlay => inplay_screen(app),
        Mode::Calendar => calendar_screen(app),
        Mode::Journal => {
            let j = &app.journal;
            json!({
                "screen": "journal",
                "entry": j.current().map(|e| e.name.clone()),
                "text": j.text.chars().take(SCREEN_JOURNAL_CHARS).collect::<String>(),
            })
        }
        Mode::Search => json!({ "screen": "search", "query": app.search.query }),
    }
}

/// What [`screen`] attaches, in a few words for the panel's title:
/// `EUR_USD 1h chart`, `the calendar (your watch)`.
pub fn screen_label(app: &App) -> String {
    match app.mode {
        Mode::Detail => match &app.detail {
            Some(d) => format!("{} {} chart", d.symbol, Timeframe::of_range(d.range).label),
            None => "the chart".into(),
        },
        Mode::Watchlist => match app.lists.get(app.expanded) {
            Some((name, _)) => format!("the {name} list and watchlist"),
            None => "the watchlist".into(),
        },
        Mode::InPlay => "in play today".into(),
        Mode::Calendar => format!(
            "the calendar ({})",
            if app.calendar.show_all { "everything" } else { "your watch" }
        ),
        Mode::Journal => match app.journal.current() {
            Some(e) => format!("journal {}", e.name),
            None => "the journal".into(),
        },
        Mode::Search => "search".into(),
    }
}

/// `{"screen": name, ...fields}`.
fn tagged(name: &str, body: Value) -> Value {
    let mut out = serde_json::Map::new();
    out.insert("screen".into(), json!(name));
    if let Value::Object(m) = body {
        out.extend(m);
    }
    Value::Object(out)
}

/// A symbol's last price and change today, as the rows show them.
fn quote_fields(app: &App, sym: &str) -> (Option<String>, Option<f64>) {
    if fx::is_fx(sym) {
        match app.fx_quotes.get(sym) {
            Some(q) => (
                Some(q.mid().to_string()),
                q.change_pct().and_then(|c| c.to_f64()),
            ),
            None => (None, None),
        }
    } else {
        match app.snapshots.get(sym) {
            Some(s) => (s.last().map(|p| p.to_string()), s.change_pct()),
            None => (None, None),
        }
    }
}

fn name_of(app: &App, sym: &str) -> Option<String> {
    app.asset(sym).map(|a| a.name.clone())
}

/// The watchlist screen: the open list's rows (symbol, name, note, plan),
/// the other lists by name, and the watchlist with prices.
fn watchlist_screen(app: &App) -> Value {
    let list = app.lists.get(app.expanded).map(|(name, entries)| {
        let rows: Vec<Value> = entries
            .iter()
            .map(|e| {
                json!({
                    "symbol": e.symbol,
                    "name": name_of(app, &e.symbol),
                    "note": e.note,
                    "plan": e.plan,
                })
            })
            .collect();
        json!({ "name": name, "rows": rows })
    });
    let other_lists: Vec<&str> = app
        .lists
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != app.expanded)
        .map(|(_, (n, _))| n.as_str())
        .collect();
    let watchlist: Vec<Value> = app
        .watchlist
        .iter()
        .map(|sym| {
            let (last, change_pct) = quote_fields(app, sym);
            json!({
                "symbol": sym,
                "name": name_of(app, sym),
                "last": last,
                "change_pct": change_pct,
            })
        })
        .collect();
    json!({
        "screen": "watchlist",
        "selected": app.selected_symbol(),
        "list": list,
        "other_lists": other_lists,
        "watchlist": watchlist,
    })
}

/// The in-play screen: its rows as filtered and sorted on screen.
fn inplay_screen(app: &App) -> Value {
    let rows: Vec<Value> = app
        .inplay
        .visible()
        .iter()
        .map(|r| {
            let tags: Vec<&str> = r.tags.iter().map(|t| t.label()).collect();
            json!({
                "symbol": r.symbol,
                "name": name_of(app, &r.symbol),
                "last": r.last,
                "change_pct": r.chg_pct,
                "from_open_pct": r.from_open_pct,
                "volume": r.volume,
                "relative_volume": r.rvol,
                "tags": tags,
                "headlines": r.news_count,
            })
        })
        .collect();
    json!({
        "screen": "in_play",
        "selected": app.inplay.selected_symbol(),
        "rows": rows,
    })
}

/// The calendar screen: every row it shows, each with the day it falls
/// under.
fn calendar_screen(app: &App) -> Value {
    use super::calendar::Row;
    let cal = &app.calendar;
    let mut day = String::new();
    let mut rows: Vec<Value> = Vec::new();
    for row in &cal.rows {
        let v = match row {
            Row::Day(label) => {
                day = label.clone();
                continue;
            }
            Row::Blank => continue,
            Row::Note(text) => json!({ "day": day, "note": text }),
            Row::Econ(e) => json!({
                "day": day,
                "type": "economic",
                "time": e.time,
                "event": e.event,
                "impact": e.impact,
                "forecast": e.forecast,
                "previous": e.previous,
                "past": e.past,
            }),
            Row::Earn(e) => json!({
                "day": day,
                "type": "earnings",
                "symbol": e.symbol,
                "name": e.name,
                "date": e.date,
                "session": super::calendar::session_text(e.session),
                "eps_forecast": e.eps_forecast.map(|d| d.to_string()),
                "last_year_eps": e.last_year_eps.map(|d| d.to_string()),
                "market_cap": e.market_cap.map(|d| d.to_string()),
                "on_slate": e.slate,
                "on_a_list": e.listed,
                "why": e.why,
                "expect": e.expect,
                "result": e.result,
                "reported": cal.reported_of(e),
                "reaction": cal.reaction_of(e),
                "past": e.past,
            }),
        };
        rows.push(v);
        if rows.len() == SCREEN_CALENDAR_ROWS {
            break;
        }
    }
    json!({
        "screen": "calendar",
        "showing": if cal.show_all { "everything" } else { "watch" },
        "selected": cal.selected_symbol(),
        "rows": rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// 5-minute regular-hours bars from 09:30 ET, with a shape that moves
    /// the Ichimoku lines around.
    fn bars(n: usize, start: &str) -> Vec<Bar> {
        let t0: DateTime<Utc> = start.parse().unwrap();
        (0..n)
            .map(|i| {
                let base = 100.0 + ((i * 7) % 23) as f64 * 0.25 - ((i * 3) % 11) as f64 * 0.5;
                Bar {
                    time: t0 + chrono::Duration::minutes(5 * i as i64),
                    open: base,
                    high: base + 1.25 + (i % 5) as f64 * 0.25,
                    low: base - 1.0 - (i % 3) as f64 * 0.25,
                    close: base + 0.5,
                    volume: 10.0 + i as f64,
                    vwap: None,
                }
            })
            .collect()
    }

    #[test]
    fn timeframes_parse_and_map_to_chart_ranges() {
        assert_eq!(Timeframe::parse("1h").unwrap().range, HistoryRange::Month1);
        assert_eq!(Timeframe::parse("1D").unwrap().range, HistoryRange::Year1);
        assert_eq!(Timeframe::parse("5min").unwrap().range, HistoryRange::Day1);
        assert_eq!(Timeframe::parse("1mo").unwrap().range, HistoryRange::Max);
        assert!(Timeframe::parse("3h").is_none());
        // Every chart range has a timeframe label.
        for r in HistoryRange::ALL {
            assert_ne!(Timeframe::of_range(r).label, "?", "{r:?}");
        }
        assert_eq!(Timeframe::of_range(HistoryRange::Month6).label, "1d");
    }

    /// AC3: the builder's Ichimoku values equal the ones the chart renders
    /// (`ichimoku_overlays`, as `render_chart_view` calls it) for the same
    /// bars, one candle per bar and merged.
    #[test]
    fn builder_ichimoku_equals_what_the_chart_renders() {
        let range = HistoryRange::Day1;
        let warm = bars(80, "2026-11-05T14:30:00Z");
        let vis = bars(78, "2026-11-06T14:30:00Z");
        let ctx = build("SPY", range, &warm, &vis, false, usize::MAX);
        assert_eq!(ctx.bars.len(), 78);
        assert_eq!(ctx.kind, Kind::Stock);
        assert_eq!(ctx.session, Session::Regular);

        // What the chart draws, from the same inputs.
        let (cw, cv, _) = chart::indicator_bars(&warm, &vis, range, false);
        for k in [1, 3] {
            let candles = chart::build(&cv, k);
            let (ov, cloud, fut) = chart::ichimoku_overlays(&cw, &cv, &candles);
            let line = |label: &str| &ov.iter().find(|o| o.label == label).unwrap().values;
            for (ci, c) in candles.iter().enumerate() {
                let b = &ctx.bars[c.end].ichimoku;
                assert_eq!(b.tenkan, line("Tenkan")[ci], "tenkan k={k} @{ci}");
                assert_eq!(b.kijun, line("Kijun")[ci], "kijun k={k} @{ci}");
                assert_eq!(b.chikou, line("Chikou")[ci], "chikou k={k} @{ci}");
                assert_eq!(b.span_a_plotted, cloud.a[ci], "span A k={k} @{ci}");
                assert_eq!(b.span_b_plotted, cloud.b[ci], "span B k={k} @{ci}");
            }
            if k == 1 {
                // The projected cloud matches the chart's future slots.
                assert_eq!(fut, ICHI_SHIFT);
                for (j, a) in ctx.cloud_ahead.iter().enumerate() {
                    assert_eq!(a.span_a, cloud.a[candles.len() + j]);
                    assert_eq!(a.span_b, cloud.b[candles.len() + j]);
                }
            }
        }
        // Warm-up makes every line defined from the first visible bar, and
        // computed spans are the plotted ones 26 bars on.
        let first = &ctx.bars[0].ichimoku;
        assert!(first.tenkan.is_some() && first.kijun.is_some() && first.span_b_plotted.is_some());
        assert_eq!(
            ctx.bars[10].ichimoku.span_a_computed,
            ctx.bars[10 + ICHI_SHIFT].ichimoku.span_a_plotted
        );
        // Chikou is the close 26 bars later; the newest 26 have none.
        assert_eq!(
            ctx.bars[5].ichimoku.chikou,
            Some(ctx.bars[5 + ICHI_SHIFT].close)
        );
        assert!(ctx.bars[77].ichimoku.chikou.is_none());
    }

    #[test]
    fn builder_keeps_the_newest_n_bars_and_daily_averages() {
        let ctx = build(
            "SPY",
            HistoryRange::Day1,
            &[],
            &bars(40, "2026-11-06T14:30:00Z"),
            false,
            10,
        );
        assert_eq!(ctx.bars_available, 40);
        assert_eq!(ctx.bars.len(), 10);
        assert_eq!(
            ctx.bars[9].close,
            bars(40, "2026-11-06T14:30:00Z")[39].close
        );
        assert_eq!(ctx.indicators, vec!["ichimoku"]);
        assert!(ctx.bars[9].ma50.is_none());

        // Daily: MA50/MA200 from the warm-up, same as the chart overlay.
        let warm = bars(250, "2025-01-01T05:00:00Z");
        let vis = bars(30, "2026-01-01T05:00:00Z");
        let ctx = build("SPY", HistoryRange::Year1, &warm, &vis, false, 30);
        assert_eq!(ctx.indicators, vec!["ichimoku", "ma50", "ma200"]);
        assert_eq!(ctx.session, Session::All);
        let ov = chart::moving_averages(&warm, &vis, &chart::build(&vis, 1));
        assert_eq!(ctx.bars[29].ma50, ov[0].values[29]);
        assert_eq!(ctx.bars[29].ma200, ov[1].values[29]);
        assert!(ctx.bars[0].ma200.is_some());
    }

    #[test]
    fn pairs_are_pairs_and_empty_history_is_empty() {
        let ctx = build(
            "EUR_USD",
            HistoryRange::Month1,
            &[],
            &bars(5, "2026-11-06T03:00:00Z"),
            false,
            30,
        );
        assert_eq!(
            (ctx.kind, ctx.session, ctx.timeframe),
            (Kind::Pair, Session::TwentyFourFive, "1h")
        );
        // Pre-market-hours bars are kept for pairs.
        assert_eq!(ctx.bars.len(), 5);
        let empty = build("SPY", HistoryRange::Day1, &[], &[], false, 30);
        assert!(empty.bars.is_empty() && empty.cloud_ahead.is_empty());
        // Serialises without panicking.
        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["kind"], "pair");
        assert_eq!(json["session"], "24/5");
        assert!(json["bars"][0]["ichimoku"]
            .as_object()
            .unwrap()
            .contains_key("span_a_computed"));
    }

    #[test]
    fn open_chart_carries_the_cursor_and_price_lines() {
        let mut a = App::new(vec!["SPY".into()], "IEX");
        assert!(open_chart(&a, 30).is_none());
        a.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(open_chart(&a, 30).is_none(), "no history yet");
        let warm = bars(80, "2026-11-05T14:30:00Z");
        let vis = bars(60, "2026-11-06T14:30:00Z");
        a.set_warmup("SPY", HistoryRange::Day1, warm.clone());
        a.set_history("SPY", HistoryRange::Day1, Ok(vis.clone()));
        {
            let d = a.detail.as_mut().unwrap();
            d.chart.cursor = Some(10);
            d.chart.price = Some(101.5);
            d.chart.levels = true;
            d.chart.plot_cols.set(200); // one candle per bar
        }
        let ctx = open_chart(&a, 30).unwrap();
        assert_eq!(ctx.chart.symbol, "SPY");
        assert_eq!(ctx.chart.bars.len(), 30);
        // Same values as the CLI builder for the same history.
        assert_eq!(
            ctx.chart,
            build("SPY", HistoryRange::Day1, &warm, &vis, false, 30)
        );
        let cur = ctx.cursor.clone().unwrap();
        assert_eq!((cur.index, cur.latest, cur.bars_merged), (10, false, 1));
        assert_eq!(cur.time, vis[10].time);
        assert_eq!(cur.close, vis[10].close);
        let full = build("SPY", HistoryRange::Day1, &warm, &vis, false, usize::MAX);
        assert_eq!(cur.ichimoku, full.bars[10].ichimoku);
        assert_eq!(ctx.price_lines.price_line, Some(101.5));
        assert!(ctx.price_lines.reference.is_none(), "no snapshot yet");
        assert!(!ctx.price_lines.levels.is_empty());
        assert!(ctx.view.levels_shown && !ctx.view.ichimoku_shown);
        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["symbol"], "SPY"); // flattened
        assert_eq!(json["cursor"]["index"], 10);
    }
}
