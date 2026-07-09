//! `wickd view` — on-demand ui-leaf views (headless otherwise).
//!
//! wickd is headless by default. `wickd view <name>` is the *only* code path
//! that opens a GUI: it spawns the `@openthink/ui-leaf` runtime (`ui-leaf
//! mount`) as a subprocess, hands it a view spec over stdio (line-delimited
//! JSON), and blocks until the window is closed. Views open as chromeless
//! Chromium `--app` windows (ui-leaf `shell: "app"` — no URL bar, no tabs),
//! falling back to a browser tab only when no Chromium browser is installed. Nothing else in the binary
//! launches a GUI, so "headless unless explicitly asked" is satisfied
//! structurally.
//!
//! ## Launch
//! ```text
//! wickd view ticket                                    # FX trade ticket, default EUR_USD
//! wickd view ticket --instrument GBP_USD
//! wickd view watcher ma-crossover EUR_USD              # live signal monitor (AGT-598)
//! wickd watch rsi EUR_USD | wickd view watcher rsi EUR_USD --stdin
//! ```
//! The retired Tauri FX-ticket window is rebuilt as the `ticket` view; the
//! `watcher` view (AGT-598) is a live monitor over the `wickd watch` daemon's
//! AGT-593 NDJSON signal stream.
//!
//! ## Watcher signal wiring (AC2 — reads the daemon's signal stream)
//! `wickd view watcher <strategy> <instrument>` reads the *exact* NDJSON the
//! `wickd watch` daemon (AGT-593) emits — `pattern-matched`, `watcher-tick`,
//! `strategy-status`, `strategy-error`, `match-status-update` — folds each line
//! into a running [`SignalState`], and pushes it to the view as an `update`
//! message (the ui-leaf v1 stdio protocol's live-data channel) so the browser
//! re-renders on every signal. Two source modes, both genuinely consuming the
//! daemon's stream (never stubbed):
//! - **default**: spawn `wickd watch <strategy> <instrument> …` as a child and
//!   forward its stdout. Single-command UX; needs OANDA creds like `wickd watch`.
//! - **`--stdin`**: read the NDJSON from this process's stdin, so you can pipe a
//!   daemon you already run: `wickd watch … | wickd view watcher … --stdin`.
//!   Credential-free at the view layer; the deterministic, offline-testable path.
//!
//! ## Teardown
//! Close the browser window/tab **or** press Ctrl-C in the terminal. Either way
//! wickd sends `{"version":"1","type":"close"}` to ui-leaf, waits for the child
//! to exit (and kills the spawned `wickd watch` child, if any), prints
//! `{"view":"<name>","status":"closed"}`, and returns. No GUI process is left
//! running.
//!
//! ## Dependency
//! Requires the `ui-leaf` binary on PATH: `npm i -g @openthink/ui-leaf`. If it
//! is absent the command fails with a structured error (it never panics).
//! Overrides: `WICKD_UI_LEAF_BIN` (binary path) and `WICKD_VIEWS_ROOT` (view
//! assets directory).

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Subcommand};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use wickd_core::config::OandaEnvironment;
use wickd_core::oanda::streaming::{
    PriceStreamer, PriceUpdate, StreamError, StreamHealthStatus,
};
use wickd_core::strategy::{
    MatchStatusUpdateEvent, PatternMatchEvent, StrategyErrorEvent, StrategyStatusEvent,
    WatcherTickEvent,
};
use wickd_core::EventSink;

use crate::alert_queue;
use crate::commands::{client, trade};
use crate::hub;
use crate::pending::{self, PendingSignal};
use crate::vault_store;

/// Most recent signal rows retained for the watcher view (newest first).
const MAX_SIGNAL_ROWS: usize = 50;

use crate::output::{exit, Out};

/// Default name of the ui-leaf binary (overridable via `WICKD_UI_LEAF_BIN`).
const UI_LEAF_BIN: &str = "ui-leaf";

/// `{"version":"1","type":"close"}` — graceful shutdown message for ui-leaf.
const CLOSE_MSG: &[u8] = b"{\"version\":\"1\",\"type\":\"close\"}\n";

/// How long to wait after a ui-leaf `disconnected` event for a `reconnected`
/// before treating the tab as genuinely closed (issue #294).
///
/// ui-leaf 1.5.0's page heartbeat can flap (beat interval == default timeout,
/// OpenThinkAi/ui-leaf#75): one late beat — timer jitter, Chrome clamping an
/// occluded `--app` window — emits `disconnected` with `reconnected` following
/// on the next beat, while the mount and window stay alive by design. Closing
/// on the first `disconnected` tears down a healthy view, so both view loops
/// debounce it: arm a grace timer, cancel on `reconnected`, and send
/// [`CLOSE_MSG`] only when the window expires. A genuinely closed tab never
/// reconnects, so the expiry path still gives the on-demand teardown UX.
const DISCONNECT_GRACE: Duration = Duration::from_secs(10);

/// Heartbeat timeout (ms) passed to `ui-leaf mount`. The page beats every 5s
/// and ui-leaf's default timeout is also 5s — zero slack, hence the #75 flap.
/// 15s (two missed beats + slack) stops the flap at the source; the
/// [`DISCONNECT_GRACE`] debounce covers whatever still gets through.
const HEARTBEAT_TIMEOUT_MS: u64 = 15_000;

#[derive(Args, Debug)]
pub struct ViewArgs {
    #[command(subcommand)]
    pub which: ViewKind,
}

#[derive(Subcommand, Debug)]
pub enum ViewKind {
    /// FX trade ticket — instrument, bid/ask/spread, buy/sell form.
    Ticket(TicketArgs),
    /// Live signal monitor over the `wickd watch` daemon (AGT-593) signal stream.
    Watcher(WatcherArgs),
}

#[derive(Args, Debug)]
pub struct TicketArgs {
    /// Instrument the ticket trades.
    #[arg(long, default_value = "EUR_USD")]
    pub instrument: String,
    /// OANDA environment for quotes and execution (practice|live). This does
    /// NOT arm real submission — the view's Live toggle triggers the AGT-613
    /// keystroke confirmation in this terminal.
    #[arg(long, default_value = "practice")]
    pub env: String,
    /// Open the ticket pre-filled from a pending strategy proposal (see
    /// `wickd pending`). The proposal's instrument takes precedence over
    /// --instrument.
    #[arg(long)]
    pub pending: Option<String>,
    /// Protocol smoke test: mount the view without opening a browser window.
    #[arg(long)]
    pub no_window: bool,
}

/// Args for `wickd view watcher` — mirror `wickd watch`'s positional + strategy
/// params so the spawned daemon child is built from the same surface.
#[derive(Args, Debug)]
pub struct WatcherArgs {
    /// Strategy to monitor: `ma-crossover` or `rsi`.
    pub strategy: String,
    /// Instrument to watch, e.g. EUR_USD.
    pub instrument: String,
    /// Candle granularity (M1, M5, M15, H1, H4, D, ...).
    #[arg(long, default_value = "H1")]
    pub granularity: String,
    /// Number of historical candles used to warm up indicators.
    #[arg(long, default_value_t = 200)]
    pub count: u32,
    /// OANDA environment whose stored credentials are used.
    #[arg(long, default_value = "practice")]
    pub env: String,

    // --- ma-crossover params (forwarded to the watch child) ---
    /// [ma-crossover] fast MA period.
    #[arg(long, default_value_t = 10)]
    pub fast: usize,
    /// [ma-crossover] slow MA period (must be > fast).
    #[arg(long, default_value_t = 30)]
    pub slow: usize,

    // --- rsi params (forwarded to the watch child) ---
    /// [rsi] lookback period.
    #[arg(long, default_value_t = 14)]
    pub period: usize,
    /// [rsi] overbought threshold.
    #[arg(long, default_value_t = 70.0)]
    pub overbought: f64,
    /// [rsi] oversold threshold.
    #[arg(long, default_value_t = 30.0)]
    pub oversold: f64,

    /// Read the daemon's NDJSON from stdin instead of spawning a `wickd watch`
    /// child (e.g. `wickd watch … | wickd view watcher … --stdin`).
    #[arg(long)]
    pub stdin: bool,
    /// Protocol smoke test: mount the view without opening a browser window.
    #[arg(long)]
    pub no_window: bool,
}

/// Why a launch could not complete. Both variants are reported as structured
/// CLI errors — neither panics.
#[derive(Debug)]
enum LaunchError {
    /// `ui-leaf` was not found on PATH.
    NotInstalled,
    /// ui-leaf spawned but the session failed at runtime.
    Runtime(String),
}

/// View name (resolves to `<views_root>/<name>.tsx`) for each kind.
fn view_name(kind: &ViewKind) -> &'static str {
    match kind {
        ViewKind::Ticket(_) => "ticket",
        ViewKind::Watcher(_) => "watcher",
    }
}

/// Mutation handler names each view may invoke back over stdio. The watcher
/// is read-only; the ticket places orders.
fn view_mutations(kind: &ViewKind) -> &'static [&'static str] {
    match kind {
        ViewKind::Ticket(_) => &["place_order", "dismiss_proposal"],
        ViewKind::Watcher(_) => &[],
    }
}

/// Whether mounting this view should open a browser window.
fn open_browser(kind: &ViewKind) -> bool {
    match kind {
        ViewKind::Ticket(a) => !a.no_window,
        ViewKind::Watcher(a) => !a.no_window,
    }
}

/// Initial app-mode window size (CSS pixels) per view: the ticket is a
/// compact always-on-screen order pad, the watcher a ~44rem table.
fn window_size(kind: &ViewKind) -> (u32, u32) {
    match kind {
        ViewKind::Ticket(_) => (300, 470),
        ViewKind::Watcher(_) => (800, 720),
    }
}

/// Window title, so the operator can tell where a chromeless window came from.
fn window_title(kind: &ViewKind) -> String {
    match kind {
        ViewKind::Ticket(a) => {
            format!("wickd ticket — {} ({})", a.instrument.replace('_', "/"), a.env)
        }
        ViewKind::Watcher(a) => {
            format!("wickd watcher — {} {}", a.strategy, a.instrument.replace('_', "/"))
        }
    }
}

/// Where the watcher view sources the daemon's NDJSON signal stream.
enum SignalSource {
    /// Spawn `wickd <args>` (i.e. `wickd watch …`) and read its stdout.
    Spawn(Vec<String>),
    /// Read the NDJSON from this process's stdin.
    Stdin,
    /// Test seam: read from an in-memory pipe. `tokio::io::stdin()` is unusable
    /// in tests — its blocking read can hang runtime shutdown on a TTY.
    #[cfg(test)]
    Reader(tokio::io::DuplexStream),
}

/// Resolve the signal source for a watcher invocation: `--stdin` reads stdin,
/// otherwise spawn a `wickd watch` child built from the same args.
fn signal_source(a: &WatcherArgs) -> SignalSource {
    if a.stdin {
        SignalSource::Stdin
    } else {
        SignalSource::Spawn(watch_child_args(a))
    }
}

/// Argv (after the binary) for the spawned `wickd watch` daemon child. Pure, so
/// it is asserted offline. Strategy-specific flags are always forwarded; the
/// `watch` command ignores the ones irrelevant to the chosen strategy.
fn watch_child_args(a: &WatcherArgs) -> Vec<String> {
    vec![
        "watch".to_string(),
        a.strategy.clone(),
        a.instrument.clone(),
        "--granularity".to_string(),
        a.granularity.clone(),
        "--count".to_string(),
        a.count.to_string(),
        "--env".to_string(),
        a.env.clone(),
        "--fast".to_string(),
        a.fast.to_string(),
        "--slow".to_string(),
        a.slow.to_string(),
        "--period".to_string(),
        a.period.to_string(),
        "--overbought".to_string(),
        a.overbought.to_string(),
        "--oversold".to_string(),
        a.oversold.to_string(),
    ]
}

/// The ui-leaf binary to invoke (`WICKD_UI_LEAF_BIN` override, else `ui-leaf`).
fn ui_leaf_bin() -> String {
    std::env::var("WICKD_UI_LEAF_BIN").unwrap_or_else(|_| UI_LEAF_BIN.to_string())
}

/// Ordered candidate locations for the bundled view assets; first that contains
/// `<view>.tsx` wins. `WICKD_VIEWS_ROOT` takes precedence over all of them.
fn views_root_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var("WICKD_VIEWS_ROOT") {
        out.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(dir.join("views")); // alongside an installed binary
            out.push(dir.join("../../crates/wickd/views")); // cargo target/<profile> layout
        }
    }
    // Source-tree location (dev builds run from the repo).
    out.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("views"));
    out
}

/// Pick the first candidate directory that contains `<view>.tsx`. Pure: the
/// caller supplies the existence check so this is unit-testable offline.
fn resolve_views_root(
    candidates: &[PathBuf],
    view: &str,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    candidates
        .iter()
        .find(|root| exists(&root.join(format!("{view}.tsx"))))
        .cloned()
}

/// Build the line-1 config object for `ui-leaf mount` (v1 stdio protocol).
/// `shell: "app"` opens a chromeless Chromium `--app` window instead of a
/// browser tab (ui-leaf falls back to a tab, with a stderr note, when no
/// Chromium browser is installed); `window` is its initial size in CSS pixels.
/// `mutations` are the handler names the view may invoke back over stdio;
/// `title` labels the window so the operator can tell where it came from;
/// `heartbeatTimeoutMs` loosens ui-leaf's flap-prone default (see
/// [`HEARTBEAT_TIMEOUT_MS`]).
fn mount_config(
    view: &str,
    views_root: &Path,
    data: Value,
    open_browser: bool,
    window: (u32, u32),
    mutations: &[&str],
    title: &str,
) -> String {
    json!({
        "version": "1",
        "view": view,
        "viewsRoot": views_root.to_string_lossy(),
        "data": data,
        "mutations": mutations,
        "port": 0,
        "openBrowser": open_browser,
        "shell": "app",
        "windowSize": { "width": window.0, "height": window.1 },
        "title": title,
        "heartbeatTimeoutMs": HEARTBEAT_TIMEOUT_MS,
    })
    .to_string()
}

/// One rendered row in the watcher's live signal log.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SignalRow {
    /// Wall-clock time the signal was observed (HH:MM:SS, for display).
    time: String,
    /// Coarse category for color/iconography: `match` / `status` / `error`.
    kind: String,
    /// Instrument the signal is about.
    instrument: String,
    /// Human-readable detail (reason / message / status text).
    label: String,
    /// `long` / `short` when the signal carries a direction.
    direction: Option<String>,
}

/// Latest watcher heartbeat — the candle the daemon just evaluated.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct TickInfo {
    time: String,
    close: String,
    signal: String,
}

/// Running state the watcher view renders. Each NDJSON line the `wickd watch`
/// daemon emits is folded into this, then pushed to ui-leaf as an `update`.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SignalState {
    strategy: String,
    instrument: String,
    granularity: String,
    /// Coarse daemon status: `starting` / `running` / `stopped` / the last
    /// `strategy-status` value the daemon reported.
    status: String,
    /// Whether the daemon is actively monitoring (false once its stream ends).
    monitoring: bool,
    /// Newest-first log of notable signals (capped at [`MAX_SIGNAL_ROWS`]).
    signals: Vec<SignalRow>,
    /// Latest watcher tick (candle heartbeat), if any.
    last_tick: Option<TickInfo>,
    /// Latest error message the daemon reported, if any.
    last_error: Option<String>,
    /// Count of ticks seen (heartbeats), for a "still alive" indicator.
    tick_count: u64,
    /// Count of pattern matches seen.
    match_count: u64,
}

impl SignalState {
    /// Initial state before any signal arrives.
    fn starting(a: &WatcherArgs) -> Self {
        Self {
            strategy: a.strategy.clone(),
            instrument: a.instrument.clone(),
            granularity: a.granularity.clone(),
            status: "starting".to_string(),
            monitoring: true,
            signals: Vec::new(),
            last_tick: None,
            last_error: None,
            tick_count: 0,
            match_count: 0,
        }
    }

    fn push_row(&mut self, row: SignalRow) {
        self.signals.insert(0, row);
        self.signals.truncate(MAX_SIGNAL_ROWS);
    }
}

/// Current wall-clock time as `HH:MM:SS` for display in signal rows.
fn now_hms() -> String {
    chrono::Utc::now().format("%H:%M:%S").to_string()
}

/// Fold one NDJSON line from the `wickd watch` daemon into the view state.
///
/// Defensive by construction: a malformed or unrecognized line is ignored (no
/// panic), so a daemon protocol addition can never crash the view. Returns
/// `true` if the line changed the state (i.e. an `update` is worth pushing).
fn fold_signal(state: &mut SignalState, line: &str) -> bool {
    let line = line.trim();
    if line.is_empty() {
        return false;
    }
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let event = match v.get("event").and_then(Value::as_str) {
        Some(e) => e,
        None => return false,
    };
    let instrument_of = |v: &Value, path: &str| {
        v.get(path)
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| state.instrument.clone())
    };

    match event {
        "watcher-tick" => {
            state.tick_count += 1;
            state.last_tick = Some(TickInfo {
                time: v
                    .get("candle_time")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(now_hms),
                close: v
                    .get("close_price")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                signal: v
                    .get("signal_result")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
            if state.status == "starting" {
                state.status = "running".to_string();
            }
            true
        }
        "pattern-matched" => {
            state.match_count += 1;
            let pm = v.get("pattern_match");
            let instrument = pm
                .and_then(|p| p.get("instrument"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| state.instrument.clone());
            let direction = pm
                .and_then(|p| p.get("direction"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let label = pm
                .and_then(|p| p.get("reason"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    v.get("strategy_name")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "pattern match".to_string());
            state.push_row(SignalRow {
                time: now_hms(),
                kind: "match".to_string(),
                instrument,
                label,
                direction,
            });
            true
        }
        "strategy-status" => {
            if let Some(s) = v.get("status").and_then(Value::as_str) {
                state.status = s.to_string();
                if s.eq_ignore_ascii_case("stopped") {
                    state.monitoring = false;
                }
            }
            let label = v
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("status: {}", state.status));
            state.push_row(SignalRow {
                time: now_hms(),
                kind: "status".to_string(),
                instrument: state.instrument.clone(),
                label,
                direction: None,
            });
            true
        }
        "strategy-error" => {
            let msg = v
                .get("message")
                .or_else(|| v.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("strategy error")
                .to_string();
            state.last_error = Some(msg.clone());
            state.push_row(SignalRow {
                time: now_hms(),
                kind: "error".to_string(),
                instrument: state.instrument.clone(),
                label: msg,
                direction: None,
            });
            true
        }
        "match-status-update" => {
            let instrument = instrument_of(&v, "instrument");
            let label = v
                .get("status")
                .and_then(Value::as_str)
                .map(|s| format!("match {s}"))
                .unwrap_or_else(|| "match status update".to_string());
            state.push_row(SignalRow {
                time: now_hms(),
                kind: "status".to_string(),
                instrument,
                label,
                direction: None,
            });
            true
        }
        // price-update / stream-health / unknown events: not surfaced as rows.
        _ => false,
    }
}

/// Serialize an `update` message (ui-leaf v1 stdio protocol live-data channel)
/// carrying the current view state.
fn update_message(state: &SignalState) -> String {
    json!({
        "version": "1",
        "type": "update",
        "data": serde_json::to_value(state).unwrap_or(Value::Null),
    })
    .to_string()
}

pub async fn run(args: ViewArgs, out: Out) -> ! {
    let kind = &args.which;
    let view = view_name(kind);

    // Locate the bundled view asset before we spawn anything.
    let candidates = views_root_candidates();
    let views_root = match resolve_views_root(&candidates, view, |p| p.exists()) {
        Some(r) => r,
        None => out.fail(
            exit::VALIDATION,
            "view_assets_missing",
            format!(
                "could not locate '{view}.tsx' view assets; set WICKD_VIEWS_ROOT to the directory holding it"
            ),
        ),
    };

    let result = match kind {
        ViewKind::Ticket(a) => {
            let env = match OandaEnvironment::from_str(&a.env) {
                Ok(e) => e,
                Err(e) => out.fail(exit::VALIDATION, "bad_env", e.to_string()),
            };
            // `--pending <id>`: seed the ticket from an existing proposal. It
            // must still be pending — a consumed signal must not become a
            // second order. The proposal's instrument wins over --instrument.
            let seed = match &a.pending {
                None => None,
                Some(pid) => match pending::get(pid) {
                    Ok(Some(sig)) if sig.status == pending::STATUS_PENDING => Some(sig),
                    Ok(Some(sig)) => out.fail(
                        exit::VALIDATION,
                        "signal_not_pending",
                        format!("pending signal '{pid}' is already {} — nothing to load", sig.status),
                    ),
                    Ok(None) => out.fail(
                        exit::VALIDATION,
                        "signal_not_found",
                        format!("no pending signal '{pid}' (see `wickd pending`)"),
                    ),
                    Err(e) => out.fail(exit::GENERIC, "view_failed", format!("{e:#}")),
                },
            };
            let instrument = seed
                .as_ref()
                .map(|s| s.instrument.clone())
                .unwrap_or_else(|| a.instrument.clone());
            let mut state = TicketState::starting(&instrument, vault_store::env_str(env));
            state.proposal = seed.as_ref().map(|s| ProposalView::from_signal(s, "launch"));
            // Persistent spread history: seed the color scale from prior
            // sessions, then keep contributing samples as quotes flow.
            let sampler = crate::spread_stats::SpreadSampler::open_default();
            if let Some(stats) = sampler.as_ref().and_then(|s| s.stats(&instrument)) {
                state.apply_spread_stats(&stats);
            }
            let (quotes_tx, quotes_rx) = unbounded_channel();
            let mut feed =
                attach_quote_feed(&mut state, env, &instrument, quotes_tx.clone()).await;
            // Surface NEW strategy signals for this instrument as loadable
            // proposals while the ticket is open.
            let _queue_tail = alert_queue::queue_path()
                .ok()
                .map(|p| spawn_queue_tail(p, instrument.clone(), quotes_tx.clone()));
            let title =
                format!("wickd ticket — {} ({})", instrument.replace('_', "/"), a.env);
            let config = mount_config(
                view,
                &views_root,
                serde_json::to_value(&state).unwrap_or(Value::Null),
                open_browser(kind),
                window_size(kind),
                view_mutations(kind),
                &title,
            );
            let r = launch_ticket(
                &ui_leaf_bin(),
                &config,
                state,
                env,
                quotes_tx,
                quotes_rx,
                &mut feed,
                sampler,
                DISCONNECT_GRACE,
            )
            .await;
            feed.shutdown();
            if let Some(t) = _queue_tail {
                t.abort();
            }
            r
        }
        ViewKind::Watcher(a) => {
            let config = mount_config(
                view,
                &views_root,
                serde_json::to_value(SignalState::starting(a)).unwrap_or(Value::Null),
                open_browser(kind),
                window_size(kind),
                view_mutations(kind),
                &window_title(kind),
            );
            launch_watcher(
                &ui_leaf_bin(),
                &config,
                signal_source(a),
                SignalState::starting(a),
                DISCONNECT_GRACE,
            )
            .await
        }
    };

    match result {
        Ok(()) => {
            out.ok(&json!({ "view": view, "status": "closed" }));
            std::process::exit(exit::OK);
        }
        Err(LaunchError::NotInstalled) => out.fail(
            exit::VALIDATION,
            "ui_leaf_not_installed",
            "ui-leaf runtime not found on PATH — install it with `npm i -g @openthink/ui-leaf` (or set WICKD_UI_LEAF_BIN)",
        ),
        Err(LaunchError::Runtime(msg)) => out.fail(exit::GENERIC, "view_failed", msg),
    }
}

/// Mount the watcher view and stream the `wickd watch` daemon's NDJSON signals
/// into it. Spawn `ui-leaf mount`, config on line 1, drain
/// events, graceful `close` on disconnect/Ctrl-C, plus a signal source:
/// each daemon line is folded into `state` and pushed to the view as an
/// `update`. The view stays alive after the signal stream ends — it is closed
/// only by the browser disconnect or Ctrl-C. `grace` is the `disconnected`
/// debounce window ([`DISCONNECT_GRACE`] in production; short in tests).
async fn launch_watcher(
    bin: &str,
    config: &str,
    source: SignalSource,
    mut state: SignalState,
    grace: Duration,
) -> Result<(), LaunchError> {
    // Spawn ui-leaf first so a missing binary is reported before we open any
    // signal source (keeps the missing-binary path a clean structured error).
    let mut child = match TokioCommand::new(bin)
        .arg("mount")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(LaunchError::NotInstalled),
        Err(e) => return Err(LaunchError::Runtime(format!("spawning {bin}: {e}"))),
    };

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| LaunchError::Runtime("ui-leaf stdin unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| LaunchError::Runtime("ui-leaf stdout unavailable".into()))?;
    let mut lines = BufReader::new(stdout).lines();

    // Open the daemon signal source: either a spawned `wickd watch` child's
    // stdout, or this process's stdin (already-running daemon, piped in).
    let mut watch_child: Option<tokio::process::Child> = None;
    let signal_reader: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match source {
        SignalSource::Stdin => Box::new(tokio::io::stdin()),
        #[cfg(test)]
        SignalSource::Reader(r) => Box::new(r),
        SignalSource::Spawn(args) => {
            let exe = std::env::current_exe()
                .map_err(|e| LaunchError::Runtime(format!("locating wickd binary: {e}")))?;
            let mut c = TokioCommand::new(exe)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(|e| LaunchError::Runtime(format!("spawning wickd watch: {e}")))?;
            let out = c
                .stdout
                .take()
                .ok_or_else(|| LaunchError::Runtime("wickd watch stdout unavailable".into()))?;
            watch_child = Some(c);
            Box::new(out)
        }
    };
    let mut signal_lines = BufReader::new(signal_reader).lines();
    let mut signals_done = false;

    // Line 1: the config object (carries the initial SignalState as `data`).
    write_line(&mut stdin, config.as_bytes()).await?;

    // `disconnected` debounce (ui-leaf#75, issue #294): armed by
    // `disconnected`, disarmed by `reconnected`, closes the view on expiry.
    let disconnect_grace = tokio::time::sleep(grace);
    tokio::pin!(disconnect_grace);
    let mut disconnect_armed = false;

    let result = loop {
        tokio::select! {
            // Ctrl-C → graceful close, then keep draining ui-leaf until `closed`.
            _ = tokio::signal::ctrl_c() => {
                let _ = stdin.write_all(CLOSE_MSG).await;
                let _ = stdin.flush().await;
            }
            // Grace expired with no `reconnected`: the tab is genuinely gone —
            // tear the mount down (on-demand UX).
            () = &mut disconnect_grace, if disconnect_armed => {
                disconnect_armed = false;
                let _ = stdin.write_all(CLOSE_MSG).await;
                let _ = stdin.flush().await;
            }
            // A daemon signal line → fold into state, push an `update` to the view.
            sig = signal_lines.next_line(), if !signals_done => {
                match sig {
                    Ok(Some(l)) => {
                        if fold_signal(&mut state, &l) {
                            let _ = write_line(&mut stdin, update_message(&state).as_bytes()).await;
                        }
                    }
                    // Stream ended (daemon exited / pipe closed): mark not-monitoring
                    // and push a final update; the view stays open until closed.
                    Ok(None) | Err(_) => {
                        signals_done = true;
                        state.monitoring = false;
                        if state.status == "starting" || state.status == "running" {
                            state.status = "stopped".to_string();
                        }
                        let _ = write_line(&mut stdin, update_message(&state).as_bytes()).await;
                    }
                }
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        let val: Value = serde_json::from_str(&l).unwrap_or(Value::Null);
                        match val.get("type").and_then(Value::as_str) {
                            // Possibly a heartbeat flap (ui-leaf#75), possibly a
                            // genuinely closed tab — start the grace window
                            // instead of closing now.
                            Some("disconnected") => {
                                disconnect_grace.as_mut().reset(tokio::time::Instant::now() + grace);
                                disconnect_armed = true;
                            }
                            // The page came back within grace: cancel the close.
                            Some("reconnected") => {
                                disconnect_armed = false;
                            }
                            Some("closed") => break Ok(()),
                            Some("error")
                                if val.get("phase").and_then(Value::as_str) == Some("runtime") =>
                            {
                                break Err(LaunchError::Runtime(l));
                            }
                            _ => { /* ready / mutate / view-swapped / build error: ignore */ }
                        }
                    }
                    Ok(None) => break Ok(()),
                    Err(e) => break Err(LaunchError::Runtime(e.to_string())),
                }
            }
        }
    };

    // Tear down both children: ask ui-leaf to exit, kill the watch daemon child.
    if let Some(mut wc) = watch_child {
        let _ = wc.start_kill();
    }
    let _ = child.wait().await;
    result
}

async fn write_line(
    stdin: &mut tokio::process::ChildStdin,
    bytes: &[u8],
) -> Result<(), LaunchError> {
    stdin
        .write_all(bytes)
        .await
        .and(stdin.write_all(b"\n").await)
        .and(stdin.flush().await)
        .map_err(|e| LaunchError::Runtime(e.to_string()))
}

// ---------------------------------------------------------------------------
// Ticket execution view: live quotes in, place_order mutations out.
// ---------------------------------------------------------------------------

/// How long to wait for the attached hub to yield a quote for the ticket's
/// instrument before falling back to a direct OANDA subscription (the hub may
/// be streaming a watchlist that doesn't cover this instrument).
const HUB_DISCOVERY_WINDOW: Duration = Duration::from_secs(6);

/// One live quote, extracted from a hub NDJSON `price-update` line or a
/// direct-stream [`PriceUpdate`] event. Keeps bid/ask/spread/tradeable — the
/// hub's `HubTick` collapses to mid and drops the fields the ticket needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Quote {
    instrument: String,
    bid: String,
    ask: String,
    spread: String,
    time: String,
    tradeable: bool,
}

/// Live state pushed to the ticket view on every quote (ui-leaf `update`).
#[derive(Debug, Clone, Serialize)]
struct TicketState {
    instrument: String,
    env: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    bid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ask: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spread: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tradeable: Option<bool>,
    /// Quote source: "connecting" | "hub" | "direct" | "none".
    feed: &'static str,
    #[serde(rename = "feedError", skip_serializing_if = "Option::is_none")]
    feed_error: Option<String>,
    /// Persisted historical spread extremes (`~/.wickd/spreads.db`) — the
    /// view grades the live spread green→red against these; absent = no
    /// history yet (purple).
    #[serde(rename = "minSpread", skip_serializing_if = "Option::is_none")]
    min_spread: Option<String>,
    #[serde(rename = "maxSpread", skip_serializing_if = "Option::is_none")]
    max_spread: Option<String>,
    /// The latest strategy-signal proposal for this instrument (auto-fill,
    /// never auto-fire: the view offers to LOAD it into the form).
    #[serde(skip_serializing_if = "Option::is_none")]
    proposal: Option<ProposalView>,
}

/// A strategy proposal as the view renders it — a [`PendingSignal`] plus
/// where it came from.
#[derive(Debug, Clone, Serialize)]
struct ProposalView {
    id: String,
    strategy: String,
    side: String,
    /// The conservative default size an approval would execute.
    units: i64,
    /// The strategy's own risk-based size, when it carried one (advisory).
    #[serde(rename = "suggestedUnits", skip_serializing_if = "Option::is_none")]
    suggested_units: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tp: Option<String>,
    #[serde(rename = "entryPrice", skip_serializing_if = "Option::is_none")]
    entry_price: Option<String>,
    reason: String,
    ts: String,
    /// "launch" (seeded by `--pending`, auto-loads) | "live" (arrived while
    /// open, offered as a chip).
    source: &'static str,
}

impl ProposalView {
    fn from_signal(s: &PendingSignal, source: &'static str) -> Self {
        ProposalView {
            id: s.id.clone(),
            strategy: s.strategy.clone(),
            side: s.side.clone(),
            units: s.units,
            suggested_units: s.suggested_units,
            sl: s.sl.clone(),
            tp: s.tp.clone(),
            entry_price: s.entry_price.clone(),
            reason: s.reason.clone(),
            ts: s.ts.clone(),
            source,
        }
    }
}

impl TicketState {
    /// Fold refreshed spread history into the state.
    fn apply_spread_stats(&mut self, stats: &crate::spread_stats::Stats) {
        self.min_spread = Some(stats.min.to_string());
        self.max_spread = Some(stats.max.to_string());
    }

    fn starting(instrument: &str, env_name: &str) -> Self {
        TicketState {
            instrument: instrument.to_string(),
            env: env_name.to_string(),
            bid: None,
            ask: None,
            spread: None,
            time: None,
            tradeable: None,
            feed: "connecting",
            feed_error: None,
            min_spread: None,
            max_spread: None,
            proposal: None,
        }
    }
}

/// Parse a hub NDJSON line into a [`Quote`]; `None` for non-price events
/// (`stream-error`, `stream-health`, signals) or malformed lines.
fn parse_quote_line(line: &str) -> Option<Quote> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("event").and_then(Value::as_str) != Some("price-update") {
        return None;
    }
    Some(Quote {
        instrument: v.get("instrument")?.as_str()?.to_string(),
        bid: v.get("bid")?.as_str()?.to_string(),
        ask: v.get("ask")?.as_str()?.to_string(),
        spread: v
            .get("spread")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        time: v
            .get("time")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        tradeable: v.get("tradeable").and_then(Value::as_bool).unwrap_or(true),
    })
}

/// Fold a quote into the ticket state. Returns true when the state changed
/// (i.e. the quote is for the ticket's instrument) so the caller knows to push
/// an `update` to the view.
fn fold_quote(state: &mut TicketState, q: &Quote) -> bool {
    if q.instrument != state.instrument {
        return false;
    }
    state.bid = Some(q.bid.clone());
    state.ask = Some(q.ask.clone());
    state.spread = Some(q.spread.clone());
    state.time = Some(q.time.clone());
    state.tradeable = Some(q.tradeable);
    true
}

/// Serialize an ui-leaf `update` message for any view data payload.
fn update_line<T: Serialize>(data: &T) -> String {
    json!({
        "version": "1",
        "type": "update",
        "data": data,
    })
    .to_string()
}

/// A `mutate` request from the view, parsed off ui-leaf's stdout channel.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MutateMsg {
    id: u64,
    name: String,
    args: Value,
}

/// Parse an already-decoded ui-leaf message as a mutation request. `None` when
/// it isn't a well-formed `mutate` message (there is then no id to reply to).
fn parse_mutate(v: &Value) -> Option<MutateMsg> {
    if v.get("type").and_then(Value::as_str) != Some("mutate") {
        return None;
    }
    Some(MutateMsg {
        id: v.get("id")?.as_u64()?,
        name: v.get("name")?.as_str()?.to_string(),
        args: v.get("args").cloned().unwrap_or(Value::Null),
    })
}

/// ui-leaf mutation success reply: resolves the view's `mutate()` promise.
fn mutate_result_line(id: u64, value: &Value) -> String {
    json!({ "version": "1", "type": "result", "id": id, "value": value }).to_string()
}

/// ui-leaf mutation failure reply: rejects the view's `mutate()` promise.
fn mutate_error_line(id: u64, message: &str) -> String {
    json!({ "version": "1", "type": "error", "id": id, "message": message }).to_string()
}

/// A validated `place_order` request from the view. `units` is signed
/// (negative = short), mirroring `trade place`.
#[derive(Debug)]
struct PlaceRequest {
    units: i64,
    kind: trade::EntryKind,
    price: Option<String>,
    sl: Option<String>,
    tp: Option<String>,
    live: bool,
    /// Pending-signal id this order executes, when the operator loaded a
    /// strategy proposal. Placing it consumes the signal (like `approve`) so
    /// one signal can never become two orders.
    signal_id: Option<String>,
}

/// Validate the view's `place_order` args. The instrument is deliberately NOT
/// taken from the args — the ticket only ever trades the instrument it was
/// launched for.
fn parse_place_request(args: &Value) -> Result<PlaceRequest, String> {
    let units = args
        .get("units")
        .and_then(Value::as_i64)
        .ok_or("place_order requires integer 'units' (negative = sell)")?;
    if units == 0 {
        return Err("'units' must be non-zero".to_string());
    }
    let kind = match args.get("type").and_then(Value::as_str).unwrap_or("market") {
        "market" => trade::EntryKind::Market,
        "limit" => trade::EntryKind::Limit,
        "stop" => trade::EntryKind::Stop,
        other => return Err(format!("unknown order type '{other}' (market|limit|stop)")),
    };
    let opt_str = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let price = opt_str("price");
    if matches!(kind, trade::EntryKind::Limit | trade::EntryKind::Stop) && price.is_none() {
        return Err(format!("a {} order requires 'price'", kind.as_str()));
    }
    Ok(PlaceRequest {
        units,
        kind,
        price,
        sl: opt_str("sl"),
        tp: opt_str("tp"),
        live: args.get("live").and_then(Value::as_bool).unwrap_or(false),
        signal_id: opt_str("signal_id"),
    })
}

/// Handle one `mutate` message. Returns an immediate reply line for
/// validation failures; otherwise spawns the guarded place path and the
/// result comes back through `results`. Only one order may be in flight at a
/// time — a live order can block on the AGT-613 terminal keystroke, and two
/// concurrent stdin prompts would interleave.
fn handle_mutate(
    m: MutateMsg,
    in_flight: &mut bool,
    results: &UnboundedSender<(u64, Result<Value, String>)>,
    env: OandaEnvironment,
    instrument: &str,
) -> Option<String> {
    if m.name != "place_order" {
        return Some(mutate_error_line(m.id, &format!("unknown mutation '{}'", m.name)));
    }
    if *in_flight {
        return Some(mutate_error_line(
            m.id,
            "an order is already in flight — wait for it to finish (a live order may be awaiting the keystroke confirmation in the terminal)",
        ));
    }
    let req = match parse_place_request(&m.args) {
        Ok(r) => r,
        Err(e) => return Some(mutate_error_line(m.id, &e)),
    };
    *in_flight = true;
    let tx = results.clone();
    let instrument = instrument.to_string();
    let id = m.id;
    tokio::spawn(async move {
        let result = match pending::pending_path() {
            Ok(p) => execute_ticket_order(env, &instrument, req, &p).await,
            Err(e) => Err(format!("{e:#}")),
        };
        let _ = tx.send((id, result));
    });
    None
}

/// Run one ticket order through the guarded path. When the order executes a
/// strategy proposal (`signal_id`), mirror `wickd approve`'s contract: refuse
/// a signal that is no longer pending, and consume it on anything but a TRUE
/// rejection — one signal, one order. `pending_path` is explicit so tests
/// never touch the real store.
async fn execute_ticket_order(
    env: OandaEnvironment,
    instrument: &str,
    req: PlaceRequest,
    pending_path: &Path,
) -> Result<Value, String> {
    // AGT-630: an order executing a strategy proposal is attributed to the
    // proposal's strategy (clientExtensions at OANDA + the audit ledger's
    // strategy column) — the signal validated here already names it. A manual
    // ticket order (no proposal link) stays unattributed.
    let mut strategy = None;
    if let Some(sid) = &req.signal_id {
        match pending::get_at(pending_path, sid) {
            Ok(Some(sig)) if sig.status == pending::STATUS_PENDING => {
                strategy = Some(sig.strategy);
            }
            Ok(Some(sig)) => {
                return Err(format!(
                    "pending signal '{sid}' is already {} — refusing to re-execute (place without the proposal link to trade anyway)",
                    sig.status
                ));
            }
            Ok(None) => return Err(format!("no pending signal '{sid}'")),
            Err(e) => return Err(format!("{e:#}")),
        }
    }

    let plan = trade::EntryPlan {
        kind: req.kind,
        price: req.price,
        tif: None,
        gtd_time: None,
        price_bound: None,
        trigger: None,
        sl: req.sl,
        tp: req.tp,
        strategy,
    };
    // The same guarded path as `wickd trade place` / `wickd approve`:
    // paper by default; live requires the AGT-613 keystroke, which
    // prompts on THIS process's terminal (stderr + stdin). No TTY →
    // fails closed. Audits identically on every path.
    let mut result = trade::execute_place(
        env,
        vault_store::DEFAULT_ACCOUNT,
        instrument,
        req.units,
        plan,
        req.live,
        false,
    )
        .await
        .map_err(|e| format!("{e:#}"))?;

    if let Some(sid) = &req.signal_id {
        let consumed = if crate::commands::approve::is_rejected(&result) {
            false
        } else {
            pending::consume_at(pending_path, sid).unwrap_or(false)
        };
        if let Some(obj) = result.as_object_mut() {
            obj.insert("signal_id".to_string(), Value::String(sid.clone()));
            obj.insert("consumed".to_string(), Value::Bool(consumed));
        }
    }
    Ok(result)
}

/// What the feeds deliver into the ticket loop: quotes, stream errors worth
/// showing the operator, or a fresh strategy-signal proposal off the alert
/// queue.
#[derive(Debug, Clone)]
enum FeedEvent {
    Quote(Quote),
    Error(String),
    Proposal(PendingSignal),
}

/// Tail the durable alert queue for NEW strategy-signal alerts on this
/// instrument and forward their proposals into the ticket loop. Seeded at the
/// current queue length so a backlog never flashes into a fresh ticket —
/// reviewing old proposals is `wickd pending` / `--pending <id>` territory.
fn spawn_queue_tail(
    queue_path: PathBuf,
    instrument: String,
    tx: UnboundedSender<FeedEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut seen = alert_queue::list_at(&queue_path).map(|e| e.len()).unwrap_or(0);
        let mut ticker = tokio::time::interval(Duration::from_millis(750));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let entries = match alert_queue::list_at(&queue_path) {
                Ok(e) => e,
                Err(_) => continue, // transient read failure; keep polling
            };
            if entries.len() <= seen {
                continue;
            }
            for entry in &entries[seen..] {
                if let Some(p) = entry.promotable_proposal() {
                    if p.instrument == instrument && tx.send(FeedEvent::Proposal(p.clone())).is_err() {
                        return; // ticket loop is gone
                    }
                }
            }
            seen = entries.len();
        }
    })
}

/// Forwards live price-updates (and stream errors) into the ticket loop's
/// feed channel; the remaining watcher events are irrelevant to the ticket
/// and dropped.
struct QuoteSink {
    tx: UnboundedSender<FeedEvent>,
}

impl EventSink for QuoteSink {
    fn price_update(&self, ev: &PriceUpdate) {
        let _ = self.tx.send(FeedEvent::Quote(Quote {
            instrument: ev.instrument.clone(),
            bid: ev.bid.clone(),
            ask: ev.ask.clone(),
            spread: ev.spread.clone(),
            time: ev.time.clone(),
            tradeable: ev.tradeable,
        }));
    }
    fn stream_error(&self, e: &StreamError) {
        let _ = self.tx.send(FeedEvent::Error(e.message.clone()));
    }
    fn pattern_matched(&self, _: &PatternMatchEvent) {}
    fn strategy_status(&self, _: &StrategyStatusEvent) {}
    fn strategy_error(&self, _: &StrategyErrorEvent) {}
    fn match_status_update(&self, _: &MatchStatusUpdateEvent) {}
    fn watcher_tick(&self, _: &WatcherTickEvent) {}
    fn stream_health(&self, _: &StreamHealthStatus) {}
}

/// Keeps the quote feed's resources alive for the window's lifetime: dropping
/// the direct streamer or aborting the hub reader kills the feed.
struct FeedGuard {
    hub_task: Option<tokio::task::JoinHandle<()>>,
    streamer: Option<PriceStreamer>,
}

impl FeedGuard {
    fn shutdown(&mut self) {
        if let Some(t) = self.hub_task.take() {
            t.abort();
        }
        if let Some(s) = self.streamer.take() {
            s.stop();
        }
    }
}

/// Read the hub socket's NDJSON fan-out and forward every price-update (and
/// relayed stream error) into the feed channel (the loop's fold filters
/// quotes by instrument).
fn spawn_hub_reader(
    stream: tokio::net::UnixStream,
    tx: UnboundedSender<FeedEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            let ev = if let Some(q) = parse_quote_line(&l) {
                Some(FeedEvent::Quote(q))
            } else {
                parse_stream_error_line(&l).map(FeedEvent::Error)
            };
            if let Some(ev) = ev {
                if tx.send(ev).is_err() {
                    break; // ticket loop is gone
                }
            }
        }
    })
}

/// Extract the message from a hub-relayed `stream-error` NDJSON line.
fn parse_stream_error_line(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("event").and_then(Value::as_str) != Some("stream-error") {
        return None;
    }
    Some(
        v.get("message")
            .and_then(Value::as_str)
            .unwrap_or("stream error")
            .to_string(),
    )
}

/// Open a direct OANDA price subscription for `instrument` (the no-hub
/// fallback). Needs stored credentials; the error string is surfaced in the
/// view as `feedError`.
async fn start_direct_feed(
    env: OandaEnvironment,
    instrument: &str,
    tx: UnboundedSender<FeedEvent>,
) -> Result<PriceStreamer, String> {
    let (env, api_key, account_id) = client::resolve_credentials(vault_store::env_str(env), vault_store::DEFAULT_ACCOUNT)
        .map_err(|e| format!("{e:#}"))?;
    let mut streamer = PriceStreamer::new(&api_key, &account_id, &env);
    streamer
        .subscribe(instrument.to_string(), Arc::new(QuoteSink { tx }))
        .await
        .map_err(|e| format!("{e:#}"))?;
    Ok(streamer)
}

/// Attach the ticket's quote feed: a running stream hub if there is one
/// (shared subscription, no credentials needed), else a direct OANDA
/// subscription, else no feed — the ticket still opens, with `feedError`
/// telling the operator how to get quotes flowing.
async fn attach_quote_feed(
    state: &mut TicketState,
    env: OandaEnvironment,
    instrument: &str,
    tx: UnboundedSender<FeedEvent>,
) -> FeedGuard {
    let mut guard = FeedGuard { hub_task: None, streamer: None };
    if let Some(h) = hub::probe_hub().await {
        state.feed = "hub";
        guard.hub_task = Some(spawn_hub_reader(h.into_stream(), tx));
        return guard;
    }
    match start_direct_feed(env, instrument, tx).await {
        Ok(s) => {
            state.feed = "direct";
            guard.streamer = Some(s);
        }
        Err(msg) => {
            state.feed = "none";
            state.feed_error = Some(format!(
                "no live quote feed: {msg} — start `wickd stream {instrument}` in another terminal, or check `wickd login --status`"
            ));
        }
    }
    guard
}

/// Mount the ticket view: quotes flow in as `update`s, `place_order`
/// mutations flow out through the guarded trade path. Mirrors
/// [`launch_watcher`]'s structure with two extra sources: the quote channel
/// and the completed-order channel. `grace` is the `disconnected` debounce
/// window ([`DISCONNECT_GRACE`] in production; short in tests).
#[allow(clippy::too_many_arguments)]
async fn launch_ticket(
    bin: &str,
    config: &str,
    mut state: TicketState,
    env: OandaEnvironment,
    quotes_tx: UnboundedSender<FeedEvent>,
    mut quotes: UnboundedReceiver<FeedEvent>,
    feed: &mut FeedGuard,
    mut sampler: Option<crate::spread_stats::SpreadSampler>,
    grace: Duration,
) -> Result<(), LaunchError> {
    let mut child = match TokioCommand::new(bin)
        .arg("mount")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => return Err(LaunchError::NotInstalled),
        Err(e) => return Err(LaunchError::Runtime(format!("spawning {bin}: {e}"))),
    };

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| LaunchError::Runtime("ui-leaf stdin unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| LaunchError::Runtime("ui-leaf stdout unavailable".into()))?;
    let mut lines = BufReader::new(stdout).lines();

    // Line 1: the config object.
    write_line(&mut stdin, config.as_bytes()).await?;

    // Completed orders come back from the spawned execution task; only one
    // may be in flight at a time (see `handle_mutate`).
    let (results_tx, mut results) = unbounded_channel::<(u64, Result<Value, String>)>();
    let mut in_flight = false;

    // Hub-coverage fallback: if the attached hub never yields our instrument
    // within the discovery window, open a direct subscription into the same
    // quote channel. One-shot.
    let mut got_quote = false;
    let fallback = tokio::time::sleep(HUB_DISCOVERY_WINDOW);
    tokio::pin!(fallback);
    let mut fallback_armed = state.feed == "hub";

    // `disconnected` debounce (ui-leaf#75, issue #294): armed by
    // `disconnected`, disarmed by `reconnected`, closes the view on expiry.
    let disconnect_grace = tokio::time::sleep(grace);
    tokio::pin!(disconnect_grace);
    let mut disconnect_armed = false;

    let result = loop {
        tokio::select! {
            // Ctrl-C → ask ui-leaf to shut down, then keep draining until `closed`.
            _ = tokio::signal::ctrl_c() => {
                let _ = stdin.write_all(CLOSE_MSG).await;
                let _ = stdin.flush().await;
            }
            // Grace expired with no `reconnected`: the tab is genuinely gone —
            // tear the mount down (on-demand UX).
            () = &mut disconnect_grace, if disconnect_armed => {
                disconnect_armed = false;
                let _ = stdin.write_all(CLOSE_MSG).await;
                let _ = stdin.flush().await;
            }
            Some(ev) = quotes.recv() => {
                let changed = match ev {
                    FeedEvent::Quote(q) => {
                        let folded = fold_quote(&mut state, &q);
                        if folded {
                            got_quote = true;
                            // A live quote supersedes any stale feed error.
                            state.feed_error = None;
                            // Contribute to the persistent spread history
                            // (throttled internally) and refresh the view's
                            // color scale when a sample lands.
                            if let Some(sm) = sampler.as_mut() {
                                if let Some(stats) = sm.on_quote(&state.instrument, &q.spread) {
                                    state.apply_spread_stats(&stats);
                                }
                            }
                        }
                        folded
                    }
                    FeedEvent::Error(msg) => {
                        state.feed_error = Some(msg);
                        true
                    }
                    // A fresh strategy signal for this instrument: offer it to
                    // the view as a loadable proposal. Latest wins — the chip
                    // is a live surface, not a queue (that's `wickd pending`).
                    FeedEvent::Proposal(sig) => {
                        state.proposal = Some(ProposalView::from_signal(&sig, "live"));
                        true
                    }
                };
                if changed {
                    let _ = write_line(&mut stdin, update_line(&state).as_bytes()).await;
                }
            }
            Some((id, r)) = results.recv() => {
                in_flight = false;
                let line = match &r {
                    Ok(v) => mutate_result_line(id, v),
                    Err(m) => mutate_error_line(id, m),
                };
                let _ = write_line(&mut stdin, line.as_bytes()).await;
            }
            () = &mut fallback, if fallback_armed => {
                fallback_armed = false;
                if !got_quote {
                    match start_direct_feed(env, &state.instrument, quotes_tx.clone()).await {
                        Ok(s) => {
                            state.feed = "direct";
                            feed.streamer = Some(s);
                        }
                        Err(msg) => {
                            state.feed_error = Some(format!(
                                "hub is attached but not streaming {}: {msg}",
                                state.instrument
                            ));
                        }
                    }
                    let _ = write_line(&mut stdin, update_line(&state).as_bytes()).await;
                }
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        let val: Value = serde_json::from_str(&l).unwrap_or(Value::Null);
                        match val.get("type").and_then(Value::as_str) {
                            Some("mutate") => {
                                if let Some(m) = parse_mutate(&val) {
                                    // dismiss_proposal mutates loop state, so it is
                                    // handled here rather than in handle_mutate.
                                    if m.name == "dismiss_proposal" {
                                        state.proposal = None;
                                        let reply = mutate_result_line(m.id, &json!({ "dismissed": true }));
                                        let _ = write_line(&mut stdin, reply.as_bytes()).await;
                                        let _ = write_line(&mut stdin, update_line(&state).as_bytes()).await;
                                    } else if let Some(reply) =
                                        handle_mutate(m, &mut in_flight, &results_tx, env, &state.instrument)
                                    {
                                        let _ = write_line(&mut stdin, reply.as_bytes()).await;
                                    }
                                }
                            }
                            // ui-leaf silently drops `update`s sent before the
                            // mount is ready — quotes that ticked during its
                            // multi-second startup would otherwise not render
                            // until the NEXT tick (a long stare at dashes in a
                            // quiet session). Re-push the current state. A
                            // `reconnected` also cancels any pending
                            // disconnect-grace close (ui-leaf#75 flap).
                            Some("ready") | Some("reconnected") => {
                                disconnect_armed = false;
                                let _ = write_line(&mut stdin, update_line(&state).as_bytes()).await;
                            }
                            // Possibly a heartbeat flap (ui-leaf#75), possibly a
                            // genuinely closed tab — start the grace window
                            // instead of closing now; only its expiry (no
                            // `reconnected`) tears the mount down.
                            Some("disconnected") => {
                                disconnect_grace.as_mut().reset(tokio::time::Instant::now() + grace);
                                disconnect_armed = true;
                            }
                            Some("closed") => break Ok(()),
                            // Runtime errors are fatal per protocol; build errors are not.
                            Some("error")
                                if val.get("phase").and_then(Value::as_str) == Some("runtime") =>
                            {
                                break Err(LaunchError::Runtime(l));
                            }
                            _ => { /* view-swapped / build error: ignore */ }
                        }
                    }
                    Ok(None) => break Ok(()), // stdout closed → child is done
                    Err(e) => break Err(LaunchError::Runtime(e.to_string())),
                }
            }
        }
    };

    let _ = child.wait().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str, ext: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "wickd-view-{}-test-{}-{}.{}",
            tag,
            std::process::id(),
            C.fetch_add(1, Ordering::Relaxed),
            ext
        ));
        p
    }

    #[test]
    fn mount_config_carries_required_protocol_fields() {
        let root = PathBuf::from("/abs/views");
        let cfg = mount_config(
            "ticket",
            &root,
            json!({ "instrument": "EUR_USD" }),
            false,
            (300, 470),
            &["place_order"],
            "wickd ticket — EUR/USD (practice)",
        );
        let v: Value = serde_json::from_str(&cfg).unwrap();
        assert_eq!(v["version"], "1");
        assert_eq!(v["view"], "ticket");
        assert_eq!(v["viewsRoot"], "/abs/views");
        assert_eq!(v["openBrowser"], false);
        assert_eq!(v["port"], 0);
        assert_eq!(v["data"]["instrument"], "EUR_USD");
        // The ticket registers its execution mutation with ui-leaf.
        assert_eq!(v["mutations"], json!(["place_order"]));
        // App-mode shell: a chromeless --app window, never a browser tab.
        assert_eq!(v["shell"], "app");
        assert_eq!(v["windowSize"]["width"], 300);
        assert_eq!(v["windowSize"]["height"], 470);
        // The window is labeled so the operator knows where it came from.
        assert_eq!(v["title"], "wickd ticket — EUR/USD (practice)");
        // Loosened heartbeat timeout so ui-leaf 1.5.0's beat-interval ==
        // timeout flap (ui-leaf#75) mostly stops at the source.
        assert_eq!(v["heartbeatTimeoutMs"], 15000);
    }

    #[test]
    fn resolve_views_root_picks_first_dir_with_view_file() {
        let cands = vec![
            PathBuf::from("/nope"),
            PathBuf::from("/yes"),
            PathBuf::from("/also"),
        ];
        let got = resolve_views_root(&cands, "ticket", |p| p == Path::new("/yes/ticket.tsx"));
        assert_eq!(got, Some(PathBuf::from("/yes")));
    }

    #[test]
    fn resolve_views_root_none_when_no_candidate_has_asset() {
        let cands = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        assert_eq!(resolve_views_root(&cands, "ticket", |_| false), None);
    }

    #[test]
    fn ticket_kind_maps_to_name_and_mutations() {
        let k = ViewKind::Ticket(TicketArgs {
            instrument: "GBP_USD".into(),
            env: "practice".into(),
            pending: None,
            no_window: true,
        });
        assert_eq!(view_name(&k), "ticket");
        assert!(!open_browser(&k)); // --no-window
        assert_eq!(view_mutations(&k), &["place_order", "dismiss_proposal"]);
        assert!(view_mutations(&ViewKind::Watcher(watcher_args())).is_empty());
    }

    #[tokio::test]
    async fn launch_ticket_missing_binary_is_structured_error_not_panic() {
        let cfg = mount_config(
            "ticket",
            Path::new("/tmp/views"),
            json!({}),
            false,
            (300, 470),
            &["place_order"],
            "wickd ticket",
        );
        let (tx, rx) = unbounded_channel();
        let mut feed = FeedGuard { hub_task: None, streamer: None };
        let err = launch_ticket(
            "wickd-no-such-ui-leaf-binary-xyz",
            &cfg,
            TicketState::starting("EUR_USD", "practice"),
            OandaEnvironment::Practice,
            tx,
            rx,
            &mut feed,
            None,
            DISCONNECT_GRACE,
        )
        .await;
        assert!(matches!(err, Err(LaunchError::NotInstalled)));
    }

    // ---- watcher view (AGT-598) ----

    fn watcher_args() -> WatcherArgs {
        WatcherArgs {
            strategy: "ma-crossover".into(),
            instrument: "EUR_USD".into(),
            granularity: "H1".into(),
            count: 200,
            env: "practice".into(),
            fast: 10,
            slow: 30,
            period: 14,
            overbought: 70.0,
            oversold: 30.0,
            stdin: false,
            no_window: false,
        }
    }

    #[test]
    fn watcher_kind_maps_to_name_and_initial_data() {
        let k = ViewKind::Watcher(watcher_args());
        assert_eq!(view_name(&k), "watcher");
        assert!(open_browser(&k)); // no --no-window
        let data = serde_json::to_value(SignalState::starting(&watcher_args())).unwrap();
        assert_eq!(data["strategy"], "ma-crossover");
        assert_eq!(data["instrument"], "EUR_USD");
        assert_eq!(data["granularity"], "H1");
        assert_eq!(data["status"], "starting");
        assert_eq!(data["monitoring"], true);
        assert!(data["signals"].as_array().unwrap().is_empty());
    }

    #[test]
    fn watcher_no_window_suppresses_browser() {
        let mut a = watcher_args();
        a.no_window = true;
        assert!(!open_browser(&ViewKind::Watcher(a)));
    }

    #[test]
    fn watcher_mount_config_targets_watcher_view_with_absolute_root() {
        let root = PathBuf::from("/abs/wickd/views");
        let kind = ViewKind::Watcher(watcher_args());
        let cfg = mount_config(
            "watcher",
            &root,
            serde_json::to_value(SignalState::starting(&watcher_args())).unwrap(),
            true,
            window_size(&kind),
            view_mutations(&kind),
            &window_title(&kind),
        );
        let v: Value = serde_json::from_str(&cfg).unwrap();
        assert_eq!(v["version"], "1");
        assert_eq!(v["view"], "watcher");
        assert_eq!(v["title"], "wickd watcher — ma-crossover EUR/USD");
        assert_eq!(v["viewsRoot"], "/abs/wickd/views");
        assert!(Path::new(v["viewsRoot"].as_str().unwrap()).is_absolute());
        assert_eq!(v["openBrowser"], true);
        assert_eq!(v["data"]["strategy"], "ma-crossover");
        assert_eq!(v["shell"], "app");
        assert_eq!(v["windowSize"]["width"], 800);
        assert_eq!(v["windowSize"]["height"], 720);
        assert_eq!(v["heartbeatTimeoutMs"], 15000);
    }

    #[test]
    fn watch_child_args_forward_strategy_and_params() {
        let a = watcher_args();
        let argv = watch_child_args(&a);
        assert_eq!(argv[0], "watch");
        assert_eq!(argv[1], "ma-crossover");
        assert_eq!(argv[2], "EUR_USD");
        // Spot-check a few forwarded flags are present as key/value pairs.
        for (flag, val) in [
            ("--granularity", "H1"),
            ("--count", "200"),
            ("--env", "practice"),
            ("--fast", "10"),
            ("--slow", "30"),
        ] {
            let i = argv.iter().position(|x| x == flag).expect("flag present");
            assert_eq!(argv[i + 1], val);
        }
    }

    #[test]
    fn signal_source_respects_stdin_flag() {
        let mut a = watcher_args();
        assert!(matches!(signal_source(&a), SignalSource::Spawn(_)));
        a.stdin = true;
        assert!(matches!(signal_source(&a), SignalSource::Stdin));
    }

    #[test]
    fn fold_pattern_matched_appends_row_and_counts() {
        let mut s = SignalState::starting(&watcher_args());
        let line = json!({
            "event": "pattern-matched",
            "strategy_name": "ma-crossover",
            "timeframe": "H1",
            "pattern_match": {
                "instrument": "GBP_USD",
                "direction": "long",
                "match_type": "entry",
                "reason": "fast SMA crossed above slow",
            }
        })
        .to_string();
        assert!(fold_signal(&mut s, &line));
        assert_eq!(s.match_count, 1);
        assert_eq!(s.signals.len(), 1);
        let row = &s.signals[0];
        assert_eq!(row.kind, "match");
        assert_eq!(row.instrument, "GBP_USD");
        assert_eq!(row.direction.as_deref(), Some("long"));
        assert_eq!(row.label, "fast SMA crossed above slow");
    }

    #[test]
    fn fold_watcher_tick_sets_last_tick_and_promotes_status() {
        let mut s = SignalState::starting(&watcher_args());
        let line = json!({
            "event": "watcher-tick",
            "config_id": "cfg-1",
            "instrument": "EUR_USD",
            "timeframe": "H1",
            "candle_time": "2024-01-01T00:00:00+00:00",
            "close_price": "1.0850",
            "signal_result": "Hold",
        })
        .to_string();
        assert!(fold_signal(&mut s, &line));
        assert_eq!(s.tick_count, 1);
        assert_eq!(s.status, "running"); // promoted from "starting"
        let tick = s.last_tick.as_ref().expect("tick");
        assert_eq!(tick.close, "1.0850");
        assert_eq!(tick.signal, "Hold");
        // Ticks are heartbeats, not log rows.
        assert!(s.signals.is_empty());
    }

    #[test]
    fn fold_strategy_status_stopped_clears_monitoring() {
        let mut s = SignalState::starting(&watcher_args());
        let line = json!({ "event": "strategy-status", "status": "stopped" }).to_string();
        assert!(fold_signal(&mut s, &line));
        assert_eq!(s.status, "stopped");
        assert!(!s.monitoring);
    }

    #[test]
    fn fold_strategy_error_records_last_error() {
        let mut s = SignalState::starting(&watcher_args());
        let line = json!({ "event": "strategy-error", "message": "boom" }).to_string();
        assert!(fold_signal(&mut s, &line));
        assert_eq!(s.last_error.as_deref(), Some("boom"));
        assert_eq!(s.signals[0].kind, "error");
    }

    #[test]
    fn fold_malformed_or_unknown_lines_are_ignored_no_panic() {
        let mut s = SignalState::starting(&watcher_args());
        assert!(!fold_signal(&mut s, "not json"));
        assert!(!fold_signal(&mut s, ""));
        assert!(!fold_signal(&mut s, &json!({ "no": "event" }).to_string()));
        assert!(!fold_signal(&mut s, &json!({ "event": "price-update", "bid": 1.0 }).to_string()));
        assert!(s.signals.is_empty());
        assert_eq!(s.status, "starting");
    }

    #[test]
    fn signal_rows_are_capped_newest_first() {
        let mut s = SignalState::starting(&watcher_args());
        for i in 0..(MAX_SIGNAL_ROWS + 10) {
            let line =
                json!({ "event": "strategy-error", "message": format!("e{i}") }).to_string();
            fold_signal(&mut s, &line);
        }
        assert_eq!(s.signals.len(), MAX_SIGNAL_ROWS);
        // Newest first: the last error pushed is at index 0.
        assert_eq!(s.signals[0].label, format!("e{}", MAX_SIGNAL_ROWS + 9));
    }

    #[test]
    fn update_message_wraps_state_in_protocol_envelope() {
        let s = SignalState::starting(&watcher_args());
        let msg = update_message(&s);
        let v: Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(v["version"], "1");
        assert_eq!(v["type"], "update");
        assert_eq!(v["data"]["strategy"], "ma-crossover");
        assert_eq!(v["data"]["status"], "starting");
    }

    #[tokio::test]
    async fn launch_watcher_missing_binary_is_structured_error_not_panic() {
        let cfg = mount_config("watcher", Path::new("/tmp/views"), json!({}), false, (800, 720), &[], "wickd watcher");
        // Stdin source so no `wickd watch` child is spawned; ui-leaf spawn fails first.
        let err = launch_watcher(
            "wickd-no-such-ui-leaf-binary-xyz",
            &cfg,
            SignalSource::Stdin,
            SignalState::starting(&watcher_args()),
            DISCONNECT_GRACE,
        )
        .await;
        assert!(matches!(err, Err(LaunchError::NotInstalled)));
    }

    // ---- disconnected debounce (issue #294 / ui-leaf#75) ----

    /// Short grace so the shim-driven debounce tests run in ~2s of wall clock.
    /// (The loops spawn a real child process, so `tokio::time::pause()` can't
    /// drive them deterministically — real, short durations instead.)
    const TEST_GRACE: Duration = Duration::from_millis(400);

    /// Write an executable bash script that stands in for `ui-leaf mount` and
    /// scripts the #75 flap: it records every stdin line wickd sends into
    /// `record`, emits `ready`, then
    ///   phase 1 — `disconnected` followed by `reconnected` well inside the
    ///   grace window, then waits past the original grace deadline (a buggy
    ///   immediate/expired close would land in the record here);
    ///   phase 2 — writes a `PHASE2` marker into the record, emits a final
    ///   `disconnected` (a genuine tab close: no reconnect ever), and waits
    ///   for wickd's `close` before answering `closed` and exiting.
    /// NB: the shim keeps its stdout open for its whole life (see the
    /// wickd-view smoke-test gotcha) — wickd treats stdout EOF as done.
    #[cfg(unix)]
    fn write_debounce_shim(record: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_path("ui-leaf-shim", "sh");
        let script = format!(
            r#"#!/bin/bash
REC="{record}"
IFS= read -r config
printf '%s\n' "$config" > "$REC"
# Record everything else wickd writes. NB: a plain `cat >>"$REC" &` gets its
# stdin implicitly reassigned to /dev/null (POSIX async-list rule) — dup the
# pipe to fd 3 first and read from that explicitly.
exec 3<&0
cat <&3 >> "$REC" &
echo '{{"version":"1","type":"ready"}}'
# Phase 1: heartbeat flap — reconnected arrives inside the grace window.
echo '{{"version":"1","type":"disconnected"}}'
sleep 0.15
echo '{{"version":"1","type":"reconnected"}}'
# Wait past the original grace deadline; a buggy close would be recorded now.
sleep 1.0
# Phase 2: genuine tab close — disconnected with no reconnect to follow.
echo 'PHASE2' >> "$REC"
echo '{{"version":"1","type":"disconnected"}}'
until grep -q '"type":"close"' "$REC"; do sleep 0.05; done
echo '{{"version":"1","type":"closed"}}'
"#,
            record = record.display()
        );
        std::fs::write(&path, script).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    /// Shared assertions on the shim's record: no close during the flap
    /// phase, exactly one close after the grace window expires unanswered.
    #[cfg(unix)]
    fn assert_debounced(record: &Path) {
        let rec = std::fs::read_to_string(record).unwrap();
        let (phase1, phase2) = rec.split_once("PHASE2").expect("marker in record");
        assert!(
            !phase1.contains(r#""type":"close""#),
            "close sent during the reconnect-within-grace phase:\n{rec}"
        );
        assert!(
            phase2.contains(r#""type":"close""#),
            "no close after the grace window expired unanswered:\n{rec}"
        );
        assert_eq!(
            rec.matches(r#""type":"close""#).count(),
            1,
            "expected exactly one close:\n{rec}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn watcher_debounces_disconnected_and_closes_only_on_grace_expiry() {
        let record = temp_path("watcher-debounce", "log");
        let shim = write_debounce_shim(&record);
        let cfg = mount_config(
            "watcher",
            Path::new("/tmp/views"),
            json!({}),
            false,
            (800, 720),
            &[],
            "wickd watcher",
        );
        // Held open so the signal branch stays pending (no daemon activity).
        let (_signals_open, reader) = tokio::io::duplex(64);
        let res = launch_watcher(
            shim.to_str().unwrap(),
            &cfg,
            SignalSource::Reader(reader),
            SignalState::starting(&watcher_args()),
            TEST_GRACE,
        )
        .await;
        assert!(res.is_ok(), "watcher run failed: {res:?}");
        assert_debounced(&record);
        let _ = std::fs::remove_file(&shim);
        let _ = std::fs::remove_file(&record);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ticket_debounces_disconnected_and_closes_only_on_grace_expiry() {
        let record = temp_path("ticket-debounce", "log");
        let shim = write_debounce_shim(&record);
        let cfg = mount_config(
            "ticket",
            Path::new("/tmp/views"),
            json!({}),
            false,
            (300, 470),
            &["place_order"],
            "wickd ticket",
        );
        let (tx, rx) = unbounded_channel();
        let mut feed = FeedGuard { hub_task: None, streamer: None };
        let res = launch_ticket(
            shim.to_str().unwrap(),
            &cfg,
            TicketState::starting("EUR_USD", "practice"),
            OandaEnvironment::Practice,
            tx,
            rx,
            &mut feed,
            None,
            TEST_GRACE,
        )
        .await;
        assert!(res.is_ok(), "ticket run failed: {res:?}");
        assert_debounced(&record);
        let _ = std::fs::remove_file(&shim);
        let _ = std::fs::remove_file(&record);
    }

    // ---- ticket execution view ----

    #[test]
    fn parse_quote_line_extracts_price_update_fields() {
        let line = r#"{"instrument":"EUR_USD","bid":"1.0850","ask":"1.0852","spread":"0.0002","time":"2026-07-01T10:30:00Z","tradeable":true,"event":"price-update"}"#;
        let q = parse_quote_line(line).expect("price-update parses");
        assert_eq!(q.instrument, "EUR_USD");
        assert_eq!(q.bid, "1.0850");
        assert_eq!(q.ask, "1.0852");
        assert_eq!(q.spread, "0.0002");
        assert!(q.tradeable);

        // Market closed rides the same event.
        let closed = line.replace("\"tradeable\":true", "\"tradeable\":false");
        assert!(!parse_quote_line(&closed).unwrap().tradeable);

        // Non-price events and garbage are ignored, not errors.
        assert!(parse_quote_line(r#"{"event":"stream-health","healthy":true}"#).is_none());
        assert!(parse_quote_line("not json").is_none());
    }

    #[test]
    fn fold_quote_filters_by_instrument_and_updates_state() {
        let mut state = TicketState::starting("EUR_USD", "practice");
        let q = Quote {
            instrument: "GBP_USD".into(),
            bid: "1.2700".into(),
            ask: "1.2702".into(),
            spread: "0.0002".into(),
            time: "t".into(),
            tradeable: true,
        };
        // Wrong instrument: no change, no update pushed.
        assert!(!fold_quote(&mut state, &q));
        assert!(state.bid.is_none());

        let q = Quote { instrument: "EUR_USD".into(), ..q };
        assert!(fold_quote(&mut state, &q));
        assert_eq!(state.bid.as_deref(), Some("1.2700"));
        assert_eq!(state.tradeable, Some(true));
    }

    #[test]
    fn ticket_state_serializes_view_contract() {
        let mut state = TicketState::starting("EUR_USD", "practice");
        let v = serde_json::to_value(&state).unwrap();
        assert_eq!(v["instrument"], "EUR_USD");
        assert_eq!(v["env"], "practice");
        assert_eq!(v["feed"], "connecting");
        // Absent quote fields are omitted, not null.
        assert!(v.get("bid").is_none());
        assert!(v.get("feedError").is_none());

        state.feed = "none";
        state.feed_error = Some("no creds".into());
        let v = serde_json::to_value(&state).unwrap();
        assert_eq!(v["feedError"], "no creds");
    }

    #[test]
    fn parse_mutate_requires_type_id_and_name() {
        let m = parse_mutate(&json!({
            "version": "1", "type": "mutate", "id": 3, "name": "place_order",
            "args": { "units": 100 }
        }))
        .expect("well-formed mutate parses");
        assert_eq!(m.id, 3);
        assert_eq!(m.name, "place_order");
        assert_eq!(m.args["units"], 100);

        assert!(parse_mutate(&json!({ "type": "ready" })).is_none());
        assert!(parse_mutate(&json!({ "type": "mutate", "name": "x" })).is_none()); // no id
        assert!(parse_mutate(&json!({ "type": "mutate", "id": 1 })).is_none()); // no name
    }

    #[test]
    fn mutation_reply_lines_match_ui_leaf_protocol() {
        let ok: Value =
            serde_json::from_str(&mutate_result_line(7, &json!({ "ok": true }))).unwrap();
        assert_eq!(ok["version"], "1");
        assert_eq!(ok["type"], "result");
        assert_eq!(ok["id"], 7);
        assert_eq!(ok["value"]["ok"], true);

        let err: Value = serde_json::from_str(&mutate_error_line(8, "nope")).unwrap();
        assert_eq!(err["version"], "1");
        assert_eq!(err["type"], "error");
        assert_eq!(err["id"], 8);
        assert_eq!(err["message"], "nope");
    }

    #[test]
    fn parse_place_request_validates_the_view_contract() {
        // Market defaults: type omitted, sl/tp/price optional, paper by default.
        let r = parse_place_request(&json!({ "units": -2500 })).unwrap();
        assert_eq!(r.units, -2500);
        assert!(matches!(r.kind, trade::EntryKind::Market));
        assert!(!r.live);
        assert!(r.price.is_none() && r.sl.is_none() && r.tp.is_none());

        // Full limit request.
        let r = parse_place_request(&json!({
            "units": 100, "type": "limit", "price": "1.0800",
            "sl": "1.0750", "tp": "1.0900", "live": true
        }))
        .unwrap();
        assert!(matches!(r.kind, trade::EntryKind::Limit));
        assert_eq!(r.price.as_deref(), Some("1.0800"));
        assert!(r.live);

        // Empty strings are treated as absent (unfilled form inputs).
        let r = parse_place_request(&json!({ "units": 100, "sl": "", "tp": "" })).unwrap();
        assert!(r.sl.is_none() && r.tp.is_none());

        // Rejections.
        assert!(parse_place_request(&json!({})).is_err()); // no units
        assert!(parse_place_request(&json!({ "units": 0 })).is_err());
        assert!(parse_place_request(&json!({ "units": 1, "type": "iceberg" })).is_err());
        let e = parse_place_request(&json!({ "units": 1, "type": "limit" })).unwrap_err();
        assert!(e.contains("requires 'price'"), "unexpected: {e}");
        let e = parse_place_request(&json!({ "units": 1, "type": "stop", "price": "" })).unwrap_err();
        assert!(e.contains("requires 'price'"), "unexpected: {e}");
    }

    // handle_mutate immediate-reply paths (nothing spawned, no audit writes):
    // unknown name, in-flight guard, invalid args. The happy path spawns the
    // real guarded place path, so it is exercised via `execute_place`'s own
    // tests, not here.
    #[tokio::test]
    async fn handle_mutate_rejects_without_spawning() {
        let (tx, mut rx) = unbounded_channel();
        let mut in_flight = false;
        let msg = |v: Value| parse_mutate(&v).expect("well-formed mutate");

        // Unknown mutation name.
        let reply = handle_mutate(
            msg(json!({ "type": "mutate", "id": 1, "name": "close_position", "args": {} })),
            &mut in_flight,
            &tx,
            OandaEnvironment::Practice,
            "EUR_USD",
        )
        .expect("immediate reply");
        assert!(reply.contains("unknown mutation"), "unexpected: {reply}");
        assert!(!in_flight);

        // Invalid args.
        let reply = handle_mutate(
            msg(json!({ "type": "mutate", "id": 2, "name": "place_order", "args": { "units": 0 } })),
            &mut in_flight,
            &tx,
            OandaEnvironment::Practice,
            "EUR_USD",
        )
        .expect("immediate reply");
        assert!(reply.contains("non-zero"), "unexpected: {reply}");
        assert!(!in_flight);

        // In-flight guard refuses a second order.
        in_flight = true;
        let reply = handle_mutate(
            msg(json!({ "type": "mutate", "id": 3, "name": "place_order", "args": { "units": 100 } })),
            &mut in_flight,
            &tx,
            OandaEnvironment::Practice,
            "EUR_USD",
        )
        .expect("immediate reply");
        assert!(reply.contains("already in flight"), "unexpected: {reply}");

        // Nothing was spawned: the results channel stays empty.
        assert!(rx.try_recv().is_err());
    }

    // A proposal-linked order refuses a signal that is missing or no longer
    // pending, mirroring `wickd approve` — one signal can never become two
    // orders. Temp store only; the guarded place path is never reached.
    #[tokio::test]
    async fn ticket_order_refuses_missing_or_consumed_signal() {
        let ppath = temp_path("pending", "json");

        // Unknown signal id → refused before any placement.
        let req =
            parse_place_request(&json!({ "units": 100, "signal_id": "nope" })).unwrap();
        let err = execute_ticket_order(OandaEnvironment::Practice, "EUR_USD", req, &ppath)
            .await
            .unwrap_err();
        assert!(err.contains("no pending signal"), "unexpected: {err}");

        // Consumed signal → refused with its status named.
        let sig = PendingSignal {
            id: "sig-consumed".into(),
            ts: "t".into(),
            instrument: "EUR_USD".into(),
            side: "long".into(),
            units: 1000,
            suggested_units: None,
            strategy: "ma-crossover".into(),
            reason: "r".into(),
            sl: None,
            tp: None,
            entry_price: None,
            status: pending::STATUS_PENDING.into(),
        };
        pending::append_at(&ppath, &sig).unwrap();
        assert!(pending::consume_at(&ppath, "sig-consumed").unwrap());
        let req =
            parse_place_request(&json!({ "units": 100, "signal_id": "sig-consumed" })).unwrap();
        let err = execute_ticket_order(OandaEnvironment::Practice, "EUR_USD", req, &ppath)
            .await
            .unwrap_err();
        assert!(err.contains("already consumed"), "unexpected: {err}");

        let _ = std::fs::remove_file(&ppath);
    }

    #[test]
    fn parse_place_request_carries_signal_link() {
        let r = parse_place_request(&json!({ "units": 100, "signal_id": "abc" })).unwrap();
        assert_eq!(r.signal_id.as_deref(), Some("abc"));
        // Absent or empty → no link.
        let r = parse_place_request(&json!({ "units": 100, "signal_id": "" })).unwrap();
        assert!(r.signal_id.is_none());
    }

    #[test]
    fn proposal_view_serializes_signal_fields() {
        let sig = PendingSignal {
            id: "sig-1".into(),
            ts: "2026-07-01T00:00:00+00:00".into(),
            instrument: "EUR_USD".into(),
            side: "long".into(),
            units: 1000,
            suggested_units: Some(5000),
            strategy: "ma-crossover".into(),
            reason: "fast SMA crossed above slow".into(),
            sl: Some("1.0800".into()),
            tp: Some("1.0950".into()),
            entry_price: Some("1.0850".into()),
            status: "pending".into(),
        };
        let v = serde_json::to_value(ProposalView::from_signal(&sig, "live")).unwrap();
        assert_eq!(v["id"], "sig-1");
        assert_eq!(v["strategy"], "ma-crossover");
        assert_eq!(v["side"], "long");
        assert_eq!(v["units"], 1000);
        assert_eq!(v["suggestedUnits"], 5000);
        assert_eq!(v["sl"], "1.0800");
        assert_eq!(v["entryPrice"], "1.0850");
        assert_eq!(v["source"], "live");
    }
}
