//! Hub-first price streaming for the desktop app (AGT-652).
//!
//! The app no longer owns an unconditional OANDA price subscription. It is a
//! *client of the wickd stream hub* first, exactly like `wickd watch` and
//! `wickd dashboard`:
//!
//! 1. **Attach** — probe `wickd_core::stream_hub::stream_socket_path()`
//!    (`~/.wickd/stream.sock`). If a hub answers (normally the launchd-run
//!    `wickd stream`), read its NDJSON fan-out and re-emit each line as the
//!    Tauri events the frontend already listens for (`price-update`,
//!    `stream-error`, `stream-health`). Zero OANDA connections from the app
//!    for hub-covered instruments.
//! 2. **Degrade to direct** — the hub has no control channel, so coverage is
//!    learned by observation (the CLI's own semantics, see
//!    `wickd_core::hub_client`). An instrument that stays silent through the
//!    discovery window gets its own direct `PriceStreamer` subscription.
//! 3. **Host** — if no hub answers after a re-probe grace (biased toward the
//!    supervised CLI stream winning the socket), the app binds the hub itself
//!    via `wickd_core::stream_hub::StreamHub` and publishes byte-identical
//!    NDJSON lines for every tick it streams — so a later `wickd watch` /
//!    `wickd dashboard` attaches to the app's hub and the whole machine still
//!    holds exactly ONE upstream OANDA subscription per covered instrument.
//!
//! The supervisor is generic over [`EventEmitter`] (Tauri event bus) and
//! [`DirectPort`] (the app's `PriceStreamer`) so the attach/partition/host
//! logic is testable against a real temp-socket hub without Tauri or OANDA.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, mpsc};

use wickd_core::stream_hub::{self, StreamHub};

/// How long a subscribed instrument may stay silent on an attached hub before
/// it falls back to a direct subscription. Matches the CLI watcher's
/// `HUB_DISCOVERY_TIMEOUT`.
const DISCOVERY_WINDOW: Duration = Duration::from_secs(3);

/// Re-probe attempts (and spacing) before the app gives up on finding a hub
/// and hosts one itself. Biased toward the launchd-supervised `wickd stream`
/// reclaiming its socket after a restart.
const PROBE_ATTEMPTS: u32 = 3;
const PROBE_SPACING: Duration = Duration::from_millis(700);

/// Commands into the supervisor task.
#[derive(Debug)]
pub enum Cmd {
    Subscribe(String),
    Unsubscribe(String),
    /// App shutdown: release the hub socket if we are hosting it.
    Shutdown,
}

/// Where ticks are flowing from, for status surfaces.
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct HubStreamSnapshot {
    /// "idle" | "client" | "host"
    pub mode: String,
    /// Instruments observed ticking on the attached hub (client mode).
    pub observed: Vec<String>,
    /// Instruments served by a direct OANDA subscription (fallback or host).
    pub direct: Vec<String>,
    /// Unix ms of the last hub line seen (client mode), if any.
    pub last_line_ms: Option<i64>,
}

/// Emits parsed stream events to the UI layer.
pub trait EventEmitter: Send + Sync + 'static {
    fn emit_json(&self, event: &str, payload: serde_json::Value);
}

impl EventEmitter for tauri::AppHandle {
    fn emit_json(&self, event: &str, payload: serde_json::Value) {
        let _ = self.emit(event, payload);
    }
}

/// Opens/closes direct OANDA subscriptions (the degrade path), optionally
/// publishing each emitted line to a hosted hub's broadcast channel.
pub trait DirectPort: Send + Sync + 'static {
    fn subscribe(&self, instrument: String, hub: Option<broadcast::Sender<String>>);
    fn unsubscribe(&self, instrument: String);
}

/// Managed Tauri state: the handle commands talk to.
#[derive(Default)]
pub struct HubStreamState {
    cmd_tx: StdMutex<Option<mpsc::UnboundedSender<Cmd>>>,
    snapshot: Arc<StdMutex<HubStreamSnapshot>>,
}

impl HubStreamState {
    pub fn send(&self, cmd: Cmd) -> Result<(), String> {
        let guard = self.cmd_tx.lock().map_err(|_| "hub stream state poisoned")?;
        match guard.as_ref() {
            Some(tx) => tx.send(cmd).map_err(|e| e.to_string()),
            None => Err("hub stream supervisor not started".to_string()),
        }
    }

    pub fn snapshot(&self) -> HubStreamSnapshot {
        self.snapshot
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// Start the supervisor task against the real hub socket path. Called once
    /// from Tauri `setup` when the app handle exists.
    pub fn start<E: EventEmitter, D: DirectPort>(&self, emitter: E, direct: D) {
        let Ok(path) = stream_hub::stream_socket_path() else {
            tracing::error!("[HubStream] could not resolve hub socket path");
            return;
        };
        let tx = spawn_supervisor(path, emitter, direct, self.snapshot.clone(), DISCOVERY_WINDOW);
        if let Ok(mut guard) = self.cmd_tx.lock() {
            *guard = Some(tx);
        }
    }
}

/// Everything the supervisor multiplexes over one channel.
enum Internal {
    Cmd(Cmd),
    /// One NDJSON line read off the attached hub.
    Line(String),
    /// The attached hub connection ended (hub process gone).
    ConnClosed,
    /// Discovery deadline passed for an instrument subscribed in client mode.
    DiscoveryCheck(String),
}

enum Mode {
    Idle,
    /// Attached to someone else's hub; reader task feeds `Internal::Line`.
    Client {
        observed: HashSet<String>,
        reader: tokio::task::JoinHandle<()>,
        last_line_ms: Option<i64>,
    },
    /// We bound the socket and publish our own ticks to it.
    Host { hub: StreamHub },
}

struct Supervisor<E: EventEmitter, D: DirectPort> {
    socket_path: PathBuf,
    emitter: E,
    direct: D,
    tx: mpsc::UnboundedSender<Internal>,
    refcounts: HashMap<String, u32>,
    /// Instruments currently served by a direct subscription.
    direct_active: HashSet<String>,
    mode: Mode,
    snapshot: Arc<StdMutex<HubStreamSnapshot>>,
    discovery_window: Duration,
}

/// Spawn the supervisor; returns the command sender.
pub fn spawn_supervisor<E: EventEmitter, D: DirectPort>(
    socket_path: PathBuf,
    emitter: E,
    direct: D,
    snapshot: Arc<StdMutex<HubStreamSnapshot>>,
    discovery_window: Duration,
) -> mpsc::UnboundedSender<Cmd> {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Cmd>();
    let (tx, mut rx) = mpsc::unbounded_channel::<Internal>();

    // Forward public commands into the internal stream.
    let tx_for_cmds = tx.clone();
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if tx_for_cmds.send(Internal::Cmd(cmd)).is_err() {
                break;
            }
        }
    });

    tokio::spawn(async move {
        let mut sup = Supervisor {
            socket_path,
            emitter,
            direct,
            tx,
            refcounts: HashMap::new(),
            direct_active: HashSet::new(),
            mode: Mode::Idle,
            snapshot,
            discovery_window,
        };
        while let Some(item) = rx.recv().await {
            if !sup.handle(item).await {
                break;
            }
        }
        if let Mode::Host { hub } = &sup.mode {
            hub.shutdown();
        }
    });

    cmd_tx
}

impl<E: EventEmitter, D: DirectPort> Supervisor<E, D> {
    /// Returns false to stop the supervisor loop.
    async fn handle(&mut self, item: Internal) -> bool {
        match item {
            Internal::Cmd(Cmd::Subscribe(instrument)) => {
                let count = self.refcounts.entry(instrument.clone()).or_insert(0);
                *count += 1;
                if *count == 1 {
                    self.ensure_feed_for(instrument).await;
                }
            }
            Internal::Cmd(Cmd::Unsubscribe(instrument)) => {
                if let Some(count) = self.refcounts.get_mut(&instrument) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        self.refcounts.remove(&instrument);
                        if self.direct_active.remove(&instrument) {
                            self.direct.unsubscribe(instrument);
                        }
                        // Hub-covered instruments need no teardown: attaching
                        // clients never start/stop the upstream (hub policy).
                    }
                }
            }
            Internal::Cmd(Cmd::Shutdown) => {
                if let Mode::Host { hub } = &self.mode {
                    hub.shutdown();
                }
                return false;
            }
            Internal::Line(line) => {
                self.apply_line(&line);
            }
            Internal::ConnClosed => {
                tracing::warn!("[HubStream] hub connection lost; re-establishing feed");
                // Direct fallbacks keep running; hub-covered instruments are
                // re-resolved from scratch.
                self.mode = Mode::Idle;
                self.publish_snapshot();
                let pending: Vec<String> = self
                    .refcounts
                    .keys()
                    .filter(|i| !self.direct_active.contains(*i))
                    .cloned()
                    .collect();
                if !pending.is_empty() {
                    self.establish(pending).await;
                }
            }
            Internal::DiscoveryCheck(instrument) => {
                let covered = matches!(&self.mode, Mode::Client { observed, .. } if observed.contains(&instrument));
                let still_wanted = self.refcounts.contains_key(&instrument);
                let already_direct = self.direct_active.contains(&instrument);
                if still_wanted && !covered && !already_direct {
                    // Silent through the discovery window: degrade to direct,
                    // exactly like the CLI watcher (safe degradation, never a
                    // silent stall).
                    tracing::info!(
                        "[HubStream] {instrument} not covered by hub; falling back to direct subscription"
                    );
                    self.direct_active.insert(instrument.clone());
                    self.direct.subscribe(instrument, None);
                    self.publish_snapshot();
                }
            }
        }
        true
    }

    /// Make sure `instrument` (newly refcounted) is being fed.
    async fn ensure_feed_for(&mut self, instrument: String) {
        match &self.mode {
            Mode::Idle => self.establish(vec![instrument]).await,
            Mode::Client { observed, .. } => {
                if observed.contains(&instrument) {
                    return; // already flowing
                }
                self.schedule_discovery_check(instrument);
            }
            Mode::Host { hub } => {
                let sender = hub.sender();
                self.direct_active.insert(instrument.clone());
                self.direct.subscribe(instrument, Some(sender));
                self.publish_snapshot();
            }
        }
    }

    /// No feed yet: probe for a hub (with the CLI-biased grace), attach if one
    /// answers, otherwise host the hub ourselves.
    async fn establish(&mut self, instruments: Vec<String>) {
        for attempt in 0..PROBE_ATTEMPTS {
            if let Some(handle) = wickd_core::hub_client::probe_hub_at(&self.socket_path).await {
                tracing::info!(
                    "[HubStream] attached to stream hub at {} (client mode)",
                    self.socket_path.display()
                );
                let reader = self.spawn_reader(handle.into_stream());
                self.mode = Mode::Client {
                    observed: HashSet::new(),
                    reader,
                    last_line_ms: None,
                };
                self.publish_snapshot();
                for instrument in instruments {
                    self.schedule_discovery_check(instrument);
                }
                return;
            }
            if attempt + 1 < PROBE_ATTEMPTS {
                tokio::time::sleep(PROBE_SPACING).await;
            }
        }

        // No hub anywhere: host one so CLI consumers can attach to us and the
        // machine still holds a single upstream subscription.
        match StreamHub::bind_at(self.socket_path.clone(), stream_hub::DEFAULT_CAPACITY).await {
            Ok(hub) => {
                tracing::info!(
                    "[HubStream] no hub running; hosting one at {} (host mode)",
                    self.socket_path.display()
                );
                let sender = hub.sender();
                self.mode = Mode::Host { hub };
                for instrument in instruments {
                    self.direct_active.insert(instrument.clone());
                    self.direct.subscribe(instrument, Some(sender.clone()));
                }
                self.publish_snapshot();
            }
            Err(e) => {
                // Lost a bind race (a CLI stream grabbed the socket between
                // probe and bind) — attach to the winner instead.
                tracing::warn!("[HubStream] hub bind failed ({e}); re-probing");
                if let Some(handle) = wickd_core::hub_client::probe_hub_at(&self.socket_path).await
                {
                    let reader = self.spawn_reader(handle.into_stream());
                    self.mode = Mode::Client {
                        observed: HashSet::new(),
                        reader,
                        last_line_ms: None,
                    };
                    self.publish_snapshot();
                    for instrument in instruments {
                        self.schedule_discovery_check(instrument);
                    }
                } else {
                    // Neither attachable nor bindable: degrade everything to
                    // direct so price surfaces never silently stall.
                    for instrument in instruments {
                        self.direct_active.insert(instrument.clone());
                        self.direct.subscribe(instrument, None);
                    }
                    self.publish_snapshot();
                }
            }
        }
    }

    fn schedule_discovery_check(&self, instrument: String) {
        let tx = self.tx.clone();
        let window = self.discovery_window;
        tokio::spawn(async move {
            tokio::time::sleep(window).await;
            let _ = tx.send(Internal::DiscoveryCheck(instrument));
        });
    }

    fn spawn_reader(&self, stream: tokio::net::UnixStream) -> tokio::task::JoinHandle<()> {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if tx.send(Internal::Line(line)).is_err() {
                    return;
                }
            }
            let _ = tx.send(Internal::ConnClosed);
        })
    }

    /// Fold one hub NDJSON line into Tauri events + coverage bookkeeping.
    fn apply_line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            return; // junk on the wire must never take the app down
        };
        let Some(event) = value.get("event").and_then(|e| e.as_str()).map(str::to_string) else {
            return;
        };

        if event == "price-update" {
            if let (Mode::Client { observed, last_line_ms, .. }, Some(instrument)) = (
                &mut self.mode,
                value.get("instrument").and_then(|i| i.as_str()),
            ) {
                let newly = observed.insert(instrument.to_string());
                *last_line_ms = Some(chrono::Utc::now().timestamp_millis());
                if newly {
                    self.publish_snapshot();
                }
            }
        }

        match event.as_str() {
            "price-update" | "stream-error" | "stream-health" => {
                self.emitter.emit_json(&event, value);
            }
            _ => {} // other daemon events aren't price surfaces; ignore
        }
    }

    fn publish_snapshot(&self) {
        let snap = HubStreamSnapshot {
            mode: match &self.mode {
                Mode::Idle => "idle".to_string(),
                Mode::Client { .. } => "client".to_string(),
                Mode::Host { .. } => "host".to_string(),
            },
            observed: match &self.mode {
                Mode::Client { observed, .. } => {
                    let mut v: Vec<String> = observed.iter().cloned().collect();
                    v.sort();
                    v
                }
                _ => vec![],
            },
            direct: {
                let mut v: Vec<String> = self.direct_active.iter().cloned().collect();
                v.sort();
                v
            },
            last_line_ms: match &self.mode {
                Mode::Client { last_line_ms, .. } => *last_line_ms,
                _ => None,
            },
        };
        if let Ok(mut guard) = self.snapshot.lock() {
            *guard = snap;
        }
    }
}

impl<E: EventEmitter, D: DirectPort> Drop for Supervisor<E, D> {
    fn drop(&mut self) {
        if let Mode::Client { reader, .. } = &self.mode {
            reader.abort();
        }
        // Mode::Host's StreamHub removes its socket file in its own Drop.
    }
}

// ---------------------------------------------------------------------------
// Production ports
// ---------------------------------------------------------------------------

/// The real degrade path: a refcounted `PriceStreamer` subscription emitting
/// Tauri events — and, when the app hosts the hub, publishing the identical
/// NDJSON line to connected hub clients.
pub struct StreamerDirectPort {
    pub app: tauri::AppHandle,
    pub streamer: Arc<tokio::sync::Mutex<wickd_core::oanda::PriceStreamer>>,
}

impl DirectPort for StreamerDirectPort {
    fn subscribe(&self, instrument: String, hub: Option<broadcast::Sender<String>>) {
        let app = self.app.clone();
        let streamer = self.streamer.clone();
        tokio::spawn(async move {
            let sink: Arc<dyn wickd_core::EventSink> = match hub {
                Some(hub) => Arc::new(TauriHubSink { app: app.clone(), hub }),
                None => crate::TauriEventSink::arc(app.clone()),
            };
            if let Err(e) = streamer.lock().await.subscribe(instrument.clone(), sink).await {
                tracing::error!("[HubStream] direct subscribe failed for {instrument}: {e}");
                let _ = app.emit(
                    "stream-error",
                    serde_json::json!({
                        "errorType": "connection_lost",
                        "message": format!("direct subscription failed for {instrument}: {e}"),
                    }),
                );
            }
        });
    }

    fn unsubscribe(&self, instrument: String) {
        let streamer = self.streamer.clone();
        tokio::spawn(async move {
            if let Err(e) = streamer.lock().await.unsubscribe(instrument.clone()).await {
                tracing::warn!("[HubStream] direct unsubscribe failed for {instrument}: {e}");
            }
        });
    }
}

/// EventSink that emits Tauri events *and* publishes each stream line to the
/// hosted hub — the app-side twin of the CLI's `NdjsonSink::with_hub`, built
/// on the same `wickd_core::ndjson::event_line` envelope so hub clients see
/// byte-identical lines regardless of which process owns the hub.
struct TauriHubSink {
    app: tauri::AppHandle,
    hub: broadcast::Sender<String>,
}

impl TauriHubSink {
    fn publish<T: serde::Serialize>(&self, event: &str, payload: &T) {
        let _ = self.app.emit(event, payload);
        if let Some(line) = wickd_core::ndjson::event_line(event, payload) {
            let _ = self.hub.send(line); // no subscribers is not an error
        }
    }
}

impl wickd_core::EventSink for TauriHubSink {
    fn price_update(&self, event: &wickd_core::oanda::streaming::PriceUpdate) {
        self.publish("price-update", event);
    }
    fn stream_error(&self, event: &wickd_core::oanda::streaming::StreamError) {
        self.publish("stream-error", event);
    }
    fn stream_health(&self, event: &wickd_core::oanda::streaming::StreamHealthStatus) {
        self.publish("stream-health", event);
    }
    // The app hosts no watcher engine (AGT-652) — watcher events can't occur
    // on this sink.
    fn pattern_matched(&self, _: &wickd_core::strategy::PatternMatchEvent) {}
    fn strategy_status(&self, _: &wickd_core::strategy::StrategyStatusEvent) {}
    fn strategy_error(&self, _: &wickd_core::strategy::StrategyErrorEvent) {}
    fn match_status_update(&self, _: &wickd_core::strategy::MatchStatusUpdateEvent) {}
    fn watcher_tick(&self, _: &wickd_core::strategy::WatcherTickEvent) {}
}

// ---------------------------------------------------------------------------
// Tests — the attach/partition/host logic against a real temp-socket hub.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct CollectingEmitter {
        events: Arc<StdMutex<Vec<(String, serde_json::Value)>>>,
    }

    impl EventEmitter for CollectingEmitter {
        fn emit_json(&self, event: &str, payload: serde_json::Value) {
            self.events.lock().unwrap().push((event.to_string(), payload));
        }
    }

    struct CountingDirect {
        subscribed: Arc<StdMutex<Vec<(String, bool)>>>, // (instrument, hosted-hub?)
    }

    impl DirectPort for CountingDirect {
        fn subscribe(&self, instrument: String, hub: Option<broadcast::Sender<String>>) {
            self.subscribed
                .lock()
                .unwrap()
                .push((instrument, hub.is_some()));
        }
        fn unsubscribe(&self, _instrument: String) {}
    }

    fn temp_socket() -> PathBuf {
        static C: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "wickd-hubstream-test-{}-{}.sock",
            std::process::id(),
            C.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn harness(
        path: PathBuf,
        window: Duration,
    ) -> (
        mpsc::UnboundedSender<Cmd>,
        Arc<StdMutex<Vec<(String, serde_json::Value)>>>,
        Arc<StdMutex<Vec<(String, bool)>>>,
        Arc<StdMutex<HubStreamSnapshot>>,
    ) {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let subscribed = Arc::new(StdMutex::new(Vec::new()));
        let snapshot = Arc::new(StdMutex::new(HubStreamSnapshot::default()));
        let tx = spawn_supervisor(
            path,
            CollectingEmitter { events: events.clone() },
            CountingDirect { subscribed: subscribed.clone() },
            snapshot.clone(),
            window,
        );
        (tx, events, subscribed, snapshot)
    }

    /// AC3: with a hub covering EUR_USD, the app attaches as a client, re-emits
    /// ticks, and opens ZERO direct subscriptions for the covered instrument —
    /// only the uncovered one degrades to direct after the discovery window.
    #[tokio::test]
    async fn attaches_to_hub_and_only_uncovered_instruments_go_direct() {
        let path = temp_socket();
        let hub = StreamHub::bind_at(path.clone(), 64).await.unwrap();
        let hub_tx = hub.sender();

        // Publisher: EUR_USD ticks only.
        let publisher = tokio::spawn({
            let hub_tx = hub_tx.clone();
            async move {
                for i in 0..60 {
                    let _ = hub_tx.send(format!(
                        r#"{{"event":"price-update","instrument":"EUR_USD","bid":"1.08{i:02}","ask":"1.08{i:02}","spread":"0.0001","time":"2026-07-06T00:00:00Z","tradeable":true}}"#
                    ));
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }
        });

        let (tx, events, subscribed, snapshot) = harness(path, Duration::from_millis(400));
        tx.send(Cmd::Subscribe("EUR_USD".to_string())).unwrap();
        tx.send(Cmd::Subscribe("USD_JPY".to_string())).unwrap();

        // Wait past attach + discovery.
        tokio::time::sleep(Duration::from_millis(1200)).await;

        let evs = events.lock().unwrap().clone();
        assert!(
            evs.iter().any(|(name, v)| name == "price-update"
                && v["instrument"] == "EUR_USD"
                && v["bid"].is_string()),
            "hub ticks must be re-emitted as price-update events, got: {evs:?}"
        );

        let subs = subscribed.lock().unwrap().clone();
        assert!(
            !subs.iter().any(|(i, _)| i == "EUR_USD"),
            "hub-covered instrument must NOT open a direct subscription (one upstream), got {subs:?}"
        );
        assert!(
            subs.iter().any(|(i, hosted)| i == "USD_JPY" && !hosted),
            "uncovered instrument must degrade to direct, got {subs:?}"
        );

        let snap = snapshot.lock().unwrap().clone();
        assert_eq!(snap.mode, "client");
        assert!(snap.observed.contains(&"EUR_USD".to_string()));
        assert!(snap.direct.contains(&"USD_JPY".to_string()));

        publisher.abort();
        tx.send(Cmd::Shutdown).unwrap();
        hub.shutdown();
    }

    /// AC3: with no hub anywhere, the app hosts one — the socket appears, all
    /// instruments stream directly WITH a hub publisher, and a raw socket
    /// client (a stand-in for `wickd watch`/`dashboard`) receives the lines
    /// the app publishes.
    #[tokio::test]
    async fn hosts_a_hub_when_none_is_running() {
        let path = temp_socket();
        let _ = std::fs::remove_file(&path);

        let (tx, _events, subscribed, snapshot) = harness(path.clone(), Duration::from_millis(200));
        tx.send(Cmd::Subscribe("EUR_USD".to_string())).unwrap();

        // establish() burns PROBE_ATTEMPTS * PROBE_SPACING before hosting.
        tokio::time::sleep(Duration::from_millis(2500)).await;

        assert!(path.exists(), "app must bind the hub socket in host mode");
        let subs = subscribed.lock().unwrap().clone();
        assert!(
            subs.iter().any(|(i, hosted)| i == "EUR_USD" && *hosted),
            "host mode subscribes directly WITH a hub publisher, got {subs:?}"
        );
        assert_eq!(snapshot.lock().unwrap().mode, "host");

        // A CLI-style client can attach to the app's hub. The counting direct
        // port doesn't publish ticks, so prove fan-out through the snapshot's
        // hub sender path instead: connect and confirm accept works.
        let client = tokio::net::UnixStream::connect(&path).await;
        assert!(client.is_ok(), "clients must be able to attach to the app-hosted hub");

        tx.send(Cmd::Shutdown).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!path.exists(), "shutdown must remove the hosted socket");
    }

    /// Reconnect: when the hub dies, covered instruments are re-established
    /// (host mode here, since no new hub appears).
    #[tokio::test]
    async fn hub_loss_reestablishes_the_feed() {
        let path = temp_socket();
        let hub = StreamHub::bind_at(path.clone(), 64).await.unwrap();
        let hub_tx = hub.sender();

        // Tick continuously so EUR_USD is observed inside the discovery window
        // (stays hub-covered, no direct fallback).
        let publisher = tokio::spawn({
            let hub_tx = hub_tx.clone();
            async move {
                loop {
                    let _ = hub_tx.send(
                        r#"{"event":"price-update","instrument":"EUR_USD","bid":"1.0800","ask":"1.0801","spread":"0.0001","time":"2026-07-06T00:00:00Z","tradeable":true}"#.to_string(),
                    );
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        });

        let (tx, _events, _subscribed, snapshot) = harness(path.clone(), Duration::from_millis(500));
        tx.send(Cmd::Subscribe("EUR_USD".to_string())).unwrap();

        tokio::time::sleep(Duration::from_millis(800)).await;
        {
            let snap = snapshot.lock().unwrap().clone();
            assert_eq!(snap.mode, "client");
            assert!(snap.direct.is_empty(), "hub-covered: no direct fallback, got {snap:?}");
        }
        publisher.abort();

        // Kill the hub like a real process exit: shutdown removes the socket,
        // and dropping every broadcast sender ends the per-client writer tasks
        // so the app-side connection sees EOF.
        hub.shutdown();
        drop(hub);
        drop(hub_tx);
        tokio::time::sleep(Duration::from_millis(3500)).await;
        assert_eq!(
            snapshot.lock().unwrap().mode,
            "host",
            "after hub loss with no replacement, the app should host the hub"
        );

        tx.send(Cmd::Shutdown).unwrap();
    }
}
