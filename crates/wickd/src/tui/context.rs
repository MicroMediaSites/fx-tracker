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
//! plus its cursor bar and price lines, for the agent panel to attach.
//!
//! Values only: nothing here judges a setup or checks a rule (D-20).

use chrono::{DateTime, Utc};
use serde::Serialize;

use wickd_core::equities::{Bar, HistoryRange};

use super::app::App;
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
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

/// What a chart shows, as numbers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChartContext {
    pub symbol: String,
    /// `stock` (Alpaca) or `pair` (OANDA).
    pub kind: &'static str,
    /// Bar size: `5m`, `15m`, `1h`, `1d`, `1w`, `1mo`.
    pub timeframe: &'static str,
    /// The chart range that shows this timeframe (`1D`, `5D`, `1M`, …).
    pub range: &'static str,
    /// `regular` / `extended` hours (intraday stocks), `24/5` (pairs) or
    /// `all` (daily and longer bars).
    pub session: &'static str,
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
    let is_fx = fx::is_fx(symbol);
    let (warm, vis) = chart::indicator_bars(warmup, bars, range, extended || is_fx);
    let (_, ext) = chart::session_bars(bars, range, extended || is_fx);
    let (mut all, cloud_ahead, ma) = all_bars(range, &warm, &vis);
    let available = all.len();
    all.drain(..available.saturating_sub(n));
    let mut indicators = vec!["ichimoku"];
    if ma {
        indicators.extend(["ma50", "ma200"]);
    }
    ChartContext {
        symbol: symbol.to_string(),
        kind: if is_fx { "pair" } else { "stock" },
        timeframe: Timeframe::of_range(range).label,
        range: range.label(),
        session: if is_fx {
            "24/5"
        } else if !range.intraday() {
            "all"
        } else if ext {
            "extended"
        } else {
            "regular"
        },
        indicators,
        ichimoku_settings: IchimokuSettings {
            tenkan: ICHI_TENKAN,
            kijun: ICHI_KIJUN,
            senkou_b: ICHI_SENKOU_B,
            shift: ICHI_SHIFT,
        },
        bars_available: available,
        bars: all,
        cloud_ahead,
    }
}

// ── the chart the TUI has open ──────────────────────────────────────────────

// Attached by the agent panel (a later ticket); until then only tests use it.
#[allow(dead_code)]
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

// Attached by the agent panel (a later ticket); until then only tests use it.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReferenceLine {
    /// `prev close` (stocks) or `day open` (pairs), on 1D only.
    pub label: &'static str,
    pub price: f64,
}

// Attached by the agent panel (a later ticket); until then only tests use it.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LevelLine {
    /// `support` or `resistance`.
    pub kind: &'static str,
    pub price: f64,
    /// Swing points that formed it.
    pub touches: usize,
}

// Attached by the agent panel (a later ticket); until then only tests use it.
#[allow(dead_code)]
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

// Attached by the agent panel (a later ticket); until then only tests use it.
#[allow(dead_code)]
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

// Attached by the agent panel (a later ticket); until then only tests use it.
#[allow(dead_code)]
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

// Attached by the agent panel (a later ticket); until then only tests use it.
#[allow(dead_code)]
/// The context of the chart open in the TUI (`None` when no chart is open or
/// its history hasn't loaded), with the newest `n` bars.
pub fn open_chart(app: &App, n: usize) -> Option<OpenChartContext> {
    let d = app.detail.as_ref()?;
    let bars = d.bars()?;
    let warm = d.warmup.get(&d.range).map_or(&[][..], Vec::as_slice);
    let extended = d.show_extended();
    let chart_ctx = build(&d.symbol, d.range, warm, bars, extended, n);
    // Per-bar values for every visible bar, so the cursor's last bar can be
    // read even when it's older than the newest N.
    let (iwarm, ivis) = chart::indicator_bars(warm, bars, d.range, extended);
    let (per_bar, _, _) = all_bars(d.range, &iwarm, &ivis);
    let (candles, _) = d.candles()?;
    let cursor = (!candles.is_empty()).then(|| {
        let i = d.chart.cursor_idx(candles.len());
        let c = &candles[i];
        let bar = per_bar.get(c.end);
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
            ichimoku: bar.map_or(
                IchimokuAt {
                    tenkan: None,
                    kijun: None,
                    span_a_plotted: None,
                    span_b_plotted: None,
                    span_a_computed: None,
                    span_b_computed: None,
                    chikou: None,
                },
                |b| b.ichimoku,
            ),
        }
    });
    let (reference, last_price) = super::ui::chart_price_refs(app, d, app.snapshots.get(&d.symbol));
    let levels = if d.chart.levels {
        let (s, e) = d.chart.window(candles.len(), d.chart.visible.get());
        chart::swing_levels(&candles[s..e], 3)
            .into_iter()
            .map(|l| LevelLine {
                kind: match l.kind {
                    LevelKind::Support => "support",
                    LevelKind::Resistance => "resistance",
                },
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
                    "day open"
                } else {
                    "prev close"
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
        assert_eq!(ctx.kind, "stock");
        assert_eq!(ctx.session, "regular");

        // What the chart draws, from the same inputs.
        let (cw, cv) = chart::indicator_bars(&warm, &vis, range, false);
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
        assert_eq!(ctx.session, "all");
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
            ("pair", "24/5", "1h")
        );
        // Pre-market-hours bars are kept for pairs.
        assert_eq!(ctx.bars.len(), 5);
        let empty = build("SPY", HistoryRange::Day1, &[], &[], false, 30);
        assert!(empty.bars.is_empty() && empty.cloud_ahead.is_empty());
        // Serialises without panicking.
        let json = serde_json::to_value(&ctx).unwrap();
        assert_eq!(json["kind"], "pair");
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
