//! Socket-hub fan-out for `wickd stream` (AGT-615).
//!
//! `wickd stream` opens exactly **one** OANDA price subscription and turns it
//! into a single NDJSON line stream. Historically that stream only went to
//! stdout, so only one consumer (a dashboard, the watcher, or an agent) could
//! read it — piping stdout to a second reader is not a thing.
//!
//! This module adds a local **Unix-domain-socket hub** at `~/.wickd/stream.sock`
//! that fans that one subscription out to *any number* of connected clients.
//! The [`NdjsonSink`] publishes each finished NDJSON line into a bounded
//! [`tokio::sync::broadcast`] channel; the hub's accept loop hands every
//! connecting client its own [`broadcast::Receiver`] and copies each line to
//! that client's socket verbatim. stdout still receives the identical lines, so
//! the old single-consumer path (`wickd stream | jq`) keeps working unchanged.
//!
//! ```text
//!            OANDA price stream (one subscription)
//!                          │
//!                          ▼
//!                    NdjsonSink::emit
//!                     │            │
//!             writeln!(stdout)   broadcast::Sender<String>
//!                                     │
//!             ┌───────────────────────┼───────────────────────┐
//!             ▼                        ▼                       ▼
//!      client A (socket)        client B (socket)       client C (socket)
//! ```
//!
//! ## Exit policy (AGT-615 AC4) — RESOLVED
//!
//! The hub lives exactly as long as the `wickd stream` **process** does. This is
//! an *on-demand* stream: the operator ran `wickd stream`, so the OANDA
//! subscription starts and stops with that invocation, **not** with client
//! connect/disconnect. Clients attaching or detaching never start or stop the
//! upstream subscription. `wickd stream` tears everything down — stops the OANDA
//! reader and removes the socket file — when it exits, which happens on:
//!   - Ctrl-C / SIGTERM (the operator ends the stream), or
//!   - the stdout consumer's pipe closing (`wickd stream | head`), i.e. the
//!     operator's own invocation ended (AGT-614's clean-pipe-exit behavior).
//!
//! We deliberately do **not** "shut down when the last client leaves": with zero
//! clients the socket must still exist and accept the next one, and the operator
//! who launched the stream is the owner of its lifetime.
//!
//! ## Stale-socket handling (AC2)
//!
//! A `SIGKILL` / crash leaves the socket file behind (no destructor runs). On
//! bind we first *probe* any existing file: if something answers a connect, a
//! hub is genuinely already running and we refuse to start a second one; if the
//! connect fails, the file is stale (or not a socket at all) and we unlink it
//! before binding. Normal exits remove the socket explicitly
//! ([`StreamHub::shutdown`]) since `std::process::exit` skips [`Drop`].
//!
//! ## Slow clients (AC3)
//!
//! `broadcast::Sender::send` never blocks — a lagging receiver simply drops the
//! oldest buffered lines and its next `recv()` returns
//! [`broadcast::error::RecvError::Lagged`]. So a slow client can never stall the
//! OANDA read loop or any other client. When a client lags we treat it as gone
//! and drop its connection rather than silently feeding it a stream with holes.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Filename of the hub socket under `~/.wickd/`.
pub const SOCKET_NAME: &str = "stream.sock";

/// How many NDJSON lines the broadcast channel buffers per client before a slow
/// reader starts lagging. Bounded (AC1) so a stalled client can't grow memory
/// without bound; a client that falls this far behind is dropped (AC3).
pub const DEFAULT_CAPACITY: usize = 1024;

/// Resolve the hub socket path: `<data home>/stream.sock` (`~/.wickd/stream.sock`
/// unless `WICKD_HOME` overrides the data home — tests/smokes only, never live).
pub fn stream_socket_path() -> Result<PathBuf> {
    let home = crate::paths::wickd_data_home().map_err(anyhow::Error::msg)?;
    Ok(home.join(SOCKET_NAME))
}

/// A running socket-hub: a bound `UnixListener` fanning one broadcast channel
/// out to every connected client.
///
/// Hold on to this for the lifetime of the stream; call [`shutdown`] before the
/// process exits to remove the socket file (`process::exit` skips [`Drop`]).
///
/// [`shutdown`]: StreamHub::shutdown
#[derive(Debug)]
pub struct StreamHub {
    socket_path: PathBuf,
    tx: broadcast::Sender<String>,
    accept_task: JoinHandle<()>,
}

impl StreamHub {
    /// Bind the hub at the default path (`~/.wickd/stream.sock`) with the default
    /// buffer capacity.
    pub async fn bind() -> Result<Self> {
        let path = stream_socket_path()?;
        Self::bind_at(path, DEFAULT_CAPACITY).await
    }

    /// Bind the hub at an explicit path and buffer capacity. Split out so tests
    /// can point the hub at a temp socket with a small capacity.
    pub async fn bind_at(socket_path: PathBuf, capacity: usize) -> Result<Self> {
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("creating hub socket directory {}", parent.display())
            })?;
            // The data home holds the IPC socket and local trading DBs; keep it
            // owner-only so other local users can't reach the socket (AGT-668).
            restrict_dir_if_open(parent)?;
        }
        clear_stale_socket(&socket_path).await?;

        let listener = UnixListener::bind(&socket_path)
            .with_context(|| format!("binding hub socket {}", socket_path.display()))?;

        // The freshly-bound socket must not be world-accessible (AGT-668).
        restrict_socket_file(&socket_path)?;

        // The hub keeps `tx` alive for its whole lifetime; each client gets its
        // own receiver via `tx.subscribe()`. `_rx` here is dropped immediately —
        // clients subscribe on connect, not up front.
        let (tx, _rx) = broadcast::channel::<String>(capacity);

        let accept_task = tokio::spawn(accept_loop(listener, tx.clone()));

        Ok(Self { socket_path, tx, accept_task })
    }

    /// A sender the NDJSON sink publishes each finished line into. Every
    /// currently connected client receives an identical copy.
    pub fn sender(&self) -> broadcast::Sender<String> {
        self.tx.clone()
    }

    /// Stop accepting connections and remove the socket file. Idempotent, and
    /// must be called explicitly before `std::process::exit` (which skips the
    /// [`Drop`] impl below).
    pub fn shutdown(&self) {
        self.accept_task.abort();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for StreamHub {
    fn drop(&mut self) {
        // Belt-and-suspenders for panic/unwind paths; the normal exit path calls
        // `shutdown()` explicitly because `process::exit` never runs destructors.
        self.accept_task.abort();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Restrict the hub socket file to owner read/write only (`0600`) so no other
/// local user can connect to the IPC hub (AGT-668). No-op on non-Unix.
#[cfg(unix)]
fn restrict_socket_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("could not set 0600 on hub socket {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_socket_file(_path: &Path) -> Result<()> {
    Ok(())
}

/// Tighten the data-home directory to owner-only (`0700`) when it is currently
/// group/other-accessible (AGT-668, "parent dir 0700 if not already"). Leaves an
/// already-restricted dir untouched. No-op on non-Unix.
#[cfg(unix)]
fn restrict_dir_if_open(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(dir)
        .with_context(|| format!("reading permissions of {}", dir.display()))?
        .permissions()
        .mode();
    // Only tighten if any group/other bit is set; never loosen.
    if mode & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("could not set 0700 on {}", dir.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_dir_if_open(_dir: &Path) -> Result<()> {
    Ok(())
}

/// If a file already exists at `path`, decide whether it's a live hub (refuse to
/// start a second one) or a stale leftover from a crash (unlink it).
async fn clear_stale_socket(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    match UnixStream::connect(path).await {
        // Someone is listening — a real hub is already up.
        Ok(_) => bail!(
            "a wickd stream hub is already running at {} (only one on-demand stream at a time)",
            path.display()
        ),
        // Connect refused / not a socket / stale file left by a crash — unlink it
        // so the fresh bind below can succeed (AC2).
        Err(_) => std::fs::remove_file(path)
            .with_context(|| format!("removing stale hub socket {}", path.display())),
    }
}

/// Accept connections forever, giving each client its own broadcast receiver.
/// Aborted by [`StreamHub::shutdown`] / [`Drop`].
async fn accept_loop(listener: UnixListener, tx: broadcast::Sender<String>) {
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let rx = tx.subscribe();
                tokio::spawn(client_writer(stream, rx));
            }
            Err(_e) => {
                // The listener is gone (socket removed / fd closed on shutdown).
                // Nothing more to accept — this task is being torn down anyway.
                break;
            }
        }
    }
}

/// Copy every broadcast line to one client's socket, appending the newline that
/// makes it NDJSON. Ends (dropping the connection) when the client goes away,
/// the channel closes, or the client lags (AC3).
async fn client_writer(mut stream: UnixStream, mut rx: broadcast::Receiver<String>) {
    loop {
        match rx.recv().await {
            Ok(line) => {
                if stream.write_all(line.as_bytes()).await.is_err()
                    || stream.write_all(b"\n").await.is_err()
                {
                    // Client closed its read end — drop it.
                    break;
                }
            }
            // AC3: this client fell too far behind. Dropping it (rather than
            // resyncing) keeps every other consumer — and the OANDA read loop —
            // moving; a broadcast send never blocks on a slow receiver.
            Err(broadcast::error::RecvError::Lagged(_)) => break,
            // All senders dropped: the stream is shutting down.
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};

    fn temp_socket_path() -> PathBuf {
        static C: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "wickd-streamhub-test-{}-{}.sock",
            std::process::id(),
            C.fetch_add(1, Ordering::Relaxed)
        ))
    }

    // AGT-668: a freshly bound hub socket is owner-only (`0600`) and its parent
    // data-home dir is tightened to `0700` when it was group/other-accessible.
    #[cfg(unix)]
    #[tokio::test]
    async fn socket_and_parent_dir_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        static N: AtomicU64 = AtomicU64::new(0);
        // Own subdir under temp_dir so we can start it world-readable and prove
        // the tightening. Short name: macOS AF_UNIX paths cap at 104 bytes.
        let dir = std::env::temp_dir().join(format!(
            "wk668-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // Simulate a data home created under the default umask (world-readable).
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        let path = dir.join(SOCKET_NAME);
        let hub = StreamHub::bind_at(path.clone(), DEFAULT_CAPACITY).await.unwrap();

        let socket_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(socket_mode, 0o600, "hub socket must be 0600, got {socket_mode:o}");

        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "data home must be tightened to 0700, got {dir_mode:o}");

        hub.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn socket_path_is_under_wickd_home_dir() {
        // Only assert the default location when no WICKD_HOME override is set
        // (mirrors crate::paths tests — env is process-global).
        if std::env::var_os(crate::paths::WICKD_HOME_ENV).is_none() {
            let path = stream_socket_path().unwrap();
            assert!(path.ends_with(".wickd/stream.sock"));
        }
    }

    // AC1: two concurrent clients both receive the identical fanned-out lines
    // from the single (synthetic) broadcast source — no live OANDA needed.
    #[tokio::test]
    async fn two_clients_receive_identical_lines() {
        let path = temp_socket_path();
        let hub = StreamHub::bind_at(path.clone(), DEFAULT_CAPACITY).await.unwrap();
        let tx = hub.sender();

        let mut a = BufReader::new(UnixStream::connect(&path).await.unwrap()).lines();
        let mut b = BufReader::new(UnixStream::connect(&path).await.unwrap()).lines();

        // Give both accept-side client tasks a moment to subscribe before we
        // emit, so neither misses the first line.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let lines = [
            r#"{"event":"price-update","instrument":"EUR_USD","bid":"1.0850"}"#,
            r#"{"event":"price-update","instrument":"GBP_USD","bid":"1.2500"}"#,
            r#"{"event":"stream-health","healthy":true}"#,
        ];
        for line in lines {
            tx.send(line.to_string()).unwrap();
        }

        for expected in lines {
            let la = a.next_line().await.unwrap().unwrap();
            let lb = b.next_line().await.unwrap().unwrap();
            assert_eq!(la, expected);
            assert_eq!(lb, expected, "both clients get byte-identical NDJSON lines");
        }

        hub.shutdown();
    }

    // AC3: a slow client that never drains its socket must not stall a fast
    // client (or, in production, the OANDA read loop). We overflow the slow
    // client's buffer; the fast client keeps receiving and the slow client's
    // writer task ends on `Lagged`.
    #[tokio::test]
    async fn lagged_client_is_dropped_without_stalling_others() {
        let path = temp_socket_path();
        let capacity = 16;
        let hub = StreamHub::bind_at(path.clone(), capacity).await.unwrap();
        let tx = hub.sender();

        // Slow client: connect but never read from it.
        let _slow = UnixStream::connect(&path).await.unwrap();
        // Fast client: we drain it.
        let mut fast = BufReader::new(UnixStream::connect(&path).await.unwrap()).lines();

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Flood well past the buffer capacity so the un-drained slow client is
        // forced to lag. The fast client we read in lockstep so it never lags.
        let total = capacity * 8;
        let sender = tx.clone();
        let producer = tokio::spawn(async move {
            for i in 0..total {
                // Ignore send errors (there is always at least one receiver here).
                let _ = sender.send(format!(r#"{{"n":{i}}}"#));
                // Yield so the fast reader can interleave and stay current.
                tokio::task::yield_now().await;
            }
        });

        // The fast client must receive every line in order despite the slow
        // client hoarding its buffer.
        for i in 0..total {
            let line = tokio::time::timeout(Duration::from_secs(5), fast.next_line())
                .await
                .expect("fast client should not stall")
                .unwrap()
                .unwrap();
            assert_eq!(line, format!(r#"{{"n":{i}}}"#));
        }

        producer.await.unwrap();
        hub.shutdown();
    }

    // AC2: a stale socket file left by a crash (no live listener) is cleaned up
    // and the hub binds successfully over it.
    #[tokio::test]
    async fn stale_socket_file_is_reclaimed() {
        let path = temp_socket_path();
        // Simulate a crash leftover: a plain file where the socket should be.
        std::fs::write(&path, b"stale").unwrap();
        assert!(path.exists());

        let hub = StreamHub::bind_at(path.clone(), DEFAULT_CAPACITY)
            .await
            .expect("should reclaim a stale socket path and bind");

        // And it actually works after reclaiming.
        let mut client = BufReader::new(UnixStream::connect(&path).await.unwrap()).lines();
        tokio::time::sleep(Duration::from_millis(50)).await;
        hub.sender().send(r#"{"ok":true}"#.to_string()).unwrap();
        let line = client.next_line().await.unwrap().unwrap();
        assert_eq!(line, r#"{"ok":true}"#);

        hub.shutdown();
    }

    // AC2: a second hub refuses to bind while a live one holds the socket.
    #[tokio::test]
    async fn second_hub_refuses_while_one_is_live() {
        let path = temp_socket_path();
        let hub = StreamHub::bind_at(path.clone(), DEFAULT_CAPACITY).await.unwrap();

        let err = StreamHub::bind_at(path.clone(), DEFAULT_CAPACITY)
            .await
            .expect_err("second bind must fail while the first hub is live");
        assert!(err.to_string().contains("already running"));

        hub.shutdown();
    }

    // AC2: shutdown removes the socket file.
    #[tokio::test]
    async fn shutdown_removes_socket_file() {
        let path = temp_socket_path();
        let hub = StreamHub::bind_at(path.clone(), DEFAULT_CAPACITY).await.unwrap();
        assert!(path.exists());
        hub.shutdown();
        assert!(!path.exists(), "socket file removed on shutdown");
    }
}
