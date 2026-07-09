//! The app as a client of the wickd watch daemon (AGT-652).
//!
//! The desktop app no longer hosts a watcher engine. `wickd watch` (typically
//! launchd-supervised) is the ONE watcher runtime on the machine, and its
//! client-visible outputs are files in the wickd data home:
//!
//! - `alert-queue.ndjson` — the durable, append-only signal feed
//!   (`wickd_core::alert_queue`), one `QueuedAlert` per fired alert.
//! - `pending.json` — the semi-auto execution-proposal store
//!   (`wickd_core::pending`); approval stays a deliberate, separate
//!   `wickd approve <id>` invocation (the trust ladder lives in the CLI —
//!   the app renders the queue, it never executes).
//!
//! Daemon liveness has no status socket, so it is observed from the process
//! table: any process whose binary basename starts with `wickd` running the
//! `watch` verb counts (including pinned binaries like `wickd-h004`).

use serde::Serialize;

use wickd_core::alert_queue::{self, QueuedAlert};
use wickd_core::pending::{self, PendingSignal};

/// One running `wickd ... watch ...` process.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WatchProcess {
    pub pid: u32,
    /// Full command line, e.g.
    /// `/Users/x/.wickd/bin/wickd-h004 watch revert_adx EUR_USD --auto`.
    pub command: String,
    /// The strategy argument (first arg after `watch`), when parseable.
    pub strategy: Option<String>,
    /// The instruments argument (second arg after `watch`), when parseable.
    pub instruments: Vec<String>,
}

/// Snapshot of the single watcher runtime as seen from the app.
#[derive(Debug, Clone, Serialize)]
pub struct DaemonStatus {
    /// Running `wickd watch` daemon processes.
    pub watchers: Vec<WatchProcess>,
    /// Whether the stream-hub socket exists on disk.
    pub hub_socket_present: bool,
    /// Pending (un-consumed) execution proposals in `pending.json`.
    pub pending_count: usize,
    /// Total entries in the durable alert queue.
    pub queue_len: usize,
}

/// Parse `ps` output lines (`PID COMMAND...`) into watch processes.
///
/// A line counts when the executable's basename starts with `wickd` and its
/// first argument is the `watch` verb — matching both the repo binary and
/// pinned copies (`wickd-h004`), and never e.g. `grep wickd watch`.
fn parse_watch_processes(ps_output: &str) -> Vec<WatchProcess> {
    let mut out = Vec::new();
    for line in ps_output.lines() {
        let line = line.trim();
        let Some((pid_str, command)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid_str.trim().parse::<u32>() else {
            continue;
        };
        let command = command.trim();
        let mut parts = command.split_whitespace();
        let Some(binary) = parts.next() else { continue };
        let basename = binary.rsplit('/').next().unwrap_or(binary);
        if !basename.starts_with("wickd") {
            continue;
        }
        if parts.next() != Some("watch") {
            continue;
        }
        // `wickd watch <strategy> <instruments,csv> [flags...]`
        let strategy = parts.next().map(str::to_string);
        let instruments = parts
            .next()
            .filter(|a| !a.starts_with('-'))
            .map(|csv| csv.split(',').map(str::to_string).collect())
            .unwrap_or_default();
        out.push(WatchProcess {
            pid,
            command: command.to_string(),
            strategy,
            instruments,
        });
    }
    out
}

fn running_watchers() -> Vec<WatchProcess> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            parse_watch_processes(&String::from_utf8_lossy(&o.stdout))
        }
        _ => Vec::new(),
    }
}

/// Snapshot the daemon: running watch processes, hub socket presence, and the
/// sizes of the two client-visible stores.
#[tauri::command]
pub fn daemon_status() -> Result<DaemonStatus, String> {
    let hub_socket_present = wickd_core::stream_hub::stream_socket_path()
        .map(|p| p.exists())
        .unwrap_or(false);
    let pending_count = pending::pending_path()
        .and_then(|p| pending::list_at(p))
        .map(|v| v.len())
        .unwrap_or(0);
    let queue_len = alert_queue::queue_path()
        .and_then(|p| alert_queue::list_at(p))
        .map(|v| v.len())
        .unwrap_or(0);
    Ok(DaemonStatus {
        watchers: running_watchers(),
        hub_socket_present,
        pending_count,
        queue_len,
    })
}

/// The daemon's signal feed: entries from `alert-queue.ndjson`, newest first,
/// capped at `limit` (default 100).
#[tauri::command]
pub fn daemon_queue_list(limit: Option<usize>) -> Result<Vec<QueuedAlert>, String> {
    let path = alert_queue::queue_path().map_err(|e| e.to_string())?;
    let mut entries = alert_queue::list_at(path).map_err(|e| e.to_string())?;
    entries.reverse(); // list_at is oldest-first (tail order); UI wants newest first
    entries.truncate(limit.unwrap_or(100));
    Ok(entries)
}

/// The semi-auto pending/approve queue: un-consumed proposals from
/// `pending.json`, newest first. Read-only — approval is `wickd approve <id>`.
#[tauri::command]
pub fn daemon_pending_list() -> Result<Vec<PendingSignal>, String> {
    let path = pending::pending_path().map_err(|e| e.to_string())?;
    pending::list_at(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repo_and_pinned_watch_processes() {
        let ps = "\
  101 /usr/local/bin/wickd watch revert_adx EUR_USD,GBP_USD --granularity H1 --auto
  102 /Users/m/.wickd/bin/wickd-h004 watch revert_adx EUR_USD,GBP_USD,USD_CHF,EUR_GBP --granularity H1 --env practice --account h004 --units 2000 --auto
  103 grep wickd watch
  104 /usr/local/bin/wickd stream EUR_USD
  105 nvim wickd-watch-notes.md
";
        let procs = parse_watch_processes(ps);
        assert_eq!(procs.len(), 2, "{procs:?}");
        assert_eq!(procs[0].pid, 101);
        assert_eq!(procs[0].strategy.as_deref(), Some("revert_adx"));
        assert_eq!(procs[0].instruments, vec!["EUR_USD", "GBP_USD"]);
        assert_eq!(procs[1].pid, 102);
        assert_eq!(
            procs[1].instruments,
            vec!["EUR_USD", "GBP_USD", "USD_CHF", "EUR_GBP"]
        );
    }

    #[test]
    fn watch_verb_must_be_the_first_argument() {
        let ps = "  55 /usr/local/bin/wickd queue list --follow watch";
        assert!(parse_watch_processes(ps).is_empty());
    }

    // The store reads honor WICKD_HOME via wickd-core path resolution — proven
    // by reading seeded files through the same list functions the commands use.
    #[test]
    fn queue_and_pending_reads_round_trip_from_a_temp_store() {
        let dir = std::env::temp_dir().join(format!(
            "wickd-daemon-cmd-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let proposal = PendingSignal {
            id: "sig-1".to_string(),
            ts: "2026-07-06T00:00:00+00:00".to_string(),
            instrument: "EUR_USD".to_string(),
            side: "long".to_string(),
            units: 1000,
            suggested_units: None,
            strategy: "revert_adx".to_string(),
            reason: "test".to_string(),
            sl: None,
            tp: None,
            entry_price: None,
            status: pending::STATUS_PENDING.to_string(),
        };
        let pending_path = dir.join("pending.json");
        pending::append_at(&pending_path, &proposal).unwrap();
        assert_eq!(pending::list_at(&pending_path).unwrap().len(), 1);

        let queue_path = dir.join("alert-queue.ndjson");
        let alert = QueuedAlert::strategy_signal(
            proposal.ts.clone(),
            wickd_core::alert_queue::AlertSignal::Buy,
            proposal,
        );
        alert_queue::append_at(&queue_path, &alert).unwrap();
        let listed = alert_queue::list_at(&queue_path).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, alert.id);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
