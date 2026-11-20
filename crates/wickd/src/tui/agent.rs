//! The agent panel: `a` on any screen opens a chat with the saved agent
//! session over whatever is on screen (wickd-learn D-19, D-21, D-22,
//! AGT-2549).
//!
//! The panel drives `wickd agent ask` (AGT-2548): each message runs the same
//! wickd binary as a child process, `wickd agent ask --context - -- <q>`,
//! with the current screen's context ([`super::context::screen`]) on its
//! stdin, and reads its JSON-lines events on a worker task
//! ([`run_ask`]). The run loop hands each event to [`Panel::apply`], so the
//! answer streams into the panel and the UI thread never waits on the agent.
//! `/clear` and `/compact` typed in the panel go through the same command,
//! so they do exactly what they do there.
//!
//! The conversation shown is kept in the app for the session and saved to
//! `transcript.json` next to the agent's `session.json`, so it reappears
//! after a restart. `/clear` empties it; so does a session cleared from the
//! command line (the transcript only loads while a saved session exists).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_width::UnicodeWidthChar;

const TRANSCRIPT_FILE: &str = "transcript.json";
/// Messages kept in the saved transcript (the newest).
const MAX_SAVED: usize = 400;
/// Give up on one ask after this long. `wickd agent ask` stops itself after
/// 300s; this only catches a child that never exits.
const ASK_TIMEOUT: Duration = Duration::from_secs(330);

const DIM: Color = Color::DarkGray;
const ACCENT: Color = Color::Cyan;
/// The panel's border: muted, so the brand colour stays a small accent.
const LINE: Color = Color::Rgb(74, 82, 98);
const WARN: Color = Color::Yellow;

/// Who said a line of the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Agent,
    /// Bookkeeping: a compaction, a notice from `wickd agent`.
    Notice,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

/// What an ask in flight is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Busy {
    Thinking,
    Compacting,
    Clearing,
}

/// The input line: text and a cursor (a char index).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Input {
    pub text: String,
    pub cursor: usize,
}

impl Input {
    fn byte_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(b, _)| b)
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn insert(&mut self, c: char) {
        let b = self.byte_at(self.cursor);
        self.text.insert(b, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let b = self.byte_at(self.cursor);
            self.text.remove(b);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            let b = self.byte_at(self.cursor);
            self.text.remove(b);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    /// Delete the word before the cursor (and the spaces after it).
    pub fn delete_word(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut start = self.cursor;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let (a, b) = (self.byte_at(start), self.byte_at(self.cursor));
        self.text.replace_range(a..b, "");
        self.cursor = start;
    }

    /// Delete from the start of the line to the cursor.
    pub fn delete_to_start(&mut self) {
        let b = self.byte_at(self.cursor);
        self.text.replace_range(..b, "");
        self.cursor = 0;
    }

    /// Delete from the cursor to the end of the line.
    pub fn delete_to_end(&mut self) {
        let b = self.byte_at(self.cursor);
        self.text.truncate(b);
    }

    fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }
}

/// A message to send: the question for `wickd agent ask`, and whether the
/// screen's context goes with it (not for `/clear` and `/compact`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submit {
    pub question: String,
    pub attach: bool,
}

/// What a key did in the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelKey {
    Nothing,
    Close,
    Send(Submit),
}

/// The agent panel's state.
#[derive(Debug, Default)]
pub struct Panel {
    pub open: bool,
    pub messages: Vec<Message>,
    pub input: Input,
    /// Lines scrolled back from the newest (0 = follow the conversation).
    pub scroll_back: usize,
    /// An ask is in flight.
    pub busy: Option<Busy>,
    /// The lookup the agent is running, shown dimly while it works.
    pub activity: Option<String>,
    /// The agent message this turn is streaming into (index into
    /// `messages`).
    streaming: Option<usize>,
    /// Where the transcript is saved; `None` keeps it in memory only.
    dir: Option<PathBuf>,
    /// Why the transcript couldn't be read or saved.
    pub save_error: Option<String>,
    /// Conversation rows on screen, as last measured by the renderer.
    page: Cell<usize>,
    /// The furthest the conversation can scroll back, as last measured.
    max_back: Cell<usize>,
}

impl Panel {
    /// The panel with the transcript saved under `dir` (the agent
    /// directory), loaded when a saved session exists.
    pub fn at(dir: PathBuf) -> Self {
        let mut p = Panel::default();
        let has_session = matches!(crate::agent_session::load_at(&dir), Ok(Some(_)));
        if has_session {
            match load_transcript(&dir) {
                Ok(m) => p.messages = m,
                Err(e) => {
                    p.save_error = Some(format!("couldn't read the saved conversation: {e:#}"))
                }
            }
        }
        p.dir = Some(dir);
        p
    }

    /// The panel saving under the wickd data home (in memory only when that
    /// can't be found).
    pub fn at_data_home() -> Self {
        match crate::agent_session::agent_dir() {
            Ok(dir) => Panel::at(dir),
            Err(e) => Panel {
                save_error: Some(format!("the conversation won't be saved: {e:#}")),
                ..Panel::default()
            },
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.scroll_back = 0;
    }

    fn save(&mut self) {
        let Some(dir) = &self.dir else { return };
        let from = self.messages.len().saturating_sub(MAX_SAVED);
        self.save_error = save_transcript(dir, &self.messages[from..])
            .err()
            .map(|e| format!("couldn't save the conversation: {e:#}"));
    }

    fn push(&mut self, role: Role, text: impl Into<String>) {
        self.messages.push(Message {
            role,
            text: text.into(),
        });
    }

    /// A key while the panel is open: everything typed goes to the input.
    pub fn on_key(&mut self, key: KeyEvent) -> PanelKey {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.page.get().saturating_sub(2).max(1);
        match key.code {
            KeyCode::Esc => return PanelKey::Close,
            KeyCode::Enter => {
                return match self.submit() {
                    Some(s) => PanelKey::Send(s),
                    None => PanelKey::Nothing,
                }
            }
            KeyCode::Char('a') if ctrl => self.input.home(),
            KeyCode::Char('e') if ctrl => self.input.end(),
            KeyCode::Char('u') if ctrl => self.input.delete_to_start(),
            KeyCode::Char('k') if ctrl => self.input.delete_to_end(),
            KeyCode::Char('w') if ctrl => self.input.delete_word(),
            KeyCode::Char(_) if ctrl || key.modifiers.contains(KeyModifiers::ALT) => {}
            KeyCode::Char(c) => self.input.insert(c),
            KeyCode::Backspace => self.input.backspace(),
            KeyCode::Delete => self.input.delete(),
            KeyCode::Left => self.input.left(),
            KeyCode::Right => self.input.right(),
            KeyCode::Home => self.input.home(),
            KeyCode::End => self.input.end(),
            KeyCode::Up => self.scroll(1),
            KeyCode::Down => self.scroll(-1),
            KeyCode::PageUp => self.scroll(page as isize),
            KeyCode::PageDown => self.scroll(-(page as isize)),
            _ => {}
        }
        PanelKey::Nothing
    }

    fn scroll(&mut self, by: isize) {
        let back = self.scroll_back.saturating_add_signed(by);
        self.scroll_back = back.min(self.max_back.get());
    }

    /// Enter: what to send, if anything. A message while an ask is out
    /// stays in the input.
    fn submit(&mut self) -> Option<Submit> {
        let q = self.input.text.trim().to_string();
        if q.is_empty() || self.busy.is_some() {
            return None;
        }
        self.input.take();
        self.scroll_back = 0;
        self.activity = None;
        self.streaming = None;
        let (busy, attach) = match q.as_str() {
            "/clear" => (Busy::Clearing, false),
            "/compact" => (Busy::Compacting, false),
            cmd if cmd.starts_with('/') => {
                self.push(
                    Role::Error,
                    format!("unknown command '{cmd}': the commands are /clear and /compact"),
                );
                return None;
            }
            _ => {
                self.push(Role::User, q.clone());
                self.save();
                (Busy::Thinking, true)
            }
        };
        self.busy = Some(busy);
        Some(Submit {
            question: q,
            attach,
        })
    }

    /// One `wickd agent ask` event.
    pub fn apply(&mut self, ev: &Value) {
        let s = |k: &str| ev.get(k).and_then(Value::as_str).unwrap_or_default();
        match s("type") {
            "text" => {
                let delta = s("delta");
                let i = match self.streaming {
                    Some(i) => i,
                    None => {
                        self.push(Role::Agent, "");
                        self.messages.len() - 1
                    }
                };
                self.streaming = Some(i);
                self.messages[i].text.push_str(delta);
                self.activity = None;
            }
            "tool" => self.activity = Some(tool_summary(ev)),
            "denied" => self.activity = Some(format!("{} wasn't allowed", s("tool"))),
            "status" => {
                if s("status") == "compacting" {
                    self.busy = Some(Busy::Compacting);
                }
            }
            "compacted" => {
                let tokens = |k: &str| ev.get(k).and_then(Value::as_u64).map(fmt_tokens);
                let text = match (tokens("pre_tokens"), tokens("post_tokens")) {
                    (Some(a), Some(b)) => format!("conversation compacted ({a} → {b} tokens)"),
                    _ => "conversation compacted".to_string(),
                };
                self.push(Role::Notice, text);
            }
            "notice" => self.push(Role::Notice, s("message")),
            "cleared" => {
                self.messages.clear();
                self.finish_turn();
            }
            "done" => {
                let answer = s("answer").trim();
                match self.streaming {
                    Some(i) if !answer.is_empty() => self.messages[i].text = answer.to_string(),
                    Some(_) => {}
                    None if !answer.is_empty() => self.push(Role::Agent, answer),
                    None => {}
                }
                if ev.get("compacted").and_then(Value::as_bool) == Some(false) {
                    self.push(Role::Notice, "nothing to compact yet");
                }
                self.finish_turn();
            }
            "error" => {
                self.push(Role::Error, s("message"));
                self.finish_turn();
            }
            _ => {}
        }
    }

    /// The child exited. If it never said `done`, `error` or `cleared`,
    /// that's an error worth showing.
    pub fn finished(&mut self, result: Result<(), String>) {
        if self.busy.is_none() {
            return;
        }
        let why = match result {
            Ok(()) => "the agent stopped without answering".to_string(),
            Err(e) => format!("the agent stopped without answering: {e}"),
        };
        self.push(Role::Error, why);
        self.finish_turn();
    }

    fn finish_turn(&mut self) {
        // A streamed message that never got any text isn't worth keeping.
        if let Some(i) = self.streaming.take() {
            if self.messages[i].text.trim().is_empty() {
                self.messages.remove(i);
            }
        }
        self.busy = None;
        self.activity = None;
        self.save();
    }
}

/// `8539` → `8.5k`.
fn fmt_tokens(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// A `tool` event in a few words: the command it ran, the search, the page.
fn tool_summary(ev: &Value) -> String {
    let name = ev.get("name").and_then(Value::as_str).unwrap_or("a tool");
    let input = ev.get("input").unwrap_or(&Value::Null);
    let field = ["command", "query", "url", "file_path", "pattern"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str));
    let text = match field {
        Some(f) => f.lines().next().unwrap_or_default().to_string(),
        None => name.to_string(),
    };
    let mut out: String = text.chars().take(60).collect();
    if text.chars().count() > 60 {
        out.push('…');
    }
    out
}

fn transcript_path(dir: &Path) -> PathBuf {
    dir.join(TRANSCRIPT_FILE)
}

/// The saved conversation (empty when there is none).
pub fn load_transcript(dir: &Path) -> anyhow::Result<Vec<Message>> {
    use anyhow::Context;
    let path = transcript_path(dir);
    match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t)
            .with_context(|| format!("{} is not a transcript", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Save the conversation (tmp file + rename, like the session).
pub fn save_transcript(dir: &Path, messages: &[Message]) -> anyhow::Result<()> {
    use anyhow::Context;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = transcript_path(dir);
    let tmp = dir.join(format!(".{TRANSCRIPT_FILE}.tmp"));
    std::fs::write(&tmp, serde_json::to_string(messages)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

// ── the child process ───────────────────────────────────────────────────────

/// Run one `wickd agent ask` with `exe` (the running wickd), sending each
/// JSON event it prints to `emit` as it arrives. `context` (JSON) goes to
/// its stdin. Errors are for a child that couldn't run or failed without
/// saying why on stdout; the panel only shows them if no `error` event
/// came first.
pub async fn run_ask(
    exe: &Path,
    question: &str,
    context: Option<String>,
    emit: impl Fn(Value),
) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::process::Command;
    let mut cmd = Command::new(exe);
    cmd.args(["agent", "ask"]);
    if context.is_some() {
        cmd.args(["--context", "-"]);
    }
    // After `--` a question starting with `-` is still the question.
    cmd.args(["--", question])
        .stdin(if context.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("couldn't start {}: {e}", exe.display()))?;
    if let (Some(ctx), Some(mut stdin)) = (context, child.stdin.take()) {
        // Written on its own task so a full pipe can't stall the reader.
        tokio::spawn(async move {
            let _ = stdin.write_all(ctx.as_bytes()).await;
        });
    }
    let stdout = child
        .stdout
        .take()
        .ok_or("no stdout from wickd agent ask")?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or("no stderr from wickd agent ask")?;
    let stderr_task = tokio::spawn(async move {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s).await;
        s
    });
    let read = async {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                emit(v);
            }
        }
        child.wait().await
    };
    let status = match tokio::time::timeout(ASK_TIMEOUT, read).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(format!("waiting for wickd agent ask: {e}")),
        Err(_) => return Err(format!("no answer after {}s", ASK_TIMEOUT.as_secs())),
    };
    if status.success() {
        return Ok(());
    }
    let err = stderr_task.await.unwrap_or_default();
    let tail: String = err
        .trim()
        .chars()
        .rev()
        .take(300)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Err(if tail.is_empty() {
        format!("wickd agent ask exited with {status}")
    } else {
        tail
    })
}

// ── drawing ─────────────────────────────────────────────────────────────────

/// Display width of a char (0 for control characters).
fn cw(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Plain text wrapped on words to `width` columns, keeping its own line
/// breaks; words longer than a line are cut.
fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    let width = width.max(4);
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        let mut cur_w = 0;
        for word in para.split_whitespace() {
            let w: usize = word.chars().map(cw).sum();
            if cur_w > 0 && cur_w + 1 + w > width {
                out.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            if cur_w > 0 {
                cur.push(' ');
                cur_w += 1;
            }
            for c in word.chars() {
                if cur_w + cw(c) > width {
                    out.push(std::mem::take(&mut cur));
                    cur_w = 0;
                }
                cur.push(c);
                cur_w += cw(c);
            }
        }
        out.push(cur);
    }
    out
}

/// The conversation as lines `width` columns wide.
pub fn conversation_lines(p: &Panel, width: usize) -> Vec<Line<'static>> {
    let dim = Style::default().fg(DIM);
    let mut out: Vec<Line<'static>> = Vec::new();
    if p.messages.is_empty() && p.busy.is_none() {
        for t in [
            "Ask about what's on screen: the chart (bars, Ichimoku, cursor, price lines), a list or the calendar goes with every message.",
            "",
            "/clear starts a new conversation · /compact shrinks a long one",
        ] {
            out.extend(wrap_plain(t, width).into_iter().map(|l| Line::from(Span::styled(l, dim))));
        }
    }
    for (i, m) in p.messages.iter().enumerate() {
        if i > 0 {
            out.push(Line::from(""));
        }
        match m.role {
            Role::User => {
                let prompt = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
                let text = Style::default().add_modifier(Modifier::BOLD);
                for (j, l) in wrap_plain(&m.text, width.saturating_sub(2))
                    .into_iter()
                    .enumerate()
                {
                    let lead = if j == 0 { "› " } else { "  " };
                    out.push(Line::from(vec![
                        Span::styled(lead, prompt),
                        Span::styled(l, text),
                    ]));
                }
            }
            Role::Agent => {
                out.extend(super::markdown::render(&m.text, width as u16).lines);
            }
            Role::Notice => {
                let style = dim.add_modifier(Modifier::ITALIC);
                for l in wrap_plain(&format!("· {}", m.text), width) {
                    out.push(Line::from(Span::styled(l, style)));
                }
            }
            Role::Error => {
                for l in wrap_plain(&format!("! {}", m.text), width) {
                    out.push(Line::from(Span::styled(l, Style::default().fg(WARN))));
                }
            }
        }
    }
    if let Some(busy) = p.busy {
        // Quiet while text is visibly arriving; otherwise say it's working.
        if p.streaming.is_none() || p.activity.is_some() {
            if !out.is_empty() {
                out.push(Line::from(""));
            }
            let word = match busy {
                Busy::Thinking => "thinking…",
                Busy::Compacting => "compacting…",
                Busy::Clearing => "clearing…",
            };
            let mut spans = vec![Span::styled(word, dim)];
            if let Some(a) = &p.activity {
                spans.push(Span::styled(
                    format!("  {a}"),
                    dim.add_modifier(Modifier::ITALIC),
                ));
            }
            out.push(Line::from(spans));
        }
    }
    out
}

/// The input as rows of at most `width` columns, and the row the cursor is
/// on. The cursor is drawn as a reversed cell.
fn input_lines(input: &Input, width: usize, placeholder: &str) -> (Vec<Line<'static>>, usize) {
    let prompt = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
    let cursor = Style::default().add_modifier(Modifier::REVERSED);
    if input.text.is_empty() {
        return (
            vec![Line::from(vec![
                Span::styled("› ", prompt),
                Span::styled(" ", cursor),
                Span::styled(placeholder.to_string(), Style::default().fg(DIM)),
            ])],
            0,
        );
    }
    let width = width.saturating_sub(2).max(4);
    let mut rows: Vec<Vec<Span<'static>>> = vec![vec![Span::styled("› ", prompt)]];
    let mut row_w = 0;
    let mut cursor_row = 0;
    let chars: Vec<char> = input.text.chars().collect();
    for i in 0..=chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        if row_w + cw(c) > width {
            rows.push(vec![Span::raw("  ")]);
            row_w = 0;
        }
        let style = if i == input.cursor {
            cursor_row = rows.len() - 1;
            cursor
        } else {
            Style::default()
        };
        if i < chars.len() || i == input.cursor {
            rows.last_mut()
                .expect("a row")
                .push(Span::styled(c.to_string(), style));
            row_w += cw(c);
        }
    }
    (rows.into_iter().map(Line::from).collect(), cursor_row)
}

/// Draw the panel over `area` (the body between the header and the
/// footer): on the right, or on the `left` (over a chart, so the newest
/// candles and the price scale stay in view). `sees` names what's attached
/// ("EUR_USD 1h chart").
pub fn render(frame: &mut Frame, area: Rect, p: &Panel, sees: &str, left: bool) {
    let w = if area.width >= 110 {
        (area.width * 46 / 100).clamp(56, 96)
    } else {
        area.width
    };
    let panel_x = if left {
        area.x
    } else {
        area.x + area.width - w
    };
    let rect = Rect::new(panel_x, area.y, w, area.height);
    frame.render_widget(Clear, rect);
    let title = vec![
        Span::styled(
            " agent ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("· sees {sees} "), Style::default().fg(DIM)),
    ];
    let keys = " enter send · esc close · ↑↓ pgup pgdn scroll ";
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(LINE))
        .title(Line::from(title))
        .title_bottom(Span::styled(keys, Style::default().fg(DIM)));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height < 4 || inner.width < 10 {
        return;
    }
    // One column of padding each side.
    let body_w = inner.width.saturating_sub(2);
    let placeholder = if p.busy.is_some() {
        ""
    } else {
        "ask about this screen…"
    };
    let (input, cursor_row) = input_lines(&p.input, body_w as usize, placeholder);
    let input_h = (input.len() as u16).clamp(1, 3);
    let first = (cursor_row + 1).saturating_sub(input_h as usize);
    let input: Vec<Line> = input
        .into_iter()
        .skip(first)
        .take(input_h as usize)
        .collect();

    let mut status: Vec<Line> = Vec::new();
    if let Some(e) = &p.save_error {
        status.push(Line::from(Span::styled(
            e.clone(),
            Style::default().fg(WARN),
        )));
    }
    let status_h = status.len() as u16;
    let convo_h = inner.height.saturating_sub(input_h + 1 + status_h);
    let x = inner.x + 1;
    let convo_rect = Rect::new(x, inner.y, body_w, convo_h);
    let lines = conversation_lines(p, body_w as usize);
    let max_back = lines.len().saturating_sub(convo_h as usize);
    p.page.set(convo_h as usize);
    p.max_back.set(max_back);
    let top = max_back - p.scroll_back.min(max_back);
    frame.render_widget(Paragraph::new(lines).scroll((top as u16, 0)), convo_rect);
    if p.scroll_back > 0 && max_back > 0 && convo_h > 0 {
        let more = format!(" ↓ {} more ", p.scroll_back.min(max_back));
        let mw = more.chars().count() as u16;
        let r = Rect::new(
            inner.x + inner.width.saturating_sub(mw + 1),
            inner.y + convo_h - 1,
            mw,
            1,
        );
        frame.render_widget(
            Paragraph::new(Span::styled(more, Style::default().fg(DIM))),
            r,
        );
    }
    let rule_y = inner.y + convo_h;
    frame.render_widget(
        Paragraph::new(Span::styled(
            "─".repeat(inner.width as usize),
            Style::default().fg(LINE),
        )),
        Rect::new(inner.x, rule_y, inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(input),
        Rect::new(x, rule_y + 1, body_w, input_h),
    );
    if status_h > 0 {
        frame.render_widget(
            Paragraph::new(status),
            Rect::new(x, rule_y + 1 + input_h, body_w, status_h),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn typed(p: &mut Panel, s: &str) {
        for c in s.chars() {
            assert_eq!(p.on_key(key(KeyCode::Char(c))), PanelKey::Nothing);
        }
    }

    #[test]
    fn input_editing() {
        let mut p = Panel::default();
        typed(&mut p, "hello world");
        assert_eq!((p.input.text.as_str(), p.input.cursor), ("hello world", 11));
        p.on_key(key(KeyCode::Left));
        p.on_key(key(KeyCode::Left));
        p.on_key(key(KeyCode::Backspace));
        assert_eq!(p.input.text, "hello wold");
        p.on_key(key(KeyCode::Delete));
        assert_eq!((p.input.text.as_str(), p.input.cursor), ("hello wod", 8));
        p.on_key(key(KeyCode::Home));
        typed(&mut p, "¡");
        assert_eq!(p.input.text, "¡hello wod");
        p.on_key(ctrl('e'));
        p.on_key(ctrl('w'));
        assert_eq!(p.input.text, "¡hello ");
        p.on_key(ctrl('w'));
        assert_eq!(p.input.text, "");
        typed(&mut p, "abc def");
        p.on_key(ctrl('a'));
        p.on_key(key(KeyCode::Right));
        p.on_key(ctrl('k'));
        assert_eq!(p.input.text, "a");
        typed(&mut p, "xyz");
        p.on_key(key(KeyCode::Left));
        p.on_key(ctrl('u'));
        assert_eq!((p.input.text.as_str(), p.input.cursor), ("z", 0));
        // Other control/alt chords don't type.
        p.on_key(ctrl('x'));
        p.on_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT));
        assert_eq!(p.input.text, "z");
        // Right stops at the end.
        for _ in 0..5 {
            p.on_key(key(KeyCode::Right));
        }
        assert_eq!(p.input.cursor, 1);
    }

    #[test]
    fn enter_sends_and_esc_closes() {
        let mut p = Panel::default();
        p.open();
        assert_eq!(p.on_key(key(KeyCode::Enter)), PanelKey::Nothing, "empty");
        typed(&mut p, "  what's the cloud doing?  ");
        assert_eq!(
            p.on_key(key(KeyCode::Enter)),
            PanelKey::Send(Submit {
                question: "what's the cloud doing?".into(),
                attach: true
            })
        );
        assert_eq!(p.input, Input::default());
        assert_eq!(p.busy, Some(Busy::Thinking));
        assert_eq!(p.messages[0].role, Role::User);
        // A second message waits for the answer.
        typed(&mut p, "and now?");
        assert_eq!(p.on_key(key(KeyCode::Enter)), PanelKey::Nothing);
        assert_eq!(p.input.text, "and now?");
        assert_eq!(p.on_key(key(KeyCode::Esc)), PanelKey::Close);
    }

    #[test]
    fn slash_commands() {
        let mut p = Panel::default();
        typed(&mut p, "/compact");
        assert_eq!(
            p.on_key(key(KeyCode::Enter)),
            PanelKey::Send(Submit {
                question: "/compact".into(),
                attach: false
            })
        );
        assert_eq!(p.busy, Some(Busy::Compacting));
        assert!(p.messages.is_empty(), "commands aren't conversation");
        p.apply(&json!({"type":"compacted","pre_tokens":8539,"post_tokens":1910}));
        p.apply(&json!({"type":"done","session_id":"s","compacted":true,"compactions":1}));
        assert_eq!(
            p.messages[0].text,
            "conversation compacted (8.5k → 1.9k tokens)"
        );
        assert_eq!(p.busy, None);
        typed(&mut p, "/compact");
        p.on_key(key(KeyCode::Enter));
        p.apply(&json!({"type":"done","session_id":null,"compacted":false,"compactions":0}));
        assert_eq!(p.messages[1].text, "nothing to compact yet");

        typed(&mut p, "/clear");
        assert_eq!(
            p.on_key(key(KeyCode::Enter)),
            PanelKey::Send(Submit {
                question: "/clear".into(),
                attach: false
            })
        );
        assert_eq!(p.busy, Some(Busy::Clearing));
        p.apply(&json!({"type":"cleared","previous_session_id":"s"}));
        assert!(p.messages.is_empty() && p.busy.is_none());

        // Anything else starting with / is refused here, without an ask.
        typed(&mut p, "/help");
        assert_eq!(p.on_key(key(KeyCode::Enter)), PanelKey::Nothing);
        assert_eq!(p.messages[0].role, Role::Error);
        assert!(p.busy.is_none());
    }

    #[test]
    fn stream_events_build_the_transcript() {
        let mut p = Panel::default();
        typed(&mut p, "EUR_USD?");
        p.on_key(key(KeyCode::Enter));
        for ev in [
            json!({"type":"session","session_id":"s1","resumed":false}),
            json!({"type":"status","status":"requesting"}),
            json!({"type":"text","delta":"Let me "}),
            json!({"type":"text","delta":"look."}),
            json!({"type":"tool","name":"Bash","input":{"command":"wickd chart context EUR_USD --tf 1h"}}),
        ] {
            p.apply(&ev);
        }
        assert_eq!(
            p.messages[1],
            Message {
                role: Role::Agent,
                text: "Let me look.".into()
            }
        );
        assert_eq!(
            p.activity.as_deref(),
            Some("wickd chart context EUR_USD --tf 1h")
        );
        let shown: Vec<String> = conversation_lines(&p, 60)
            .iter()
            .map(|l| l.to_string())
            .collect();
        assert!(
            shown
                .iter()
                .any(|l| l.contains("thinking…") && l.contains("wickd chart")),
            "{shown:?}"
        );
        p.apply(&json!({"type":"text","delta":"\n\nAbove the cloud."}));
        assert!(p.activity.is_none());
        let shown: Vec<String> = conversation_lines(&p, 60)
            .iter()
            .map(|l| l.to_string())
            .collect();
        assert!(
            !shown.iter().any(|l| l.contains("thinking")),
            "quiet while text arrives"
        );
        p.apply(&json!({"type":"done","answer":"Let me look.\n\nAbove the cloud.","session_id":"s1","turns":1}));
        assert_eq!(p.messages.len(), 2);
        assert_eq!(p.messages[1].text, "Let me look.\n\nAbove the cloud.");
        assert!(p.busy.is_none() && p.activity.is_none());

        // An answer that only arrives on `done` is still shown.
        typed(&mut p, "more?");
        p.on_key(key(KeyCode::Enter));
        p.apply(&json!({"type":"done","answer":"No.","session_id":"s1","turns":2}));
        assert_eq!(
            p.messages[3],
            Message {
                role: Role::Agent,
                text: "No.".into()
            }
        );
    }

    #[test]
    fn errors_show_inline() {
        let mut p = Panel::default();
        typed(&mut p, "hi");
        p.on_key(key(KeyCode::Enter));
        p.apply(&json!({"type":"error","code":"agent_ask_failed","message":"spawning claude at 'claude': not found"}));
        assert_eq!(p.messages[1].role, Role::Error);
        assert!(p.busy.is_none());
        // The exit after an error event adds nothing.
        p.finished(Err("exit status: 1".into()));
        assert_eq!(p.messages.len(), 2);

        // A child that dies without a word.
        typed(&mut p, "again");
        p.on_key(key(KeyCode::Enter));
        p.apply(&json!({"type":"text","delta":"  "}));
        p.finished(Err("no answer after 330s".into()));
        assert_eq!(
            p.messages.last().unwrap().text,
            "the agent stopped without answering: no answer after 330s"
        );
        assert!(p
            .messages
            .iter()
            .all(|m| !(m.role == Role::Agent && m.text.trim().is_empty())));
        assert!(p.busy.is_none());
    }

    #[test]
    fn transcript_survives_a_restart_and_clear_empties_it() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().to_path_buf();
        // No saved session: nothing loads, even if a transcript is there.
        save_transcript(
            &d,
            &[Message {
                role: Role::User,
                text: "old".into(),
            }],
        )
        .unwrap();
        assert!(Panel::at(d.clone()).messages.is_empty());

        crate::agent_session::save_at(
            &d,
            &crate::agent_session::Session::new("s1", "2026-11-19T10:00:00Z"),
        )
        .unwrap();
        let mut p = Panel::at(d.clone());
        p.messages.clear();
        typed(&mut p, "q1");
        p.on_key(key(KeyCode::Enter));
        p.apply(&json!({"type":"text","delta":"a1"}));
        p.apply(&json!({"type":"done","answer":"a1","session_id":"s1","turns":1}));
        let again = Panel::at(d.clone());
        assert_eq!(
            again.messages,
            vec![
                Message {
                    role: Role::User,
                    text: "q1".into()
                },
                Message {
                    role: Role::Agent,
                    text: "a1".into()
                },
            ]
        );
        assert!(again.save_error.is_none());

        let mut p = again;
        typed(&mut p, "/clear");
        p.on_key(key(KeyCode::Enter));
        p.apply(&json!({"type":"cleared","previous_session_id":"s1"}));
        assert!(load_transcript(&d).unwrap().is_empty());
    }

    #[test]
    fn scrolling_stops_at_the_top() {
        let mut p = Panel::default();
        p.max_back.set(5);
        p.page.set(10);
        p.on_key(key(KeyCode::PageUp));
        assert_eq!(p.scroll_back, 5);
        p.on_key(key(KeyCode::Down));
        assert_eq!(p.scroll_back, 4);
        p.on_key(key(KeyCode::PageDown));
        assert_eq!(p.scroll_back, 0);
    }

    #[test]
    fn wraps_and_tool_summaries() {
        assert_eq!(wrap_plain("one two three", 7), vec!["one two", "three"]);
        assert_eq!(wrap_plain("a\n\nb", 10), vec!["a", "", "b"]);
        assert_eq!(wrap_plain("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(
            tool_summary(&json!({"name":"WebSearch","input":{"query":"nvda ceo"}})),
            "nvda ceo"
        );
        assert_eq!(tool_summary(&json!({"name":"Glob","input":{}})), "Glob");
        assert_eq!(fmt_tokens(950), "950");
    }

    /// The child-process path, with a stand-in for `wickd`: a script that
    /// prints events and saves its arguments and stdin.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_ask_streams_a_childs_events() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("fake-wickd");
        let out = dir.path().join("out");
        std::fs::write(
            &exe,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{o}.args'\ncat > '{o}.stdin'\n\
                 echo '{{\"type\":\"text\",\"delta\":\"hi\"}}'\necho 'not json'\n\
                 echo '{{\"type\":\"done\",\"answer\":\"hi\"}}'\n",
                o = out.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let seen = std::sync::Mutex::new(Vec::new());
        let r = run_ask(
            &exe,
            "-is it up?",
            Some(r#"{"screen":"chart"}"#.into()),
            |v| seen.lock().unwrap().push(v),
        )
        .await;
        assert_eq!(r, Ok(()));
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert_eq!(seen[1]["type"], "done");
        let args = std::fs::read_to_string(format!("{}.args", out.display())).unwrap();
        assert_eq!(args, "agent\nask\n--context\n-\n--\n-is it up?\n");
        let stdin = std::fs::read_to_string(format!("{}.stdin", out.display())).unwrap();
        assert_eq!(stdin, r#"{"screen":"chart"}"#);

        // A child that fails without an event: its stderr is the error.
        std::fs::write(&exe, "#!/bin/sh\necho 'boom' >&2\nexit 3\n").unwrap();
        let r = run_ask(&exe, "q", None, |_| {}).await;
        assert_eq!(r, Err("boom".into()));
        // One that can't start at all.
        let r = run_ask(&dir.path().join("missing"), "q", None, |_| {}).await;
        assert!(r.unwrap_err().starts_with("couldn't start"));
    }
}
