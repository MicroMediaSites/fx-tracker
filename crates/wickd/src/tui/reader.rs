//! News reader window for `wickd tui`.
//!
//! The in-play headlines pane can open an article in a chromeless ui-leaf
//! window beside the terminal. The TUI stays the controller: moving the
//! headline cursor makes the window load the next article.
//!
//! ## How
//!
//! 1. Spawn `ui-leaf mount` with the `reader` view (a placeholder page),
//!    `shell: "app"`, an initial window size, a throwaway Chrome profile (no
//!    `profile` field → ui-leaf uses a temp dir and deletes it on unmount),
//!    and `debugPort` on a free loopback port.
//! 2. When ui-leaf reports `ready`, attach to that Chrome over the DevTools
//!    protocol: `GET /json/list` → the page target's WebSocket URL.
//! 3. Each article is a `Page.navigate` on that target. The window shows the
//!    real article page (embedding it in a frame doesn't work: Benzinga, the
//!    main source, sends `X-Frame-Options: SAMEORIGIN`).
//!
//! ## Security
//!
//! The DevTools port has no authentication; any local process can control
//! the window while it's open. That's why the profile is throwaway (no
//! cookies or logins to steal) and the port is loopback-only and random.
//!
//! ## Lifecycle
//!
//! The window closing (or Chrome exiting) ends the ui-leaf mount; the reader
//! notices `closed` and the next "open" starts a fresh window. Quitting the
//! TUI sends `close`; ui-leaf also exits on its own if its parent dies.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Initial window size in CSS pixels (tall reading column).
pub const WINDOW: (u32, u32) = (1000, 1000);
/// How long to wait for ui-leaf to report `ready`.
const READY_TIMEOUT: Duration = Duration::from_secs(25);
/// How long to wait for Chrome's DevTools endpoint to list the page.
const CDP_TIMEOUT: Duration = Duration::from_secs(8);
/// ui-leaf heartbeat silence before `disconnected` (informational only — it
/// never terminates the mount). Once the window navigates to an article the
/// ui-leaf page is gone, so heartbeats stop by design; keep this long.
const HEARTBEAT_TIMEOUT_MS: u64 = 3_600_000;
const CLOSE_MSG: &[u8] = b"{\"version\":\"1\",\"type\":\"close\"}\n";

/// Commands from the TUI.
#[derive(Debug)]
pub enum ReaderCmd {
    /// Show this article, opening the window first if needed.
    Show(String),
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

/// From a DevTools `/json/list` response, the WebSocket URL of the ui-leaf
/// page (the window's first `page` target).
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
    // Prefer ui-leaf's own page (the one we just opened), else any page
    // (after a navigation the URL is the article's).
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

/// A free loopback port for Chrome's DevTools endpoint. Bind-then-release
/// can race another process grabbing the port in between; acceptable for a
/// loopback port picked at random (Chrome would just fail to bind, and the
/// attach step reports that).
fn free_port() -> std::io::Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Session {
    /// Distinguishes this window's `closed` notice from an older one's.
    id: u64,
    child: Child,
    stdin: ChildStdin,
    port: u16,
    ws: Option<Ws>,
    next_id: u64,
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
            ws: None,
            next_id: 1,
        })
    }

    /// (Re)attach to the window's page over CDP.
    async fn attach(&mut self) -> Result<(), String> {
        let url = format!("http://127.0.0.1:{}/json/list", self.port);
        let deadline = tokio::time::Instant::now() + CDP_TIMEOUT;
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
        let (ws, _) = tokio_tungstenite::connect_async(ws_url.as_str())
            .await
            .map_err(|e| format!("connecting to the reader window: {e}"))?;
        self.ws = Some(ws);
        Ok(())
    }

    async fn navigate(&mut self, url: &str) -> Result<(), String> {
        for attempt in 0..2 {
            if self.ws.is_none() {
                self.attach().await?;
            }
            let id = self.next_id;
            self.next_id += 1;
            let msg = json!({ "id": id, "method": "Page.navigate", "params": { "url": url } });
            let Some(ws) = self.ws.as_mut() else {
                return Err("lost the reader window".into());
            };
            let sent = ws.send(Message::Text(msg.to_string().into())).await;
            if sent.is_err() {
                self.ws = None; // stale socket: reattach once
                if attempt == 0 {
                    continue;
                }
                return Err("lost the reader window".into());
            }
            // Wait for the matching response (ignore any events).
            let reply = tokio::time::timeout(Duration::from_secs(10), async {
                while let Some(Ok(m)) = ws.next().await {
                    if let Message::Text(t) = m {
                        let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                        if v.get("id").and_then(Value::as_u64) == Some(id) {
                            return Some(v);
                        }
                    }
                }
                None
            })
            .await;
            return match reply {
                Ok(Some(v)) => match v.get("error") {
                    Some(e) => Err(format!("couldn't open the article: {e}")),
                    None => Ok(()),
                },
                Ok(None) => {
                    self.ws = None;
                    Err("lost the reader window".into())
                }
                Err(_) => Err("the article is taking a while to load".into()),
            };
        }
        Err("lost the reader window".into())
    }

    async fn close(mut self) {
        let _ = self.stdin.write_all(CLOSE_MSG).await;
        let _ = self.stdin.flush().await;
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
    }
}

/// Bring the terminal back to the front after the window opens (macOS), so
/// keystrokes keep going to the TUI. Best effort.
fn refocus_terminal() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let app = if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        Some("kitty")
    } else {
        match std::env::var("TERM_PROGRAM").as_deref() {
            Ok("iTerm.app") => Some("iTerm"),
            Ok("Apple_Terminal") => Some("Terminal"),
            Ok("WezTerm") => Some("WezTerm"),
            Ok("ghostty") => Some("Ghostty"),
            _ => None,
        }
    };
    if let Some(app) = app {
        let _ = std::process::Command::new("open")
            .args(["-a", app])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

/// The reader task: owns at most one window, serialises commands.
pub async fn run(
    mut rx: UnboundedReceiver<ReaderCmd>,
    events: impl Fn(ReaderEvent) + Send + 'static,
    bin: String,
    views_root: Option<std::path::PathBuf>,
) {
    let (closed_tx, mut closed_rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
    let mut session: Option<Session> = None;
    let mut next_id: u64 = 0;
    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                None => {
                    if let Some(s) = session.take() {
                        s.close().await;
                    }
                    return;
                }
                Some(ReaderCmd::Close(ack)) => {
                    if let Some(s) = session.take() {
                        s.close().await;
                        events(ReaderEvent::Closed);
                    }
                    if let Some(ack) = ack {
                        let _ = ack.send(());
                    }
                }
                Some(ReaderCmd::Show(url)) => {
                    if !is_navigable(&url) {
                        events(ReaderEvent::Error("this headline has no web link".into()));
                        continue;
                    }
                    if session.is_none() {
                        let Some(root) = views_root.as_deref() else {
                            events(ReaderEvent::Error("reader view assets not found (WICKD_VIEWS_ROOT)".into()));
                            continue;
                        };
                        next_id += 1;
                        match Session::open(next_id, &bin, root, closed_tx.clone()).await {
                            Ok(s) => {
                                session = Some(s);
                                events(ReaderEvent::Opened);
                                refocus_terminal();
                            }
                            Err(e) => {
                                events(ReaderEvent::Error(e));
                                continue;
                            }
                        }
                    }
                    if let Some(s) = session.as_mut() {
                        if let Err(e) = s.navigate(&url).await {
                            events(ReaderEvent::Error(e));
                        }
                    }
                }
            },
            // Only the *current* window's close notice counts: a window closed
            // on purpose (x) reports late, and must not tear down a new one.
            Some(id) = closed_rx.recv() => {
                if session.as_ref().is_some_and(|s| s.id == id) {
                    session = None;
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
        // No persistent profile: ui-leaf uses a throwaway one.
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
        // After navigating away, any page will do.
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
    fn free_port_is_loopback_bindable() {
        assert!(free_port().unwrap() > 0);
    }

    /// Live smoke test (opens a real Chrome window for a few seconds):
    /// `cargo test -p wickd reader_live_smoke -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn reader_live_smoke() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("views");
        let (closed_tx, _closed_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut s = Session::open(1, "ui-leaf", &root, closed_tx)
            .await
            .expect("open");
        s.navigate("https://example.com/")
            .await
            .expect("navigate 1");
        tokio::time::sleep(Duration::from_secs(2)).await;
        s.navigate("https://www.iana.org/help/example-domains")
            .await
            .expect("navigate 2");
        tokio::time::sleep(Duration::from_secs(2)).await;
        let list: Value = reqwest::get(format!("http://127.0.0.1:{}/json/list", s.port))
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let urls: Vec<String> = list
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["type"] == "page")
            .filter_map(|t| t["url"].as_str().map(str::to_string))
            .collect();
        println!("page urls: {urls:?}");
        assert!(urls.iter().any(|u| u.contains("iana.org")), "{urls:?}");
        s.close().await;
    }
}
