//! The app as a reader of the CLI's multi-account performance glance.
//!
//! The desktop app's own credential vault is single-account (one API key, one
//! account id, in `~/.wickd/app.db`). The *CLI* is the multi-account side:
//! `~/.wickd/config.json` holds every named account (`h004`, `tf-m1`, …) and
//! the keys live in the OS keychain. Rather than duplicate that resolution here
//! — `vault_store` lives in the `wickd` binary crate, not importable from the
//! app — this shells out to `wickd trade glance`, the same trust boundary
//! `feed_ask` uses: the CLI owns credentials and OANDA, the app only renders.
//!
//! Unlike the feed/calendar readers this one is NOT offline: it hits OANDA
//! through the CLI and takes ~5s for a full account fan-out. So it is cached
//! with a TTL and the UI is expected to render the last known value while a
//! refresh runs — never to block a panel on it. It is never on the boot path.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::daemon::find_wickd_binary;

/// How long a fetched glance stays fresh. The underlying numbers move only when
/// a trade closes, so a minute of staleness is invisible in practice and keeps
/// a re-rendering panel from re-hitting OANDA.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// Hard ceiling on the CLI call. The fan-out is one round trip per account in
/// parallel (~5s observed for 6 accounts); 90s only trips if OANDA is hanging.
const GLANCE_TIMEOUT: Duration = Duration::from_secs(90);

/// One account's row. Every metric is optional because a row whose fetch failed
/// carries only `account`/`names`/`error` — one revoked key must not blank the
/// whole panel, so failures are per-row, not per-request.
///
/// Money crosses as strings (exact decimals, never lossy floats) — same
/// convention as the CLI's audit ledger and backtest metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountGlance {
    /// Primary display name (the informative one when an account is aliased).
    pub account: String,
    /// Every configured name resolving to this OANDA account, primary first.
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub nav: Option<String>,
    #[serde(default)]
    pub balance: Option<String>,
    #[serde(default)]
    pub unrealized_pl: Option<String>,
    #[serde(default)]
    pub open_trade_count: Option<i32>,
    /// Realized P&L summed over the window.
    #[serde(default)]
    pub realized: Option<String>,
    #[serde(default)]
    pub trades: Option<u64>,
    #[serde(default)]
    pub wins: Option<u64>,
    #[serde(default)]
    pub losses: Option<u64>,
    /// Null when nothing was decided in the window — render "—", not 0%.
    #[serde(default)]
    pub win_rate: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountsGlance {
    pub environment: String,
    pub days: u32,
    /// Start of the window (RFC3339).
    pub since: String,
    /// When the CLI produced these numbers (RFC3339) — the UI shows this as the
    /// as-of stamp so a cached render never looks live when it isn't.
    pub generated_at: String,
    #[serde(default)]
    pub accounts: Vec<AccountGlance>,
}

struct Cached {
    key: (String, u32),
    value: AccountsGlance,
    fetched: Instant,
}

/// Serialized so two panels mounting at once produce one CLI call, not two.
static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<Cached>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Rolling-window performance for every account configured in `env`.
///
/// `refresh: true` bypasses the TTL (the panel's manual refresh button).
#[tauri::command]
pub async fn accounts_glance(
    days: Option<u32>,
    env: Option<String>,
    refresh: Option<bool>,
) -> Result<AccountsGlance, String> {
    let days = days.unwrap_or(7).clamp(1, 365);
    let env = match env.as_deref().unwrap_or("practice") {
        // Allowlist, not passthrough: this string becomes a CLI argument.
        e @ ("practice" | "live") => e.to_string(),
        other => return Err(format!("unknown environment '{other}'")),
    };
    let key = (env.clone(), days);

    // Hold the lock across the fetch so concurrent callers coalesce onto one
    // CLI run rather than each spawning their own.
    let mut guard = cache().lock().await;
    if !refresh.unwrap_or(false) {
        if let Some(c) = guard.as_ref() {
            if c.key == key && c.fetched.elapsed() < CACHE_TTL {
                return Ok(c.value.clone());
            }
        }
    }

    let wickd = find_wickd_binary().ok_or_else(|| {
        "wickd CLI not found — install it (cargo install) to see account performance".to_string()
    })?;

    let output = tokio::time::timeout(
        GLANCE_TIMEOUT,
        tokio::process::Command::new(&wickd)
            .args(["trade", "glance", "--env", &env, "--days", &days.to_string()])
            .output(),
    )
    .await
    .map_err(|_| "account fetch timed out".to_string())?
    .map_err(|e| format!("running wickd trade glance: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|_| {
        // The CLI emits JSON on both paths; anything else means it never ran.
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() { stdout } else { stderr };
        format!("unexpected wickd output: {}", detail.chars().take(200).collect::<String>())
    })?;

    if let Some(err) = value.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        return Err(msg.to_string());
    }

    let glance: AccountsGlance =
        serde_json::from_value(value).map_err(|e| format!("unexpected glance shape: {e}"))?;

    *guard = Some(Cached { key, value: glance.clone(), fetched: Instant::now() });
    Ok(glance)
}
