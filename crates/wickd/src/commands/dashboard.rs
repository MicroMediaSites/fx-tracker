//! `wickd dashboard` — a live terminal watchlist over the price stream (AGT-616).
//!
//!   wickd dashboard
//!
//! Renders a ratatui/crossterm table (one row per instrument: bid / ask /
//! spread) that updates as prices tick. It is a pure *consumer* of the stream:
//!
//! ## Attaches to the running stream hub — never opens its own OANDA feed (AC2)
//!
//! The dashboard connects to the `wickd stream` socket hub at
//! `~/.wickd/stream.sock` (AGT-615, [`crate::stream_hub`]) and renders from the
//! byte-identical NDJSON lines it fans out. It does **not** open a competing
//! OANDA subscription — that's the whole point of the hub. If no hub is
//! answering (no `wickd stream` is running), it exits with a clear error
//! telling the operator to start `wickd stream` first, rather than silently
//! spinning up a second upstream connection.
//!
//! ## Clean terminal restore (AC3)
//!
//! Entering the alternate screen + raw mode is wrapped in a RAII
//! [`TerminalGuard`] whose `Drop` restores the terminal on every exit path —
//! normal quit (`q` / `Esc`), `Ctrl-C` (delivered as a key event in raw mode,
//! not a signal), the hub going away, *and* a panic (via a panic hook that
//! restores before the default hook prints). The user's terminal is never left
//! in raw mode or the alt screen.

use std::io::{self, BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::Args;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Terminal;

use crate::dashboard::DashboardState;
use crate::output::{exit, Out};
use crate::stream_hub;

#[derive(Args, Debug)]
pub struct DashboardArgs {}

pub async fn run(_args: DashboardArgs, out: Out) -> ! {
    match run_dashboard().await {
        Ok(()) => std::process::exit(exit::OK),
        Err(e) => {
            let msg = format!("{e:#}");
            // A missing hub is an expected, operator-actionable condition, not a
            // crash: report it as a validation error so an agent can branch on
            // the exit code.
            let code = if msg.contains("stream hub") {
                exit::VALIDATION
            } else {
                exit::GENERIC
            };
            out.fail(code, "dashboard_failed", msg);
        }
    }
}

async fn run_dashboard() -> Result<()> {
    // AC2: attach to the AGT-615 hub socket. Reuse the crate's path helper so we
    // can never drift from the real `~/.wickd/stream.sock` contract.
    let socket_path = stream_hub::stream_socket_path()?;
    let conn = UnixStream::connect(&socket_path).map_err(|e| {
        anyhow!(
            "no wickd stream hub is answering at {} ({e}) — start `wickd stream` in another \
             terminal first, then run `wickd dashboard`",
            socket_path.display()
        )
    })?;

    // The render loop is blocking (crossterm event polling + terminal I/O), so
    // run it off the async runtime rather than stalling a worker thread.
    let path = socket_path.clone();
    tokio::task::spawn_blocking(move || run_tui(conn, path))
        .await
        .context("dashboard render task panicked")?
}

/// A message from the socket-reader thread to the render loop.
enum ReaderMsg {
    Line(String),
    Disconnected,
}

/// RAII terminal guard: enters raw mode + the alternate screen on construction
/// and restores both on `Drop`, so *every* exit path (quit, Ctrl-C, hub gone,
/// early `?` return, panic-unwind) leaves the terminal as it found it (AC3).
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("enabling raw mode")?;
        execute!(io::stdout(), EnterAlternateScreen).context("entering alternate screen")?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Best-effort restore — nothing useful to do if these fail while we're
        // already tearing down, and Drop can't return an error.
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// Restore the terminal from a panic hook before the default hook prints, so a
/// panic inside the render loop doesn't leave the user in raw mode / alt screen.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));
}

fn run_tui(conn: UnixStream, socket_path: PathBuf) -> Result<()> {
    // Read NDJSON lines off the socket on a dedicated thread and hand them to
    // the render loop over a channel, so blocking socket reads never freeze the
    // UI (which must stay responsive to the quit key regardless of tick rate).
    let (tx, rx) = mpsc::channel::<ReaderMsg>();
    std::thread::spawn(move || {
        let reader = BufReader::new(conn);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if tx.send(ReaderMsg::Line(l)).is_err() {
                        return; // render loop gone
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(ReaderMsg::Disconnected);
    });

    install_panic_hook();
    let _guard = TerminalGuard::enter()?;
    let mut terminal =
        Terminal::new(CrosstermBackend::new(io::stdout())).context("initializing terminal")?;

    let mut state = DashboardState::new();
    let mut disconnected = false;

    loop {
        // Drain everything the reader has queued since the last frame.
        loop {
            match rx.try_recv() {
                Ok(ReaderMsg::Line(line)) => {
                    state.apply_line(&line);
                }
                Ok(ReaderMsg::Disconnected) => {
                    disconnected = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }

        terminal
            .draw(|frame| render(frame, &state, &socket_path, disconnected))
            .context("drawing frame")?;

        // Poll for a keypress with a short timeout so the UI still repaints as
        // ticks arrive even when the user isn't typing.
        if event::poll(Duration::from_millis(200)).context("polling for input")? {
            if let Event::Key(key) = event::read().context("reading input event")? {
                if key.kind == KeyEventKind::Press && is_quit(key.code, key.modifiers) {
                    break;
                }
            }
        }
    }

    Ok(())
    // `_guard` drops here (or on any `?` above / on panic-unwind) → terminal restored.
}

/// Quit on `q`, `Esc`, or `Ctrl-C`. In raw mode Ctrl-C is delivered as a key
/// event (CONTROL + 'c'), not a SIGINT, so we catch it here.
fn is_quit(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::Char('q') | KeyCode::Esc)
        || (modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c'))
}

fn render(
    frame: &mut ratatui::Frame,
    state: &DashboardState,
    socket_path: &Path,
    disconnected: bool,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Min(1),    // table
            Constraint::Length(1), // status/help
        ])
        .split(frame.area());

    // Title.
    let title = Paragraph::new(Line::from(vec![
        Span::styled("wickd dashboard", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("  —  {} instrument(s), {} updates", state.len(), state.updates)),
    ]));
    frame.render_widget(title, chunks[0]);

    // Watchlist table.
    let header = Row::new(["INSTRUMENT", "BID", "ASK", "SPREAD"])
        .style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED));
    let rows = state.rows().map(|r| {
        let symbol = if r.tradeable {
            r.instrument.clone()
        } else {
            format!("{} (halted)", r.instrument)
        };
        Row::new(vec![
            Cell::from(symbol),
            Cell::from(r.bid.clone()),
            Cell::from(r.ask.clone()),
            Cell::from(r.spread.clone()),
        ])
    });
    let widths = [
        Constraint::Percentage(40),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
    ];
    let block = Block::default().borders(Borders::ALL).title(" watchlist ");
    if state.is_empty() {
        // No ticks yet — show a hint inside the bordered box instead of an empty
        // headerless table, so the operator knows the attach succeeded.
        let waiting = Paragraph::new("waiting for the first tick…").block(block);
        frame.render_widget(waiting, chunks[1]);
    } else {
        let table = Table::new(rows, widths).header(header).block(block);
        frame.render_widget(table, chunks[1]);
    }

    // Status / help line.
    let status = if disconnected {
        "stream hub disconnected — press q to quit".to_string()
    } else if let Some(err) = &state.last_error {
        format!("stream error: {err}  ·  q/Esc/Ctrl-C to quit")
    } else {
        format!("attached to {}  ·  q/Esc/Ctrl-C to quit", socket_path.display())
    };
    frame.render_widget(Paragraph::new(status), chunks[2]);
}
