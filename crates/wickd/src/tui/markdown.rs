//! Markdown → terminal lines for the journal screen.
//!
//! [`render`] is a pure function of the source text and a width: it parses
//! with `pulldown-cmark` and lays the document out as ratatui [`Line`]s,
//! wrapped on display width, with nothing truncated. A terminal can't vary
//! font size, so structure is carried by weight, colour and a few box
//! characters:
//!
//! - `#` bold with a rule under it; `##` bold in the accent colour; `###`
//!   bold; deeper levels bold and dim
//! - bullets `•` / `◦` / `▪` and numbered lists, with a hanging indent
//! - block quotes behind a dim `▎` bar
//! - tables in box characters, columns sized to the width and cells wrapped
//!   inside their column
//!
//! Prose is capped at [`PROSE_WIDTH`] columns for readability; tables may use
//! the full width.

use pulldown_cmark::{Alignment, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// Longest prose line, in columns.
pub const PROSE_WIDTH: usize = 110;

const DIM: Color = Color::DarkGray;
const ACCENT: Color = Color::Cyan;
/// Inline code and code blocks: a muted sand, readable on dark backgrounds.
const CODE: Color = Color::Rgb(206, 178, 128);
/// Table and rule lines.
const LINE: Color = Color::Rgb(74, 82, 98);

/// A laid-out document.
#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub lines: Vec<Line<'static>>,
    /// Line index of every heading, in order (jump targets).
    pub headings: Vec<usize>,
}

/// One styled character; the unit everything wraps on.
type Cell = (char, Style);

fn cw(c: char) -> usize {
    c.width().unwrap_or(0)
}

fn width_of(cells: &[Cell]) -> usize {
    cells.iter().map(|(c, _)| cw(*c)).sum()
}

fn cells_of(s: &str, style: Style) -> Vec<Cell> {
    s.chars().map(|c| (c, style)).collect()
}

/// Group adjacent same-style characters into spans.
fn to_spans(cells: &[Cell]) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut style = Style::default();
    for (c, st) in cells {
        if *st != style && !run.is_empty() {
            out.push(Span::styled(std::mem::take(&mut run), style));
        }
        style = *st;
        run.push(*c);
    }
    if !run.is_empty() {
        out.push(Span::styled(run, style));
    }
    out
}

/// Greedy word wrap on display width. `\n` is a hard break; runs of spaces
/// collapse to one; a word wider than the line is split, never dropped.
/// Always returns at least one (possibly empty) line.
fn wrap(cells: &[Cell], width: usize) -> Vec<Vec<Cell>> {
    let width = width.max(1);
    let mut lines: Vec<Vec<Cell>> = Vec::new();
    for para in cells.split(|(c, _)| *c == '\n') {
        let mut line: Vec<Cell> = Vec::new();
        let mut used = 0usize;
        // The space waiting to go before the next word (carries its style so
        // underlines and strikes run through the gaps).
        let mut gap: Option<Cell> = None;
        let mut i = 0;
        while i < para.len() {
            if para[i].0.is_whitespace() {
                if !line.is_empty() {
                    gap = Some((' ', para[i].1));
                }
                i += 1;
                continue;
            }
            let start = i;
            while i < para.len() && !para[i].0.is_whitespace() {
                i += 1;
            }
            let word = &para[start..i];
            let ww = width_of(word);
            let need = ww + usize::from(gap.is_some());
            if used + need <= width {
                if let Some(g) = gap.take() {
                    line.push(g);
                    used += 1;
                }
                line.extend_from_slice(word);
                used += ww;
                continue;
            }
            gap = None;
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            // A word wider than a whole line: split it across lines.
            for cell in word {
                let w = cw(cell.0);
                if used + w > width && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(*cell);
                used += w;
            }
        }
        lines.push(line);
    }
    lines
}

/// An enclosing block: what goes in front of the first line of the next
/// block inside it, and in front of every other line.
struct Container {
    first: Vec<Cell>,
    cont: Vec<Cell>,
    used: bool,
    quote: bool,
}

#[derive(Default)]
struct TableBuf {
    aligns: Vec<Alignment>,
    head: Vec<Vec<Cell>>,
    rows: Vec<Vec<Vec<Cell>>>,
    row: Vec<Vec<Cell>>,
    in_head: bool,
}

struct Layout {
    width: usize,
    out: Vec<Line<'static>>,
    headings: Vec<usize>,
    stack: Vec<Container>,
    /// Open lists: `Some(n)` = ordered, next number `n`.
    lists: Vec<Option<u64>>,
    /// The inline text of the block being read.
    inline: Vec<Cell>,
    /// Open inline styles.
    bold: u32,
    italic: u32,
    strike: u32,
    link: u32,
    heading: Option<HeadingLevel>,
    /// A blank line is owed before the next block.
    gap: bool,
    code_block: bool,
    table: Option<TableBuf>,
}

impl Layout {
    fn style(&self) -> Style {
        let mut st = match self.heading {
            Some(HeadingLevel::H1) => Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            Some(HeadingLevel::H2) => Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            Some(HeadingLevel::H3) => Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            Some(_) => Style::default().fg(DIM).add_modifier(Modifier::BOLD),
            None => Style::default(),
        };
        if self.bold > 0 {
            st = st.add_modifier(Modifier::BOLD);
            if self.heading.is_none() {
                st = st.fg(Color::White);
            }
        }
        if self.italic > 0 {
            st = st.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            st = st.fg(DIM).add_modifier(Modifier::CROSSED_OUT);
        }
        if self.link > 0 {
            st = st.add_modifier(Modifier::UNDERLINED);
        }
        st
    }

    fn text(&mut self, s: &str) {
        let st = self.style();
        self.inline.extend(s.chars().map(|c| (c, st)));
    }

    /// Prefix for the next line: `first` once per container, then `cont`.
    fn prefix(&mut self) -> Vec<Cell> {
        let mut p = Vec::new();
        for c in &mut self.stack {
            if c.used {
                p.extend_from_slice(&c.cont);
            } else {
                p.extend_from_slice(&c.first);
                c.used = true;
            }
        }
        p
    }

    /// Prefix of a blank separator line: only the quote bars.
    fn blank_prefix(&self) -> Vec<Cell> {
        let mut p = Vec::new();
        for c in &self.stack {
            if c.quote || c.used {
                p.extend_from_slice(&c.cont);
            }
        }
        while p.last().is_some_and(|(c, _)| *c == ' ') {
            p.pop();
        }
        p
    }

    fn push(&mut self, cells: Vec<Cell>) {
        self.out.push(Line::from(to_spans(&cells)));
    }

    /// Pay the owed blank line (never at the very top).
    fn settle_gap(&mut self) {
        if self.gap && !self.out.is_empty() {
            let p = self.blank_prefix();
            self.push(p);
        }
        self.gap = false;
    }

    fn avail(&self, cap: usize) -> usize {
        let pre: usize = self.stack.iter().map(|c| width_of(&c.cont)).sum();
        self.width.min(cap + pre).saturating_sub(pre).max(8)
    }

    /// Emit the pending inline text as one wrapped block.
    fn flush(&mut self) {
        if self.inline.is_empty() {
            return;
        }
        let cells = std::mem::take(&mut self.inline);
        self.settle_gap();
        let w = self.avail(PROSE_WIDTH);
        for line in wrap(&cells, w) {
            let mut l = self.prefix();
            l.extend(line);
            self.push(l);
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush();
                self.gap = true;
                self.heading = Some(level);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.settle_gap();
                let bar = cells_of("▎ ", Style::default().fg(DIM));
                self.stack.push(Container {
                    first: bar.clone(),
                    cont: bar,
                    used: false,
                    quote: true,
                });
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.settle_gap();
                self.code_block = true;
            }
            Tag::List(start) => {
                // A nested list starts right under its item's text.
                self.flush();
                if self.lists.is_empty() {
                    self.settle_gap();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let depth = self.lists.len().saturating_sub(1);
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => ["• ", "◦ ", "▪ "][depth.min(2)].to_string(),
                };
                let w: usize = marker.chars().map(cw).sum();
                self.stack.push(Container {
                    first: cells_of(&marker, Style::default().fg(DIM)),
                    cont: cells_of(&" ".repeat(w), Style::default()),
                    used: false,
                    quote: false,
                });
            }
            Tag::Table(aligns) => {
                self.flush();
                self.settle_gap();
                self.table = Some(TableBuf {
                    aligns,
                    ..TableBuf::default()
                });
            }
            Tag::TableHead => {
                if let Some(t) = &mut self.table {
                    t.in_head = true;
                }
            }
            Tag::TableRow | Tag::TableCell => {}
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { .. } => self.link += 1,
            Tag::Image { .. } => self.text("[image: "),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                self.gap = true;
            }
            TagEnd::Heading(level) => {
                let cells = std::mem::take(&mut self.inline);
                self.settle_gap();
                self.headings.push(self.out.len());
                let w = self.avail(PROSE_WIDTH);
                let mut widest = 0;
                for line in wrap(&cells, w) {
                    widest = widest.max(width_of(&line));
                    let mut l = self.prefix();
                    l.extend(line);
                    self.push(l);
                }
                if level == HeadingLevel::H1 {
                    let mut l = self.prefix();
                    l.extend(cells_of(
                        &"━".repeat(widest.max(1)),
                        Style::default().fg(LINE),
                    ));
                    self.push(l);
                }
                self.heading = None;
                self.gap = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.stack.pop();
                self.gap = true;
            }
            TagEnd::CodeBlock => {
                self.code_block = false;
                self.gap = true;
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                // Only the outermost list is followed by a blank line.
                self.gap = self.lists.is_empty();
            }
            TagEnd::Item => {
                self.flush();
                // An empty item still shows its marker.
                if self.stack.last().is_some_and(|c| !c.used) {
                    let l = self.prefix();
                    self.push(l);
                }
                self.stack.pop();
                // Items of a tight list sit on consecutive lines.
                self.gap = false;
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.table_out(t);
                }
                self.gap = true;
            }
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    t.head = std::mem::take(&mut t.row);
                    t.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inline);
                if let Some(t) = &mut self.table {
                    t.row.push(cell);
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => self.link = self.link.saturating_sub(1),
            TagEnd::Image => self.text("]"),
            _ => {}
        }
    }

    fn code_lines(&mut self, text: &str) {
        let w = self.avail(usize::MAX / 2).saturating_sub(2).max(4);
        let st = Style::default().fg(CODE);
        for raw in text.strip_suffix('\n').unwrap_or(text).split('\n') {
            // Code is split hard at the width (no reflow), never cut off.
            let cells = cells_of(&raw.replace('\t', "    "), st);
            let mut rows: Vec<Vec<Cell>> = vec![Vec::new()];
            let mut used = 0;
            for c in cells {
                let cwid = cw(c.0);
                if used + cwid > w {
                    rows.push(Vec::new());
                    used = 0;
                }
                rows.last_mut().expect("non-empty").push(c);
                used += cwid;
            }
            for r in rows {
                let mut l = self.prefix();
                l.extend(cells_of("  ", Style::default()));
                l.extend(r);
                self.push(l);
            }
        }
    }

    fn rule(&mut self) {
        self.flush();
        self.settle_gap();
        let w = self.avail(PROSE_WIDTH);
        let mut l = self.prefix();
        l.extend(cells_of(&"─".repeat(w), Style::default().fg(LINE)));
        self.push(l);
        self.gap = true;
    }

    fn table_out(&mut self, t: TableBuf) {
        let n = t
            .rows
            .iter()
            .map(Vec::len)
            .chain([t.head.len()])
            .max()
            .unwrap_or(0);
        if n == 0 {
            return;
        }
        let bold = |cells: &[Cell]| -> Vec<Cell> {
            cells
                .iter()
                .map(|(c, st)| (*c, st.fg(Color::White).add_modifier(Modifier::BOLD)))
                .collect()
        };
        let head: Vec<Vec<Cell>> = (0..n)
            .map(|i| bold(t.head.get(i).map_or(&[][..], Vec::as_slice)))
            .collect();
        let avail = self.avail(usize::MAX / 2);
        // Borders and one space of padding each side of every cell.
        let chrome = n * 3 + 1;
        const MIN_COL: usize = 3;
        if avail < chrome + n * MIN_COL {
            self.table_stacked(&head, &t.rows, avail);
            return;
        }
        let room = avail - chrome;
        let all = || std::iter::once(&head).chain(t.rows.iter());
        // Natural width, and the width below which a word would be split.
        let mut widths: Vec<usize> = vec![MIN_COL; n];
        let mut floor: Vec<usize> = vec![MIN_COL; n];
        for row in all() {
            for (i, cell) in row.iter().enumerate().take(n) {
                widths[i] = widths[i].max(width_of(cell));
                let longest = cell
                    .split(|(c, _)| c.is_whitespace())
                    .map(width_of)
                    .max()
                    .unwrap_or(0);
                floor[i] = floor[i].max(longest.min(24));
            }
        }
        // Too wide: take a column at a time from the widest, first down to
        // its longest word, then (if that isn't enough) below it.
        for pass in 0..2 {
            while widths.iter().sum::<usize>() > room {
                let min = |i: usize| if pass == 0 { floor[i] } else { MIN_COL };
                let Some(i) = (0..n)
                    .filter(|&i| widths[i] > min(i))
                    .max_by_key(|&i| widths[i])
                else {
                    break;
                };
                widths[i] -= 1;
            }
        }
        let line_st = Style::default().fg(LINE);
        let border = |l: &str, m: &str, r: &str| -> Vec<Cell> {
            let mut s = String::from(l);
            for (i, w) in widths.iter().enumerate() {
                s.push_str(&"─".repeat(w + 2));
                s.push_str(if i + 1 == n { r } else { m });
            }
            cells_of(&s, line_st)
        };
        let row_lines = |row: &[Vec<Cell>]| -> Vec<Vec<Cell>> {
            let wrapped: Vec<Vec<Vec<Cell>>> = (0..n)
                .map(|i| wrap(row.get(i).map_or(&[][..], Vec::as_slice), widths[i]))
                .collect();
            let h = wrapped.iter().map(Vec::len).max().unwrap_or(1);
            (0..h)
                .map(|r| {
                    let mut l = cells_of("│", line_st);
                    for (i, col) in wrapped.iter().enumerate() {
                        let cell = col.get(r).map_or(&[][..], Vec::as_slice);
                        let pad = widths[i] - width_of(cell).min(widths[i]);
                        let (lp, rp) = match t.aligns.get(i) {
                            Some(Alignment::Right) => (pad, 0),
                            Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                            _ => (0, pad),
                        };
                        l.extend(cells_of(&" ".repeat(lp + 1), Style::default()));
                        l.extend_from_slice(cell);
                        l.extend(cells_of(&" ".repeat(rp + 1), Style::default()));
                        l.extend(cells_of("│", line_st));
                    }
                    l
                })
                .collect()
        };
        let mut block: Vec<Vec<Cell>> = vec![border("┌", "┬", "┐")];
        block.extend(row_lines(&head));
        let body: Vec<Vec<Vec<Cell>>> = t.rows.iter().map(|r| row_lines(r)).collect();
        // Rows that wrap need a line between them to stay readable.
        let ruled = body.iter().any(|r| r.len() > 1);
        for (i, r) in body.into_iter().enumerate() {
            if i == 0 || ruled {
                block.push(border("├", "┼", "┤"));
            }
            block.extend(r);
        }
        block.push(border("└", "┴", "┘"));
        for b in block {
            let mut l = self.prefix();
            l.extend(b);
            self.push(l);
        }
    }

    /// A table too wide for even three columns per cell: each row as
    /// `Header: value` lines.
    fn table_stacked(&mut self, head: &[Vec<Cell>], rows: &[Vec<Vec<Cell>>], avail: usize) {
        for (r, row) in rows.iter().enumerate() {
            if r > 0 {
                let p = self.blank_prefix();
                self.push(p);
            }
            for (i, cell) in row.iter().enumerate() {
                let mut cells = head.get(i).cloned().unwrap_or_default();
                if !cells.is_empty() {
                    cells.extend(cells_of(": ", Style::default().fg(DIM)));
                }
                cells.extend_from_slice(cell);
                for line in wrap(&cells, avail) {
                    let mut l = self.prefix();
                    l.extend(line);
                    self.push(l);
                }
            }
        }
    }
}

/// Lay `src` out for a page `width` columns wide.
pub fn render(src: &str, width: u16) -> Rendered {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    let mut l = Layout {
        width: usize::from(width).max(8),
        out: Vec::new(),
        headings: Vec::new(),
        stack: Vec::new(),
        lists: Vec::new(),
        inline: Vec::new(),
        bold: 0,
        italic: 0,
        strike: 0,
        link: 0,
        heading: None,
        gap: false,
        code_block: false,
        table: None,
    };
    for ev in Parser::new_ext(src, opts) {
        match ev {
            Event::Start(tag) => l.start(tag),
            Event::End(tag) => l.end(tag),
            Event::Text(t) if l.code_block => l.code_lines(&t),
            Event::Text(t) => l.text(&t),
            Event::Code(t) => {
                let st = l.style().fg(CODE);
                l.inline.extend(t.chars().map(|c| (c, st)));
            }
            Event::SoftBreak => l.text(" "),
            Event::HardBreak => l.inline.push(('\n', Style::default())),
            Event::Rule => l.rule(),
            Event::TaskListMarker(done) => {
                let st = Style::default().fg(if done { ACCENT } else { DIM });
                l.inline
                    .extend(cells_of(if done { "☑ " } else { "☐ " }, st));
            }
            Event::Html(t) | Event::InlineHtml(t) => {
                let st = Style::default().fg(DIM);
                l.inline.extend(t.trim_end().chars().map(|c| (c, st)));
            }
            Event::FootnoteReference(t) => l.text(&format!("[{t}]")),
            _ => {}
        }
    }
    l.flush();
    Rendered {
        lines: l.out,
        headings: l.headings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn texts(src: &str, w: u16) -> Vec<String> {
        render(src, w).lines.iter().map(text).collect()
    }

    #[test]
    fn paragraphs_wrap_on_words_and_never_exceed_the_width() {
        let src = "one two three four five six seven eight nine ten eleven twelve";
        let out = texts(src, 20);
        assert_eq!(
            out,
            [
                "one two three four",
                "five six seven eight",
                "nine ten eleven",
                "twelve"
            ]
        );
        // A word longer than the line is split, not dropped.
        let out = texts("abcdefghijklmnopqrstuvwxyz end", 10);
        assert_eq!(out, ["abcdefghij", "klmnopqrst", "uvwxyz end"]);
        // Prose stops at the readable width even on a huge terminal.
        let long = "word ".repeat(100);
        assert!(texts(&long, 300).iter().all(|l| l.width() <= PROSE_WIDTH));
    }

    #[test]
    fn blocks_are_separated_by_one_blank_line() {
        let out = texts("# Title\n\nFirst.\n\n\n\nSecond.\n\n## Next\nBody.", 40);
        assert_eq!(
            out,
            [
                "Title",
                "━━━━━",
                "",
                "First.",
                "",
                "Second.",
                "",
                "Next",
                "",
                "Body."
            ]
        );
    }

    #[test]
    fn bullets_hang_their_wrapped_lines_under_the_text() {
        let out = texts("- alpha beta gamma delta\n- two", 14);
        assert_eq!(out, ["• alpha beta", "  gamma delta", "• two"]);
        let out = texts("9. nine nine nine nine\n10. ten ten ten ten", 14);
        assert_eq!(
            out,
            ["9. nine nine", "   nine nine", "10. ten ten", "    ten ten"]
        );
    }

    #[test]
    fn nested_lists_indent_under_their_item() {
        let src =
            "Intro:\n- top one\n  - inner a\n  - inner b that wraps here\n- top two\n\nAfter.";
        let out = texts(src, 18);
        assert_eq!(
            out,
            [
                "Intro:",
                "",
                "• top one",
                "  ◦ inner a",
                "  ◦ inner b that",
                "    wraps here",
                "• top two",
                "",
                "After."
            ]
        );
    }

    #[test]
    fn quotes_carry_a_bar_on_every_line() {
        let out = texts("> quoted words that wrap\n>\n> second para", 14);
        assert_eq!(out, ["▎ quoted words", "▎ that wrap", "▎", "▎ second para"]);
    }

    #[test]
    fn inline_styles_survive_wrapping() {
        let r = render("plain **bold** `code` *it* ~~gone~~ [link](http://x)", 80);
        let spans = &r.lines[0].spans;
        let find = |s: &str| spans.iter().find(|sp| sp.content == s).unwrap().style;
        assert!(find("bold").add_modifier.contains(Modifier::BOLD));
        assert_eq!(find("code").fg, Some(CODE));
        assert!(find("it").add_modifier.contains(Modifier::ITALIC));
        assert!(find("gone").add_modifier.contains(Modifier::CROSSED_OUT));
        assert!(find("link").add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(text(&r.lines[0]), "plain bold code it gone link");
    }

    #[test]
    fn tasks_code_blocks_and_rules() {
        let out = texts(
            "- [x] done\n- [ ] todo\n\n```\nlet a = 1;\n  b\n```\n\n---",
            20,
        );
        assert_eq!(
            out,
            [
                "• ☑ done",
                "• ☐ todo",
                "",
                "  let a = 1;",
                "    b",
                "",
                "────────────────────"
            ]
        );
    }

    #[test]
    fn a_table_that_fits_is_drawn_at_its_natural_width() {
        let out = texts("| A | Bee |\n|---|---|\n| 1 | two |\n| 333 | 4 |", 80);
        assert_eq!(
            out,
            [
                "┌─────┬─────┐",
                "│ A   │ Bee │",
                "├─────┼─────┤",
                "│ 1   │ two │",
                "│ 333 │ 4   │",
                "└─────┴─────┘"
            ]
        );
    }

    #[test]
    fn a_wide_table_wraps_inside_its_columns_and_keeps_every_word() {
        let src = "| Stock | Call | The question that drove it |\n|---|---|---|\n\
                   | NET | −2 to +2% | Priced in: +16% in four days, now stalling |\n\
                   | CEG | flat to +3% | The news: best report of the week, new high |";
        for w in [30u16, 44, 60] {
            let out = texts(src, w);
            // Every line is exactly as wide as the frame: borders line up.
            let frame = out[0].width();
            assert!(frame <= usize::from(w), "{w}: {frame}");
            for l in &out {
                assert_eq!(l.width(), frame, "{w}: {l}");
            }
            // Nothing is cut: every word is still there.
            let all = out.join(" ");
            for word in ["Stock", "stalling", "question", "week,", "+16%", "flat"] {
                assert!(all.contains(word), "{w}: lost {word}\n{}", out.join("\n"));
            }
            // Wrapped rows are ruled off from each other.
            assert!(out.iter().filter(|l| l.starts_with('├')).count() >= 2);
        }
    }

    #[test]
    fn a_table_with_no_room_for_columns_stacks_each_row() {
        let src = "| A | B | C | D |\n|---|---|---|---|\n| 1 | 2 | 3 | 4 |\n| 5 | 6 | 7 | 8 |";
        let out = texts(src, 12);
        assert_eq!(
            out,
            ["A: 1", "B: 2", "C: 3", "D: 4", "", "A: 5", "B: 6", "C: 7", "D: 8"]
        );
    }

    #[test]
    fn headings_are_recorded_as_jump_targets() {
        let r = render("# One\n\ntext\n\n## Two\n\ntext\n\n### Three", 40);
        assert_eq!(r.headings, [0, 5, 9]);
        for (i, name) in r.headings.iter().zip(["One", "Two", "Three"]) {
            assert_eq!(text(&r.lines[*i]), name);
        }
    }
}
