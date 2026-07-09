//! `wickd stream` — live OANDA prices as JSON-lines (NDJSON).
//!
//!   wickd stream EUR_USD,GBP_USD
//!   wickd stream --list majors
//!   wickd stream all
//!
//! Subscribes to the OANDA price stream and writes one JSON object per event to
//! stdout until interrupted (Ctrl-C) or the consumer's pipe closes (e.g.
//! `wickd stream | head`). The agent watches this stream and decides what to
//! do — no pattern-matching happens here.
//!
//! ## Socket-hub fan-out (AGT-615)
//!
//! stdout is a single consumer. To let *any number* of readers (a dashboard,
//! the watcher, an agent) share the one OANDA subscription, the command also
//! stands up a Unix-domain-socket hub at `~/.wickd/stream.sock`: every NDJSON
//! line written to stdout is fanned out, byte-identical, to each connected
//! client. Attach with e.g. `nc -U ~/.wickd/stream.sock` or `socat - UNIX-CONNECT:~/.wickd/stream.sock`.
//!
//! **Exit policy (RESOLVED):** the hub lives exactly as long as this `wickd
//! stream` process. Clients connecting/disconnecting never start or stop the
//! OANDA subscription — it is on-demand, tied to the operator's invocation.
//! Ctrl-C / SIGTERM (or the stdout consumer's pipe closing) tears the hub down
//! and removes the socket. A stale socket left by a crash is reclaimed on the
//! next start; a slow client that lags is dropped rather than stalling the read
//! loop. See [`crate::stream_hub`] for the full write-up.
//!
//! Instruments come from (AGT-614, see `crate::watchlist` for the full
//! precedence): an explicit CLI comma-list, a named `--list`, the watchlist
//! file's `default`, or the built-in `majors` fallback. `"all"` resolves at
//! request time via `endpoints::get_instruments` — it is never persisted.
//! Every requested symbol is checked against that instrument list; unknown
//! symbols are skipped with a warning rather than 400ing the whole
//! subscription (OANDA's multi-instrument stream endpoint rejects the entire
//! request if *any* instrument in it is invalid).

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::Args;

use wickd_core::event_sink::EventSink;
use wickd_core::oanda::endpoints;
use wickd_core::oanda::streaming::PriceStreamer;
use wickd_core::oanda::OandaClient;

use crate::commands::client;
use crate::output::{exit, Out};
use crate::sink::NdjsonSink;
use crate::stream_hub::StreamHub;
use crate::watchlist::{self, ListSpec};

#[derive(Args, Debug)]
pub struct StreamArgs {
    /// Comma-separated instruments, e.g. EUR_USD,GBP_USD. A single `all`
    /// resolves to every instrument OANDA offers this account. Omit to use
    /// `--list`, the watchlist file's default, or the built-in `majors`.
    #[arg(value_delimiter = ',')]
    pub instruments: Vec<String>,
    /// Named watchlist to stream (see `~/.wickd/watchlist.json`), or `all`.
    /// Ignored if instruments are given explicitly.
    #[arg(long)]
    pub list: Option<String>,
    /// OANDA environment whose stored credentials are used.
    #[arg(long, default_value = "practice")]
    pub env: String,
    /// Named account within --env whose credentials are used (AGT-625), e.g.
    /// h004. Default: the single/default account.
    #[arg(long, default_value = crate::vault_store::DEFAULT_ACCOUNT)]
    pub account: String,
}

pub async fn run(args: StreamArgs, out: Out) -> ! {
    match stream(args).await {
        Ok(()) => std::process::exit(exit::OK),
        Err(e) => {
            let msg = format!("{e:#}");
            let code = if msg.contains("keychain") || msg.contains("credentials") {
                exit::AUTH
            } else if msg.contains("unknown watchlist") || msg.contains("no valid instruments") {
                exit::VALIDATION
            } else {
                exit::OANDA
            };
            out.fail(code, "stream_failed", msg);
        }
    }
}

async fn stream(args: StreamArgs) -> Result<()> {
    let spec = watchlist::resolve_spec(
        (!args.instruments.is_empty()).then_some(args.instruments.as_slice()),
        args.list.as_deref(),
    )?;

    let (env, api_key, account_id) = client::resolve_credentials(&args.env, &args.account)?;
    let client = OandaClient::with_credentials(&api_key, &account_id, env)
        .map_err(|e| anyhow!("failed to construct OANDA client: {e}"))?;

    // Always fetch the account's tradeable instruments: "all" needs them to
    // expand, and every other source needs them to validate against, so one
    // bad/stale symbol in a watchlist file doesn't 400 the whole batch
    // subscription (AC2/AC3).
    let known = endpoints::get_instruments(&client)
        .await
        .context("fetching OANDA instrument list")?;
    let known_names: HashSet<&str> = known.iter().map(|i| i.name.as_str()).collect();

    let instruments: Vec<String> = match spec {
        ListSpec::All => known.iter().map(|i| i.name.clone()).collect(),
        ListSpec::Symbols(requested) => {
            let mut resolved = Vec::with_capacity(requested.len());
            for symbol in requested {
                if known_names.contains(symbol.as_str()) {
                    resolved.push(symbol);
                } else {
                    eprintln!(
                        "warning: skipping unknown instrument '{symbol}' (not offered by this OANDA account)"
                    );
                }
            }
            resolved
        }
    };

    if instruments.is_empty() {
        return Err(anyhow!("no valid instruments to stream (after filtering unknown symbols)"));
    }

    let mut streamer = PriceStreamer::new(&api_key, &account_id, &env);

    // AGT-615: stand up the socket hub before subscribing, so the single OANDA
    // subscription fans out to every client connected on `~/.wickd/stream.sock`
    // (identical NDJSON lines) as well as stdout. Binding also reclaims a stale
    // socket left by a crash and refuses to start a second concurrent stream.
    let hub = StreamHub::bind().await.context("starting stream socket hub")?;
    let (sink_impl, consumer_gone) = NdjsonSink::with_hub(hub.sender());
    // The always-on stream process is the main contributor to the persistent
    // per-instrument spread history (`~/.wickd/spreads.db`) that grades live
    // spreads in the ticket view. Best-effort: wrap() falls back to the bare
    // sink if the DB can't open.
    let sink: Arc<dyn EventSink> =
        crate::spread_stats::SpreadSamplingSink::wrap(Arc::new(sink_impl));

    // AC3: one batch subscribe call for the whole watchlist — a single OANDA
    // connection — instead of looping `subscribe()` per instrument, which
    // restarts the stream (and its connection) on every addition and trips
    // OANDA's reconnect-rate limit once a watchlist has more than a couple of
    // instruments.
    #[allow(deprecated)]
    streamer
        .start(instruments, sink)
        .await
        .context("subscribing to instrument stream")?;

    // AC4 (AGT-615 exit policy — RESOLVED): the hub lives exactly as long as
    // this `wickd stream` process. Clients connecting/disconnecting on the
    // socket never start or stop the OANDA subscription (on-demand = the
    // operator ran `wickd stream`). We exit — and tear the hub down, removing
    // the socket — on Ctrl-C (SIGINT), SIGTERM (e.g. a supervisor stopping us),
    // *or* when the stdout consumer's pipe closes (e.g. `wickd stream | head`),
    // instead of hanging forever waiting on a Ctrl-C a closed pipe never sends.
    // SIGKILL can't be caught — that leaves a stale socket, reclaimed on the
    // next start (AC2). See `crate::stream_hub` for the full policy write-up.
    wait_for_shutdown(&consumer_gone).await;
    streamer.stop();
    // Remove `~/.wickd/stream.sock` explicitly: `run` calls `process::exit`, so
    // `StreamHub`'s `Drop` would never run (AC2 cleanup-on-exit).
    hub.shutdown();
    Ok(())
}

/// Block until any of our clean-exit triggers fires: Ctrl-C (SIGINT), SIGTERM,
/// or the stdout consumer's pipe closing. SIGTERM is handled explicitly (Unix
/// only) because `tokio::signal::ctrl_c()` covers only SIGINT — without this a
/// `kill <pid>` would default-terminate the process and skip socket cleanup.
async fn wait_for_shutdown(consumer_gone: &tokio::sync::Notify) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        // If we can't install the SIGTERM handler, fall back to just SIGINT +
        // pipe-close rather than failing the stream.
        let mut sigterm = signal(SignalKind::terminate()).ok();
        let sigterm_fut = async {
            match sigterm.as_mut() {
                Some(s) => {
                    s.recv().await;
                }
                // Never resolves: leaves the other two arms in control.
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm_fut => {}
            _ = consumer_gone.notified() => {}
        }
    }
    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = consumer_gone.notified() => {}
        }
    }
}
