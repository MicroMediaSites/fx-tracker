//! Candlestick + volume chart for `wickd tui`, drawn straight into the
//! ratatui buffer (ratatui has no candlestick widget).
//!
//! - One terminal column per candle (two or three when there's room, so few
//!   candles don't look like a barcode). When there are more bars than
//!   columns, consecutive bars are merged — grouped from the *newest* end so
//!   the latest candle is always complete — keeping each group's real
//!   open/high/low/close and summed volume.
//! - Vertical precision is half a row: bodies end in `▀`/`▄` when a price
//!   falls mid-cell; wicks are `│` (`╵`/`╷` for half-cell ends).
//! - A volume strip (3 rows of ⅛-block bars, 24 levels) sits under the
//!   price pane when the area is tall enough.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Widget;

use wickd_core::equities::Bar;

/// Rows given to the volume strip, when the area is tall enough for it.
pub const VOLUME_ROWS: u16 = 3;
/// Minimum total height before the volume strip is shown.
const VOLUME_MIN_HEIGHT: u16 = 14;

/// One drawn candle (possibly several merged bars).
#[derive(Debug, Clone, PartialEq)]
pub struct Candle {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// Index of the first source bar in this candle.
    pub first: usize,
    /// Index of the last source bar in this candle.
    pub last: usize,
}

impl Candle {
    pub fn up(&self) -> bool {
        self.close >= self.open
    }
}

/// Bars per candle needed to fit `n` bars into `cols` columns.
pub fn group_size(n: usize, cols: usize) -> usize {
    if cols == 0 {
        return n.max(1);
    }
    n.div_ceil(cols).max(1)
}

/// Merge consecutive bars `k` at a time, grouping from the newest end so the
/// last candle is complete and any short group is the oldest one.
pub fn aggregate(bars: &[Bar], k: usize) -> Vec<Candle> {
    let k = k.max(1);
    let n = bars.len();
    if n == 0 {
        return Vec::new();
    }
    let first_len = match n % k {
        0 => k,
        r => r,
    };
    let mut out = Vec::with_capacity(n.div_ceil(k));
    let mut start = 0;
    let mut len = first_len;
    while start < n {
        let g = &bars[start..start + len];
        out.push(Candle {
            open: g[0].open,
            close: g[g.len() - 1].close,
            high: g.iter().map(|b| b.high).fold(f64::NEG_INFINITY, f64::max),
            low: g.iter().map(|b| b.low).fold(f64::INFINITY, f64::min),
            volume: g.iter().map(|b| b.volume).sum(),
            first: start,
            last: start + len - 1,
        });
        start += len;
        len = k;
    }
    out
}

/// Map a price to a half-row index (0 = top) within `rows` rows.
pub fn half_row(p: f64, lo: f64, hi: f64, rows: u16) -> usize {
    let units = (rows as usize * 2).max(1);
    if hi - lo <= f64::EPSILON {
        return units / 2;
    }
    let frac = ((hi - p) / (hi - lo)).clamp(0.0, 1.0);
    ((frac * (units - 1) as f64).round() as usize).min(units - 1)
}

/// Glyph for row `r` of a candle whose wick and body span the given
/// half-row ranges (inclusive, top ≤ bottom).
pub fn glyph(r: usize, wick: (usize, usize), body: (usize, usize)) -> Option<&'static str> {
    let (top, bot) = (2 * r, 2 * r + 1);
    let in_body = |u: usize| u >= body.0 && u <= body.1;
    let in_wick = |u: usize| u >= wick.0 && u <= wick.1;
    match (in_body(top), in_body(bot)) {
        (true, true) => Some("█"),
        (true, false) => Some("▀"),
        (false, true) => Some("▄"),
        _ => match (in_wick(top), in_wick(bot)) {
            (true, true) => Some("│"),
            (true, false) => Some("╵"),
            (false, true) => Some("╷"),
            _ => None,
        },
    }
}

/// ⅛-block glyph for a volume cell filled `eighths` of 8.
fn vol_glyph(eighths: usize) -> &'static str {
    const G: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    G[eighths.min(8)]
}

pub struct CandleChart<'a> {
    pub bars: &'a [Bar],
    /// Drawn as a dim reference line when set (the 1D prior close).
    pub reference: Option<f64>,
    /// Price label formatter.
    pub fmt_price: fn(f64) -> String,
    /// Time label for a source-bar index.
    pub time_label: &'a dyn Fn(usize) -> String,
    pub up: Color,
    pub down: Color,
    pub dim: Color,
}

/// Price bounds over all bars (+ the reference line), padded 3%.
fn bounds(bars: &[Bar], reference: Option<f64>) -> (f64, f64) {
    let mut lo = bars.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    let mut hi = bars
        .iter()
        .map(|b| b.high)
        .fold(f64::NEG_INFINITY, f64::max);
    if let Some(r) = reference {
        lo = lo.min(r);
        hi = hi.max(r);
    }
    let pad = ((hi - lo) * 0.03).max(hi.abs() * 0.0005);
    (lo - pad, hi + pad)
}

/// Width of the y-axis label gutter.
fn gutter(fmt_price: fn(f64) -> String, lo: f64, hi: f64) -> u16 {
    let w = [fmt_price(hi), fmt_price(lo)]
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(4);
    w as u16 + 1
}

/// Bars merged per candle when drawing `bars` into `width` columns — the
/// same math the widget uses, so a title can say "× k" truthfully.
pub fn bars_per_candle(
    bars: &[Bar],
    reference: Option<f64>,
    width: u16,
    fmt_price: fn(f64) -> String,
) -> usize {
    if bars.is_empty() {
        return 1;
    }
    let (lo, hi) = bounds(bars, reference);
    let cols = width.saturating_sub(gutter(fmt_price, lo, hi)) as usize;
    group_size(bars.len(), cols)
}

impl Widget for CandleChart<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.bars.is_empty() || area.width < 8 || area.height < 4 {
            return;
        }
        let (lo, hi) = bounds(self.bars, self.reference);
        let gutter = gutter(self.fmt_price, lo, hi);
        let cols = area.width.saturating_sub(gutter) as usize;
        if cols == 0 {
            return;
        }
        let k = group_size(self.bars.len(), cols);
        let candles = aggregate(self.bars, k);
        let step = (cols / candles.len().max(1)).clamp(1, 3);

        let vol_rows = if area.height >= VOLUME_MIN_HEIGHT {
            VOLUME_ROWS
        } else {
            0
        };
        let price_rows = area.height - 1 - vol_rows; // last row: time labels
        let x0 = area.x + gutter;
        // Right-align so the newest candle sits at the right edge.
        let xs = x0 + cols.saturating_sub(candles.len() * step) as u16;
        let y0 = area.y;
        let dim = Style::default().fg(self.dim);

        // Y labels: top, middle, bottom of the price pane.
        for (row, price) in [
            (0, hi),
            (price_rows / 2, (hi + lo) / 2.0),
            (price_rows - 1, lo),
        ] {
            let s = (self.fmt_price)(price);
            let x = area.x + gutter.saturating_sub(1 + s.chars().count() as u16);
            buf.set_string(x, y0 + row, s, dim);
        }

        // Reference line first, so candles draw over it.
        if let Some(r) = self.reference {
            let row = (half_row(r, lo, hi, price_rows) / 2) as u16;
            for x in 0..cols as u16 {
                buf.set_string(x0 + x, y0 + row, "┈", dim);
            }
        }

        let vmax = candles.iter().map(|c| c.volume).fold(0.0, f64::max);
        for (i, c) in candles.iter().enumerate() {
            let x = xs + (i * step) as u16;
            if x >= area.x + area.width {
                break;
            }
            let color = if c.up() { self.up } else { self.down };
            let style = Style::default().fg(color);
            let wick = (
                half_row(c.high, lo, hi, price_rows),
                half_row(c.low, lo, hi, price_rows),
            );
            let (bo, bc) = (
                half_row(c.open, lo, hi, price_rows),
                half_row(c.close, lo, hi, price_rows),
            );
            let body = (bo.min(bc), bo.max(bc));
            for r in (wick.0 / 2)..=(wick.1 / 2).min(price_rows as usize - 1) {
                if let Some(g) = glyph(r, wick, body) {
                    buf.set_string(x, y0 + r as u16, g, style);
                }
            }
            // Volume strip.
            if vol_rows > 0 && vmax > 0.0 {
                let level = ((c.volume / vmax) * (vol_rows as f64 * 8.0)).round() as usize;
                for j in 0..vol_rows {
                    let fill = level.saturating_sub(j as usize * 8).min(8);
                    if fill > 0 {
                        let y = y0 + price_rows + vol_rows - 1 - j;
                        buf.set_string(x, y, vol_glyph(fill), style.add_modifier(Modifier::DIM));
                    }
                }
            }
        }
        if vol_rows > 0 {
            buf.set_string(area.x, y0 + price_rows + vol_rows - 1, "vol", dim);
        }

        // Time labels: first, middle and last candle.
        let label_y = y0 + area.height - 1;
        let n = candles.len();
        let picks = [(0usize, 0u8), (n / 2, 1), (n - 1, 2)];
        let mut taken_until = xs;
        for (i, align) in picks {
            let c = &candles[i];
            let s = (self.time_label)(if align == 2 { c.last } else { c.first });
            let w = s.chars().count() as u16;
            let cx = xs + (i * step) as u16;
            let x = match align {
                0 => xs,
                1 => cx.saturating_sub(w / 2),
                _ => cx.saturating_sub(w.saturating_sub(1)),
            }
            .max(taken_until)
            .min((area.x + area.width).saturating_sub(w));
            if x < taken_until || (align != 0 && x == xs && taken_until > xs) {
                continue; // would overlap the previous label
            }
            buf.set_string(x, label_y, &s, dim);
            taken_until = x + w + 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(o: f64, h: f64, l: f64, c: f64, v: f64) -> Bar {
        Bar {
            time: "2026-11-06T15:00:00Z".parse().unwrap(),
            open: o,
            high: h,
            low: l,
            close: c,
            volume: v,
            vwap: None,
        }
    }

    #[test]
    fn aggregate_groups_from_the_newest_end() {
        let bars: Vec<Bar> = (0..5)
            .map(|i| {
                let f = f64::from(i);
                bar(f, f + 1.0, f - 1.0, f + 0.5, 10.0)
            })
            .collect();
        let c = aggregate(&bars, 2);
        assert_eq!(c.len(), 3);
        // Oldest group is the short one; newest is complete.
        assert_eq!((c[0].first, c[0].last), (0, 0));
        assert_eq!((c[2].first, c[2].last), (3, 4));
        assert_eq!(c[2].open, 3.0);
        assert_eq!(c[2].close, 4.5);
        assert_eq!(c[2].high, 5.0);
        assert_eq!(c[2].low, 2.0);
        assert_eq!(c[2].volume, 20.0);
        // k = 1 is identity.
        assert_eq!(aggregate(&bars, 1).len(), 5);
        assert!(aggregate(&[], 3).is_empty());
    }

    #[test]
    fn group_size_fits_columns() {
        assert_eq!(group_size(78, 110), 1);
        assert_eq!(group_size(260, 110), 3);
        assert_eq!(group_size(220, 110), 2);
        assert_eq!(group_size(5, 0), 5);
    }

    #[test]
    fn half_rows_and_glyphs() {
        // 4 rows → 8 half-units over [0, 7].
        assert_eq!(half_row(7.0, 0.0, 7.0, 4), 0);
        assert_eq!(half_row(0.0, 0.0, 7.0, 4), 7);
        assert_eq!(half_row(3.5, 0.0, 7.0, 4), 4); // rounds
        assert_eq!(half_row(99.0, 0.0, 7.0, 4), 0); // clamped
        assert_eq!(half_row(5.0, 5.0, 5.0, 4), 4); // flat range → middle

        // Wick 0..=7, body 2..=4: rows 0 wick, 1 full body, 2 top-half body, 3 wick.
        let (w, b) = ((0, 7), (2, 4));
        assert_eq!(glyph(0, w, b), Some("│"));
        assert_eq!(glyph(1, w, b), Some("█"));
        assert_eq!(glyph(2, w, b), Some("▀"));
        assert_eq!(glyph(3, w, b), Some("│"));
        // Half-cell wick ends.
        assert_eq!(glyph(0, (1, 3), (3, 3)), Some("╷"));
        assert_eq!(glyph(1, (1, 3), (3, 3)), Some("▄"));
        assert_eq!(glyph(2, (1, 3), (3, 3)), None);
    }

    fn render(bars: &[Bar], w: u16, h: u16, reference: Option<f64>) -> Buffer {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let label = |i: usize| format!("t{i}");
        CandleChart {
            bars,
            reference,
            fmt_price: |p| format!("{p:.2}"),
            time_label: &label,
            up: Color::Green,
            down: Color::Red,
            dim: Color::DarkGray,
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

    #[test]
    fn draws_up_and_down_candles_with_colors_and_volume() {
        let bars = vec![
            bar(10.0, 12.0, 9.0, 11.5, 100.0), // up
            bar(11.5, 11.8, 9.5, 10.0, 300.0), // down
        ];
        let buf = render(&bars, 40, 16, None);
        let t = text(&buf);
        assert!(t.contains('█') && t.contains('│'), "{t}");
        assert!(
            t.contains("vol") && t.contains("t0") && t.contains("t1"),
            "{t}"
        );
        // Colors: find a body cell of each candle.
        let colored = |c: Color| {
            (0..buf.area.height).any(|y| {
                (0..buf.area.width).any(|x| buf[(x, y)].symbol() == "█" && buf[(x, y)].fg == c)
            })
        };
        assert!(colored(Color::Green) && colored(Color::Red), "{t}");
    }

    #[test]
    fn many_bars_are_merged_to_fit_and_short_areas_drop_volume() {
        let bars: Vec<Bar> = (0..500)
            .map(|i| {
                let f = 100.0 + (f64::from(i) * 0.1).sin() * 5.0;
                bar(f, f + 1.0, f - 1.0, f + 0.2, 1000.0)
            })
            .collect();
        let buf = render(&bars, 60, 10, Some(100.0));
        let t = text(&buf);
        assert!(
            !t.contains("vol"),
            "no volume strip under the min height:\n{t}"
        );
        assert!(t.contains('┈'), "reference line drawn:\n{t}");
        // Last label names the newest source bar.
        assert!(t.contains("t499"), "{t}");
    }

    #[test]
    fn tiny_areas_and_empty_bars_do_not_panic() {
        render(&[], 40, 16, None);
        let one = vec![bar(1.0, 1.0, 1.0, 1.0, 0.0)];
        render(&one, 8, 4, Some(1.0));
        render(&one, 3, 2, None);
        render(&one, 40, 16, None);
    }
}
