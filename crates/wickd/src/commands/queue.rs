//! `wickd queue` — poll the durable alert queue, and the promote-to-pending
//! bridge (AGT-620).
//!
//!   wickd queue list                 # queued alert events (oldest first)
//!   wickd queue list --limit 20
//!   wickd queue list --follow        # tail: emit new queued alerts as NDJSON
//!   wickd queue promote <id>         # bridge a strategy-signal alert → pending
//!
//! `list` surfaces the alert events that `wickd alert run` (price-level) and
//! `wickd watch` (strategy-signal) durably appended to
//! `~/.wickd/alert-queue.ndjson` — see [`crate::alert_queue`] for the schema.
//! Read-only: listing never executes or mutates anything.
//!
//! `promote` is the **explicit** bridge (AC3) from an actionable strategy-signal
//! alert to an execution proposal: it takes the [`PendingSignal`] the alert
//! carries and appends it into `~/.wickd/pending.json` (the store owned by
//! [`crate::pending`]). It is deliberate and manual — a queued alert never
//! promotes itself. Only strategy-signal (Buy/Sell) alerts are promotable;
//! price-level alerts carry no order intent and are rejected. Promoting does
//! NOT place an order: the proposal still awaits a separate `wickd approve <id>`.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use clap::{Args, Subcommand};

use crate::alert_queue;
use crate::output::{exit, Out};
use crate::pending::{self, PendingSignal};

#[derive(Args, Debug)]
pub struct QueueArgs {
    #[command(subcommand)]
    cmd: QueueCmd,
}

#[derive(Subcommand, Debug)]
enum QueueCmd {
    /// List queued alert events (oldest first), or tail with --follow.
    List(ListArgs),
    /// Promote a queued strategy-signal alert into a pending execution proposal.
    Promote(PromoteArgs),
}

#[derive(Args, Debug)]
struct ListArgs {
    /// Cap the number of (most-recent) queued alerts returned.
    #[arg(long, default_value_t = 50)]
    limit: usize,
    /// Keep watching and emit each newly-queued alert as an NDJSON line until
    /// Ctrl-C — the poll/tail feed an agent consumes (AC2).
    #[arg(long)]
    follow: bool,
}

#[derive(Args, Debug)]
struct PromoteArgs {
    /// Queue-entry id to promote (see `wickd queue list`).
    id: String,
}

pub async fn run(args: QueueArgs, out: Out) -> ! {
    match args.cmd {
        QueueCmd::List(a) => {
            if a.follow {
                follow(a.limit, out).await
            } else {
                finish(list(a), out)
            }
        }
        QueueCmd::Promote(p) => finish(promote(p), out),
    }
}

/// Print `result` and exit — the shared success/failure envelope.
fn finish(result: Result<serde_json::Value>, out: Out) -> ! {
    match result {
        Ok(v) => {
            out.ok(&v);
            std::process::exit(exit::OK);
        }
        Err(e) => {
            let msg = format!("{e:#}");
            out.fail(classify(&msg), "queue_failed", msg);
        }
    }
}

/// Map a queue-command error message to a stable exit code.
fn classify(msg: &str) -> i32 {
    if msg.contains("no queued alert")
        || msg.contains("price-level")
        || msg.contains("already awaiting approval")
        || msg.contains("already been acted on")
    {
        exit::VALIDATION
    } else {
        exit::GENERIC
    }
}

fn list(args: ListArgs) -> Result<serde_json::Value> {
    let path = alert_queue::queue_path()?;
    let mut entries = alert_queue::list_at(&path)?;
    // Keep the most-recent `limit`, still in oldest-first (tail) order.
    if entries.len() > args.limit {
        entries = entries.split_off(entries.len() - args.limit);
    }
    Ok(serde_json::json!({
        "count": entries.len(),
        "queue": entries,
    }))
}

/// `--follow`: emit the current tail as NDJSON, then poll for new entries and
/// emit each new one as a line, until Ctrl-C. Never returns.
async fn follow(limit: usize, out: Out) -> ! {
    let path = match alert_queue::queue_path() {
        Ok(p) => p,
        Err(e) => out.fail(exit::GENERIC, "queue_failed", format!("{e:#}")),
    };

    // Seed with the existing tail (bounded by `limit`), emitting each as a line.
    let mut seen = match alert_queue::list_at(&path) {
        Ok(entries) => {
            let start = entries.len().saturating_sub(limit);
            for entry in &entries[start..] {
                emit_line(entry);
            }
            entries.len()
        }
        Err(e) => out.fail(exit::GENERIC, "queue_failed", format!("{e:#}")),
    };

    let mut ticker = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => std::process::exit(exit::OK),
            _ = ticker.tick() => {
                match alert_queue::list_at(&path) {
                    Ok(entries) if entries.len() > seen => {
                        for entry in &entries[seen..] {
                            emit_line(entry);
                        }
                        seen = entries.len();
                    }
                    Ok(_) => {}
                    // A transient read error mid-tail shouldn't kill the follow;
                    // report it and keep polling.
                    Err(e) => eprintln!("warning: alert queue read failed: {e:#}"),
                }
            }
        }
    }
}

/// Emit one queued alert as a single NDJSON line (the tail feed's line shape).
fn emit_line(entry: &alert_queue::QueuedAlert) {
    if let Ok(line) = serde_json::to_string(entry) {
        println!("{line}");
    }
}

fn promote(args: PromoteArgs) -> Result<serde_json::Value> {
    let proposal = promote_at(
        alert_queue::queue_path()?,
        pending::pending_path()?,
        &args.id,
    )?;
    Ok(serde_json::json!({
        "promoted": true,
        "queue_id": args.id,
        "pending_id": proposal.id,
        "proposal": proposal,
    }))
}

/// Core of `promote`, split out so it is testable against temp stores: load the
/// named queued alert, verify it is a promotable strategy-signal, and append its
/// proposal into the pending store — refusing to promote the same alert twice.
///
/// This is the ONLY code that moves a record from the alert queue into
/// `pending.json`, and it runs solely on an explicit `wickd queue promote`
/// invocation (AC3). It never places an order.
fn promote_at(
    queue_path: impl AsRef<std::path::Path>,
    pending_path: impl AsRef<std::path::Path>,
    id: &str,
) -> Result<PendingSignal> {
    let entry = alert_queue::get_at(&queue_path, id)?
        .ok_or_else(|| anyhow!("no queued alert '{}'", id))?;

    let proposal = entry.promotable_proposal().cloned().ok_or_else(|| {
        anyhow!(
            "queued alert '{}' is a price-level alert and carries no order intent — \
             only strategy-signal (buy/sell) alerts can be promoted",
            id
        )
    })?;

    // Duplicate guard: a pending record for this signal may already exist — from
    // a prior `queue promote` OR from `watch --semi-auto`, which records a
    // pending directly under the same deterministic proposal id. Either way a
    // second append would duplicate the id in pending.json, so refuse — but say
    // what actually happened instead of claiming a promotion did (#287).
    if let Some(existing) = pending::get_at(&pending_path, &proposal.id)? {
        if existing.status == pending::STATUS_PENDING {
            bail!(
                "a pending signal '{}' for this alert is already awaiting approval \
                 (recorded by a prior promote or `watch --semi-auto`) — \
                 see `wickd pending`, then `wickd approve {}`",
                proposal.id,
                proposal.id
            );
        }
        bail!(
            "the signal behind queued alert '{}' has already been acted on \
             (pending signal '{}' status: {}) — promoting again would duplicate \
             a completed proposal",
            id,
            proposal.id,
            existing.status
        );
    }

    pending::append_at(&pending_path, &proposal)?;
    Ok(proposal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alert::Direction;
    use crate::alert_queue::QueuedAlert;
    use crate::pending::STATUS_PENDING;
    use crate::signal_alert::AlertSignal;

    fn temp_path(tag: &str, ext: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static C: AtomicU64 = AtomicU64::new(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "wickd-{}-test-{}-{}.{}",
            tag,
            std::process::id(),
            C.fetch_add(1, Ordering::Relaxed),
            ext
        ));
        p
    }

    fn proposal(id: &str, side: &str, units: i64) -> PendingSignal {
        PendingSignal {
            id: id.to_string(),
            ts: "2026-06-30T00:00:00+00:00".to_string(),
            instrument: "EUR_USD".to_string(),
            side: side.to_string(),
            units,
            suggested_units: None,
            strategy: "ma-crossover".to_string(),
            reason: "fast SMA crossed above slow".to_string(),
            sl: Some("1.0800".to_string()),
            tp: Some("1.0950".to_string()),
            entry_price: Some("1.0850".to_string()),
            status: STATUS_PENDING.to_string(),
        }
    }

    // AC3: promoting a queued strategy-signal alert lands a well-formed
    // PendingSignal in the (temp) pending store, while the queue entry itself is
    // untouched — promotion is a copy into pending.json, not a consume of the
    // queue. Never touches the real ~/.wickd stores.
    #[test]
    fn promote_lands_a_pending_proposal_in_temp_store() {
        let qpath = temp_path("queue", "ndjson");
        let ppath = temp_path("pending", "json");

        let alert = QueuedAlert::strategy_signal(
            "2026-06-30T00:00:00Z".to_string(),
            AlertSignal::Buy,
            proposal("match-1", "long", 1000),
        );
        alert_queue::append_at(&qpath, &alert).unwrap();

        // Pending store starts empty.
        assert!(pending::list_at(&ppath).unwrap().is_empty());

        let promoted = promote_at(&qpath, &ppath, &alert.id).unwrap();
        assert_eq!(promoted.id, "match-1");

        // The proposal is now a pending signal in the temp pending store.
        let pending_list = pending::list_at(&ppath).unwrap();
        assert_eq!(pending_list.len(), 1);
        assert_eq!(pending_list[0].id, "match-1");
        assert_eq!(pending_list[0].side, "long");
        assert_eq!(pending_list[0].status, STATUS_PENDING);

        // The queue entry is unchanged — promote copies, it does not consume.
        assert!(alert_queue::get_at(&qpath, &alert.id).unwrap().is_some());

        let _ = std::fs::remove_file(&qpath);
        let _ = std::fs::remove_file(&ppath);
    }

    // AC3: a price-level alert carries no order intent, so promoting it is a
    // validation error and nothing is written to the pending store.
    #[test]
    fn promote_rejects_a_price_level_alert() {
        let qpath = temp_path("queue", "ndjson");
        let ppath = temp_path("pending", "json");

        let alert = QueuedAlert::price_level(
            "2026-06-30T00:00:05Z".to_string(),
            "EUR_USD".to_string(),
            "1.0900".to_string(),
            Direction::CrossUp,
            "1.0905".to_string(),
        );
        alert_queue::append_at(&qpath, &alert).unwrap();

        let err = promote_at(&qpath, &ppath, &alert.id).unwrap_err();
        assert!(err.to_string().contains("price-level"), "unexpected: {err}");
        assert_eq!(classify(&format!("{err:#}")), exit::VALIDATION);
        // Nothing promoted.
        assert!(pending::list_at(&ppath).unwrap().is_empty());

        let _ = std::fs::remove_file(&qpath);
        let _ = std::fs::remove_file(&ppath);
    }

    // An unknown queue id is a validation error.
    #[test]
    fn promote_unknown_id_is_a_validation_error() {
        let qpath = temp_path("queue", "ndjson");
        let ppath = temp_path("pending", "json");
        let err = promote_at(&qpath, &ppath, "definitely-not-real").unwrap_err();
        assert!(err.to_string().contains("no queued alert"), "unexpected: {err}");
        assert_eq!(classify(&format!("{err:#}")), exit::VALIDATION);
        let _ = std::fs::remove_file(&qpath);
        let _ = std::fs::remove_file(&ppath);
    }

    // Promoting the same alert twice is refused — no silent duplicate proposal.
    #[test]
    fn double_promote_is_refused() {
        let qpath = temp_path("queue", "ndjson");
        let ppath = temp_path("pending", "json");

        let alert = QueuedAlert::strategy_signal(
            "2026-06-30T00:00:00Z".to_string(),
            AlertSignal::Sell,
            proposal("match-2", "short", -1000),
        );
        alert_queue::append_at(&qpath, &alert).unwrap();

        promote_at(&qpath, &ppath, &alert.id).unwrap();
        let err = promote_at(&qpath, &ppath, &alert.id).unwrap_err();
        assert!(err.to_string().contains("already awaiting approval"), "unexpected: {err}");
        assert_eq!(classify(&format!("{err:#}")), exit::VALIDATION);
        // Still exactly one pending signal.
        assert_eq!(pending::list_at(&ppath).unwrap().len(), 1);

        let _ = std::fs::remove_file(&qpath);
        let _ = std::fs::remove_file(&ppath);
    }

    // #287 case 1: `watch --semi-auto` records a pending directly under the same
    // deterministic proposal id, so promoting the mirrored queue entry collides
    // with a pending the user never promoted. The refusal must not claim a
    // promotion happened — it reports an awaiting-approval pending instead.
    #[test]
    fn promote_with_semi_auto_pending_reports_awaiting_approval() {
        let qpath = temp_path("queue", "ndjson");
        let ppath = temp_path("pending", "json");

        let sig = proposal("match-3", "long", 1000);
        let alert = QueuedAlert::strategy_signal(
            "2026-06-30T00:00:00Z".to_string(),
            AlertSignal::Buy,
            sig.clone(),
        );
        alert_queue::append_at(&qpath, &alert).unwrap();
        // Simulate the semi-auto sink: the pending exists, but no promote ran.
        pending::append_at(&ppath, &sig).unwrap();

        let err = promote_at(&qpath, &ppath, &alert.id).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("already awaiting approval"), "unexpected: {err}");
        assert!(!msg.contains("promoted"), "must not claim a promotion happened: {err}");
        assert_eq!(classify(&format!("{err:#}")), exit::VALIDATION);
        // No duplicate appended.
        assert_eq!(pending::list_at(&ppath).unwrap().len(), 1);

        let _ = std::fs::remove_file(&qpath);
        let _ = std::fs::remove_file(&ppath);
    }

    // #287 case 2: a terminal (consumed) pending still blocks promotion — the
    // signal was already acted on and a re-promote would duplicate its id in
    // the store — but the refusal now says why, naming the terminal status.
    #[test]
    fn promote_with_consumed_pending_reports_already_acted_on() {
        let qpath = temp_path("queue", "ndjson");
        let ppath = temp_path("pending", "json");

        let sig = proposal("match-4", "short", -1000);
        let alert = QueuedAlert::strategy_signal(
            "2026-06-30T00:00:00Z".to_string(),
            AlertSignal::Sell,
            sig.clone(),
        );
        alert_queue::append_at(&qpath, &alert).unwrap();
        pending::append_at(&ppath, &sig).unwrap();
        // Approve path finished with it: pending → consumed (terminal).
        assert!(pending::consume_at(&ppath, "match-4").unwrap());

        let err = promote_at(&qpath, &ppath, &alert.id).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("already been acted on"), "unexpected: {err}");
        assert!(msg.contains("consumed"), "should name the terminal status: {err}");
        assert_eq!(classify(&format!("{err:#}")), exit::VALIDATION);
        // Store untouched: one record, still consumed, no duplicate.
        assert!(pending::list_at(&ppath).unwrap().is_empty());
        assert_eq!(
            pending::get_at(&ppath, "match-4").unwrap().unwrap().status,
            pending::STATUS_CONSUMED
        );

        let _ = std::fs::remove_file(&qpath);
        let _ = std::fs::remove_file(&ppath);
    }
}
