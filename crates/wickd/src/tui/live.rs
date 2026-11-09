//! Live forex ticks from the stream daemon (wickd-forex AGT-2389, D-1/D-2).
//!
//! The TUI is only ever a *client* of `~/.wickd/stream.sock`: it never hosts
//! the hub, so quitting a TUI can't cut anyone else's feed. A background task
//! connects, forwards `price-update` lines for the pairs on screen, and
//! reconnects every few seconds if the daemon isn't there or restarts. While
//! it's disconnected the forex rows keep their REST refresh (the quote poll),
//! so nothing goes blank — they just update every refresh instead of per
//! tick.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;

use wickd_core::hub_client::{self, HubQuote};

/// Wait between connection attempts while the daemon is absent.
const RETRY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub enum LiveEvent {
    /// Connected to (`true`) or lost (`false`) the stream daemon.
    Connected(bool),
    Quote(HubQuote),
}

/// The pairs the UI wants ticks for (updated by the run loop).
pub type Wanted = Arc<Mutex<HashSet<String>>>;

/// Run forever: attach to the daemon, forward wanted ticks, reconnect.
pub async fn run(wanted: Wanted, send: impl Fn(LiveEvent) + Send + 'static) {
    loop {
        if let Some(hub) = hub_client::probe_hub().await {
            pump(hub.into_stream(), &wanted, &send).await;
        }
        tokio::time::sleep(RETRY).await;
    }
}

/// One connection's lifetime: report connected, forward each wanted
/// `price-update` until the hub closes or errors, report disconnected.
async fn pump(stream: UnixStream, wanted: &Wanted, send: &impl Fn(LiveEvent)) {
    send(LiveEvent::Connected(true));
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Some(q) = hub_client::parse_quote_line(&line) else {
            continue;
        };
        let keep = wanted
            .lock()
            .map(|w| w.contains(&q.instrument))
            .unwrap_or(false);
        if keep {
            send(LiveEvent::Quote(q));
        }
    }
    send(LiveEvent::Connected(false));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixListener;

    /// Live: ticks from the real stream daemon for 5s.
    /// `cargo test -p wickd live_daemon_ticks -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_daemon_ticks() {
        let wanted: Wanted = Arc::new(Mutex::new(HashSet::from(["EUR_USD".to_string()])));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(run(wanted, move |e| {
            let _ = tx.send(e);
        }));
        tokio::time::sleep(Duration::from_secs(5)).await;
        task.abort();
        let mut n = 0;
        while let Ok(e) = rx.try_recv() {
            if let LiveEvent::Quote(q) = &e {
                assert_eq!(q.instrument, "EUR_USD");
                n += 1;
            }
            println!("{e:?}");
        }
        assert!(n > 0, "no EUR_USD ticks in 5s");
    }

    /// Against a fake hub: wanted ticks arrive, others are dropped, and a
    /// hub that goes away reports the disconnect.
    #[tokio::test]
    async fn forwards_wanted_ticks_and_reports_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stream.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let hub = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            for inst in ["GBP_USD", "EUR_USD"] {
                let line = format!(
                    "{{\"event\":\"price-update\",\"instrument\":\"{inst}\",\"bid\":\"1.1\",\"ask\":\"1.2\",\"spread\":\"0.1\",\"time\":\"2026-11-09T03:00:00Z\",\"tradeable\":true}}\n"
                );
                s.write_all(line.as_bytes()).await.unwrap();
            }
            // Dropping `s` closes the connection.
        });
        let wanted: Wanted = Arc::new(Mutex::new(HashSet::from(["EUR_USD".to_string()])));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let conn = hub_client::probe_hub_at(&path).await.unwrap();
        pump(conn.into_stream(), &wanted, &|e| {
            let _ = tx.send(e);
        })
        .await;
        hub.await.unwrap();
        let mut got = Vec::new();
        while let Ok(e) = rx.try_recv() {
            got.push(e);
        }
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(got[0], LiveEvent::Connected(true));
        assert!(matches!(&got[1], LiveEvent::Quote(q) if q.instrument == "EUR_USD"));
        assert_eq!(got[2], LiveEvent::Connected(false));
    }
}
