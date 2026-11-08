//! News reader window for `wickd tui`.
//!
//! The in-play headlines pane opens an article in a chromeless ui-leaf window.
//! Once it's open, focus is in the window, so the window has its own keys:
//! **ctrl+shift+→ / ←** load the next / previous headline (a small badge in
//! the corner shows where you are). The TUI's own j/k still work too.
//!
//! ## How
//!
//! 1. Spawn `ui-leaf mount` with the `reader` view (a placeholder page),
//!    `shell: "app"`, an initial window size, a throwaway Chrome profile (no
//!    `profile` field → ui-leaf uses a temp dir and deletes it on unmount),
//!    and `debugPort` on a free loopback port.
//! 2. When ui-leaf reports `ready`, attach to that Chrome over the DevTools
//!    protocol (`GET /json/list` → the page's WebSocket), enable the Page and
//!    Runtime domains and add a `__wickdNav` binding.
//! 3. Each article: register a script for the next document (keyboard
//!    handler + badge, calling `__wickdNav("next"|"prev")`), then
//!    `Page.navigate`. Binding calls arrive as `Runtime.bindingCalled` events
//!    and become [`ReaderEvent::Nav`]. The window shows the real article page
//!    (framing it doesn't work: Benzinga sends `X-Frame-Options: SAMEORIGIN`).
//!
//! ## Security
//!
//! The DevTools port has no authentication; any local process can control
//! the window while it's open. That's why the profile is throwaway (no
//! cookies or logins to steal) and the port is loopback-only and random. A
//! page can call the binding itself, so only `"next"`/`"prev"` are accepted.
//!
//! ## Lifecycle
//!
//! A user closing the window may or may not produce a timely `closed` from
//! ui-leaf, so a navigation that finds the window gone drops it, opens a
//! fresh one and retries. Each window has an id so a late `closed` from an
//! old window can't tear down a new one. Quitting the TUI closes the window.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Initial window size in CSS pixels (tall reading column).
pub const WINDOW: (u32, u32) = (1000, 1000);
/// How long to wait for ui-leaf to report `ready`.
const READY_TIMEOUT: Duration = Duration::from_secs(25);
/// How long to wait for Chrome's DevTools endpoint to list the page.
const CDP_TIMEOUT: Duration = Duration::from_secs(8);
/// Re-attaching to a window that was already attached: if the endpoint
/// doesn't answer quickly, the window is gone (closed by the user).
const REATTACH_TIMEOUT: Duration = Duration::from_millis(1500);
/// Per-command reply timeout.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// ui-leaf heartbeat silence before `disconnected` (informational only — it
/// never terminates the mount). Once the window navigates to an article the
/// ui-leaf page is gone, so heartbeats stop by design; keep this long.
const HEARTBEAT_TIMEOUT_MS: u64 = 3_600_000;
const CLOSE_MSG: &[u8] = b"{\"version\":\"1\",\"type\":\"close\"}\n";
/// The page→wickd binding name.
const BINDING: &str = "__wickdNav";

/// Which way the window's keys asked to move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDir {
    Next,
    Prev,
}

/// Commands from the TUI.
#[derive(Debug)]
pub enum ReaderCmd {
    /// Show this article (opening the window first if needed); `label` is the
    /// badge text shown on the page.
    Show { url: String, label: String },
    /// Close the window; the sender (if any) is signalled once it's closed,
    /// so the TUI can wait for a clean shutdown instead of guessing a delay.
    Close(Option<std::sync::mpsc::Sender<()>>),
}

/// Events back to the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReaderEvent {
    Opened,
    Error(String),
    Closed,
    /// ctrl+shift+→/← pressed in the window.
    Nav(NavDir),
}

/// Why a navigation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NavError {
    /// The window/Chrome is gone (e.g. the user closed it): reopen and retry.
    Gone(String),
    /// Anything else (slow page, Chrome refused the URL): report it.
    Other(String),
}

/// The line-1 config for `ui-leaf mount`.
pub fn mount_config(views_root: &Path, debug_port: u16) -> String {
    json!({
        "version": "1",
        "view": "reader",
        "viewsRoot": views_root.to_string_lossy(),
        "data": { "status": "opening…" },
        "mutations": [],
        "port": 0,
        "openBrowser": true,
        "shell": "app",
        "windowSize": { "width": WINDOW.0, "height": WINDOW.1 },
        "debugPort": debug_port,
        "title": "wickd · news",
        "heartbeatTimeoutMs": HEARTBEAT_TIMEOUT_MS,
    })
    .to_string()
}

/// From a DevTools `/json/list` response, the WebSocket URL of the window's
/// page: ui-leaf's own page if present, else any page.
pub fn page_ws_url(list: &Value) -> Option<String> {
    let pages: Vec<&Value> = list
        .as_array()?
        .iter()
        .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
        .collect();
    let is_local = |t: &&&Value| {
        t.get("url")
            .and_then(Value::as_str)
            .is_some_and(|u| u.starts_with("http://127.0.0.1") || u.starts_with("http://localhost"))
    };
    let page = pages.iter().find(is_local).or_else(|| pages.first())?;
    page.get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Only plain web links are navigated to (never `javascript:`, `file:`, …).
pub fn is_navigable(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("https://") || u.starts_with("http://")
}

/// A `Runtime.bindingCalled` event for our binding → a direction. Anything
/// else (other events, other bindings, odd payloads) → `None`.
pub fn parse_nav(event: &Value) -> Option<NavDir> {
    if event.get("method").and_then(Value::as_str) != Some("Runtime.bindingCalled") {
        return None;
    }
    let params = event.get("params")?;
    if params.get("name").and_then(Value::as_str) != Some(BINDING) {
        return None;
    }
    match params.get("payload").and_then(Value::as_str) {
        Some("next") => Some(NavDir::Next),
        Some("prev") => Some(NavDir::Prev),
        _ => None,
    }
}

/// The script injected into each article page: ctrl+shift+→/← call the
/// binding; a small badge shows `label` in the corner.
pub fn page_script(label: &str) -> String {
    let label = serde_json::to_string(label).unwrap_or_else(|_| "\"\"".into());
    format!(
        r#"(() => {{
  const label = {label};
  const send = (d) => {{ try {{ window.{BINDING} && window.{BINDING}(d); }} catch (e) {{}} }};
  window.addEventListener("keydown", (e) => {{
    if (e.ctrlKey && e.shiftKey && (e.key === "ArrowRight" || e.key === "ArrowLeft")) {{
      e.preventDefault();
      e.stopImmediatePropagation();
      send(e.key === "ArrowRight" ? "next" : "prev");
    }}
  }}, true);
  const badge = () => {{
    if (document.getElementById("__wickd_badge")) return;
    const b = document.createElement("div");
    b.id = "__wickd_badge";
    b.textContent = label;
    b.style.cssText = "position:fixed;bottom:12px;right:12px;z-index:2147483647;" +
      "background:rgba(21,26,34,.92);color:#d1d5db;font:12px -apple-system,BlinkMacSystemFont,sans-serif;" +
      "padding:6px 10px;border-radius:6px;pointer-events:none;box-shadow:0 2px 8px rgba(0,0,0,.4)";
    (document.body || document.documentElement).appendChild(b);
  }};
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", badge);
  else badge();
}})();"#
    )
}

/// A free loopback port for Chrome's DevTools endpoint. Bind-then-release
/// can race another process grabbing the port in between; acceptable for a
/// loopback port picked at random (Chrome would just fail to bind, and the
/// attach step reports that).
fn free_port() -> std::io::Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;
type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

/// A live DevTools connection: commands go out on `sink`; a background task
/// routes replies to `pending` and binding calls to the nav channel.
struct Cdp {
    sink: SplitSink<Ws, Message>,
    pending: Pending,
    alive: Arc<AtomicBool>,
    next_id: u64,
}

impl Cdp {
    async fn connect(ws_url: &str, nav: UnboundedSender<NavDir>) -> Result<Self, String> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| format!("connecting to the reader window: {e}"))?;
        let (sink, mut stream) = ws.split();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let (p, a) = (pending.clone(), alive.clone());
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                let Message::Text(t) = msg else { continue };
                let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                if let Some(id) = v.get("id").and_then(Value::as_u64) {
                    if let Some(tx) = p.lock().ok().and_then(|mut m| m.remove(&id)) {
                        let _ = tx.send(v);
                    }
                } else if let Some(dir) = parse_nav(&v) {
                    let _ = nav.send(dir);
                }
            }
            a.store(false, Ordering::SeqCst);
            // Fail any callers still waiting.
            if let Ok(mut m) = p.lock() {
                m.clear();
            }
        });
        Ok(Self {
            sink,
            pending,
            alive,
            next_id: 1,
        })
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Send a command and wait for its reply. `Err(None)` = connection gone.
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, Option<String>> {
        let id = self.next_id;
        self.next_id += 1;
        let (tx, rx) = oneshot::channel();
        if let Ok(mut m) = self.pending.lock() {
            m.insert(id, tx);
        }
        let msg = json!({ "id": id, "method": method, "params": params });
        if self
            .sink
            .send(Message::Text(msg.to_string().into()))
            .await
            .is_err()
        {
            return Err(None);
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(v)) => match v.get("error") {
                Some(e) => Err(Some(e.to_string())),
                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            },
            Ok(Err(_)) => Err(None), // reader task ended: socket closed
            Err(_) => Err(Some(format!("{method} timed out"))),
        }
    }
}

struct Session {
    /// Distinguishes this window's `closed` notice from an older one's.
    id: u64,
    child: Child,
    stdin: ChildStdin,
    port: u16,
    cdp: Option<Cdp>,
    /// Attached successfully at least once (re-attaches use a short timeout).
    attached: bool,
    /// The current per-document script, replaced before each article.
    script_id: Option<String>,
}

impl Session {
    async fn open(
        id: u64,
        bin: &str,
        views_root: &Path,
        closed: UnboundedSender<u64>,
    ) -> Result<Self, String> {
        let port = free_port().map_err(|e| format!("no free port for the reader: {e}"))?;
        let mut child = Command::new(bin)
            .arg("mount")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    "ui-leaf isn't installed — npm i -g @openthink/ui-leaf".to_string()
                } else {
                    format!("could not start ui-leaf: {e}")
                }
            })?;
        let mut stdin = child.stdin.take().ok_or("ui-leaf stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("ui-leaf stdout unavailable")?;
        stdin
            .write_all(format!("{}\n", mount_config(views_root, port)).as_bytes())
            .await
            .map_err(|e| format!("talking to ui-leaf: {e}"))?;
        stdin.flush().await.map_err(|e| e.to_string())?;

        // Wait for `ready`, then keep draining stdout to notice `closed`.
        let mut lines = BufReader::new(stdout).lines();
        tokio::time::timeout(READY_TIMEOUT, async {
            while let Ok(Some(line)) = lines.next_line().await {
                let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                match v.get("type").and_then(Value::as_str) {
                    Some("ready") => return Ok(()),
                    Some("closed") => {
                        return Err("the reader window closed while opening".to_string())
                    }
                    Some("error") => {
                        let msg = v
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown error");
                        return Err(format!("ui-leaf: {msg}"));
                    }
                    _ => {}
                }
            }
            Err("ui-leaf exited before the window opened".to_string())
        })
        .await
        .map_err(|_| "the reader window didn't open in time".to_string())??;
        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
                if v.get("type").and_then(Value::as_str) == Some("closed") {
                    break;
                }
            }
            let _ = closed.send(id);
        });

        Ok(Self {
            id,
            child,
            stdin,
            port,
            cdp: None,
            attached: false,
            script_id: None,
        })
    }

    /// (Re)attach to the window's page over CDP and set up the binding.
    async fn attach(&mut self, nav: &UnboundedSender<NavDir>) -> Result<(), String> {
        let url = format!("http://127.0.0.1:{}/json/list", self.port);
        let wait = if self.attached {
            REATTACH_TIMEOUT
        } else {
            CDP_TIMEOUT
        };
        let deadline = tokio::time::Instant::now() + wait;
        let ws_url = loop {
            let found = match reqwest::get(&url).await {
                Ok(r) => r.json::<Value>().await.ok().and_then(|v| page_ws_url(&v)),
                Err(_) => None,
            };
            if let Some(u) = found {
                break u;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("couldn't reach the reader window's control port".into());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        };
        let mut cdp = Cdp::connect(&ws_url, nav.clone()).await?;
        for (method, params) in [
            ("Page.enable", json!({})),
            ("Runtime.enable", json!({})),
            ("Runtime.addBinding", json!({ "name": BINDING })),
        ] {
            cdp.call(method, params)
                .await
                .map_err(|e| e.unwrap_or_else(|| "lost the reader window".into()))?;
        }
        self.cdp = Some(cdp);
        self.script_id = None;
        self.attached = true;
        Ok(())
    }

    async fn navigate(
        &mut self,
        url: &str,
        label: &str,
        nav: &UnboundedSender<NavDir>,
    ) -> Result<(), NavError> {
        let gone = |m: &str| NavError::Gone(m.to_string());
        if !self.cdp.as_ref().is_some_and(Cdp::is_alive) {
            self.cdp = None;
            self.attach(nav).await.map_err(NavError::Gone)?;
        }
        let Some(cdp) = self.cdp.as_mut() else {
            return Err(gone("lost the reader window"));
        };
        let lost = |e: Option<String>| match e {
            None => NavError::Gone("lost the reader window".into()),
            Some(m) => NavError::Other(m),
        };
        // Swap the per-document script for this article's badge.
        if let Some(old) = self.script_id.take() {
            let _ = cdp
                .call(
                    "Page.removeScriptToEvaluateOnNewDocument",
                    json!({ "identifier": old }),
                )
                .await;
        }
        let added = cdp
            .call(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": page_script(label) }),
            )
            .await
            .map_err(lost)?;
        self.script_id = added
            .get("identifier")
            .and_then(Value::as_str)
            .map(str::to_string);
        let r = cdp
            .call("Page.navigate", json!({ "url": url }))
            .await
            .map_err(lost)?;
        if let Some(e) = r.get("errorText").and_then(Value::as_str) {
            return Err(NavError::Other(format!("couldn't open the article: {e}")));
        }
        Ok(())
    }

    async fn close(mut self) {
        let _ = self.stdin.write_all(CLOSE_MSG).await;
        let _ = self.stdin.flush().await;
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
    }
}

/// Owns at most one reader window.
struct Reader {
    bin: String,
    views_root: Option<std::path::PathBuf>,
    closed_tx: UnboundedSender<u64>,
    nav_tx: UnboundedSender<NavDir>,
    session: Option<Session>,
    next_id: u64,
}

impl Reader {
    async fn open(&mut self, events: &impl Fn(ReaderEvent)) -> bool {
        let Some(root) = self.views_root.as_deref() else {
            events(ReaderEvent::Error(
                "reader view assets not found (WICKD_VIEWS_ROOT)".into(),
            ));
            return false;
        };
        self.next_id += 1;
        match Session::open(self.next_id, &self.bin, root, self.closed_tx.clone()).await {
            Ok(s) => {
                self.session = Some(s);
                events(ReaderEvent::Opened);
                true
            }
            Err(e) => {
                events(ReaderEvent::Error(e));
                false
            }
        }
    }

    /// Show `url`, opening a window if there isn't one. If the window turns
    /// out to be gone (the user closed it and the close notice hasn't landed,
    /// or never will), drop it, open a fresh one and retry once.
    async fn show(&mut self, url: &str, label: &str, events: &impl Fn(ReaderEvent)) {
        if !is_navigable(url) {
            events(ReaderEvent::Error("this headline has no web link".into()));
            return;
        }
        for attempt in 0..2 {
            if self.session.is_none() && !self.open(events).await {
                return;
            }
            let nav = self.nav_tx.clone();
            let Some(s) = self.session.as_mut() else {
                return;
            };
            match s.navigate(url, label, &nav).await {
                Ok(()) => return,
                Err(NavError::Gone(e)) => {
                    if let Some(dead) = self.session.take() {
                        dead.close().await;
                    }
                    if attempt == 1 {
                        events(ReaderEvent::Error(e));
                    }
                }
                Err(NavError::Other(e)) => {
                    events(ReaderEvent::Error(e));
                    return;
                }
            }
        }
    }

    async fn close(&mut self, events: &impl Fn(ReaderEvent)) {
        if let Some(s) = self.session.take() {
            s.close().await;
            events(ReaderEvent::Closed);
        }
    }
}

/// The reader task: owns at most one window, serialises commands, and
/// forwards the window's ctrl+shift+arrow presses to the TUI.
pub async fn run(
    mut rx: UnboundedReceiver<ReaderCmd>,
    events: impl Fn(ReaderEvent) + Send + 'static,
    bin: String,
    views_root: Option<std::path::PathBuf>,
) {
    let (closed_tx, mut closed_rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
    let (nav_tx, mut nav_rx) = tokio::sync::mpsc::unbounded_channel::<NavDir>();
    let mut r = Reader {
        bin,
        views_root,
        closed_tx,
        nav_tx,
        session: None,
        next_id: 0,
    };
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                None => {
                    if let Some(s) = r.session.take() {
                        s.close().await;
                    }
                    return;
                }
                Some(ReaderCmd::Close(ack)) => {
                    r.close(&events).await;
                    if let Some(ack) = ack {
                        let _ = ack.send(());
                    }
                }
                Some(ReaderCmd::Show { url, label }) => r.show(&url, &label, &events).await,
            },
            Some(dir) = nav_rx.recv() => {
                if r.session.is_some() {
                    events(ReaderEvent::Nav(dir));
                }
            }
            // Only the *current* window's close notice counts: a window closed
            // on purpose (x) reports late, and must not tear down a new one.
            Some(id) = closed_rx.recv() => {
                if r.session.as_ref().is_some_and(|s| s.id == id) {
                    r.session = None;
                    events(ReaderEvent::Closed);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_is_app_mode_throwaway_profile_with_debug_port() {
        let c: Value = serde_json::from_str(&mount_config(Path::new("/v"), 9333)).unwrap();
        assert_eq!(c["view"], "reader");
        assert_eq!(c["shell"], "app");
        assert_eq!(c["debugPort"], 9333);
        assert_eq!(c["windowSize"]["width"], WINDOW.0);
        assert!(c.get("profile").is_none());
    }

    #[test]
    fn picks_the_ui_leaf_page_target() {
        let list = json!([
            {"type":"service_worker","url":"chrome://x","webSocketDebuggerUrl":"ws://sw"},
            {"type":"page","url":"https://www.benzinga.com/a","webSocketDebuggerUrl":"ws://other"},
            {"type":"page","url":"http://127.0.0.1:51234/#token=abc","webSocketDebuggerUrl":"ws://leaf"}
        ]);
        assert_eq!(page_ws_url(&list).as_deref(), Some("ws://leaf"));
        let list = json!([{"type":"page","url":"https://www.benzinga.com/a","webSocketDebuggerUrl":"ws://p"}]);
        assert_eq!(page_ws_url(&list).as_deref(), Some("ws://p"));
        assert_eq!(page_ws_url(&json!([])), None);
        assert_eq!(page_ws_url(&json!({"not":"a list"})), None);
    }

    #[test]
    fn only_web_links_are_navigable() {
        assert!(is_navigable("https://www.benzinga.com/news/1"));
        assert!(is_navigable("http://example.com"));
        assert!(!is_navigable("javascript:alert(1)"));
        assert!(!is_navigable("file:///etc/passwd"));
        assert!(!is_navigable(""));
    }

    #[test]
    fn binding_events_parse_only_our_two_directions() {
        let ev = |name: &str, payload: &str| json!({"method":"Runtime.bindingCalled","params":{"name":name,"payload":payload,"executionContextId":1}});
        assert_eq!(parse_nav(&ev(BINDING, "next")), Some(NavDir::Next));
        assert_eq!(parse_nav(&ev(BINDING, "prev")), Some(NavDir::Prev));
        assert_eq!(parse_nav(&ev(BINDING, "rm -rf")), None);
        assert_eq!(parse_nav(&ev("other", "next")), None);
        assert_eq!(
            parse_nav(&json!({"method":"Page.loadEventFired","params":{}})),
            None
        );
    }

    #[test]
    fn page_script_escapes_the_label_and_wires_the_keys() {
        let s = page_script(r#"NVDA "3 of 12" </script>"#);
        assert!(
            s.contains(r#"const label = "NVDA \"3 of 12\" </script>";"#),
            "{s}"
        );
        assert!(s.contains("ArrowRight") && s.contains(BINDING));
    }

    #[test]
    fn free_port_is_loopback_bindable() {
        assert!(free_port().unwrap() > 0);
    }

    fn reader() -> (Reader, UnboundedReceiver<NavDir>) {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("views");
        let (closed_tx, _closed_rx) = tokio::sync::mpsc::unbounded_channel();
        let (nav_tx, nav_rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Reader {
                bin: "ui-leaf".into(),
                views_root: Some(root),
                closed_tx,
                nav_tx,
                session: None,
                next_id: 0,
            },
            nav_rx,
        )
    }

    async fn page_urls(port: u16) -> Vec<String> {
        let list: Value = reqwest::get(format!("http://127.0.0.1:{port}/json/list"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        list.as_array()
            .unwrap()
            .iter()
            .filter(|t| t["type"] == "page")
            .filter_map(|t| t["url"].as_str().map(str::to_string))
            .collect()
    }

    /// Live (opens a real Chrome window for a few seconds):
    /// `cargo test -p wickd reader_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn reader_live_navigates_and_window_keys_reach_wickd() {
        let (mut r, mut nav_rx) = reader();
        let events = |_e: ReaderEvent| {};
        r.show("https://example.com/", "TEST · 1 of 2", &events)
            .await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        r.show(
            "https://www.iana.org/help/example-domains",
            "TEST · 2 of 2",
            &events,
        )
        .await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let port = r.session.as_ref().expect("window open").port;
        let urls = page_urls(port).await;
        println!("page urls: {urls:?}");
        assert!(urls.iter().any(|u| u.contains("iana.org")), "{urls:?}");
        // Press ctrl+shift+→ inside the page; wickd must hear "next".
        let s = r.session.as_mut().unwrap();
        let cdp = s.cdp.as_mut().unwrap();
        for kind in ["keyDown", "keyUp"] {
            cdp.call(
                "Input.dispatchKeyEvent",
                json!({ "type": kind, "modifiers": 2 | 8, "key": "ArrowRight",
                        "code": "ArrowRight", "windowsVirtualKeyCode": 39 }),
            )
            .await
            .unwrap();
        }
        let got = tokio::time::timeout(Duration::from_secs(3), nav_rx.recv())
            .await
            .expect("binding call arrived");
        println!("nav: {got:?}");
        assert_eq!(got, Some(NavDir::Next));
        // The badge rendered on the article page.
        let badge = cdp
            .call(
                "Runtime.evaluate",
                json!({ "expression": "document.getElementById('__wickd_badge')?.textContent" }),
            )
            .await
            .unwrap();
        println!("badge: {badge}");
        assert_eq!(badge["result"]["value"], "TEST · 2 of 2");
        r.close(&events).await;
    }

    /// The reported bug: the user closes the window, then opens another
    /// article. The reader must notice and open a fresh window.
    #[tokio::test]
    #[ignore]
    async fn reader_live_reopen_after_user_closes_window() {
        let (mut r, _nav_rx) = reader();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let events = move |e: ReaderEvent| sink.lock().unwrap().push(e);
        r.show("https://example.com/", "1", &events).await;
        let port = r.session.as_ref().expect("window open").port;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let list: Value = reqwest::get(format!("http://127.0.0.1:{port}/json/list"))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        for t in list.as_array().unwrap() {
            if t["type"] == "page" {
                let id = t["id"].as_str().unwrap();
                let _ = reqwest::get(format!("http://127.0.0.1:{port}/json/close/{id}")).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
        r.show("https://www.iana.org/help/example-domains", "2", &events)
            .await;
        let ev = seen.lock().unwrap().clone();
        println!("events: {ev:?}");
        assert!(
            !ev.iter().any(|e| matches!(e, ReaderEvent::Error(_))),
            "{ev:?}"
        );
        assert_eq!(ev.iter().filter(|e| **e == ReaderEvent::Opened).count(), 2);
        r.close(&events).await;
    }
}
