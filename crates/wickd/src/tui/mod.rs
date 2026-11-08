//! `wickd tui` — a stock ticker for the terminal: a live watchlist, symbol
//! search, and a drill-down into each symbol's price history.
//!
//! Layout of the module:
//! - [`app`]    — pure state machine (keys → state + [`app::Action`]s)
//! - [`ui`]     — rendering
//! - [`search`] — local symbol ranking over the cached asset list
//! - [`store`]  — watchlist + asset cache under `~/.wickd`
//! - this file  — terminal lifecycle, the polling schedule, and the async
//!   fetch worker
//!
//! ## Threads
//!
//! The render/input loop is blocking (crossterm polling), so it runs on a
//! `spawn_blocking` thread exactly like `wickd dashboard`. Network calls run
//! as tokio tasks: the loop sends [`Request`]s over an unbounded tokio channel
//! (sendable from a sync thread) and drains [`Reply`]s from a std channel each
//! frame, so a slow request never freezes the UI.
//!
//! ## Request budget
//!
//! Alpaca's free plan allows 200 data calls/min. Defaults: one batched
//! snapshot call every 15s, one batched sparkline-bars call and one clock
//! call per minute, history on demand — well under 10/min at rest.

pub mod app;
pub mod candles;
pub mod keymap;
pub mod search;
pub mod store;
pub mod ui;

use std::collections::HashMap;
use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{Local, Utc};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use wickd_core::equities::alpaca::{AlpacaClient, AlpacaError};
use wickd_core::equities::{Asset, Bar, HistoryRange, MarketClock, Snapshot};

use app::{Action, App, Mode};

/// Work the run loop asks the worker to do.
#[derive(Debug)]
enum Request {
    Snapshots(Vec<String>),
    Sparks(Vec<String>),
    History(String, HistoryRange),
    Assets,
    Clock,
}

/// Results coming back to the run loop.
enum Reply {
    Snapshots(Result<HashMap<String, Snapshot>, String>),
    Sparks(Result<HashMap<String, Vec<f64>>, String>),
    History(String, HistoryRange, Result<Vec<Bar>, String>),
    Assets(Result<Vec<Asset>, String>),
    Clock(Result<MarketClock, String>),
}

pub struct Options {
    pub refresh: Duration,
}

fn err_string(e: AlpacaError) -> String {
    match e {
        AlpacaError::Auth(_) => "Alpaca rejected the keys — run `wickd alpaca login`".into(),
        AlpacaError::Forbidden(m) => format!("not allowed on this Alpaca plan ({m})"),
        other => other.to_string(),
    }
}

/// Spawn the worker: one tokio task per request so slow history never blocks
/// a quote refresh.
fn spawn_worker(client: AlpacaClient, replies: mpsc::Sender<Reply>) -> UnboundedSender<Request> {
    let (tx, mut rx) = unbounded_channel::<Request>();
    tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            let client = client.clone();
            let replies = replies.clone();
            tokio::spawn(async move {
                let reply = match req {
                    Request::Snapshots(syms) => {
                        Reply::Snapshots(client.snapshots(&syms).await.map_err(err_string))
                    }
                    Request::Sparks(syms) => {
                        let now = Utc::now();
                        let r = client
                            .bars(
                                &syms,
                                HistoryRange::Day1.timeframe(),
                                HistoryRange::Day1.start(now),
                            )
                            .await
                            .map(|m| {
                                m.into_iter()
                                    .map(|(s, bars)| {
                                        let closes = HistoryRange::Day1
                                            .trim(bars)
                                            .into_iter()
                                            .map(|b| b.close)
                                            .collect();
                                        (s, closes)
                                    })
                                    .collect()
                            })
                            .map_err(err_string);
                        Reply::Sparks(r)
                    }
                    Request::History(sym, range) => {
                        let r = client
                            .history(&sym, range, Utc::now())
                            .await
                            .map_err(err_string);
                        Reply::History(sym, range, r)
                    }
                    Request::Assets => {
                        let now = Utc::now();
                        let r = match store::cached_assets(now) {
                            Some(a) => Ok(a),
                            None => client.assets().await.map_err(err_string).map(|mut a| {
                                a.sort_by(|x, y| x.symbol.cmp(&y.symbol));
                                // Cache write failure only costs a refetch next launch.
                                let _ = store::save_assets(&a, now);
                                a
                            }),
                        };
                        Reply::Assets(r)
                    }
                    Request::Clock => Reply::Clock(client.clock().await.map_err(err_string)),
                };
                let _ = replies.send(reply);
            });
        }
    });
    tx
}

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
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));
}

/// Simple "is it time yet" timer.
struct Every {
    period: Duration,
    last: Option<Instant>,
}

impl Every {
    fn new(period: Duration) -> Self {
        Self { period, last: None }
    }
    fn due(&mut self) -> bool {
        let now = Instant::now();
        if self
            .last
            .is_none_or(|l| now.duration_since(l) >= self.period)
        {
            self.last = Some(now);
            true
        } else {
            false
        }
    }
    fn reset(&mut self) {
        self.last = None;
    }
}

/// Run the TUI until the user quits. Must be called from within a tokio
/// runtime (the worker is spawned onto it).
pub async fn run(client: AlpacaClient, watchlist: Vec<String>, opts: Options) -> Result<()> {
    let feed_label = client.feed().display_label();
    let (reply_tx, reply_rx) = mpsc::channel::<Reply>();
    let requests = spawn_worker(client, reply_tx);
    let app = App::new(watchlist, feed_label);
    tokio::task::spawn_blocking(move || run_loop(app, requests, reply_rx, opts))
        .await
        .context("tui task panicked")?
}

fn run_loop(
    mut app: App,
    requests: UnboundedSender<Request>,
    replies: mpsc::Receiver<Reply>,
    opts: Options,
) -> Result<()> {
    install_panic_hook();
    let _guard = TerminalGuard::enter()?;
    let mut terminal =
        Terminal::new(CrosstermBackend::new(io::stdout())).context("initializing terminal")?;

    let send = |r: Request| {
        let _ = requests.send(r);
    };
    send(Request::Assets);

    let mut quotes = Every::new(opts.refresh);
    let mut sparks = Every::new(Duration::from_secs(60));
    let mut clock = Every::new(Duration::from_secs(60));
    // Live 1D chart refresh while a detail view is open.
    let mut intraday = Every::new(Duration::from_secs(60));
    let mut quotes_in_flight = false;

    loop {
        // 1. Apply replies.
        while let Ok(reply) = replies.try_recv() {
            match reply {
                Reply::Snapshots(r) => {
                    quotes_in_flight = false;
                    match r {
                        Ok(map) => {
                            app.snapshots.extend(map);
                            app.quotes_error = None;
                            app.last_quotes = Some(Local::now());
                        }
                        Err(e) => app.quotes_error = Some(e),
                    }
                }
                Reply::Sparks(r) => {
                    if let Ok(map) = r {
                        app.sparks.extend(map);
                    }
                }
                Reply::History(sym, range, r) => app.set_history(&sym, range, r),
                Reply::Assets(r) => match r {
                    Ok(a) => app.set_assets(a),
                    Err(e) => app.assets_error = Some(e),
                },
                Reply::Clock(r) => {
                    if let Ok(c) = r {
                        app.clock = Some(c);
                    }
                }
            }
        }

        // 2. Scheduled polls.
        let syms = app.quote_symbols();
        if quotes.due() && !syms.is_empty() && !quotes_in_flight {
            quotes_in_flight = true;
            send(Request::Snapshots(syms));
        }
        if sparks.due() && !app.watchlist.is_empty() {
            send(Request::Sparks(app.watchlist.clone()));
        }
        // Re-check the clock on schedule and as soon as an open/close passes.
        let boundary_passed = app
            .clock
            .as_ref()
            .is_some_and(|c| Utc::now() >= if c.is_open { c.next_close } else { c.next_open });
        if clock.due() || boundary_passed {
            if boundary_passed {
                app.clock = None; // avoid re-firing every frame until the reply lands
            }
            send(Request::Clock);
        }
        if app.mode == Mode::Detail {
            let open = app.clock.as_ref().is_some_and(|c| c.is_open);
            if intraday.due() && open {
                if let Some(d) = &app.detail {
                    if d.range == HistoryRange::Day1 && d.bars().is_some() {
                        if let Some(Action::FetchHistory(s, r)) = app.refresh_history() {
                            send(Request::History(s, r));
                        }
                    }
                }
            }
        } else {
            intraday.reset();
        }

        // 3. Draw.
        terminal
            .draw(|f| ui::render(f, &app))
            .context("drawing frame")?;

        // 4. Input.
        if event::poll(Duration::from_millis(150)).context("polling for input")? {
            if let Event::Key(key) = event::read().context("reading input")? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                for action in app.on_key(key) {
                    match action {
                        Action::Quit => return Ok(()),
                        Action::RefreshQuotes => {
                            quotes.reset();
                            sparks.reset();
                        }
                        Action::FetchHistory(s, r) => send(Request::History(s, r)),
                        Action::SaveWatchlist => {
                            if let Err(e) = store::save_watchlist(&app.watchlist) {
                                app.flash = Some(format!("could not save watchlist: {e:#}"));
                            }
                        }
                    }
                }
            }
        }
    }
}
