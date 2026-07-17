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
use wickd_core::watchers::{running_watchers, WatchProcess};

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

/// Snapshot the daemon: running watch processes, hub socket presence, and the
/// sizes of the two client-visible stores.
#[tauri::command]
pub async fn daemon_status() -> Result<DaemonStatus, String> {
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
pub async fn daemon_queue_list(limit: Option<usize>) -> Result<Vec<QueuedAlert>, String> {
    let path = alert_queue::queue_path().map_err(|e| e.to_string())?;
    let mut entries = alert_queue::list_at(path).map_err(|e| e.to_string())?;
    entries.reverse(); // list_at is oldest-first (tail order); UI wants newest first
    entries.truncate(limit.unwrap_or(100));
    Ok(entries)
}

/// The semi-auto pending/approve queue: un-consumed proposals from
/// `pending.json`, newest first. Read-only — approval is `wickd approve <id>`.
#[tauri::command]
pub async fn daemon_pending_list() -> Result<Vec<PendingSignal>, String> {
    let path = pending::pending_path().map_err(|e| e.to_string())?;
    pending::list_at(path).map_err(|e| e.to_string())
}

// ============================================================================
// Start a watcher from the UI (trust-ladder capped)
// ============================================================================

/// Granularities the UI may start a watcher with — all native OANDA candle
/// granularities the watch daemon accepts (superset of the chart selector).
const UI_WATCH_GRANULARITIES: &[&str] = &[
    "M1", "M5", "M15", "M30", "H1", "H2", "H4", "H6", "H8", "H12", "D", "W",
];

/// Built-in (non-Rhai) strategies `wickd watch` accepts by name.
const BUILTIN_STRATEGIES: &[&str] = &["ma-crossover", "rsi"];

/// Locate the wickd CLI binary. GUI apps don't inherit a login-shell PATH, so
/// probe the conventional install locations before falling back to PATH.
pub(crate) fn find_wickd_binary() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(home) = &home {
        candidates.push(home.join(".cargo/bin/wickd"));
        candidates.push(home.join(".local/bin/wickd"));
    }
    candidates.push("/usr/local/bin/wickd".into());
    candidates.push("/opt/homebrew/bin/wickd".into());
    candidates.into_iter().find(|p| p.is_file()).or_else(|| {
        // Last resort: whatever PATH the app inherited
        let out = std::process::Command::new("which").arg("wickd").output().ok()?;
        if !out.status.success() {
            return None;
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if path.is_empty() { None } else { Some(path.into()) }
    })
}

/// Start a `wickd watch` process from the UI (AGT-652 boundary, deliberately
/// relaxed one notch): the app may LAUNCH the CLI daemon, but the trust
/// ladder is capped at `--semi-auto` — arming autonomous execution (`--auto`)
/// remains a deliberate CLI act, as does approval. Every argument is
/// validated against an allowlist/shape and passed as a discrete argv entry
/// (no shell interpretation).
///
/// The spawned process is detached (survives app quit) but NOT
/// launchd-supervised: it dies at logout and does not auto-restart. The UI
/// surfaces that caveat.
#[tauri::command]
pub async fn start_watcher(
    strategy: String,
    instruments: Vec<String>,
    granularity: String,
    semi_auto: bool,
    units: Option<i64>,
    env: String,
) -> Result<u32, String> {
    // --- validate ---
    let strategy = strategy.trim().to_string();
    let strategy_shape_ok = !strategy.is_empty()
        && strategy.len() <= 64
        && strategy
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !strategy_shape_ok {
        return Err("Invalid strategy name".into());
    }
    if !BUILTIN_STRATEGIES.contains(&strategy.as_str()) {
        let store_path = wickd_core::paths::wickd_data_home()
            .map_err(|e| e.to_string())?
            .join("strategies")
            .join(format!("{strategy}.rhai"));
        if !store_path.is_file() {
            return Err(format!(
                "Strategy '{strategy}' not found in the store (~/.wickd/strategies)"
            ));
        }
    }

    if instruments.is_empty() || instruments.len() > 10 {
        return Err("Select between 1 and 10 instruments".into());
    }
    let instrument_ok = |i: &str| {
        let parts: Vec<&str> = i.split('_').collect();
        parts.len() == 2
            && parts.iter().all(|p| {
                (2..=5).contains(&p.len())
                    && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            })
    };
    if !instruments.iter().all(|i| instrument_ok(i)) {
        return Err("Invalid instrument format (expected e.g. EUR_USD)".into());
    }

    if !UI_WATCH_GRANULARITIES.contains(&granularity.as_str()) {
        return Err(format!("Unsupported granularity '{granularity}'"));
    }

    if let Some(u) = units {
        if !(1..=1_000_000).contains(&u) {
            return Err("Units must be between 1 and 1,000,000".into());
        }
    }

    if env != "practice" && env != "live" {
        return Err("Environment must be 'practice' or 'live'".into());
    }

    // --- spawn ---
    let bin = find_wickd_binary()
        .ok_or("wickd CLI not found — install it (cargo install) or start the watcher from a terminal")?;

    let log_dir = wickd_core::paths::wickd_data_home()
        .map_err(|e| e.to_string())?
        .join("logs");
    std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    let log_path = log_dir.join(format!(
        "watch.ui-{}-{}.log",
        strategy,
        granularity.to_lowercase()
    ));
    let log = std::fs::File::options()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| e.to_string())?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;

    let mut cmd = tokio::process::Command::new(&bin);
    cmd.arg("watch")
        .arg(&strategy)
        .arg(instruments.join(","))
        .args(["--granularity", &granularity])
        .args(["--env", &env]);
    if semi_auto {
        cmd.arg("--semi-auto");
    }
    if let Some(u) = units {
        cmd.args(["--units", &u.to_string()]);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(log_err)
        // The watcher outlives the app on purpose; never kill it on drop
        .kill_on_drop(false);

    let mut child = cmd.spawn().map_err(|e| format!("Failed to start wickd watch: {e}"))?;
    let pid = child
        .id()
        .ok_or_else(|| "wickd watch exited before it could be observed".to_string())?;

    // Reap the child when it exits. Without this the Child handle is dropped
    // unawaited and every watcher stopped from the UI lingers as a <defunct>
    // zombie owned by the app until the app quits.
    tauri::async_runtime::spawn(async move {
        let _ = child.wait().await;
    });

    Ok(pid)
}

/// Stop a UI-manageable watcher process (SIGTERM).
///
/// Safety rail: only processes whose binary basename is exactly `wickd` (the
/// installed CLI) can be stopped. Pinned/supervised binaries — e.g. the
/// `~/.wickd/bin/wickd-h021` eval watchers under launchd — are refused, so a
/// long-running eval can't be disturbed from the UI.
#[tauri::command]
pub async fn stop_watcher(pid: u32) -> Result<(), String> {
    // Targeted single-pid lookup — a full `ps -axo` table scan is measurably
    // slower and this runs on user clicks.
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output()
        .map_err(|e| e.to_string())?;
    let command = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || command.is_empty() {
        return Err(format!("No running wickd watch process with pid {pid}"));
    }
    let mut parts = command.split_whitespace();
    let bin = parts.next().unwrap_or("");
    let base = std::path::Path::new(bin)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if base != "wickd" || parts.next() != Some("watch") {
        return Err(format!(
            "Refusing to stop pid {pid} ('{base}') — only plain `wickd watch` processes may be stopped from the app; pinned/supervised watchers are managed outside it"
        ));
    }
    let status = std::process::Command::new("kill")
        .arg(pid.to_string())
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("Failed to signal pid {pid}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The ps-parsing tests moved to wickd_core::watchers with the code
    // (AGT-652-style lift so the CLI's feed producer shares the scan).

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
            Some("h004".to_string()),
            Some("H4".to_string()),
        );
        alert_queue::append_at(&queue_path, &alert).unwrap();
        let listed = alert_queue::list_at(&queue_path).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, alert.id);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
