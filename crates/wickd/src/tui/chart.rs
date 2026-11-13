//! The symbol chart for `wickd tui`: candles (default) or a close line,
//! with zoom, pan and a crosshair cursor. Drawn straight into the ratatui
//! buffer — ratatui has no candlestick widget.
//!
//! ## Candles
//!
//! Hollow-candle scheme, matching the wickd desktop app (OANDA/TradingView):
//! - **colour** is direction vs the *previous candle's close* (teal up / red
//!   down); the first candle falls back to close vs open;
//! - **fill** is hollow when close ≥ open, solid when close < open.
//!
//! Narrow candles (1 column) draw hollow bodies as a heavy line `┃` and solid
//! bodies as blocks `█`, both at half-row precision. Wide candles (3 or 5
//! columns, zoomed in) draw hollow bodies as a box outline.
//!
//! ## Zoom, pan, cursor
//!
//! [`ZOOMS`] runs from merged candles (8/4/2 bars per candle) through
//! 1-column candles to 2/4/6-column slots. The window shows as many candles as
//! fit; [`ChartView`] tracks the cursor and the rightmost visible candle
//! (`None` = pinned to the latest, so live updates keep scrolling in). The
//! renderer reports how many candles fit back through `visible`, so key
//! handling can page and keep the cursor on screen.
//!
//! ## Session
//!
//! Intraday ranges show regular hours (09:30–16:00 ET) by default; extended
//! hours are a toggle. If a window has no regular-hours bars yet (pre-market
//! on a trading day), it falls back to everything rather than show nothing.

use std::cell::Cell;

use chrono::{DateTime, Duration, Timelike, Utc};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::Widget;

use wickd_core::equities::{eastern_offset_hours, Bar, HistoryRange};

// ── zoom ────────────────────────────────────────────────────────────────────

/// One zoom level: merge `n` bars per candle, or give each candle a slot of
/// `n` columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    Merge(usize),
    Slot(u16),
}

pub const ZOOMS: [Zoom; 7] = [
    Zoom::Merge(8),
    Zoom::Merge(4),
    Zoom::Merge(2),
    Zoom::Slot(1),
    Zoom::Slot(2),
    Zoom::Slot(4),
    Zoom::Slot(6),
];

/// Default: 1-column candles with a 1-column gap.
pub const DEFAULT_ZOOM: usize = 4;

impl Zoom {
    pub fn merge(self) -> usize {
        match self {
            Zoom::Merge(n) => n,
            Zoom::Slot(_) => 1,
        }
    }

    /// Columns per candle (merged levels use 2: candle + gap).
    pub fn slot(self) -> u16 {
        match self {
            Zoom::Merge(_) => 2,
            Zoom::Slot(n) => n,
        }
    }

    /// Body width in columns.
    pub fn body(self) -> u16 {
        match self.slot() {
            0..=2 => 1,
            s => s - 1,
        }
    }
}

// ── candles ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Candle {
    pub time: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// Close of the candle before this one, if any.
    pub prev_close: Option<f64>,
    /// Number of source bars merged into this candle.
    pub bars: usize,
    /// Index (into the bars it was built from) of its last source bar, so
    /// per-bar series (moving averages, …) line up with merged candles.
    pub end: usize,
}

impl Candle {
    /// Direction for colour: vs previous close, else close vs open.
    pub fn up(&self) -> bool {
        match self.prev_close {
            Some(p) => self.close >= p,
            None => self.close >= self.open,
        }
    }

    /// Hollow body when the candle closed at or above its open.
    pub fn hollow(&self) -> bool {
        self.close >= self.open
    }

    /// Change vs the previous candle's close, in percent.
    pub fn change_pct(&self) -> Option<f64> {
        let p = self.prev_close?;
        (p != 0.0).then(|| (self.close - p) / p * 100.0)
    }
}

/// True when `t` (a bar's start) falls in the regular session, 09:30–16:00 ET.
pub fn is_regular_session(t: DateTime<Utc>) -> bool {
    let et = t + Duration::hours(eastern_offset_hours(t));
    let m = et.hour() * 60 + et.minute();
    (9 * 60 + 30..16 * 60).contains(&m)
}

/// Bars to chart for a range: regular hours only for intraday ranges unless
/// `extended`. Returns the bars and whether extended hours are included.
pub fn session_bars(bars: &[Bar], range: HistoryRange, extended: bool) -> (Vec<Bar>, bool) {
    if extended || !range.intraday() {
        return (bars.to_vec(), range.intraday());
    }
    let regular: Vec<Bar> = bars
        .iter()
        .filter(|b| is_regular_session(b.time))
        .cloned()
        .collect();
    if regular.is_empty() {
        (bars.to_vec(), true)
    } else {
        (regular, false)
    }
}

/// Merge consecutive bars `k` at a time (grouped from the newest end, so the
/// latest candle is complete) and link each candle to its predecessor's close.
pub fn build(bars: &[Bar], k: usize) -> Vec<Candle> {
    let k = k.max(1);
    let n = bars.len();
    let mut out: Vec<Candle> = Vec::with_capacity(n.div_ceil(k));
    if n == 0 {
        return out;
    }
    let mut start = 0;
    let mut len = match n % k {
        0 => k,
        r => r,
    };
    while start < n {
        let g = &bars[start..start + len];
        let prev_close = out.last().map(|c| c.close);
        out.push(Candle {
            time: g[0].time,
            open: g[0].open,
            close: g[g.len() - 1].close,
            high: g.iter().map(|b| b.high).fold(f64::NEG_INFINITY, f64::max),
            low: g.iter().map(|b| b.low).fold(f64::INFINITY, f64::min),
            volume: g.iter().map(|b| b.volume).sum(),
            prev_close,
            bars: g.len(),
            end: start + len - 1,
        });
        start += len;
        len = k;
    }
    out
}

// ── overlays ────────────────────────────────────────────────────────────────

/// A line drawn over the candles (moving average, …): one value per candle,
/// `None` where it isn't defined (not enough history yet).
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    pub label: &'static str,
    pub color: Color,
    pub values: Vec<Option<f64>>,
}

/// 50/200-day averages are computed from daily bars, so they only show on
/// the daily-bar ranges (6M, 1Y) — never a "50-day" from weekly candles.
pub fn ma_range(range: HistoryRange) -> bool {
    matches!(range, HistoryRange::Month6 | HistoryRange::Year1)
}

/// Calendar days of extra daily history fetched before the visible window
/// so a 200-day average spans the whole chart (~200 trading days).
pub const MA_WARMUP_DAYS: i64 = 300;

/// Calendar days of extra history fetched before a range's window: ~200
/// trading days for the daily MAs, and on intraday ranges enough sessions of
/// the same timeframe for Ichimoku (78 bars: 52 for span B + 26 shift) to
/// span the whole chart, with slack for weekends and holidays.
pub fn warmup_days(range: HistoryRange) -> i64 {
    match range {
        HistoryRange::Day1 => 6,    // 5Min: 78 bars/session
        HistoryRange::Day5 => 8,    // 15Min: 26 bars/session
        HistoryRange::Month1 => 21, // 1Hour: ~7 bars/session
        r if ma_range(r) => MA_WARMUP_DAYS,
        _ => 0,
    }
}

/// Split a warm-up fetch into (warm-up, visible): the visible part is what
/// the range shows (bars from `start`, then [`HistoryRange::trim`]);
/// everything older than its first bar is warm-up.
pub fn split_warmup(
    range: HistoryRange,
    start: DateTime<Utc>,
    all: Vec<Bar>,
) -> (Vec<Bar>, Vec<Bar>) {
    let vis = range.trim(all.iter().filter(|b| b.time >= start).cloned().collect());
    let first = vis.first().map(|b| b.time);
    let warm = all
        .into_iter()
        .filter(|b| first.is_some_and(|f| b.time < f))
        .collect();
    (warm, vis)
}

pub const MA50_COLOR: Color = Color::Rgb(59, 130, 246);
pub const MA200_COLOR: Color = Color::Rgb(245, 158, 11);

/// The 50/200-day overlays for `candles` built from `visible` daily bars,
/// with `warmup` bars before them.
pub fn moving_averages(warmup: &[Bar], visible: &[Bar], candles: &[Candle]) -> Vec<Overlay> {
    let closes: Vec<f64> = warmup.iter().chain(visible).map(|b| b.close).collect();
    let off = warmup.len();
    vec![
        Overlay {
            label: "MA50",
            color: MA50_COLOR,
            values: per_candle(&sma(&closes, 50), candles, off),
        },
        Overlay {
            label: "MA200",
            color: MA200_COLOR,
            values: per_candle(&sma(&closes, 200), candles, off),
        },
    ]
}

// ── Ichimoku ────────────────────────────────────────────────────────────────

/// Standard settings: tenkan 9, kijun 26, senkou B 52, displacement 26.
pub const ICHI_TENKAN: usize = 9;
pub const ICHI_KIJUN: usize = 26;
pub const ICHI_SENKOU_B: usize = 52;
pub const ICHI_SHIFT: usize = 26;

// Same palette as the wickd desktop app (INDICATOR_COLORS).
pub const TENKAN_COLOR: Color = Color::Rgb(212, 164, 57);
pub const KIJUN_COLOR: Color = Color::Rgb(74, 144, 194);
pub const CHIKOU_COLOR: Color = Color::Rgb(220, 220, 220);
/// Faint cloud tints (span A above B / below B).
pub const CLOUD_UP: Color = Color::Rgb(22, 52, 44);
pub const CLOUD_DOWN: Color = Color::Rgb(60, 30, 36);

/// The cloud's two edges, one value per candle slot (candles, then the
/// projected slots to the right of the newest candle).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cloud {
    pub a: Vec<Option<f64>>,
    pub b: Vec<Option<f64>>,
}

/// Ichimoku in *bar* space, extended [`ICHI_SHIFT`] bars into the future
/// (where only the displaced spans have values).
#[derive(Debug, Clone, PartialEq)]
pub struct IchimokuSeries {
    pub tenkan: Vec<Option<f64>>,
    pub kijun: Vec<Option<f64>>,
    /// Senkou A, already displaced forward by [`ICHI_SHIFT`].
    pub span_a: Vec<Option<f64>>,
    /// Senkou B, already displaced forward by [`ICHI_SHIFT`].
    pub span_b: Vec<Option<f64>>,
    /// Chikou: the close [`ICHI_SHIFT`] bars later, plotted here.
    pub chikou: Vec<Option<f64>>,
}

/// (highest high + lowest low) / 2 over the `p` bars ending at `i`.
fn midpoint(bars: &[Bar], i: usize, p: usize) -> Option<f64> {
    if i + 1 < p || i >= bars.len() {
        return None;
    }
    let w = &bars[i + 1 - p..=i];
    let h = w.iter().map(|b| b.high).fold(f64::NEG_INFINITY, f64::max);
    let l = w.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    Some((h + l) / 2.0)
}

pub fn ichimoku_series(bars: &[Bar]) -> IchimokuSeries {
    let n = bars.len();
    let len = n + ICHI_SHIFT;
    let tenkan: Vec<Option<f64>> = (0..len).map(|i| midpoint(bars, i, ICHI_TENKAN)).collect();
    let kijun: Vec<Option<f64>> = (0..len).map(|i| midpoint(bars, i, ICHI_KIJUN)).collect();
    let back = |i: usize| i.checked_sub(ICHI_SHIFT).filter(|j| *j < n);
    let span_a = (0..len)
        .map(|i| {
            let j = back(i)?;
            Some((tenkan[j]? + kijun[j]?) / 2.0)
        })
        .collect();
    let span_b = (0..len)
        .map(|i| midpoint(bars, back(i)?, ICHI_SENKOU_B))
        .collect();
    let chikou = (0..len)
        .map(|i| bars.get(i + ICHI_SHIFT).map(|b| b.close))
        .collect();
    IchimokuSeries {
        tenkan,
        kijun,
        span_a,
        span_b,
        chikou,
    }
}

/// Ichimoku overlays + cloud for `candles` built from `visible` bars (with
/// `warmup` bars before them), and how many projected slots they need to
/// the right of the newest candle (26 bars, in candle units when merged).
pub fn ichimoku_overlays(
    warmup: &[Bar],
    visible: &[Bar],
    candles: &[Candle],
) -> (Vec<Overlay>, Cloud, usize) {
    let all: Vec<Bar> = warmup.iter().chain(visible).cloned().collect();
    let s = ichimoku_series(&all);
    let off = warmup.len();
    let Some(last) = candles.last() else {
        return (Vec::new(), Cloud::default(), 0);
    };
    let k = last.bars.max(1);
    let future = ICHI_SHIFT.div_ceil(k);
    let sample = |v: &[Option<f64>]| -> Vec<Option<f64>> {
        candles
            .iter()
            .map(|c| v.get(off + c.end).copied().flatten())
            .chain((1..=future).map(|j| v.get(off + last.end + j * k).copied().flatten()))
            .collect()
    };
    // Span A/B are not drawn as lines: the cloud tint already shows both edges.
    let overlays = vec![
        Overlay {
            label: "Tenkan",
            color: TENKAN_COLOR,
            values: sample(&s.tenkan),
        },
        Overlay {
            label: "Kijun",
            color: KIJUN_COLOR,
            values: sample(&s.kijun),
        },
        Overlay {
            label: "Chikou",
            color: CHIKOU_COLOR,
            values: sample(&s.chikou),
        },
    ];
    let cloud = Cloud {
        a: sample(&s.span_a),
        b: sample(&s.span_b),
    };
    (overlays, cloud, future)
}

// ── support / resistance ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelKind {
    Support,
    Resistance,
}

/// A horizontal level built from clustered swing highs/lows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub price: f64,
    /// Swing points that formed it (more = stronger).
    pub touches: usize,
    pub kind: LevelKind,
}

pub const SUPPORT_COLOR: Color = Color::Rgb(110, 160, 190);
pub const RESISTANCE_COLOR: Color = Color::Rgb(200, 140, 100);
/// Background behind level tags so they stay legible over candles.
const GRID_BG: Color = Color::Rgb(21, 26, 34);

/// Support/resistance from the swing highs and lows in `candles`.
///
/// - A swing high is a candle whose high is the highest of the `L` candles
///   either side (swing low: lowest low), with `L` scaled to the window
///   (2..=8).
/// - Swing points within half an average candle range of each other merge
///   into one level (its price is their mean; `touches` counts them).
/// - Levels above the last close are resistance, below are support; the
///   `per_side` strongest of each are kept (ties: nearest to price).
pub fn swing_levels(candles: &[Candle], per_side: usize) -> Vec<Level> {
    let n = candles.len();
    if n < 7 || per_side == 0 {
        return Vec::new();
    }
    let l = (n / 30).clamp(2, 8);
    let mut pivots: Vec<f64> = Vec::new();
    for i in l..n.saturating_sub(l) {
        let w = &candles[i - l..=i + l];
        let c = &candles[i];
        if w.iter().all(|x| x.high <= c.high) {
            pivots.push(c.high);
        }
        if w.iter().all(|x| x.low >= c.low) {
            pivots.push(c.low);
        }
    }
    let avg_range = candles.iter().map(|c| c.high - c.low).sum::<f64>() / n as f64;
    let tol = (avg_range * 0.5).max(f64::EPSILON);
    pivots.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Greedy clusters over the sorted prices.
    let mut clusters: Vec<Vec<f64>> = Vec::new();
    for p in pivots {
        match clusters.last_mut() {
            Some(c) if p - c.iter().sum::<f64>() / c.len() as f64 <= tol => c.push(p),
            _ => clusters.push(vec![p]),
        }
    }
    let last = candles[n - 1].close;
    let mut levels: Vec<Level> = clusters
        .into_iter()
        .map(|c| {
            let price = c.iter().sum::<f64>() / c.len() as f64;
            Level {
                price,
                touches: c.len(),
                kind: if price >= last {
                    LevelKind::Resistance
                } else {
                    LevelKind::Support
                },
            }
        })
        .collect();
    levels.sort_by(|a, b| {
        b.touches.cmp(&a.touches).then(
            (a.price - last)
                .abs()
                .partial_cmp(&(b.price - last).abs())
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    let mut out: Vec<Level> = Vec::new();
    for kind in [LevelKind::Resistance, LevelKind::Support] {
        out.extend(levels.iter().filter(|l| l.kind == kind).take(per_side));
    }
    out
}

/// Simple moving average: element `i` averages `values[i+1-n..=i]`; `None`
/// until `n` values exist (no fake early values).
pub fn sma(values: &[f64], n: usize) -> Vec<Option<f64>> {
    let mut out = Vec::with_capacity(values.len());
    if n == 0 {
        return vec![None; values.len()];
    }
    let mut sum = 0.0;
    for (i, v) in values.iter().enumerate() {
        sum += v;
        if i >= n {
            sum -= values[i - n];
        }
        out.push((i + 1 >= n).then(|| sum / n as f64));
    }
    out
}

/// Sample a per-bar series at each candle's last bar. `offset` is how many
/// warm-up bars precede the candles' first bar in `series`.
pub fn per_candle(series: &[Option<f64>], candles: &[Candle], offset: usize) -> Vec<Option<f64>> {
    candles
        .iter()
        .map(|c| series.get(offset + c.end).copied().flatten())
        .collect()
}

// ── view state ──────────────────────────────────────────────────────────────

/// Candles or a close line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style2 {
    Candles,
    Line,
}

#[derive(Debug, Clone)]
pub struct ChartView {
    /// Index into [`ZOOMS`].
    pub zoom: usize,
    /// Cursor candle index; `None` = the latest candle.
    pub cursor: Option<usize>,
    /// Rightmost visible candle; `None` = pinned to the latest.
    pub right: Option<usize>,
    /// Include pre/post-market bars on intraday ranges.
    pub extended: bool,
    /// Show the 50/200-day moving averages (daily ranges).
    pub ma: bool,
    /// Show the Ichimoku cloud.
    pub ichimoku: bool,
    /// Show support/resistance levels.
    pub levels: bool,
    /// Candles that fit, as last measured by the renderer.
    pub visible: Cell<usize>,
    /// Fit mode (the default): the whole range spread across the width,
    /// merging bars only when there are more bars than columns.
    pub fit: bool,
    /// Plot width in columns, as last measured by the renderer.
    pub plot_cols: Cell<usize>,
    /// The price line: a horizontal cursor at an exact price (`j`/`k` step
    /// it, `p` types a value). `None` = no line.
    /// Display only (f64 like the rest of the chart): anything that turns
    /// this into an order, stop or alert price must use `Decimal`.
    pub price: Option<f64>,
    /// One small price-line step: a round number no bigger than one chart
    /// row, as last measured by the renderer.
    pub price_step: Cell<f64>,
}

impl Default for ChartView {
    fn default() -> Self {
        Self {
            zoom: DEFAULT_ZOOM,
            cursor: None,
            right: None,
            extended: false,
            ma: false,
            ichimoku: false,
            levels: false,
            visible: Cell::new(60),
            fit: true,
            plot_cols: Cell::new(120),
            price: None,
            price_step: Cell::new(0.01),
        }
    }
}

impl ChartView {
    pub fn zoom_level(&self) -> Zoom {
        ZOOMS[self.zoom.min(ZOOMS.len() - 1)]
    }

    pub fn cursor_idx(&self, n: usize) -> usize {
        self.cursor
            .map_or(n.saturating_sub(1), |c| c.min(n.saturating_sub(1)))
    }

    /// Bars merged per candle for `bars` source bars: in fit mode, just
    /// enough to fit the plot width; otherwise the zoom level's.
    pub fn merge_for(&self, bars: usize) -> usize {
        if self.fit {
            bars.div_ceil(self.plot_cols.get().max(1)).max(1)
        } else {
            self.zoom_level().merge()
        }
    }

    /// Zoom in one step. From fit, jump to the first level that is closer
    /// than fit currently is.
    pub fn zoom_in(&mut self, bars: usize) {
        if self.fit {
            let k = self.merge_for(bars);
            self.fit = false;
            self.zoom = if k > 1 {
                // Largest merge smaller than the current one, else 1 column.
                (0..ZOOMS.len())
                    .find(|&i| matches!(ZOOMS[i], Zoom::Merge(m) if m < k))
                    .unwrap_or(3)
            } else {
                let avg = self.plot_cols.get() as f64 / bars.max(1) as f64;
                (0..ZOOMS.len())
                    .find(|&i| matches!(ZOOMS[i], Zoom::Slot(s) if f64::from(s) > avg))
                    .unwrap_or(ZOOMS.len() - 1)
            };
        } else {
            self.zoom = (self.zoom + 1).min(ZOOMS.len() - 1);
        }
    }

    /// Zoom out one step; returns false when already fully zoomed out.
    pub fn zoom_out(&mut self, bars: usize) -> bool {
        if self.fit {
            let k = self.merge_for(bars);
            // Smallest merge larger than the current one.
            match (0..ZOOMS.len())
                .rev()
                .find(|&i| matches!(ZOOMS[i], Zoom::Merge(m) if m > k))
            {
                Some(i) => {
                    self.fit = false;
                    self.zoom = i;
                    true
                }
                None => false,
            }
        } else if self.zoom > 0 {
            self.zoom -= 1;
            true
        } else {
            false
        }
    }

    pub fn zoom_fit(&mut self) {
        self.fit = true;
        self.right = None;
    }

    /// Visible window `[start, end)` for `n` candles, `visible` at a time.
    pub fn window(&self, n: usize, visible: usize) -> (usize, usize) {
        if n == 0 {
            return (0, 0);
        }
        if self.fit {
            return (n.saturating_sub(self.plot_cols.get().max(1)), n);
        }
        let visible = visible.max(1);
        let end = self.right.map_or(n, |r| (r + 1).min(n));
        let end = end.max(visible.min(n));
        (end.saturating_sub(visible), end)
    }

    fn settle(&mut self, n: usize) {
        let visible = self.visible.get().max(1);
        let c = self.cursor_idx(n);
        if c + 1 >= n {
            self.cursor = None;
        }
        // Keep the cursor inside the window.
        let (start, end) = self.window(n, visible);
        if c < start {
            self.right = Some(c + visible - 1);
        } else if c >= end {
            self.right = Some(c);
        }
        if self.right.is_some_and(|r| r + 1 >= n) {
            self.right = None;
        }
    }

    pub fn move_cursor(&mut self, n: usize, delta: isize) {
        if n == 0 {
            return;
        }
        let c = self.cursor_idx(n) as isize + delta;
        self.cursor = Some(c.clamp(0, n as isize - 1) as usize);
        self.settle(n);
    }

    /// Shift the window by `delta` candles, dragging the cursor along.
    pub fn pan(&mut self, n: usize, delta: isize) {
        if n == 0 {
            return;
        }
        let visible = self.visible.get().max(1);
        let (_, end) = self.window(n, visible);
        let new_end = (end as isize + delta).clamp(visible.min(n) as isize, n as isize) as usize;
        self.right = Some(new_end - 1);
        let c = self.cursor_idx(n) as isize + delta;
        self.cursor = Some(c.clamp(0, n as isize - 1) as usize);
        self.settle(n);
    }

    /// Step the price line `steps` small steps (negative = down). With no
    /// line yet it starts from `from` (the cursor candle's close). The line
    /// lands on a multiple of the step, so its values stay round.
    pub fn move_price(&mut self, steps: i32, from: f64) {
        let step = self.price_step.get();
        if step <= 0.0 || !step.is_finite() {
            return;
        }
        let base = self.price.unwrap_or(from);
        // Snap toward the direction of travel first, so a typed off-grid
        // value moves to the next round number rather than past it.
        let units = base / step;
        let snapped = if (units - units.round()).abs() < 1e-6 {
            units.round() + f64::from(steps)
        } else if steps > 0 {
            units.floor() + f64::from(steps)
        } else {
            units.ceil() + f64::from(steps)
        };
        self.price = Some((snapped * step).max(0.0));
    }

    pub fn jump_start(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        self.cursor = Some(0);
        self.right = None;
        self.right = Some(self.visible.get().max(1).min(n) - 1);
        self.settle(n);
    }

    pub fn jump_end(&mut self) {
        self.cursor = None;
        self.right = None;
    }

    /// Re-anchor after the candle list changed shape (zoom / session
    /// toggle): keep the cursor on the candle covering the same time.
    pub fn reanchor(&mut self, old: &[Candle], new: &[Candle]) {
        let Some(c) = self.cursor else {
            self.right = None;
            return;
        };
        let t = old.get(c).map(|c| c.time);
        let idx = t.map_or(new.len().saturating_sub(1), |t| {
            new.iter().rposition(|c| c.time <= t).unwrap_or(0)
        });
        self.cursor = Some(idx);
        // Centre the cursor in the new window.
        let half = self.visible.get().max(1) / 2;
        self.right = Some((idx + half).min(new.len().saturating_sub(1)));
        self.settle(new.len());
    }
}

// ── axis helpers ────────────────────────────────────────────────────────────

/// Map a price to a half-row index (0 = top) within `rows` rows.
pub fn half_row(p: f64, lo: f64, hi: f64, rows: u16) -> usize {
    let units = (rows as usize * 2).max(1);
    if hi - lo <= f64::EPSILON {
        return units / 2;
    }
    let frac = ((hi - p) / (hi - lo)).clamp(0.0, 1.0);
    ((frac * (units - 1) as f64).round() as usize).min(units - 1)
}

/// The largest 1 / 2 / 5 × 10ⁿ value that is not above `raw` (never below
/// `min`): the price line's small step.
pub fn nice_floor(raw: f64, min: f64) -> f64 {
    if raw <= 0.0 || !raw.is_finite() {
        return min;
    }
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let nice = if norm >= 5.0 {
        5.0
    } else if norm >= 2.0 {
        2.0
    } else {
        1.0
    };
    (nice * mag).max(min)
}

/// "Nice" tick step (1, 2, 2.5, 5 × 10ⁿ) giving about `target` ticks.
pub fn nice_step(range: f64, target: usize) -> f64 {
    if range <= 0.0 || target == 0 {
        return 1.0;
    }
    let raw = range / target as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let nice = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 2.5 {
        2.5
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

/// Tick values in `[lo, hi]` at multiples of `step`.
pub fn ticks(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    if step <= 0.0 || hi <= lo {
        return Vec::new();
    }
    let mut v = Vec::new();
    let mut t = (lo / step).ceil() * step;
    while t <= hi + step * 1e-9 && v.len() < 64 {
        v.push(t);
        t += step;
    }
    v
}

/// Volume cap: the 95th-percentile volume (so one closing-auction bar doesn't
/// flatten the rest), never below 1/4 of the max.
pub fn volume_cap(vols: &[f64]) -> f64 {
    let max = vols.iter().cloned().fold(0.0, f64::max);
    if vols.len() < 10 {
        return max;
    }
    let mut v: Vec<f64> = vols.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95 = v[((v.len() as f64) * 0.95) as usize - 1];
    p95.max(max / 4.0)
}

// ── glyphs ──────────────────────────────────────────────────────────────────

/// Glyph for row `r` of a 1-column candle. `wick`/`body` are inclusive
/// half-row ranges.
pub fn narrow_glyph(
    r: usize,
    wick: (usize, usize),
    body: (usize, usize),
    hollow: bool,
) -> Option<&'static str> {
    let (top, bot) = (2 * r, 2 * r + 1);
    let in_body = |u: usize| u >= body.0 && u <= body.1;
    let in_wick = |u: usize| u >= wick.0 && u <= wick.1;
    match (in_body(top), in_body(bot), hollow) {
        (true, true, false) => Some("█"),
        (true, false, false) => Some("▀"),
        (false, true, false) => Some("▄"),
        (true, true, true) => Some("┃"),
        (true, false, true) => Some("╹"),
        (false, true, true) => Some("╻"),
        _ => match (in_wick(top), in_wick(bot)) {
            (true, true) => Some("│"),
            (true, false) => Some("╵"),
            (false, true) => Some("╷"),
            _ => None,
        },
    }
}

/// Row `r` of a wide candle (`bw` ≥ 3 columns), as one glyph per column.
/// `None` entries leave the cell untouched.
pub fn wide_row(
    r: usize,
    bw: usize,
    wick: (usize, usize),
    body: (usize, usize),
    hollow: bool,
) -> Vec<Option<&'static str>> {
    let mid = bw / 2;
    let (wt, wb) = (wick.0 / 2, wick.1 / 2);
    let (bt, bb) = (body.0 / 2, body.1 / 2);
    let mut row = vec![None; bw];
    if r < wt.min(bt) || r > wb.max(bb) {
        return row;
    }
    if r >= bt && r <= bb {
        if !hollow {
            // Solid: half-row precision on the end rows.
            let g = if bt == bb {
                "█"
            } else if r == bt && body.0 % 2 == 1 {
                "▄"
            } else if r == bb && body.1.is_multiple_of(2) {
                "▀"
            } else {
                "█"
            };
            return vec![Some(g); bw];
        }
        let wick_above = wt < bt;
        let wick_below = wb > bb;
        if bt == bb {
            for (i, cell) in row.iter_mut().enumerate() {
                *cell = Some(if i == 0 {
                    "├"
                } else if i == bw - 1 {
                    "┤"
                } else {
                    "─"
                });
            }
            return row;
        }
        for (i, cell) in row.iter_mut().enumerate() {
            let edge_l = i == 0;
            let edge_r = i == bw - 1;
            *cell = if r == bt {
                Some(if edge_l {
                    "┌"
                } else if edge_r {
                    "┐"
                } else if i == mid && wick_above {
                    "┴"
                } else {
                    "─"
                })
            } else if r == bb {
                Some(if edge_l {
                    "└"
                } else if edge_r {
                    "┘"
                } else if i == mid && wick_below {
                    "┬"
                } else {
                    "─"
                })
            } else if edge_l || edge_r {
                Some("│")
            } else {
                Some(" ")
            };
        }
        return row;
    }
    // Wick only.
    let top_half = 2 * r >= wick.0 && 2 * r <= wick.1;
    let bot_half = 2 * r + 1 >= wick.0 && 2 * r < wick.1;
    row[mid] = match (top_half, bot_half) {
        (true, true) => Some("│"),
        (true, false) => Some("╵"),
        (false, true) => Some("╷"),
        _ => None,
    };
    row
}

fn vol_glyph(eighths: usize) -> &'static str {
    const G: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    G[eighths.min(8)]
}

// ── widget ──────────────────────────────────────────────────────────────────

pub struct Palette {
    pub up: Color,
    pub down: Color,
    pub dim: Color,
    pub grid: Color,
    pub text: Color,
}

pub struct ChartWidget<'a> {
    pub candles: &'a [Candle],
    pub view: &'a ChartView,
    pub style: Style2,
    /// The smallest price-line step (a cent; a tenth of a pip for pairs).
    pub price_tick: f64,
    /// Dim reference line (the 1D prior close).
    pub reference: Option<f64>,
    /// Price for the right-axis tag (live last), else the newest close.
    pub last_price: Option<f64>,
    /// Candles lag the live price (free plan): draw a faint marker from the
    /// newest candle to the live price so the gap reads as "moved since".
    pub delayed: bool,
    /// Lines drawn over the candles (behind them), included in the scale.
    /// May be longer than `candles` by `future` projected slots.
    pub overlays: &'a [Overlay],
    /// Shaded band between two lines (the Ichimoku cloud), same indexing.
    pub cloud: Option<&'a Cloud>,
    /// Empty slots reserved right of the newest candle (Ichimoku projection)
    /// while the window is pinned to the latest candle.
    pub future: usize,
    /// Draw support/resistance levels found in the visible window.
    pub levels: bool,
    pub fmt_price: fn(f64) -> String,
    pub fmt_volume: fn(f64) -> String,
    /// Full timestamp label for the readout.
    pub time_label: &'a dyn Fn(DateTime<Utc>) -> String,
    /// Axis label for a candle when its "period" (hour / day / month / year)
    /// differs from the previous candle's; `None` = not a boundary.
    #[allow(clippy::type_complexity)]
    pub boundary: &'a dyn Fn(Option<DateTime<Utc>>, DateTime<Utc>) -> Option<(String, bool)>,
    pub palette: Palette,
}

impl Widget for ChartWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let n = self.candles.len();
        if n == 0 || area.width < 16 || area.height < 6 {
            return;
        }
        let pal = &self.palette;
        let dim = Style::default().fg(pal.dim);
        let zoom = self.view.zoom_level();

        // Right axis width from the widest price label.
        let (all_lo, all_hi) = self
            .candles
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), c| {
                (l.min(c.low), h.max(c.high))
            });
        let axis_w = [(self.fmt_price)(all_hi), (self.fmt_price)(all_lo)]
            .iter()
            .map(|s| s.chars().count())
            .max()
            .unwrap_or(6) as u16
            + 2;
        let plot_w = area.width.saturating_sub(axis_w);
        self.view.plot_cols.set(plot_w as usize);
        // Columns per candle (fractional in fit mode) and body width.
        let (slot_f, bw, visible, fut) = if self.view.fit {
            // Fit everything plus the projected slots across the width.
            let total = (n + self.future).min(plot_w.max(1) as usize).max(1);
            let fut = self.future.min(total.saturating_sub(1));
            let shown = (total - fut).min(n).max(1);
            let avg = f64::from(plot_w) / total as f64;
            let bw = if avg >= 6.0 {
                5
            } else if avg >= 4.0 {
                3
            } else {
                1
            };
            (avg, bw, shown, fut)
        } else {
            let slot = zoom.slot().max(1) as usize;
            let slots = (plot_w as usize / slot).max(1);
            let fut = if self.view.right.is_none() {
                self.future.min(slots.saturating_sub(1))
            } else {
                0
            };
            (slot as f64, zoom.body() as usize, (slots - fut).max(1), fut)
        };
        self.view.visible.set(visible);
        let (start, end) = if self.view.fit {
            (n - visible, n)
        } else {
            self.view.window(n, visible)
        };
        // Projection only when the newest candle is on screen.
        let fut = if end == n { fut } else { 0 };
        let win = &self.candles[start..end];
        let cursor = self.view.cursor_idx(n);

        // Vertical layout: readout, price pane, volume pane, time axis.
        let inner_h = area.height - 2;
        let vol_rows = if inner_h >= 14 {
            (inner_h / 5).max(3)
        } else {
            0
        };
        let price_rows = inner_h - vol_rows;
        let y_read = area.y;
        let y0 = area.y + 1;
        let y_vol = y0 + price_rows;
        let y_axis = area.y + area.height - 1;
        let x0 = area.x;
        let x_axis = area.x + plot_w;
        // Right-align a short window so the newest candle hugs the axis.
        let used = ((win.len() + fut) as f64 * slot_f).min(f64::from(plot_w));
        let xs = f64::from(x0) + (f64::from(plot_w) - used).max(0.0);
        let x_of = |i: usize| -> u16 { (xs + (i as f64 * slot_f).floor()) as u16 };

        // Price bounds over the window (+ reference), padded 4%.
        let (mut lo, mut hi) = win
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), c| {
                (l.min(c.low), h.max(c.high))
            });
        if let Some(r) = self.reference {
            lo = lo.min(r);
            hi = hi.max(r);
        }
        // Keep the live price on scale while the newest candle is visible.
        if let Some(p) = self.last_price.filter(|_| end == n) {
            lo = lo.min(p);
            hi = hi.max(p);
        }
        // Overlays in the visible window stay on scale too.
        let span = start..end + fut;
        for vals in self
            .overlays
            .iter()
            .map(|o| &o.values)
            .chain(self.cloud.iter().flat_map(|c| [&c.a, &c.b]))
        {
            let slice = vals
                .get(span.start..span.end.min(vals.len()))
                .unwrap_or(&[]);
            for v in slice.iter().flatten() {
                lo = lo.min(*v);
                hi = hi.max(*v);
            }
        }
        // The price line is always on scale: stepping it past an edge
        // stretches the axis.
        if let Some(p) = self.view.price {
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let pad = ((hi - lo) * 0.04).max(hi.abs() * 0.0005);
        let (lo, hi) = (lo - pad, hi + pad);
        self.view.price_step.set(nice_floor(
            (hi - lo) / f64::from(price_rows.max(1)),
            self.price_tick,
        ));
        let row_of = |p: f64| (half_row(p, lo, hi, price_rows) / 2) as u16;

        // Gridlines + right-axis labels.
        let step = nice_step(hi - lo, (price_rows as usize / 3).max(2));
        for t in ticks(lo, hi, step) {
            let r = row_of(t);
            for x in x0..x_axis {
                buf.set_string(x, y0 + r, "┈", Style::default().fg(pal.grid));
            }
            buf.set_string(x_axis + 1, y0 + r, (self.fmt_price)(t), dim);
        }

        // Day/period separators + time axis labels. Strong boundaries (days,
        // years) are placed first, then weaker ones wherever they fit.
        let mut labels: Vec<(u16, String, bool)> = Vec::new();
        for (i, c) in win.iter().enumerate() {
            let prev = (start + i > 0).then(|| self.candles[start + i - 1].time);
            if let Some((label, strong)) = (self.boundary)(prev, c.time) {
                let x = x_of(i) + (bw / 2) as u16;
                if strong && i > 0 {
                    for r in 0..price_rows {
                        buf.set_string(x, y0 + r, "┆", Style::default().fg(pal.grid));
                    }
                }
                labels.push((x, label, strong));
            }
        }
        let mut taken: Vec<(u16, u16)> = Vec::new();
        for pass in [true, false] {
            for (x, label, strong) in &labels {
                if *strong != pass {
                    continue;
                }
                let w = label.chars().count() as u16;
                if x + w > x_axis {
                    continue;
                }
                let (a, b) = (x.saturating_sub(2), x + w + 2);
                if taken.iter().any(|&(s, e)| a < e && s < b) {
                    continue;
                }
                buf.set_string(*x, y_axis, label, dim);
                taken.push((*x, x + w));
            }
        }

        // Reference line.
        if let Some(r) = self.reference {
            let row = row_of(r);
            for x in x0..x_axis {
                buf.set_string(x, y0 + row, "╌", Style::default().fg(pal.dim));
            }
        }

        // Support / resistance (from the visible candles): lines behind the
        // candles now; their tags are drawn on top after the candles.
        let mut level_tags: Vec<(u16, String, Color)> = Vec::new();
        if self.levels {
            for lv in swing_levels(win, 3) {
                if lv.price < lo || lv.price > hi {
                    continue;
                }
                let color = match lv.kind {
                    LevelKind::Support => SUPPORT_COLOR,
                    LevelKind::Resistance => RESISTANCE_COLOR,
                };
                let row = y0 + row_of(lv.price);
                for x in x0..x_axis {
                    buf.set_string(x, row, "╌", Style::default().fg(color));
                }
                let tag = format!(
                    " {} {} ×{} ",
                    if lv.kind == LevelKind::Support {
                        "S"
                    } else {
                        "R"
                    },
                    (self.fmt_price)(lv.price),
                    lv.touches
                );
                level_tags.push((row, tag, color));
            }
        }

        // Overlay lines (behind the candles; broken where undefined).
        if !self.overlays.is_empty() {
            let segs: Vec<(f64, f64, f64, f64, Color)> = self
                .overlays
                .iter()
                .flat_map(|o| {
                    let pts: Vec<Option<(f64, f64)>> = (start..end + fut)
                        .map(|i| {
                            o.values.get(i).copied().flatten().map(|v| {
                                let x = f64::from(x_of(i - start) - x0) + bw as f64 / 2.0;
                                (x, v)
                            })
                        })
                        .collect();
                    pts.windows(2)
                        .filter_map(|w| match (w[0], w[1]) {
                            (Some(a), Some(b)) => Some((a.0, a.1, b.0, b.1, o.color)),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            Canvas::default()
                .marker(Marker::Braille)
                .x_bounds([0.0, f64::from(plot_w)])
                .y_bounds([lo, hi])
                .paint(move |ctx| {
                    for &(x1, y1, x2, y2, color) in &segs {
                        ctx.draw(&CanvasLine {
                            x1,
                            y1,
                            x2,
                            y2,
                            color,
                        });
                    }
                })
                .render(Rect::new(x0, y0, plot_w, price_rows), buf);
        }

        // Cloud: a faint background tint between its edges. Painted after the
        // overlay Canvas (which resets the background of its whole area) and
        // before the candles; it only sets backgrounds, so lines show through.
        if let Some(cl) = self.cloud {
            for i in start..end + fut {
                let (Some(a), Some(b)) = (
                    cl.a.get(i).copied().flatten(),
                    cl.b.get(i).copied().flatten(),
                ) else {
                    continue;
                };
                let tint = if a >= b { CLOUD_UP } else { CLOUD_DOWN };
                let (top, bot) = (row_of(a.max(b)), row_of(a.min(b)));
                let xa = x_of(i - start);
                let xb = x_of(i - start + 1).max(xa + 1).min(x_axis);
                for x in xa..xb {
                    for r in top..=bot.min(price_rows - 1) {
                        buf[(x, y0 + r)].set_bg(tint);
                    }
                }
            }
        }

        // Cursor column (behind the candles).
        if cursor >= start && cursor < end {
            let x = x_of(cursor - start) + (bw / 2) as u16;
            for r in 0..(price_rows + vol_rows) {
                buf.set_string(x, y0 + r, "┊", Style::default().fg(pal.dim));
            }
        }

        // Price line (behind the candles), crossing the cursor column.
        let price_row = self.view.price.map(|p| row_of(p).min(price_rows - 1));
        if let Some(r) = price_row {
            let st = Style::default().fg(pal.dim);
            let xc =
                (cursor >= start && cursor < end).then(|| x_of(cursor - start) + (bw / 2) as u16);
            for x in x0..x_axis {
                let g = if Some(x) == xc { "┼" } else { "╌" };
                buf.set_string(x, y0 + r, g, st);
            }
        }

        // Candles or line.
        let color_of = |c: &Candle| if c.up() { pal.up } else { pal.down };
        match self.style {
            Style2::Candles => {
                for (i, c) in win.iter().enumerate() {
                    let x = x_of(i);
                    let st = Style::default().fg(color_of(c));
                    let wick = (
                        half_row(c.high, lo, hi, price_rows),
                        half_row(c.low, lo, hi, price_rows),
                    );
                    let (o, cl) = (
                        half_row(c.open, lo, hi, price_rows),
                        half_row(c.close, lo, hi, price_rows),
                    );
                    let body = (o.min(cl), o.max(cl));
                    let last_row = price_rows as usize - 1;
                    for r in
                        (wick.0 / 2).min(body.0 / 2)..=(wick.1 / 2).max(body.1 / 2).min(last_row)
                    {
                        if bw == 1 {
                            if let Some(g) = narrow_glyph(r, wick, body, c.hollow()) {
                                buf.set_string(x, y0 + r as u16, g, st);
                            }
                        } else {
                            for (j, g) in wide_row(r, bw, wick, body, c.hollow())
                                .into_iter()
                                .enumerate()
                            {
                                if let Some(g) = g {
                                    buf.set_string(x + j as u16, y0 + r as u16, g, st);
                                }
                            }
                        }
                    }
                }
            }
            Style2::Line => {
                let first = win.first().map(|c| c.open).unwrap_or(0.0);
                let last = win.last().map(|c| c.close).unwrap_or(0.0);
                let color = if last >= first { pal.up } else { pal.down };
                let pts: Vec<(f64, f64)> = win
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (f64::from(x_of(i) - x0) + bw as f64 / 2.0, c.close))
                    .collect();
                let canvas = Canvas::default()
                    .marker(Marker::Braille)
                    .x_bounds([0.0, plot_w as f64])
                    .y_bounds([lo, hi])
                    .paint(move |ctx| {
                        for w in pts.windows(2) {
                            ctx.draw(&CanvasLine {
                                x1: w[0].0,
                                y1: w[0].1,
                                x2: w[1].0,
                                y2: w[1].1,
                                color,
                            });
                        }
                    });
                canvas.render(Rect::new(x0, y0, plot_w, price_rows), buf);
            }
        }

        // Level tags on top of the candles, at the right edge of the plot.
        for (row, tag, color) in &level_tags {
            let w = tag.chars().count() as u16;
            if w + 2 < plot_w {
                buf.set_string(
                    x_axis - w - 1,
                    *row,
                    tag,
                    Style::default().fg(*color).bg(GRID_BG),
                );
            }
        }

        // Volume.
        if vol_rows > 0 {
            let cap = volume_cap(&win.iter().map(|c| c.volume).collect::<Vec<_>>());
            if cap > 0.0 {
                for (i, c) in win.iter().enumerate() {
                    let level =
                        ((c.volume / cap).min(1.0) * (vol_rows as f64 * 8.0)).round() as usize;
                    let st = Style::default().fg(color_of(c)).add_modifier(Modifier::DIM);
                    for j in 0..vol_rows {
                        let fill = level.saturating_sub(j as usize * 8).min(8);
                        if fill == 0 {
                            continue;
                        }
                        for k in 0..bw {
                            let x = x_of(i) + k as u16;
                            buf.set_string(x, y_vol + vol_rows - 1 - j, vol_glyph(fill), st);
                        }
                    }
                }
                buf.set_string(x_axis + 1, y_vol, (self.fmt_volume)(cap), dim);
                buf.set_string(x_axis + 1, y_vol + vol_rows - 1, "vol", dim);
            }
        }

        // Last-price tag on the axis.
        let newest = &self.candles[n - 1];
        let last = self.last_price.unwrap_or(newest.close);
        // Delay marker: from just right of the newest candle, a dotted
        // vertical from its close to the live price, then across to the tag.
        if self.delayed && self.last_price.is_some() && end == n && last >= lo && last <= hi {
            let xg = x_of(end - 1 - start) + bw as u16;
            let (rc, rl) = (row_of(newest.close), row_of(last));
            let st = Style::default().fg(pal.dim);
            if xg < x_axis {
                for r in rc.min(rl)..=rc.max(rl) {
                    buf.set_string(xg, y0 + r, "┆", st);
                }
                for x in xg + 1..x_axis {
                    buf.set_string(x, y0 + rl, "·", st);
                }
            }
        }
        if last >= lo && last <= hi {
            let up = newest
                .prev_close
                .map_or(newest.close >= newest.open, |p| last >= p);
            let tag = format!(" {} ", (self.fmt_price)(last));
            buf.set_string(
                x_axis,
                y0 + row_of(last),
                &tag,
                Style::default()
                    .fg(Color::Black)
                    .bg(if up { pal.up } else { pal.down })
                    .add_modifier(Modifier::BOLD),
            );
        }

        // Price-line tag on the axis (over the last-price tag if they meet).
        if let (Some(p), Some(r)) = (self.view.price, price_row) {
            buf.set_string(
                x_axis,
                y0 + r,
                format!(" {} ", (self.fmt_price)(p)),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            );
        }

        // Readout for the cursor candle.
        let c = &self.candles[cursor];
        let col = color_of(c);
        let mut x = area.x;
        let mut put = |s: String, st: Style| {
            buf.set_string(x, y_read, &s, st);
            x += s.chars().count() as u16;
        };
        put(
            (self.time_label)(c.time),
            Style::default().fg(pal.text).add_modifier(Modifier::BOLD),
        );
        if c.bars > 1 {
            put(format!(" ×{}", c.bars), dim);
        }
        for (k, v) in [("O", c.open), ("H", c.high), ("L", c.low), ("C", c.close)] {
            put(format!("  {k} "), dim);
            put((self.fmt_price)(v), Style::default().fg(pal.text));
        }
        if let Some(p) = c.change_pct() {
            put(format!("  {p:+.2}%"), Style::default().fg(col));
        }
        put("  vol ".into(), dim);
        put((self.fmt_volume)(c.volume), Style::default().fg(pal.text));
        for o in self.overlays {
            if let Some(v) = o.values.get(cursor).copied().flatten() {
                put(format!("  {} ", o.label), dim);
                put((self.fmt_price)(v), Style::default().fg(o.color));
            }
        }
        if let Some(p) = self.view.price {
            put("   line ".into(), dim);
            put(
                (self.fmt_price)(p),
                Style::default().fg(pal.text).add_modifier(Modifier::BOLD),
            );
            if last > 0.0 {
                let pct = (p - last) / last * 100.0;
                put(format!(" ({pct:+.2}% from last)"), dim);
            }
        }
        if self.view.cursor.is_some() {
            put("   g e latest".into(), dim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(t: &str, o: f64, h: f64, l: f64, c: f64, v: f64) -> Bar {
        Bar {
            time: t.parse().unwrap(),
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
            vwap: None,
        }
    }

    #[test]
    fn hollow_and_direction_follow_the_desktop_scheme() {
        let bars = vec![
            bar("2026-11-06T14:30:00Z", 10.0, 11.0, 9.0, 10.5, 1.0),
            // Gaps down below prior close but closes above its own open:
            // hollow (close ≥ open) yet red (close < previous close).
            bar("2026-11-06T14:35:00Z", 9.0, 10.2, 8.8, 10.0, 1.0),
            // Closes below its open but above the previous close: solid teal.
            bar("2026-11-06T14:40:00Z", 10.8, 11.0, 10.1, 10.4, 1.0),
        ];
        let c = build(&bars, 1);
        assert!(c[0].up() && c[0].hollow());
        assert!(!c[1].up() && c[1].hollow());
        assert!(c[2].up() && !c[2].hollow());
        assert_eq!(c[1].prev_close, Some(10.5));
    }

    #[test]
    fn build_merges_from_the_newest_end() {
        let bars: Vec<Bar> = (0..5)
            .map(|i| {
                let f = f64::from(i);
                bar("2026-11-06T15:00:00Z", f, f + 1.0, f - 1.0, f + 0.5, 10.0)
            })
            .collect();
        let c = build(&bars, 2);
        assert_eq!(c.iter().map(|c| c.bars).collect::<Vec<_>>(), vec![1, 2, 2]);
        assert_eq!(
            (c[2].open, c[2].close, c[2].high, c[2].low),
            (3.0, 4.5, 5.0, 2.0)
        );
        assert_eq!(c[2].volume, 20.0);
        assert_eq!(c[2].prev_close, Some(c[1].close));
    }

    #[test]
    fn regular_session_filter_and_fallback() {
        // 2026-11-06 is EST (UTC-5): 09:30 ET = 14:30Z, 16:00 ET = 21:00Z.
        assert!(!is_regular_session("2026-11-06T14:25:00Z".parse().unwrap()));
        assert!(is_regular_session("2026-11-06T14:30:00Z".parse().unwrap()));
        assert!(is_regular_session("2026-11-06T20:55:00Z".parse().unwrap()));
        assert!(!is_regular_session("2026-11-06T21:00:00Z".parse().unwrap()));
        let bars = vec![
            bar("2026-11-06T12:00:00Z", 1.0, 1.0, 1.0, 1.0, 1.0),
            bar("2026-11-06T15:00:00Z", 2.0, 2.0, 2.0, 2.0, 1.0),
        ];
        let (b, ext) = session_bars(&bars, HistoryRange::Day1, false);
        assert_eq!((b.len(), ext), (1, false));
        let (b, ext) = session_bars(&bars, HistoryRange::Day1, true);
        assert_eq!((b.len(), ext), (2, true));
        // Pre-market only → falls back to everything.
        let (b, ext) = session_bars(&bars[..1], HistoryRange::Day1, false);
        assert_eq!((b.len(), ext), (1, true));
        // Daily ranges are never filtered.
        assert_eq!(session_bars(&bars, HistoryRange::Year1, false).0.len(), 2);
    }

    #[test]
    fn price_step_is_a_round_number_within_one_row() {
        // 30 of range over 40 rows = 0.75 a row → 0.5.
        assert_eq!(nice_floor(0.75, 0.01), 0.5);
        assert_eq!(nice_floor(2.4, 0.01), 2.0);
        assert_eq!(nice_floor(9.9, 0.01), 5.0);
        assert!((nice_floor(0.00037, 0.00001) - 0.0002).abs() < 1e-12);
        // Never finer than the instrument's tick.
        assert_eq!(nice_floor(0.004, 0.01), 0.01);
        assert_eq!(nice_floor(0.0, 0.01), 0.01);
    }

    #[test]
    fn view_window_cursor_and_pan() {
        let mut v = ChartView {
            fit: false,
            ..ChartView::default()
        };
        v.visible.set(10);
        assert_eq!(v.window(100, 10), (90, 100));
        assert_eq!(v.cursor_idx(100), 99);
        // Step left within the window: window stays.
        v.move_cursor(100, -3);
        assert_eq!((v.cursor, v.window(100, 10)), (Some(96), (90, 100)));
        // Past the left edge: window follows.
        v.move_cursor(100, -10);
        assert_eq!(v.cursor, Some(86));
        assert_eq!(v.window(100, 10), (86, 96));
        // Back to the end re-pins to latest.
        v.move_cursor(100, 50);
        assert_eq!((v.cursor, v.right), (None, None));
        // Pan a page left and right.
        v.pan(100, -10);
        assert_eq!(v.window(100, 10), (80, 90));
        v.pan(100, 10);
        assert_eq!(v.right, None);
        // Start / end.
        v.jump_start(100);
        assert_eq!((v.cursor, v.window(100, 10)), (Some(0), (0, 10)));
        v.jump_end();
        assert_eq!(v.window(100, 10), (90, 100));
        // Fewer candles than fit.
        assert_eq!(v.window(4, 10), (0, 4));
    }

    #[test]
    fn reanchor_keeps_the_cursor_time() {
        let bars: Vec<Bar> = (0..40)
            .map(|i| {
                let t = format!(
                    "2026-11-06T{:02}:{:02}:00Z",
                    14 + (30 + i * 5) / 60,
                    (30 + i * 5) % 60
                );
                bar(&t, 1.0, 2.0, 0.5, 1.5, 1.0)
            })
            .collect();
        let fine = build(&bars, 1);
        let coarse = build(&bars, 4);
        let mut v = ChartView::default();
        v.visible.set(8);
        v.cursor = Some(21);
        v.reanchor(&fine, &coarse);
        let c = v.cursor.unwrap();
        assert!(coarse[c].time <= fine[21].time);
        assert!(c + 1 == coarse.len() || coarse[c + 1].time > fine[21].time);
    }

    #[test]
    fn fit_is_the_default_and_zoom_steps_from_it() {
        let mut v = ChartView::default();
        assert!(v.fit);
        v.plot_cols.set(100);
        // 300 bars into 100 columns → 3 per candle; whole range visible.
        assert_eq!(v.merge_for(300), 3);
        assert_eq!(v.window(100, 10), (0, 100));
        // Zoom in from a 3× fit → 2× merge.
        v.zoom_in(300);
        assert_eq!((v.fit, v.zoom_level()), (false, Zoom::Merge(2)));
        v.zoom_fit();
        // 40 bars in 100 columns (2.5 each) → zoom in lands on 4-column slots.
        v.zoom_in(40);
        assert_eq!(v.zoom_level(), Zoom::Slot(4));
        v.zoom_fit();
        // Zoom out from a 3× fit → 4×; from 8× there's nowhere to go.
        assert!(v.zoom_out(300));
        assert_eq!(v.zoom_level(), Zoom::Merge(4));
        v.zoom_fit();
        assert!(!v.zoom_out(1000));
        assert!(v.fit);
        // Fit rendering spreads candles over the whole width.
        let candles = build(&walk(40), 1);
        let v = ChartView::default();
        let t = text(&render(&candles, &v, 100, 24, Style2::Candles));
        assert_eq!(v.visible.get(), 40);
        assert!(t.lines().nth(5).is_some());
    }

    #[test]
    fn sma_matches_hand_computed_values() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(
            sma(&v, 3),
            vec![None, None, Some(2.0), Some(3.0), Some(4.0)]
        );
        assert_eq!(sma(&v, 1), v.iter().map(|x| Some(*x)).collect::<Vec<_>>());
        assert_eq!(sma(&v, 6), vec![None; 5]);
        assert!(sma(&[], 50).is_empty());
    }

    #[test]
    fn per_candle_samples_the_last_bar_of_each_merged_candle() {
        let bars: Vec<Bar> = (0..6)
            .map(|i| {
                let f = f64::from(i);
                bar("2026-11-06T15:00:00Z", f, f, f, f, 1.0)
            })
            .collect();
        // 2 warm-up values precede the 6 visible bars.
        let series: Vec<Option<f64>> = (0..8).map(|i| Some(f64::from(i) * 10.0)).collect();
        let fine = build(&bars, 1);
        assert_eq!(per_candle(&series, &fine, 2)[0], Some(20.0));
        let merged = build(&bars, 3); // candles end at bars 2 and 5
        assert_eq!(
            per_candle(&series, &merged, 2),
            vec![Some(40.0), Some(70.0)]
        );
        // Missing values stay None.
        assert_eq!(per_candle(&[None; 8], &fine, 2), vec![None; 6]);
    }

    #[test]
    fn overlays_draw_behind_candles_extend_the_scale_and_show_in_the_readout() {
        let candles = build(&walk(40), 1);
        // A line far above the candles: the scale must stretch to include it.
        let high = candles.iter().map(|c| c.high).fold(f64::MIN, f64::max) + 50.0;
        let ov = vec![Overlay {
            label: "MA50",
            color: Color::Blue,
            values: (0..40).map(|i| (i >= 10).then_some(high)).collect(),
        }];
        let v = ChartView::default();
        let area = Rect::new(0, 0, 100, 24);
        let mut buf = Buffer::empty(area);
        let label = |t: DateTime<Utc>| t.format("%H:%M").to_string();
        let boundary = |_p: Option<DateTime<Utc>>, _t: DateTime<Utc>| None;
        ChartWidget {
            candles: &candles,
            view: &v,
            style: Style2::Candles,
            price_tick: 0.01,
            reference: None,
            last_price: None,
            delayed: false,
            overlays: &ov,
            cloud: None,
            future: 0,
            levels: false,
            fmt_price: |p| format!("{p:.2}"),
            fmt_volume: |v| format!("{v:.0}"),
            time_label: &label,
            boundary: &boundary,
            palette: Palette {
                up: Color::Green,
                down: Color::Red,
                dim: Color::DarkGray,
                grid: Color::Gray,
                text: Color::White,
            },
        }
        .render(area, &mut buf);
        let t = text(&buf);
        // Blue braille cells exist (the line), and the axis reaches its level.
        let blue = (0..24u16).any(|y| {
            (0..100u16).any(|x| {
                let c = &buf[(x, y)];
                c.fg == Color::Blue
                    && ('\u{2801}'..='\u{28FF}').contains(&c.symbol().chars().next().unwrap_or(' '))
            })
        });
        assert!(blue, "{t}");
        assert!(
            t.lines()
                .next()
                .unwrap()
                .contains(&format!("MA50 {high:.2}")),
            "{t}"
        );
    }

    #[test]
    fn moving_averages_use_warmup_so_the_200_day_spans_the_window() {
        let mk = |n: usize, base: f64| -> Vec<Bar> {
            (0..n)
                .map(|i| {
                    let c = base + i as f64;
                    bar("2026-01-01T05:00:00Z", c, c, c, c, 1.0)
                })
                .collect()
        };
        let warm = mk(210, 0.0); // closes 0..209
        let vis = mk(5, 210.0); // closes 210..214
        let candles = build(&vis, 1);
        let ov = moving_averages(&warm, &vis, &candles);
        assert_eq!(ov[0].label, "MA50");
        // First visible candle (close 210): MA50 = mean(161..=210) = 185.5.
        assert_eq!(ov[0].values[0], Some(185.5));
        // MA200 = mean(11..=210) = 110.5 — defined from the first candle.
        assert_eq!(ov[1].values[0], Some(110.5));
        // Without warm-up the 200-day isn't defined yet (no fake values).
        let cold = moving_averages(&[], &vis, &candles);
        assert!(cold[1].values.iter().all(Option::is_none));
        assert!(
            ma_range(HistoryRange::Year1)
                && !ma_range(HistoryRange::Day1)
                && !ma_range(HistoryRange::Year5)
        );
    }

    #[test]
    fn intraday_ranges_fetch_enough_warmup_for_the_cloud() {
        use HistoryRange::*;
        // Every range the cloud draws on gets warm-up; long ranges don't need it.
        for r in [Day1, Day5, Month1, Month6, Year1] {
            assert!(warmup_days(r) > 0, "{r:?}");
        }
        assert_eq!(warmup_days(Year5), 0);
        // 1D: bars over three sessions; the window starts mid-way, trim keeps
        // the last session; everything before it is warm-up.
        let bars: Vec<Bar> = [
            "2026-11-04T15:00:00Z",
            "2026-11-05T15:00:00Z",
            "2026-11-05T16:00:00Z",
            "2026-11-06T15:00:00Z",
            "2026-11-06T16:00:00Z",
        ]
        .iter()
        .map(|t| bar(t, 1.0, 1.0, 1.0, 1.0, 1.0))
        .collect();
        let start = "2026-11-05T00:00:00Z".parse().unwrap();
        let (warm, vis) = split_warmup(Day1, start, bars.clone());
        assert_eq!((warm.len(), vis.len()), (3, 2));
        assert!(warm.iter().all(|w| w.time < vis[0].time));
        // Month1 doesn't trim: the window is everything from `start`.
        let (warm, vis) = split_warmup(Month1, start, bars);
        assert_eq!((warm.len(), vis.len()), (1, 4));
    }

    fn ichi_bars(n: usize) -> Vec<Bar> {
        // Exact quarter values so Decimal and f64 agree bit for bit.
        (0..n)
            .map(|i| {
                let base = 100.0 + ((i * 7) % 23) as f64 * 0.25 - ((i * 3) % 11) as f64 * 0.5;
                bar(
                    "2026-01-01T05:00:00Z",
                    base,
                    base + 1.25 + (i % 5) as f64 * 0.25,
                    base - 1.0 - (i % 3) as f64 * 0.25,
                    base + 0.5,
                    1.0,
                )
            })
            .collect()
    }

    #[test]
    fn ichimoku_matches_the_existing_wickd_core_indicator() {
        use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
        use wickd_core::backtest::indicators::{IchimokuIndicator, Indicator};
        use wickd_core::models::{Candle as FxCandle, Ohlc};
        let bars = ichi_bars(120);
        let ours = ichimoku_series(&bars);
        let mut theirs = IchimokuIndicator::new(9, 26, 52, 26);
        let d = |x: f64| rust_decimal::Decimal::from_f64(x).unwrap();
        for (i, b) in bars.iter().enumerate() {
            let out = theirs.on_candle(&FxCandle {
                time: b.time,
                mid: Ohlc {
                    open: d(b.open),
                    high: d(b.high),
                    low: d(b.low),
                    close: d(b.close),
                },
                volume: 1,
                complete: true,
            });
            let get = |k: &str| out.get(k).and_then(|v| v.to_f64());
            assert_eq!(ours.tenkan[i], get("tenkan"), "tenkan @{i}");
            assert_eq!(ours.kijun[i], get("kijun"), "kijun @{i}");
            // Raw (undisplaced) senkou at bar i is ours at i + 26.
            assert_eq!(
                ours.span_a[i + ICHI_SHIFT],
                get("senkou_a_raw"),
                "span A @{i}"
            );
            assert_eq!(
                ours.span_b[i + ICHI_SHIFT],
                get("senkou_b_raw"),
                "span B @{i}"
            );
            // Chikou: the close at bar i, plotted 26 bars back.
            if i >= ICHI_SHIFT {
                assert_eq!(
                    ours.chikou[i - ICHI_SHIFT],
                    get("chikou_raw"),
                    "chikou @{i}"
                );
            }
        }
        // The future tail only has the displaced spans.
        let last = bars.len() + ICHI_SHIFT - 1;
        assert!(
            ours.span_a[last].is_some()
                && ours.tenkan[last].is_none()
                && ours.chikou[last].is_none()
        );
    }

    #[test]
    fn ichimoku_overlays_project_26_bars_in_candle_units() {
        let warm = ichi_bars(80);
        let vis = ichi_bars(60);
        let fine = build(&vis, 1);
        let (ov, cloud, fut) = ichimoku_overlays(&warm, &vis, &fine);
        assert_eq!(fut, 26);
        assert_eq!(ov.len(), 3);
        assert_eq!(cloud.a.len(), 60 + 26);
        // With warm-up, the cloud and kijun exist from the first candle.
        assert!(cloud.a[0].is_some() && cloud.b[0].is_some() && ov[1].values[0].is_some());
        // Merged ×3: 26 bars ahead = 9 candle slots.
        let merged = build(&vis, 3);
        let (_, cloud3, fut3) = ichimoku_overlays(&warm, &vis, &merged);
        assert_eq!(fut3, 9);
        assert_eq!(cloud3.a.len(), merged.len() + 9);
    }

    #[test]
    fn cloud_is_tinted_and_the_projection_leaves_room_right_of_the_last_candle() {
        let warm = ichi_bars(80);
        let vis = ichi_bars(60);
        let candles = build(&vis, 1);
        let (ov, cloud, fut) = ichimoku_overlays(&warm, &vis, &candles);
        let v = ChartView::default(); // fit
        let area = Rect::new(0, 0, 120, 26);
        let mut buf = Buffer::empty(area);
        let label = |t: DateTime<Utc>| t.format("%H:%M").to_string();
        let boundary = |_p: Option<DateTime<Utc>>, _t: DateTime<Utc>| None;
        ChartWidget {
            candles: &candles,
            view: &v,
            style: Style2::Candles,
            price_tick: 0.01,
            reference: None,
            last_price: None,
            delayed: false,
            overlays: &ov,
            cloud: Some(&cloud),
            future: fut,
            levels: false,
            fmt_price: |p| format!("{p:.2}"),
            fmt_volume: |v| format!("{v:.0}"),
            time_label: &label,
            boundary: &boundary,
            palette: Palette {
                up: Color::Green,
                down: Color::Red,
                dim: Color::DarkGray,
                grid: Color::Gray,
                text: Color::White,
            },
        }
        .render(area, &mut buf);
        let tinted = (0..26u16)
            .flat_map(|y| (0..120u16).map(move |x| (x, y)))
            .filter(|&(x, y)| matches!(buf[(x, y)].bg, CLOUD_UP | CLOUD_DOWN))
            .count();
        assert!(tinted > 50, "cloud tint cells: {tinted}");
        // The rightmost candle body sits well left of the axis (room for 26
        // projected slots); the projection still has tint.
        let body_cols: Vec<u16> = (0..120u16)
            .filter(|&x| {
                (1..20u16)
                    .any(|y| matches!(buf[(x, y)].symbol(), "█" | "┃" | "▀" | "▄" | "╹" | "╻"))
            })
            .collect();
        let last_body = *body_cols.iter().max().unwrap();
        assert!(last_body < 85, "last candle at col {last_body}");
        let tint_right = (last_body + 3..105u16)
            .any(|x| (1..20u16).any(|y| matches!(buf[(x, y)].bg, CLOUD_UP | CLOUD_DOWN)));
        assert!(tint_right, "projected cloud drawn right of the last candle");
    }

    #[test]
    fn swing_levels_find_a_double_top_and_double_bottom() {
        // Oscillates between ~100 and ~110 twice, ending mid-range at 105.
        let path = [
            103.0, 105.0, 107.0, 109.0, 110.0, 108.0, 106.0, 104.0, 102.0, 100.0, 102.0, 104.0,
            106.0, 108.0, 110.2, 108.0, 106.0, 104.0, 102.0, 100.2, 102.0, 104.0, 105.0, 105.0,
        ];
        let bars: Vec<Bar> = path
            .iter()
            .map(|&c| bar("2026-01-01T05:00:00Z", c, c + 0.3, c - 0.3, c, 1.0))
            .collect();
        let lv = swing_levels(&build(&bars, 1), 3);
        let r = lv
            .iter()
            .find(|l| l.kind == LevelKind::Resistance)
            .expect("resistance");
        let s = lv
            .iter()
            .find(|l| l.kind == LevelKind::Support)
            .expect("support");
        assert!((r.price - 110.4).abs() < 0.3 && r.touches == 2, "{r:?}");
        assert!((s.price - 99.8).abs() < 0.3 && s.touches == 2, "{s:?}");
        // At most 3 per side; tiny inputs give nothing.
        assert!(lv.iter().filter(|l| l.kind == LevelKind::Support).count() <= 3);
        assert!(swing_levels(&build(&bars[..5], 1), 3).is_empty());
    }

    #[test]
    fn levels_draw_as_lines_with_tags() {
        let candles = build(&walk(120), 1);
        let v = ChartView::default();
        let area = Rect::new(0, 0, 120, 26);
        let mut buf = Buffer::empty(area);
        let label = |t: DateTime<Utc>| t.format("%H:%M").to_string();
        let boundary = |_p: Option<DateTime<Utc>>, _t: DateTime<Utc>| None;
        ChartWidget {
            candles: &candles,
            view: &v,
            style: Style2::Candles,
            price_tick: 0.01,
            reference: None,
            last_price: None,
            delayed: false,
            overlays: &[],
            cloud: None,
            future: 0,
            levels: true,
            fmt_price: |p| format!("{p:.2}"),
            fmt_volume: |v| format!("{v:.0}"),
            time_label: &label,
            boundary: &boundary,
            palette: Palette {
                up: Color::Green,
                down: Color::Red,
                dim: Color::DarkGray,
                grid: Color::Gray,
                text: Color::White,
            },
        }
        .render(area, &mut buf);
        let t = text(&buf);
        assert!(t.contains(" R ") || t.contains(" S "), "{t}");
        let colored = (0..26u16).any(|y| {
            (0..120u16).any(|x| matches!(buf[(x, y)].fg, SUPPORT_COLOR | RESISTANCE_COLOR))
        });
        assert!(colored, "{t}");
    }

    #[test]
    fn ticks_and_volume_cap() {
        assert_eq!(nice_step(10.0, 5), 2.0);
        assert_eq!(nice_step(0.9, 4), 0.25);
        assert_eq!(ticks(190.2, 200.6, 2.5), vec![192.5, 195.0, 197.5, 200.0]);
        let mut vols = vec![100.0; 40];
        vols.push(10_000.0); // closing auction
        let cap = volume_cap(&vols);
        assert!((2_500.0..10_000.0).contains(&cap));
        assert_eq!(volume_cap(&[1.0, 5.0]), 5.0);
    }

    #[test]
    fn glyphs_for_narrow_and_wide_candles() {
        let (w, b) = ((0, 7), (2, 4));
        assert_eq!(narrow_glyph(1, w, b, false), Some("█"));
        assert_eq!(narrow_glyph(1, w, b, true), Some("┃"));
        assert_eq!(narrow_glyph(2, w, b, true), Some("╹"));
        assert_eq!(narrow_glyph(0, w, b, true), Some("│"));
        // Wide hollow: box outline with the wick entering top and bottom.
        let rows: Vec<String> = (0..4)
            .map(|r| {
                wide_row(r, 3, (0, 7), (2, 5), true)
                    .into_iter()
                    .map(|g| g.unwrap_or("·"))
                    .collect()
            })
            .collect();
        assert_eq!(rows, vec!["·│·", "┌┴┐", "└┬┘", "·│·"]);
        // Wide solid fills the body.
        assert_eq!(wide_row(1, 3, (0, 7), (2, 5), false), vec![Some("█"); 3]);
        // Single-row hollow body.
        assert_eq!(
            wide_row(1, 3, (0, 7), (2, 3), true),
            vec![Some("├"), Some("─"), Some("┤")]
        );
    }

    fn render(candles: &[Candle], view: &ChartView, w: u16, h: u16, style: Style2) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let label = |t: DateTime<Utc>| t.format("%H:%M").to_string();
        let boundary = |_p: Option<DateTime<Utc>>, _t: DateTime<Utc>| None;
        ChartWidget {
            candles,
            view,
            style,
            price_tick: 0.01,
            reference: Some(candles[0].open),
            last_price: None,
            delayed: false,
            overlays: &[],
            cloud: None,
            future: 0,
            levels: false,
            fmt_price: |p| format!("{p:.2}"),
            fmt_volume: |v| format!("{v:.0}"),
            time_label: &label,
            boundary: &boundary,
            palette: Palette {
                up: Color::Green,
                down: Color::Red,
                dim: Color::DarkGray,
                grid: Color::Gray,
                text: Color::White,
            },
        }
        .render(area, &mut buf);
        buf
    }

    fn text(buf: &Buffer) -> String {
        let a = buf.area;
        (0..a.height)
            .map(|y| {
                (0..a.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn walk(n: usize) -> Vec<Bar> {
        let mut p = 100.0;
        (0..n)
            .map(|i| {
                let f = i as f64;
                let o = p;
                let c = o + (f * 0.7).sin() * 0.6;
                p = c;
                bar(
                    "2026-11-06T15:00:00Z",
                    o,
                    o.max(c) + 0.3,
                    o.min(c) - 0.3,
                    c,
                    100.0 + f,
                )
            })
            .collect()
    }

    #[test]
    fn renders_every_zoom_and_both_styles_without_panicking() {
        let candles = build(&walk(300), 1);
        for (z, level) in ZOOMS.iter().enumerate() {
            let v = ChartView {
                fit: false,
                zoom: z,
                ..ChartView::default()
            };
            let c = build(&walk(300), level.merge());
            let t = text(&render(&c, &v, 100, 24, Style2::Candles));
            assert!(t.contains("vol") && t.contains(" O "), "zoom {z}:\n{t}");
        }
        let v = ChartView::default();
        let t = text(&render(&candles, &v, 100, 24, Style2::Line));
        assert!(
            t.chars().any(|c| ('\u{2801}'..='\u{28FF}').contains(&c)),
            "{t}"
        );
        // Wide zoom draws box outlines for hollow candles.
        let mut v = ChartView {
            fit: false,
            ..ChartView::default()
        };
        v.zoom = ZOOMS.len() - 1;
        let t = text(&render(&candles, &v, 100, 24, Style2::Candles));
        assert!(t.contains('┌') || t.contains('├'), "{t}");
        // The renderer reports how many candles fit.
        assert_eq!(v.visible.get(), (100 - 8) / 6);
        // Tiny areas are a no-op.
        render(&candles, &ChartView::default(), 10, 4, Style2::Candles);
    }

    #[test]
    fn delay_marker_joins_the_newest_candle_to_the_live_price() {
        let candles = build(&walk(30), 1);
        let v = ChartView {
            fit: false,
            zoom: 4, // 2-column slots leave a gap column after each candle
            ..ChartView::default()
        };
        let area = Rect::new(0, 0, 100, 24);
        let label = |t: DateTime<Utc>| t.format("%H:%M").to_string();
        let boundary = |_p: Option<DateTime<Utc>>, _t: DateTime<Utc>| None;
        let draw = |delayed: bool| {
            let mut buf = Buffer::empty(area);
            ChartWidget {
                candles: &candles,
                view: &v,
                style: Style2::Candles,
                price_tick: 0.01,
                reference: None,
                last_price: Some(candles[29].close + 2.0),
                delayed,
                overlays: &[],
                cloud: None,
                future: 0,
                levels: false,
                fmt_price: |p| format!("{p:.2}"),
                fmt_volume: |v| format!("{v:.0}"),
                time_label: &label,
                boundary: &boundary,
                palette: Palette {
                    up: Color::Green,
                    down: Color::Red,
                    dim: Color::DarkGray,
                    grid: Color::Gray,
                    text: Color::White,
                },
            }
            .render(area, &mut buf);
            text(&buf)
        };
        assert!(draw(true).contains('┆'), "{}", draw(true));
        assert!(!draw(false).contains('┆'));
    }

    #[test]
    fn readout_tracks_the_cursor() {
        let candles = build(&walk(50), 1);
        let mut v = ChartView::default();
        v.visible.set(40);
        v.move_cursor(candles.len(), -5);
        let t = text(&render(&candles, &v, 100, 24, Style2::Candles));
        let first = t.lines().next().unwrap();
        assert!(
            first.contains(&format!("{:.2}", candles[44].close)),
            "{first}"
        );
        assert!(first.contains("g e latest"), "{first}");
    }
}
