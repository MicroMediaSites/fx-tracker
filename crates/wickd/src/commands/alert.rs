//! `wickd alert` — price-level alerts: define, evaluate, and fire (AGT-617).
//!
//!   wickd alert add --instrument EUR_USD --price 1.0900 --direction cross-up
//!   wickd alert add --instrument EUR_USD --price 1.0800 --direction cross-down --source bid --rearm 3
//!   wickd alert list
//!   wickd alert remove <id>
//!   wickd alert run [--env practice]
//!
//! `add`/`list`/`remove` are one-shot verbs that manage alerts persisted to
//! `~/.wickd/alerts.json` — see [`crate::alert`] for the schema and the
//! evaluate/fire/re-arm mechanism. `run` is the long-running command that
//! actually watches a live OANDA price stream and evaluates every stored
//! alert against it in real time.
//!
//! `run` needs no stream-hub/substrate ticket (AGT-614/615): it opens its
//! own single OANDA price subscription — the same [`PriceStreamer`] `wickd
//! stream` uses — covering whatever instruments the alert store currently
//! references, and drives each tick through [`crate::sink::AlertSink`]. Like
//! `watch`, it is monitoring-only and runs until Ctrl-C/SIGTERM.

use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use rust_decimal::Decimal;
use std::str::FromStr;

use wickd_core::event_sink::EventSink;
use wickd_core::oanda::streaming::PriceStreamer;

use crate::alert::{self, Alert, Direction, Source};
use crate::commands::client;
use crate::vault_store;
use crate::feed::Format;
use crate::output::{exit, Out};
use crate::sink::AlertSink;

#[derive(Args, Debug)]
pub struct AlertArgs {
    #[command(subcommand)]
    cmd: AlertCmd,
}

#[derive(Subcommand, Debug)]
enum AlertCmd {
    /// Define a new armed price alert.
    Add(AddArgs),
    /// List all alerts (armed + fired) with current status.
    List,
    /// Remove an alert by id.
    Remove(RemoveArgs),
    /// Watch a live price stream and evaluate/fire every stored alert.
    Run(RunArgs),
}

#[derive(Args, Debug)]
struct AddArgs {
    /// Instrument to watch, e.g. EUR_USD.
    #[arg(long)]
    instrument: String,
    /// Price level to watch for a cross.
    #[arg(long)]
    price: String,
    /// cross-up | cross-down | either.
    #[arg(long)]
    direction: String,
    /// Which quoted price triggers the alert: bid | ask | mid.
    #[arg(long, default_value = "mid")]
    source: String,
    /// Hysteresis re-arm band, in pips.
    #[arg(long, default_value = "5")]
    rearm: String,
}

#[derive(Args, Debug)]
struct RemoveArgs {
    /// Id of the alert to remove (see `wickd alert list`).
    id: String,
}

#[derive(Args, Debug)]
struct RunArgs {
    /// OANDA environment whose stored credentials are used.
    #[arg(long, default_value = "practice")]
    env: String,
    /// Delivery format for fires: `ndjson` (default, machine-readable) or
    /// `human` for a live terminal feed — one clear line per fire (AGT-619).
    #[arg(long, value_enum, default_value_t = Format::Ndjson)]
    format: Format,
}

pub async fn run(args: AlertArgs, out: Out) -> ! {
    match args.cmd {
        AlertCmd::Add(a) => finish(add(a), out),
        AlertCmd::List => finish(list(), out),
        AlertCmd::Remove(r) => finish(remove(r), out),
        AlertCmd::Run(r) => run_watch(r, out).await,
    }
}

/// Print `result` and exit — the shared success/failure envelope for the
/// one-shot verbs (`add`/`list`/`remove`).
fn finish(result: Result<serde_json::Value>, out: Out) -> ! {
    match result {
        Ok(v) => {
            out.ok(&v);
            std::process::exit(exit::OK);
        }
        Err(e) => {
            let msg = format!("{e:#}");
            out.fail(classify(&msg), "alert_failed", msg);
        }
    }
}

/// Classify an alert-command error message into a stable exit code, shared by
/// the one-shot verbs and `run`'s error path.
fn classify(msg: &str) -> i32 {
    if msg.contains("keychain") || msg.contains("credentials") {
        exit::AUTH
    } else if msg.contains("invalid")
        || msg.contains("unknown")
        || msg.contains("no alert")
        || msg.contains("must not be")
    {
        exit::VALIDATION
    } else {
        exit::GENERIC
    }
}

fn add(args: AddArgs) -> Result<serde_json::Value> {
    if args.instrument.trim().is_empty() {
        bail!("--instrument must not be empty");
    }
    let price = Decimal::from_str(&args.price).map_err(|_| anyhow!("invalid --price '{}'", args.price))?;
    let direction = Direction::from_str(&args.direction)?;
    let source = Source::from_str(&args.source)?;
    let rearm = Decimal::from_str(&args.rearm).map_err(|_| anyhow!("invalid --rearm '{}'", args.rearm))?;
    if rearm.is_sign_negative() {
        bail!("--rearm must not be negative");
    }

    let created = Alert::new(args.instrument, price, direction, source, rearm);
    alert::add(&created)?;
    Ok(serde_json::json!({ "alert": created }))
}

fn list() -> Result<serde_json::Value> {
    let alerts = alert::list()?;
    Ok(serde_json::json!({ "count": alerts.len(), "alerts": alerts }))
}

fn remove(args: RemoveArgs) -> Result<serde_json::Value> {
    let removed = alert::remove(&args.id)?;
    if !removed {
        bail!("no alert '{}'", args.id);
    }
    Ok(serde_json::json!({ "removed": true, "id": args.id }))
}

async fn run_watch(args: RunArgs, out: Out) -> ! {
    match watch_alerts(args).await {
        Ok(()) => std::process::exit(exit::OK),
        Err(e) => {
            let msg = format!("{e:#}");
            out.fail(classify(&msg), "alert_run_failed", msg);
        }
    }
}

async fn watch_alerts(args: RunArgs) -> Result<()> {
    let store_path = alert::alerts_path()?;
    let instruments = alert::instruments_at(&store_path)?;
    if instruments.is_empty() {
        bail!("no alerts defined yet — add one with `wickd alert add` before `wickd alert run`");
    }

    let (env, api_key, account_id) = client::resolve_credentials(&args.env, vault_store::DEFAULT_ACCOUNT)?;
    let mut streamer = PriceStreamer::new(&api_key, &account_id, &env);
    // AGT-620: mirror every fire into the durable alert queue for agent polling.
    let sink: Arc<dyn EventSink> =
        Arc::new(AlertSink::new(store_path, args.format).with_queue(crate::alert_queue::queue_path()?));

    for instrument in &instruments {
        streamer
            .subscribe(instrument.clone(), sink.clone())
            .await
            .with_context(|| format!("subscribing to {instrument}"))?;
    }

    // Stream runs in background tasks; block here until Ctrl-C, then stop —
    // same shutdown shape as `wickd stream`.
    let _ = tokio::signal::ctrl_c().await;
    streamer.stop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_rejects_unknown_direction() {
        let args = AddArgs {
            instrument: "EUR_USD".to_string(),
            price: "1.0900".to_string(),
            direction: "sideways".to_string(),
            source: "mid".to_string(),
            rearm: "5".to_string(),
        };
        assert!(add(args).is_err());
    }

    #[test]
    fn add_rejects_unknown_source() {
        let args = AddArgs {
            instrument: "EUR_USD".to_string(),
            price: "1.0900".to_string(),
            direction: "cross-up".to_string(),
            source: "last".to_string(),
            rearm: "5".to_string(),
        };
        assert!(add(args).is_err());
    }

    #[test]
    fn add_rejects_invalid_price() {
        let args = AddArgs {
            instrument: "EUR_USD".to_string(),
            price: "not-a-price".to_string(),
            direction: "cross-up".to_string(),
            source: "mid".to_string(),
            rearm: "5".to_string(),
        };
        assert!(add(args).is_err());
    }

    #[test]
    fn add_rejects_negative_rearm() {
        let args = AddArgs {
            instrument: "EUR_USD".to_string(),
            price: "1.0900".to_string(),
            direction: "cross-up".to_string(),
            source: "mid".to_string(),
            rearm: "-1".to_string(),
        };
        assert!(add(args).is_err());
    }

    #[test]
    fn add_rejects_empty_instrument() {
        let args = AddArgs {
            instrument: "   ".to_string(),
            price: "1.0900".to_string(),
            direction: "cross-up".to_string(),
            source: "mid".to_string(),
            rearm: "5".to_string(),
        };
        assert!(add(args).is_err());
    }

    #[test]
    fn remove_unknown_id_is_a_validation_error() {
        // Uses the default store path (like the real `wickd alert remove`),
        // so this only asserts the error shape for a plainly-bogus id rather
        // than exercising an isolated temp store.
        let msg = "no alert 'definitely-not-a-real-id'";
        assert_eq!(classify(msg), exit::VALIDATION);
    }

    #[test]
    fn classify_maps_known_error_tokens() {
        assert_eq!(classify("keychain locked"), exit::AUTH);
        assert_eq!(classify("missing credentials"), exit::AUTH);
        assert_eq!(classify("invalid --price 'x'"), exit::VALIDATION);
        assert_eq!(classify("no alert 'abc'"), exit::VALIDATION);
        assert_eq!(classify("--rearm must not be negative"), exit::VALIDATION);
        assert_eq!(classify("OANDA returned 500"), exit::GENERIC);
    }
}
