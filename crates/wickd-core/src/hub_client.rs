//! Socket-hub attach for `wickd watch` — AGT-618 AC2 / D4 (wired for AGT-615).
//!
//! When `wickd stream` is running it binds a Unix socket at
//! `~/.wickd/stream.sock` ([`crate::stream_hub`], AGT-615) and fans its single
//! OANDA price subscription out to every connected client as byte-identical
//! NDJSON lines. This module lets `wickd watch` attach to that hub instead of
//! opening its own OANDA subscription, so N watchers share one upstream
//! connection ("single-subscription honored").
//!
//! ## What this module provides
//!
//! - [`probe_hub`] — connect to the hub socket and, if something answers, keep
//!   the live connection (via [`HubHandle`]). The path comes from
//!   [`stream_hub::stream_socket_path`] — the single source of truth — so it can
//!   never drift from the real `~/.wickd/stream.sock` contract.
//! - [`HubFeed`] — read NDJSON off the connection, parse `price-update` ticks,
//!   and fan each instrument's ticks out to a per-instrument channel that feeds
//!   a [`crate::strategy::TickStreamSource`].
//! - [`parse_price_update_line`] — the line parser (ignores non-price events and
//!   malformed lines).
//! - [`partition_watchlist`] — decide which requested instruments the hub is
//!   actually streaming vs. which must fall back to a direct source.
//!
//! ## Hub coverage / fallback
//!
//! The hub carries only whatever instruments `wickd stream` was told to stream,
//! and the wire protocol is a one-way price feed with no control channel — so
//! there is no way to *ask* the hub which instruments it covers. We learn by
//! observation: [`HubFeed`] records every instrument it sees a tick for, and
//! `wickd watch` gives it a brief discovery window before
//! [`partition_watchlist`] splits the watchlist. Instruments the hub isn't
//! streaming fall back to a direct OANDA source so they still get data; we never
//! open a second *streaming* subscription for instruments the hub already
//! covers. (A watched instrument that stays completely silent through the
//! discovery window is treated as not-covered and gets its own direct source —
//! a safe degradation, never a silent stall.)
//!
//! ## Platform
//!
//! Unix-only, deliberately. The socket types here ([`tokio::net::UnixStream`])
//! are not available on Windows, matching the rest of wickd's stream surface
//! (`crate::stream_hub`'s `UnixListener`, `crate::commands::dashboard`'s
//! `std::os::unix::net::UnixStream`) — the CLI is a *nix agent tool, so we don't
//! carry a `#[cfg(not(unix))]` no-op fallback for a target it never ships to.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::str::FromStr;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;
use tokio::task::JoinHandle;

use crate::strategy::Tick;

use crate::stream_hub;

/// How long we wait for the hub to accept a connection before assuming it isn't
/// running. Short — this is a local Unix socket, not a network call.
const PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// A live connection to a running socket-hub.
///
/// [`probe_hub`] returns this only when a hub actually answered; it carries the
/// connected [`UnixStream`] so the caller can start reading the fanned-out
/// NDJSON feed (via [`HubFeed::attach`]).
#[derive(Debug)]
pub struct HubHandle {
    socket_path: std::path::PathBuf,
    stream: UnixStream,
}

impl HubHandle {
    /// The hub socket this handle is connected to.
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// Take ownership of the live connection (to hand to [`HubFeed::attach`]).
    pub fn into_stream(self) -> UnixStream {
        self.stream
    }
}

/// Attempt to attach to a running socket-hub stream (AGT-615).
///
/// Returns `Some(HubHandle)` — holding the live connection — only if a hub is
/// actually listening at [`stream_hub::stream_socket_path`]. On any failure
/// (socket absent, connection refused, timeout, non-unix platform) returns
/// `None`, and the caller falls back to opening its own subscription.
pub async fn probe_hub() -> Option<HubHandle> {
    let path = stream_hub::stream_socket_path().ok()?;
    probe_hub_at(&path).await
}

/// Same as [`probe_hub`] but against an explicit socket path — split out so
/// tests can point it at a temp socket instead of the real `~/.wickd`.
pub async fn probe_hub_at(path: &std::path::Path) -> Option<HubHandle> {
    if !path.exists() {
        return None;
    }

    let connect = UnixStream::connect(path);
    match tokio::time::timeout(PROBE_TIMEOUT, connect).await {
        Ok(Ok(stream)) => Some(HubHandle {
            socket_path: path.to_path_buf(),
            stream,
        }),
        // Timed out, or connect failed (e.g. a stale socket file with nothing
        // listening) — either way, no hub to attach to.
        _ => None,
    }
}

/// A parsed `price-update` tick off the hub feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubTick {
    /// Instrument, e.g. `EUR_USD`.
    pub instrument: String,
    /// Mid price (`(bid + ask) / 2`), as [`Decimal`] — never f64.
    pub mid: Decimal,
    /// Event time (UTC).
    pub time: DateTime<Utc>,
}

/// Parse one hub NDJSON line into a [`HubTick`].
///
/// Returns `None` for anything that isn't a well-formed `price-update` line: a
/// non-`price-update` event (`stream-health`, `stream-error`, …), a malformed
/// line, a missing/zero bid or ask, or an unparseable timestamp. Filtering here
/// keeps junk on the wire from ever reaching the candle aggregator.
pub fn parse_price_update_line(line: &str) -> Option<HubTick> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let value: serde_json::Value = serde_json::from_str(line).ok()?;

    // Only price-update events carry quotes; ignore every other event type.
    if value.get("event").and_then(|e| e.as_str()) != Some("price-update") {
        return None;
    }

    let instrument = value.get("instrument")?.as_str()?.to_string();
    let bid = Decimal::from_str(value.get("bid")?.as_str()?).ok()?;
    let ask = Decimal::from_str(value.get("ask")?.as_str()?).ok()?;
    if bid.is_zero() || ask.is_zero() {
        return None;
    }
    let mid = (bid + ask) / Decimal::from(2);

    let time = DateTime::parse_from_rfc3339(value.get("time")?.as_str()?)
        .ok()?
        .with_timezone(&Utc);

    Some(HubTick {
        instrument,
        mid,
        time,
    })
}

/// Split a requested watchlist by whether the hub is streaming each instrument.
///
/// Returns `(on_hub, direct)`: instruments the hub is streaming (drive them off
/// the shared feed) and instruments it isn't (fall back to a direct OANDA
/// source). Order within each group follows `requested`.
pub fn partition_watchlist(
    requested: &[String],
    hub_instruments: &HashSet<String>,
) -> (Vec<String>, Vec<String>) {
    let mut on_hub = Vec::new();
    let mut direct = Vec::new();
    for instrument in requested {
        if hub_instruments.contains(instrument) {
            on_hub.push(instrument.clone());
        } else {
            direct.push(instrument.clone());
        }
    }
    (on_hub, direct)
}

/// The background fan-out of a hub connection into per-instrument tick channels.
///
/// [`attach`](Self::attach) spawns a reader task that parses each NDJSON line
/// and dispatches its tick to the matching instrument's channel. Keep the
/// `HubFeed` alive for as long as the watcher runs; dropping it aborts the
/// reader task.
pub struct HubFeed {
    task: JoinHandle<()>,
    observed: Arc<Mutex<HashSet<String>>>,
}

impl HubFeed {
    /// Attach to the hub `stream`, fanning each requested instrument's ticks out
    /// to its own channel.
    ///
    /// Returns the feed (keep it alive) plus a [`Tick`] receiver per requested
    /// instrument. Ticks for instruments not in `instruments` — or whose
    /// receiver has since been dropped (the instrument fell back to a direct
    /// source) — are discarded.
    pub fn attach(
        stream: UnixStream,
        instruments: &[String],
    ) -> (Self, HashMap<String, std::sync::mpsc::Receiver<Tick>>) {
        let mut senders: HashMap<String, std::sync::mpsc::Sender<Tick>> = HashMap::new();
        let mut receivers: HashMap<String, std::sync::mpsc::Receiver<Tick>> = HashMap::new();
        for instrument in instruments {
            let (tx, rx) = std::sync::mpsc::channel::<Tick>();
            senders.insert(instrument.clone(), tx);
            receivers.insert(instrument.clone(), rx);
        }

        let observed = Arc::new(Mutex::new(HashSet::new()));
        let observed_for_task = observed.clone();

        let task = tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Some(hub_tick) = parse_price_update_line(&line) else {
                    continue;
                };
                // Don't panic the background reader on a poisoned lock — that
                // would silently stall every hub-backed instrument. Skip the
                // coverage record instead; the tick itself still gets dispatched.
                if let Ok(mut guard) = observed_for_task.lock() {
                    guard.insert(hub_tick.instrument.clone());
                }
                if let Some(tx) = senders.get(&hub_tick.instrument) {
                    // A dropped receiver (instrument fell back to a direct
                    // source) makes send fail; ignore it and keep going.
                    let _ = tx.send(Tick {
                        price: hub_tick.mid,
                        time: hub_tick.time,
                    });
                }
            }
            // Stream ended (hub gone): the task exits and every tick receiver
            // sees its channel disconnect on the next drain.
        });

        (Self { task, observed }, receivers)
    }

    /// The set of instruments the hub has been observed streaming so far. A
    /// poisoned lock yields an empty set rather than panicking the caller (the
    /// `wickd watch` startup path) — the worst case is every instrument falling
    /// back to a direct source.
    pub fn observed(&self) -> HashSet<String> {
        self.observed.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl Drop for HubFeed {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Watch the feed until it has been observed streaming every `requested`
/// instrument, or `timeout` elapses — whichever comes first — then return the
/// set of instruments seen. Used to decide hub-vs-direct per instrument.
pub async fn discover_instruments(
    feed: &HubFeed,
    requested: &[String],
    timeout: Duration,
) -> HashSet<String> {
    let deadline = Instant::now() + timeout;
    loop {
        let observed = feed.observed();
        if requested.iter().all(|i| observed.contains(i)) || Instant::now() >= deadline {
            return observed;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn falls_back_when_socket_file_does_not_exist() {
        let path = std::env::temp_dir().join("wickd-hub-test-does-not-exist.sock");
        let _ = std::fs::remove_file(&path);
        assert!(probe_hub_at(&path).await.is_none());
    }

    #[tokio::test]
    async fn falls_back_when_socket_file_exists_but_nothing_listens() {
        // A stale/garbage file at the path (not a real socket) — connect must
        // fail, and we must treat that as "no hub", not panic or hang.
        let path = std::env::temp_dir()
            .join(format!("wickd-hub-test-stale-{}.sock", std::process::id()));
        std::fs::write(&path, b"not a socket").unwrap();
        assert!(probe_hub_at(&path).await.is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn attaches_and_keeps_the_live_connection_when_a_hub_listens() {
        use tokio::net::UnixListener;

        let path = std::env::temp_dir()
            .join(format!("wickd-hub-test-live-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let acceptor = tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let handle = probe_hub_at(&path).await.expect("probe should attach");
        assert_eq!(handle.socket_path(), path);
        // The handle carries a usable live connection.
        let _stream = handle.into_stream();

        let _ = acceptor.await;
        let _ = std::fs::remove_file(&path);
    }

    // AC4: the parser accepts a well-formed price-update line and computes the
    // mid as a Decimal.
    #[test]
    fn parses_a_price_update_line() {
        let line = r#"{"instrument":"EUR_USD","bid":"1.0850","ask":"1.0852","spread":"0.0002","time":"2024-01-15T10:30:00Z","tradeable":true,"event":"price-update"}"#;
        let tick = parse_price_update_line(line).expect("valid price-update line");
        assert_eq!(tick.instrument, "EUR_USD");
        assert_eq!(tick.mid, rust_decimal_macros::dec!(1.0851));
        assert_eq!(tick.time, "2024-01-15T10:30:00Z".parse::<DateTime<Utc>>().unwrap());
    }

    // AC4: non-price events are ignored, not misparsed as ticks.
    #[test]
    fn ignores_non_price_events() {
        assert!(parse_price_update_line(r#"{"event":"stream-health","healthy":true}"#).is_none());
        assert!(
            parse_price_update_line(r#"{"event":"stream-error","message":"boom"}"#).is_none()
        );
    }

    // AC4: malformed / incomplete lines are ignored rather than crashing.
    #[test]
    fn ignores_malformed_lines() {
        assert!(parse_price_update_line("").is_none());
        assert!(parse_price_update_line("not json").is_none());
        assert!(parse_price_update_line("{").is_none());
        // price-update with a zero bid — no usable quote.
        assert!(parse_price_update_line(
            r#"{"event":"price-update","instrument":"EUR_USD","bid":"0","ask":"1.0","time":"2024-01-15T10:30:00Z"}"#
        )
        .is_none());
        // price-update missing the time field.
        assert!(parse_price_update_line(
            r#"{"event":"price-update","instrument":"EUR_USD","bid":"1.0","ask":"1.1"}"#
        )
        .is_none());
    }

    // AC4: the hub-not-streaming-this-instrument fallback decision. Given the
    // instruments the hub is observed streaming, the requested watchlist splits
    // cleanly into hub-backed vs. direct-fallback.
    #[test]
    fn partitions_watchlist_by_hub_coverage() {
        let requested = vec![
            "EUR_USD".to_string(),
            "GBP_USD".to_string(),
            "USD_JPY".to_string(),
        ];
        let hub: HashSet<String> =
            ["EUR_USD".to_string(), "USD_JPY".to_string()].into_iter().collect();

        let (on_hub, direct) = partition_watchlist(&requested, &hub);
        assert_eq!(on_hub, vec!["EUR_USD".to_string(), "USD_JPY".to_string()]);
        assert_eq!(direct, vec!["GBP_USD".to_string()], "uncovered instrument falls back to direct");
    }

    // When the hub streams nothing we asked for, every instrument falls back.
    #[test]
    fn partitions_all_to_direct_when_hub_covers_none() {
        let requested = vec!["EUR_USD".to_string(), "GBP_USD".to_string()];
        let hub: HashSet<String> = ["AUD_USD".to_string()].into_iter().collect();
        let (on_hub, direct) = partition_watchlist(&requested, &hub);
        assert!(on_hub.is_empty());
        assert_eq!(direct, requested);
    }

    // The feed fans a real hub connection out to the right per-instrument
    // channel, records coverage, and drops ticks for unrequested instruments.
    #[tokio::test]
    async fn feed_dispatches_ticks_to_the_matching_instrument_channel() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::UnixListener;

        let path = std::env::temp_dir()
            .join(format!("wickd-hub-feed-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        // Server side stands in for `wickd stream`: emit a couple of ticks.
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let lines = [
                r#"{"event":"price-update","instrument":"EUR_USD","bid":"1.0850","ask":"1.0852","time":"2024-01-15T10:00:00Z","tradeable":true}"#,
                r#"{"event":"stream-health","healthy":true}"#,
                r#"{"event":"price-update","instrument":"GBP_USD","bid":"1.2500","ask":"1.2502","time":"2024-01-15T10:00:01Z","tradeable":true}"#,
            ];
            for l in lines {
                sock.write_all(l.as_bytes()).await.unwrap();
                sock.write_all(b"\n").await.unwrap();
            }
            // Keep the connection open briefly so the client can drain.
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        let client = UnixStream::connect(&path).await.unwrap();
        // Only EUR_USD is requested; GBP_USD ticks must be dropped.
        let (feed, mut receivers) = HubFeed::attach(client, &["EUR_USD".to_string()]);

        let observed = discover_instruments(&feed, &["EUR_USD".to_string()], Duration::from_secs(2)).await;
        assert!(observed.contains("EUR_USD"));
        // Coverage is recorded for every instrument seen, even unrequested ones.
        assert!(observed.contains("GBP_USD"));

        let rx = receivers.remove("EUR_USD").unwrap();
        let tick = rx.recv_timeout(Duration::from_secs(1)).expect("EUR_USD tick delivered");
        assert_eq!(tick.price, rust_decimal_macros::dec!(1.0851));

        drop(feed);
        let _ = server.await;
        let _ = std::fs::remove_file(&path);
    }
}
