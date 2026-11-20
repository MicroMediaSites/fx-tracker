//! `wickd tui` — a stock ticker for the terminal: a live watchlist, symbol
//! search, and a drill-down into each symbol's price history.
//!
//! Layout of the module:
//! - [`app`]    — pure state machine (keys → state + [`app::Action`]s)
//! - [`agent`]  — the agent panel (`a`): chat over `wickd agent ask`
//! - [`context`] — what's on screen, as numbers, for the agent
//! - [`ui`]     — rendering
//! - [`search`] — local symbol ranking over the cached asset list
//! - [`inplay`] — the "in play today" list (movers, actives, rvol, news)
//! - [`calendar`] — the calendar screen: upcoming economic events and earnings
//! - [`journal`] — the journal screen: markdown files from a folder
//! - [`markdown`] — markdown → terminal lines
//! - [`fx`]     — forex pairs: OANDA history and trading-day rules
//! - [`live`]   — live forex ticks from the stream daemon (client only)
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

pub mod agent;
pub mod app;
pub mod chart;
pub mod context;
pub mod fx;
pub mod inplay;
pub mod calendar;
pub mod journal;
pub mod keymap;
pub mod live;
pub mod markdown;
pub mod reader;
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

use wickd_core::equities::alpaca::{AlpacaClient, AlpacaError, NewsItem};
use wickd_core::equities::{Asset, Bar, HistoryRange, MarketClock, Snapshot};
use wickd_core::oanda::OandaClient;

use app::{Action, App, Mode};

/// Work the run loop asks the worker to do.
#[derive(Debug)]
enum Request {
    Snapshots(Vec<String>),
    Sparks(Vec<String>),
    History(String, HistoryRange),
    Assets,
    Clock,
    InPlay(Vec<String>),
    News(String),
    FxQuotes(Vec<String>),
    FxAssets,
    Info(String),
    /// Fetch these days of the earnings calendar into this folder.
    Earnings(std::path::PathBuf, Vec<chrono::NaiveDate>),
    /// Work out how these stocks reacted to their reports.
    Reactions(Vec<calendar::ReactionAsk>),
    /// Fetch and store the results headlines for these reports.
    Headlines(std::path::PathBuf, Vec<calendar::ReactionAsk>),
    /// Fetch what these companies reported into the results folder.
    Results(std::path::PathBuf, Vec<String>),
    /// Ask the agent: `wickd agent ask`, with the screen's context.
    Agent {
        question: String,
        context: Option<String>,
    },
}

/// Results coming back to the run loop.
enum Reply {
    Snapshots(Result<HashMap<String, Snapshot>, String>),
    Sparks(Result<HashMap<String, Vec<f64>>, String>),
    History(String, HistoryRange, Result<Vec<Bar>, String>),
    Assets(Result<Vec<Asset>, String>),
    Clock(Result<MarketClock, String>),
    InPlay(Result<(Vec<inplay::Row>, Vec<String>), String>),
    News(String, Result<Vec<NewsItem>, String>),
    Reader(reader::ReaderEvent),
    Warmup(String, HistoryRange, Vec<Bar>),
    FxQuotes(Result<HashMap<String, fx::FxQuote>, String>),
    FxAssets(Result<Vec<Asset>, String>),
    Live(live::LiveEvent),
    Info(String, Result<crate::info::SymbolInfo, String>),
    Earnings(Result<Vec<wickd_core::earnings_store::DayOutcome>, String>),
    Results(Result<Vec<wickd_core::earnings_results::ResultOutcome>, String>),
    Headlines((usize, usize, Option<String>)),
    /// One `wickd agent ask` event, as it arrives.
    AgentEvent(serde_json::Value),
    /// The ask's process ended.
    AgentDone(Result<(), String>),
    Reactions(
        Result<
            Vec<(
                calendar::ReactionAsk,
                wickd_core::earnings_reaction::Reaction,
            )>,
            String,
        >,
    ),
}

/// Headline lookback, shared by the list's NEWS column and the side pane so
/// the two agree.
pub const NEWS_DAYS: i64 = 4;
/// Headline requests allowed in flight at once.
const NEWS_CONCURRENCY: usize = 6;

/// Build the in-play list: movers + most-actives + watchlist, enriched with
/// snapshots, 20-day volume averages (same feed as the snapshots) and recent
/// headlines. Screener/news failures degrade to notes; a snapshot failure
/// fails the whole refresh.
async fn build_inplay(
    client: &AlpacaClient,
    watchlist: &[String],
) -> Result<(Vec<inplay::Row>, Vec<String>), String> {
    let now = Utc::now();
    let mut notes = Vec::new();
    let (movers, actives) = tokio::join!(client.movers(50), client.most_actives(50));
    let movers = movers
        .map_err(|e| notes.push(format!("top movers unavailable: {}", err_string(e))))
        .ok();
    let actives = actives
        .map_err(|e| notes.push(format!("most active unavailable: {}", err_string(e))))
        .unwrap_or_default();
    let universe = inplay::universe(movers.as_ref(), &actives, watchlist);
    let symbols: Vec<String> = universe.iter().map(|(s, _)| s.clone()).collect();

    let mut snapshots = HashMap::new();
    let mut daily = HashMap::new();
    for chunk in symbols.chunks(100) {
        let (snaps, bars) = tokio::join!(
            client.snapshots(chunk),
            client.bars_on(
                chunk,
                "1Day",
                now - chrono::Duration::days(40),
                client.volume_feed()
            )
        );
        snapshots.extend(snaps.map_err(err_string)?);
        match bars {
            Ok(b) => daily.extend(b),
            Err(e) => notes.push(format!("volume history unavailable: {}", err_string(e))),
        }
    }

    // NEWS ages are filled per symbol by the app (see App::request_news),
    // from the same per-symbol fetch the headlines pane uses.
    Ok((
        inplay::build_rows(&universe, &snapshots, &daily, &[], now),
        notes,
    ))
}

/// History for a stock chart, split into (warm-up, visible): one fetch that
/// also covers history before the window when the range has warm-up (MA
/// warm-up on daily ranges, Ichimoku warm-up on intraday ones); otherwise
/// just the range, with no warm-up. The chart and `wickd chart context` both
/// load stock history through here.
pub async fn stock_history(
    client: &AlpacaClient,
    sym: &str,
    range: HistoryRange,
    now: chrono::DateTime<Utc>,
) -> Result<(Vec<Bar>, Vec<Bar>), String> {
    if chart::warmup_days(range) > 0 {
        let start = range.start(now);
        let from = start - chrono::Duration::days(chart::warmup_days(range));
        let sym = sym.to_string();
        let all = client
            .bars(std::slice::from_ref(&sym), range.timeframe(), from)
            .await
            .map(|mut m| m.remove(&sym).unwrap_or_default())
            .map_err(err_string)?;
        Ok(chart::split_warmup(range, start, all))
    } else {
        let vis = client.history(sym, range, now).await.map_err(err_string)?;
        Ok((Vec::new(), vis))
    }
}

/// The OANDA practice client for forex data (wickd-forex D-4: read-only
/// prices; the default practice account's stored credentials).
pub fn oanda_client() -> Result<OandaClient, String> {
    let (_, client) = crate::commands::client::resolve(
        "practice",
        crate::vault_store::DEFAULT_ACCOUNT,
    )
    .map_err(|e| format!("forex needs OANDA practice credentials (`wickd login`): {e:#}"))?;
    Ok(client)
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
    // The in-play list asks for up to ~60 symbols' headlines at once; cap how
    // many run concurrently so a burst doesn't trip Alpaca's rate limits.
    let news_slots = std::sync::Arc::new(tokio::sync::Semaphore::new(NEWS_CONCURRENCY));
    // OANDA (forex pairs) is set up on first use, so a stocks-only session
    // never touches the OANDA credentials. Only a working client is kept: a
    // failed lookup is retried next time (e.g. after `wickd login`).
    let oanda: std::sync::Arc<std::sync::Mutex<Option<OandaClient>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    tokio::spawn(async move {
        while let Some(req) = rx.recv().await {
            let client = client.clone();
            let replies = replies.clone();
            let news_slots = news_slots.clone();
            let oanda = oanda.clone();
            tokio::spawn(async move {
                // Alpaca only knows stocks: forex pairs never reach it.
                let stocks = |syms: Vec<String>| -> Vec<String> {
                    syms.into_iter().filter(|s| !fx::is_fx(s)).collect()
                };
                let get_oanda = || -> Result<OandaClient, String> {
                    if let Some(c) = oanda.lock().ok().and_then(|g| g.clone()) {
                        return Ok(c);
                    }
                    oanda_client().inspect(|c| {
                        if let Ok(mut g) = oanda.lock() {
                            *g = Some(c.clone());
                        }
                    })
                };
                let reply = match req {
                    // Streams: each event is sent on as it arrives, then
                    // the exit.
                    Request::Agent { question, context } => {
                        let r = match std::env::current_exe() {
                            Ok(exe) => {
                                let tx = replies.clone();
                                agent::run_ask(&exe, &question, context, move |ev| {
                                    let _ = tx.send(Reply::AgentEvent(ev));
                                })
                                .await
                            }
                            Err(e) => Err(format!("couldn't find the wickd binary: {e}")),
                        };
                        Reply::AgentDone(r)
                    }
                    Request::Snapshots(syms) => {
                        let syms = stocks(syms);
                        if syms.is_empty() {
                            Reply::Snapshots(Ok(HashMap::new()))
                        } else {
                            Reply::Snapshots(client.snapshots(&syms).await.map_err(err_string))
                        }
                    }
                    Request::Sparks(syms) if stocks(syms.clone()).is_empty() => {
                        Reply::Sparks(Ok(HashMap::new()))
                    }
                    Request::Reactions(asks) => {
                        Reply::Reactions(calendar::fetch_reactions(&client, asks, Utc::now()).await)
                    }
                    Request::Headlines(dir, asks) => Reply::Headlines(
                        calendar::fetch_headlines(&client, &dir, asks, Utc::now()).await,
                    ),
                    Request::Results(dir, symbols) => {
                        use wickd_core::earnings_results as er;
                        Reply::Results(match wickd_core::earnings_store::http_client() {
                            Ok(http) => {
                                Ok(er::sync_results(&http, er::NASDAQ_SURPRISE_URL, &dir, &symbols)
                                    .await)
                            }
                            Err(e) => Err(e),
                        })
                    }
                    Request::Earnings(dir, dates) => {
                        use wickd_core::earnings_store as es;
                        Reply::Earnings(match es::http_client() {
                            Ok(http) => Ok(es::sync_days(
                                &http,
                                es::NASDAQ_EARNINGS_URL,
                                &dir,
                                &dates,
                            )
                            .await),
                            Err(e) => Err(e),
                        })
                    }
                    Request::Info(sym) => {
                        let r = crate::info::fetch(&sym, false)
                            .await
                            .map_err(|e| format!("{e:#}"));
                        Reply::Info(sym, r)
                    }
                    Request::FxQuotes(pairs) => Reply::FxQuotes(match get_oanda() {
                        Ok(c) => fx::quotes(&c, &pairs, Utc::now()).await,
                        Err(e) => Err(e),
                    }),
                    Request::FxAssets => {
                        let now = Utc::now();
                        Reply::FxAssets(match store::cached_fx_assets(now) {
                            Some(a) => Ok(a),
                            None => match get_oanda() {
                                Ok(c) => fx::instruments(&c).await.inspect(|a| {
                                    // Cache write failure only costs a refetch.
                                    let _ = store::save_fx_assets(a, now);
                                }),
                                Err(e) => Err(e),
                            },
                        })
                    }
                    Request::History(sym, range) if fx::is_fx(&sym) => {
                        let r = match get_oanda() {
                            Ok(c) => fx::history(&c, &sym, range, Utc::now()).await,
                            Err(e) => Err(e),
                        };
                        match r {
                            Ok((warm, vis)) => {
                                let _ = replies.send(Reply::Warmup(sym.clone(), range, warm));
                                Reply::History(sym, range, Ok(vis))
                            }
                            Err(e) => Reply::History(sym, range, Err(e)),
                        }
                    }
                    Request::News(sym) if fx::is_fx(&sym) => Reply::News(sym, Ok(Vec::new())),
                    Request::Sparks(syms) => {
                        let syms = stocks(syms);
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
                        match stock_history(&client, &sym, range, Utc::now()).await {
                            Ok((warm, vis)) => {
                                // Ranges without warm-up never sent one.
                                if chart::warmup_days(range) > 0 {
                                    let _ = replies.send(Reply::Warmup(sym.clone(), range, warm));
                                }
                                Reply::History(sym, range, Ok(vis))
                            }
                            Err(e) => Reply::History(sym, range, Err(e)),
                        }
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
                    Request::InPlay(watchlist) => {
                        Reply::InPlay(build_inplay(&client, &stocks(watchlist)).await)
                    }
                    Request::News(sym) => {
                        // Held for the request; the semaphore is never closed.
                        let _slot = news_slots.acquire().await.ok();
                        let since = Utc::now() - chrono::Duration::days(NEWS_DAYS);
                        let r = client
                            .news(std::slice::from_ref(&sym), since, 15)
                            .await
                            .map_err(err_string);
                        Reply::News(sym, r)
                    }
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
    let history_delayed = client.feed() != wickd_core::equities::alpaca::QuoteFeed::Sip;
    let (reply_tx, reply_rx) = mpsc::channel::<Reply>();
    // The reader window gets its own task so its commands stay in order.
    let (reader_tx, reader_rx) = unbounded_channel::<reader::ReaderCmd>();
    {
        use crate::commands::view::{resolve_views_root, ui_leaf_bin, views_root_candidates};
        let root = resolve_views_root(&views_root_candidates(), "reader", |p| p.exists());
        let tx = reply_tx.clone();
        tokio::spawn(reader::run(
            reader_rx,
            move |ev| {
                let _ = tx.send(Reply::Reader(ev));
            },
            ui_leaf_bin(),
            root,
        ));
    }
    // Live forex ticks from the stream daemon (client only; reconnects).
    let wanted: live::Wanted = Default::default();
    {
        let tx = reply_tx.clone();
        tokio::spawn(live::run(wanted.clone(), move |ev| {
            let _ = tx.send(Reply::Live(ev));
        }));
    }
    let budget = client.rate_limit_cell();
    let requests = spawn_worker(client, reply_tx);
    let mut app = App::new(watchlist, feed_label);
    app.history_delayed = history_delayed;
    app.api_budget = Some(budget);
    app.refresh_secs = opts.refresh.as_secs();
    app.agent = agent::Panel::at_data_home();
    tokio::task::spawn_blocking(move || run_loop(app, requests, reader_tx, reply_rx, wanted, opts))
        .await
        .context("tui task panicked")?
}

fn run_loop(
    mut app: App,
    requests: UnboundedSender<Request>,
    reader_tx: UnboundedSender<reader::ReaderCmd>,
    replies: mpsc::Receiver<Reply>,
    wanted: live::Wanted,
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
    send(Request::FxAssets);

    let mut quotes = Every::new(opts.refresh);
    let mut sparks = Every::new(Duration::from_secs(60));
    let mut clock = Every::new(Duration::from_secs(60));
    // Live 1D chart refresh while a detail view is open.
    let mut intraday = Every::new(Duration::from_secs(60));
    let mut quotes_in_flight = false;
    let mut fx_in_flight = false;
    let mut pairs_wanted: Vec<String> = Vec::new();
    // `wickd list` edits land while the TUI runs: reload lists.json when its
    // modification time changes (checked every 2s).
    let mut lists_check = Every::new(Duration::from_secs(2));
    let mut lists_seen: Option<std::time::SystemTime> = None;
    // Journal files are written by an editor or an agent while the TUI runs:
    // re-read the folder and the open entry while that screen is up.
    let mut journal_check = Every::new(Duration::from_secs(2));
    // Which stocks report today or next: re-read from the earnings store
    // (disk only) at start and every minute, so the markers follow a sync
    // made elsewhere and the change of day.
    let mut reports_check = Every::new(Duration::from_secs(60));
    // While the calendar is open: pick up slate edits made by an agent
    // (every 2s, by modification time), and re-ask for the watched stocks'
    // reactions every minute so today's figures keep moving.
    let mut slate_check = Every::new(Duration::from_secs(2));
    let mut reactions_timer = Every::new(Duration::from_secs(60));
    // Rebuild the in-play list every minute while it's on screen.
    let mut inplay_timer = Every::new(Duration::from_secs(60));

    loop {
        let mut actions: Vec<Action> = Vec::new();
        // 1. Apply replies.
        while let Ok(reply) = replies.try_recv() {
            match reply {
                Reply::Snapshots(r) => {
                    quotes_in_flight = false;
                    match r {
                        Ok(map) => {
                            app.set_snapshots(map);
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
                Reply::Warmup(sym, range, bars) => app.set_warmup(&sym, range, bars),
                Reply::FxQuotes(r) => {
                    fx_in_flight = false;
                    match r {
                        Ok(map) => {
                            app.set_fx_quotes(map);
                            app.fx_error = None;
                        }
                        Err(e) => app.fx_error = Some(e),
                    }
                }
                Reply::Info(sym, r) => app.set_info(&sym, r),
                Reply::Earnings(r) => actions.extend(app.set_earnings_fetch(r)),
                Reply::Reactions(r) => app.set_reactions(r),
                Reply::Results(r) => app.set_results(r),
                Reply::Headlines(r) => app.set_headlines(Ok(r)),
                Reply::Live(live::LiveEvent::Connected(up)) => app.fx_live = up,
                Reply::Live(live::LiveEvent::Quote(q)) => app.apply_tick(q),
                Reply::FxAssets(r) => match r {
                    Ok(a) => app.set_fx_assets(a),
                    Err(e) => app.fx_error = Some(e),
                },
                Reply::Assets(r) => match r {
                    Ok(a) => app.set_assets(a),
                    Err(e) => app.assets_error = Some(e),
                },
                Reply::Clock(r) => {
                    if let Ok(c) = r {
                        app.clock = Some(c);
                    }
                }
                Reply::InPlay(r) => actions.extend(app.set_inplay(r)),
                Reply::News(sym, r) => app.set_news(sym, r),
                Reply::Reader(ev) => actions.extend(app.set_reader(ev)),
                Reply::AgentEvent(ev) => app.agent.apply(&ev),
                Reply::AgentDone(r) => app.agent.finished(r),
            }
        }

        if lists_check.due() || lists_seen.is_none() {
            if let Ok(path) = crate::lists::lists_path() {
                let mtime = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                if lists_seen != Some(mtime) {
                    lists_seen = Some(mtime);
                    match crate::lists::load_at(&path) {
                        Ok(f) => app.set_lists(f),
                        Err(e) => app.flash = Some(format!("lists.json: {e:#}")),
                    }
                }
            }
        }

        if reports_check.due() {
            app.refresh_reports_soon();
        }

        // Keep the live feed's filter in step with the pairs on screen.
        let pairs_now = app.fx_symbols();
        if pairs_now != pairs_wanted {
            if let Ok(mut w) = wanted.lock() {
                *w = pairs_now.iter().cloned().collect();
            }
            pairs_wanted = pairs_now;
        }

        // 2. Scheduled polls.
        let syms = app.quote_symbols();
        if quotes.due() {
            if !syms.is_empty() && !quotes_in_flight {
                quotes_in_flight = true;
                send(Request::Snapshots(syms));
            }
            let pairs = app.fx_symbols();
            if !pairs.is_empty() && !fx_in_flight {
                fx_in_flight = true;
                send(Request::FxQuotes(pairs));
            }
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
        if app.mode == Mode::InPlay {
            // The first tick after opening is skipped: opening already fetched.
            if inplay_timer.due() && app.inplay.updated.is_some() && !app.inplay.loading {
                app.inplay.loading = true;
                actions.push(Action::FetchInPlay);
            }
        } else {
            inplay_timer.reset();
        }

        if app.mode == Mode::Calendar {
            if slate_check.due() && app.calendar.slate_changed() {
                app.reload_calendar();
                actions.extend(app.calendar_asks(false));
            }
            // Opening already asked. Reactions aren't asked for again while a
            // request is out; reported figures at most every half hour per
            // company.
            if reactions_timer.due() {
                actions.extend(app.calendar_asks(false));
            }
        } else {
            slate_check.reset();
            reactions_timer.reset();
        }

        if app.mode == Mode::Journal {
            if journal_check.due() {
                app.journal.poll();
            }
        } else {
            journal_check.reset();
        }

        // 3. Draw.
        terminal
            .draw(|f| ui::render(f, &app))
            .context("drawing frame")?;

        // 4. Input.
        if event::poll(Duration::from_millis(150)).context("polling for input")? {
            if let Event::Key(key) = event::read().context("reading input")? {
                if key.kind == KeyEventKind::Press {
                    actions.extend(app.on_key(key));
                }
            }
        }

        // 5. Side effects from replies and keys. A queue, because carrying
        // out one action can ask for more (a slate change asks for the new
        // company's reaction and reported figures).
        let mut queue: std::collections::VecDeque<Action> = actions.into();
        while let Some(action) = queue.pop_front() {
            match action {
                Action::Quit => {
                    if app.inplay.reader_open {
                        // Close the reader window and wait (bounded) for the
                        // reader task to confirm, instead of guessing a delay.
                        let (ack_tx, ack_rx) = mpsc::channel();
                        if reader_tx
                            .send(reader::ReaderCmd::Close(Some(ack_tx)))
                            .is_ok()
                        {
                            let _ = ack_rx.recv_timeout(Duration::from_secs(4));
                        }
                    }
                    return Ok(());
                }
                Action::ReaderShow { url, label } => {
                    let _ = reader_tx.send(reader::ReaderCmd::Show { url, label });
                }
                Action::ReaderClose => {
                    let _ = reader_tx.send(reader::ReaderCmd::Close(None));
                }
                Action::ReaderNotice(text) => {
                    let _ = reader_tx.send(reader::ReaderCmd::Notice(text));
                }
                Action::RefreshQuotes => {
                    quotes.reset();
                    sparks.reset();
                }
                Action::FetchHistory(s, r) => send(Request::History(s, r)),
                Action::FetchInPlay => send(Request::InPlay(app.watchlist.clone())),
                Action::FetchNews(s) => send(Request::News(s)),
                Action::FetchReactions(asks) => send(Request::Reactions(asks)),
                Action::FetchHeadlines(asks) => match app.calendar.earn_dir.as_deref() {
                    Some(dir) => send(Request::Headlines(
                        wickd_core::earnings_headlines::headlines_dir(dir),
                        asks,
                    )),
                    None => app.set_headlines(Err("couldn't find the wickd data folder".into())),
                },
                Action::FetchResults(symbols) => match app.calendar.earn_dir.as_deref() {
                    Some(dir) => send(Request::Results(
                        wickd_core::earnings_results::results_dir(dir),
                        symbols,
                    )),
                    None => app.set_results(Err("couldn't find the wickd data folder".into())),
                },
                Action::SlateAdd {
                    symbol,
                    date,
                    session,
                } => {
                    let r = app.calendar.slate_add(&symbol, date, session);
                    queue.extend(app.slate_written(r.map(|()| format!("{symbol} is on the slate"))));
                }
                Action::SlateRemove(symbol) => {
                    let r = app.calendar.slate_remove(&symbol);
                    queue.extend(app.slate_written(r.map(|()| format!("{symbol} is off the slate"))));
                }
                Action::FetchEarnings(dates) => match app.calendar.earn_dir.clone() {
                    Some(dir) => send(Request::Earnings(dir, dates)),
                    None => {
                        app.set_earnings_fetch(Err("couldn't find the wickd data folder".into()));
                    }
                },
                Action::SaveWatchlist => {
                    if let Err(e) = store::save_watchlist(&app.watchlist) {
                        app.flash = Some(format!("could not save watchlist: {e:#}"));
                    }
                }
                Action::FetchInfo(sym) => send(Request::Info(sym)),
                Action::AgentAsk { question, context } => {
                    send(Request::Agent { question, context })
                }
                Action::RemoveFromList { list, symbol } => {
                    // Locked read-modify-write: never clobbers an agent's edits.
                    let r = crate::lists::lists_path().and_then(|p| {
                        crate::lists::update_at(&p, |f| {
                            crate::lists::remove(f, &list, std::slice::from_ref(&symbol))
                        })
                    });
                    if let Err(e) = r {
                        app.flash = Some(format!("could not update list {list}: {e:#}"));
                    }
                    lists_seen = None; // reload to the file's truth
                }
                Action::RestoreToList { list, entry } => {
                    let r = crate::lists::lists_path().and_then(|p| {
                        crate::lists::update_at(&p, |f| {
                            let e = f.lists.entry(list.clone()).or_default();
                            if !e.iter().any(|x| x.symbol == entry.symbol) {
                                e.push(entry.clone());
                            }
                            Ok(())
                        })
                    });
                    if let Err(e) = r {
                        app.flash = Some(format!("could not update list {list}: {e:#}"));
                    }
                    lists_seen = None;
                }
            }
        }
    }
}
